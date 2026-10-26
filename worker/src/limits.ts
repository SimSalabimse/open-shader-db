// SPDX-License-Identifier: MIT

/** Local client JSON cap (64 MiB). The Worker does not parse this size. */
export const LOCAL_MAX_JSON_BYTES = 64 * 1024 * 1024;

/** Wire request-body cap. Enforced before JSON parse. */
export const WIRE_MAX_JSON_BYTES = 8 * 1024 * 1024;

/** Description-hash list cap (`50 000` records × 3 keys). */
export const MAX_DESCRIPTION_HASHES = 150_000;

/** Bytecode-hash list cap (`50 000` records × 8 stages). */
export const MAX_BYTECODE_HASHES = 400_000;

/** `pipeline_hashes` cap. One hash per record (`records.max_records`). */
export const MAX_PIPELINE_HASHES = 50_000;

/** `{` / `[` nesting cap. */
export const MAX_JSON_NESTING = 8;

/** Maximum characters in a JSON number token (`schema_version` is one digit: 1 or 2). */
export const MAX_NUMBER_TOKEN_CHARS = 16;

/** Staging objects per `(game_id, build_id)` prefix. Approximate under races. */
export const PER_BUILD_UPLOAD_CAP = 64;

/** Parse the per-build cap. Bad or missing values fail closed to the default. */
export function parsePerBuildCap(value: string | undefined): number {
  if (value === undefined || value.length === 0) {
    return PER_BUILD_UPLOAD_CAP;
  }
  if (!/^[0-9]+$/.test(value)) {
    return PER_BUILD_UPLOAD_CAP;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || !Number.isSafeInteger(parsed) || parsed < 1) {
    return PER_BUILD_UPLOAD_CAP;
  }
  return parsed;
}

export const SCHEMA_VERSION_V1 = 1;
export const SCHEMA_VERSION_V2 = 2;

export const TURNSTILE_HEADER = "X-Turnstile-Token";

/** Cloudflare documents siteverify tokens as at most 2048 characters. */
export const TURNSTILE_TOKEN_MAX_CHARS = 2048;

/** Siteverify timeout. Failure is closed (no accept on abort). */
export const TURNSTILE_TIMEOUT_MS = 4000;

export const RATE_LIMIT_KEY = "v1-upload";

export const DEFAULT_SITEVERIFY_URL =
  "https://challenges.cloudflare.com/turnstile/v0/siteverify";
