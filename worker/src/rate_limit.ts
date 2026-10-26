// SPDX-License-Identifier: MIT

import { RATE_LIMIT_KEY } from "./limits.js";
import type { RateGate, RateLimitBinding } from "./types.js";

export function createRateGate(binding: RateLimitBinding): RateGate {
  return {
    async allow(): Promise<boolean> {
      const result = await binding.limit({ key: RATE_LIMIT_KEY });
      return result.success;
    },
  };
}
