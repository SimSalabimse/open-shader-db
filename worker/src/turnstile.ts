// SPDX-License-Identifier: MIT

import type { InjectedFetch, TurnstileClient } from "./types.js";

export function createTurnstileClient(
  secret: string,
  siteverifyUrl: string,
  fetchImpl: InjectedFetch,
): TurnstileClient {
  return {
    async verify(token: string): Promise<boolean> {
      if (token.length === 0) {
        return false;
      }
      const body = `secret=${encodeURIComponent(secret)}&response=${encodeURIComponent(token)}`;
      try {
        const response = await fetchImpl(siteverifyUrl, {
          method: "POST",
          headers: { "content-type": "application/x-www-form-urlencoded" },
          body,
        });
        if (!response.ok) {
          return false;
        }
        const json: unknown = await response.json();
        return siteverifySuccess(json);
      } catch {
        return false;
      }
    },
  };
}

function siteverifySuccess(json: unknown): boolean {
  if (typeof json !== "object" || json === null) {
    return false;
  }
  if (!("success" in json)) {
    return false;
  }
  return json.success === true;
}
