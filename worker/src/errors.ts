// SPDX-License-Identifier: MIT

export const ERROR_CODES = [
  "METHOD_NOT_ALLOWED",
  "NOT_FOUND",
  "RATE_LIMITED",
  "TURNSTILE_MISSING",
  "TURNSTILE_FAILED",
  "BODY_TOO_LARGE",
  "MANIFEST_TOO_LARGE",
  "BYTE_ORDER_MARK",
  "TRAILING_GARBAGE",
  "INVALID_JSON",
  "NESTING_TOO_DEEP",
  "DUPLICATE_KEY",
  "UNKNOWN_FIELD",
  "MISSING_FIELD",
  "UNSUPPORTED_SCHEMA_VERSION",
  "INVALID_INTEGER",
  "MALFORMED_HEX",
  "WRONG_HEX_LENGTH",
  "UNSORTED_OR_DUPLICATE_HASHES",
  "TOO_MANY_HASHES",
  "INVALID_BUILD_ID",
  "INVALID_TIMESTAMP",
  "INVALID_CLIENT_VERSION",
  "UNKNOWN_OWNERSHIP_KIND",
  "UNKNOWN_OWNERSHIP_REASON",
  "BUILD_CAP_EXCEEDED",
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];

const MESSAGES: Record<ErrorCode, string> = {
  METHOD_NOT_ALLOWED: "method is not allowed",
  NOT_FOUND: "path is not the upload endpoint",
  RATE_LIMITED: "rate limit exceeded",
  TURNSTILE_MISSING: "turnstile token is missing",
  TURNSTILE_FAILED: "turnstile verification failed",
  BODY_TOO_LARGE: "request body exceeds the wire size limit",
  MANIFEST_TOO_LARGE: "upload manifest exceeds size limit",
  BYTE_ORDER_MARK: "upload manifest begins with a UTF-8 byte order mark",
  TRAILING_GARBAGE: "trailing data after upload manifest JSON",
  INVALID_JSON: "truncated or invalid upload manifest JSON",
  NESTING_TOO_DEEP: "JSON nesting exceeds limit of 8",
  DUPLICATE_KEY: "duplicate JSON key",
  UNKNOWN_FIELD: "unknown field in upload manifest",
  MISSING_FIELD: "missing field in upload manifest",
  UNSUPPORTED_SCHEMA_VERSION: "unsupported upload manifest schema_version",
  INVALID_INTEGER: "invalid integer in upload manifest",
  MALFORMED_HEX: "malformed hex in upload manifest",
  WRONG_HEX_LENGTH: "hex value has the wrong length",
  UNSORTED_OR_DUPLICATE_HASHES: "hash list is not sorted unique",
  TOO_MANY_HASHES: "hash count exceeds limit",
  INVALID_BUILD_ID: "invalid build identifier",
  INVALID_TIMESTAMP: "invalid collected_hour timestamp",
  INVALID_CLIENT_VERSION: "invalid client version",
  UNKNOWN_OWNERSHIP_KIND: "unknown ownership proof kind",
  UNKNOWN_OWNERSHIP_REASON: "unknown ownership reason",
  BUILD_CAP_EXCEEDED: "per-build upload cap exceeded",
};

export interface ErrorBody {
  readonly code: ErrorCode;
  readonly message: string;
}

export function errorBody(code: ErrorCode): ErrorBody {
  return { code, message: MESSAGES[code] };
}

export function httpStatus(code: ErrorCode): number {
  switch (code) {
    case "METHOD_NOT_ALLOWED":
      return 405;
    case "NOT_FOUND":
      return 404;
    case "RATE_LIMITED":
    case "BUILD_CAP_EXCEEDED":
      return 429;
    case "TURNSTILE_MISSING":
    case "TURNSTILE_FAILED":
      return 403;
    case "BODY_TOO_LARGE":
    case "MANIFEST_TOO_LARGE":
      return 413;
    default:
      return 400;
  }
}
