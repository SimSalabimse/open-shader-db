// SPDX-License-Identifier: MIT

import { TURNSTILE_TIMEOUT_MS, TURNSTILE_TOKEN_MAX_CHARS } from "./limits.js";
import type { InjectedFetch, TurnstileClient } from "./types.js";

export interface TurnstileOptions {
  readonly secret: string;
  readonly siteverifyUrl: string;
  readonly fetchImpl: InjectedFetch;
  readonly timeoutMs?: number;
  readonly expectedHostname?: string;
  readonly expectedAction?: string;
  readonly schedule?: typeof setTimeout;
}

export function createTurnstileClient(
  secretOrOptions: string | TurnstileOptions,
  siteverifyUrl?: string,
  fetchImpl?: InjectedFetch,
): TurnstileClient {
  const options = normalizeOptions(secretOrOptions, siteverifyUrl, fetchImpl);
  return {
    async verify(token: string): Promise<boolean> {
      if (token.length === 0 || token.length > TURNSTILE_TOKEN_MAX_CHARS) {
        return false;
      }
      const body = `secret=${encodeURIComponent(options.secret)}&response=${encodeURIComponent(token)}`;
      const timeoutMs = options.timeoutMs ?? TURNSTILE_TIMEOUT_MS;
      const schedule = options.schedule ?? setTimeout;
      const controller = new AbortController();
      const timer = schedule(() => controller.abort(), timeoutMs);
      try {
        const response = await raceDeadline(
          options.fetchImpl(options.siteverifyUrl, {
            method: "POST",
            headers: { "content-type": "application/x-www-form-urlencoded" },
            body,
            signal: controller.signal,
          }),
          timeoutMs,
          schedule,
        );
        if (!response.ok) {
          return false;
        }
        let json: unknown;
        try {
          json = await raceDeadline(response.json(), timeoutMs, schedule);
        } catch {
          return false;
        }
        return siteverifySuccess(json, options.expectedHostname, options.expectedAction);
      } catch {
        return false;
      } finally {
        clearTimeout(timer);
      }
    },
  };
}

export function raceDeadline<T>(
  work: Promise<T>,
  timeoutMs: number,
  schedule: typeof setTimeout = setTimeout,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<T>((_, reject) => {
    timer = schedule(() => reject(new Error("turnstile-deadline")), timeoutMs);
  });
  return Promise.race([work, timeout]).finally(() => {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
  });
}

function normalizeOptions(
  secretOrOptions: string | TurnstileOptions,
  siteverifyUrl?: string,
  fetchImpl?: InjectedFetch,
): TurnstileOptions {
  if (typeof secretOrOptions === "object") {
    return secretOrOptions;
  }
  if (siteverifyUrl === undefined || fetchImpl === undefined) {
    throw new Error("turnstile siteverify url and fetch are required");
  }
  return { secret: secretOrOptions, siteverifyUrl, fetchImpl };
}

function siteverifySuccess(
  json: unknown,
  expectedHostname: string | undefined,
  expectedAction: string | undefined,
): boolean {
  if (typeof json !== "object" || json === null || Array.isArray(json)) {
    return false;
  }
  const record = json as Record<string, unknown>;
  if (record.success !== true) {
    return false;
  }
  if (expectedHostname !== undefined && expectedHostname.length > 0) {
    if (record.hostname !== expectedHostname) {
      return false;
    }
  }
  if (expectedAction !== undefined && expectedAction.length > 0) {
    if (record.action !== expectedAction) {
      return false;
    }
  }
  return true;
}
