// SPDX-License-Identifier: MIT

/** Local client JSON cap (64 MiB). The Worker does not parse this size. */
export const LOCAL_MAX_JSON_BYTES = 64 * 1024 * 1024;

/** Wire request-body cap. Enforced before JSON parse. */
export const WIRE_MAX_JSON_BYTES = 8 * 1024 * 1024;

/** Description-hash list cap (`50 000` records × 3 keys). */
export const MAX_DESCRIPTION_HASHES = 150_000;

/** Bytecode-hash list cap (`50 000` records × 8 stages). */
export const MAX_BYTECODE_HASHES = 400_000;

/** `{` / `[` nesting cap. */
export const MAX_JSON_NESTING = 8;

/** Staging objects per `(game_id, build_id)` prefix. Approximate under races. */
export const PER_BUILD_UPLOAD_CAP = 64;

export const SCHEMA_VERSION = 1;

export const TURNSTILE_HEADER = "X-Turnstile-Token";

export const RATE_LIMIT_KEY = "v1-upload";

export const DEFAULT_SITEVERIFY_URL =
  "https://challenges.cloudflare.com/turnstile/v0/siteverify";
