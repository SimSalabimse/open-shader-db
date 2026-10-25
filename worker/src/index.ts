// SPDX-License-Identifier: MIT

import { handleUpload } from "./handler.js";
import {
  DEFAULT_SITEVERIFY_URL,
  PER_BUILD_UPLOAD_CAP,
  WIRE_MAX_JSON_BYTES,
} from "./limits.js";
import { createRateGate } from "./rate_limit.js";
import { R2StagingStore } from "./storage.js";
import { createTurnstileClient } from "./turnstile.js";
import type { WorkerEnv } from "./types.js";

export default {
  async fetch(request: Request, env: WorkerEnv): Promise<Response> {
    const cap = parseCap(env.PER_BUILD_UPLOAD_CAP);
    const siteverify =
      env.TURNSTILE_SITEVERIFY_URL.length > 0
        ? env.TURNSTILE_SITEVERIFY_URL
        : DEFAULT_SITEVERIFY_URL;
    return handleUpload(request, {
      store: new R2StagingStore(env.STAGING_BUCKET),
      rate: createRateGate(env.UPLOAD_RATE_LIMITER),
      turnstile: createTurnstileClient(env.TURNSTILE_SECRET, siteverify, async (input, init) => {
        const response = await fetch(input, init);
        return {
          ok: response.ok,
          json: () => response.json(),
        };
      }),
      perBuildCap: cap,
      wireMaxBytes: WIRE_MAX_JSON_BYTES,
    });
  },
};

function parseCap(value: string | undefined): number {
  if (value === undefined) {
    return PER_BUILD_UPLOAD_CAP;
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    return PER_BUILD_UPLOAD_CAP;
  }
  return parsed;
}
