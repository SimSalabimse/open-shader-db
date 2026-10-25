// SPDX-License-Identifier: MIT

export const ALLOWED_REQUEST_HEADERS = [
  "content-length",
  "content-type",
  "x-turnstile-token",
] as const;

const ALLOWED = new Set<string>(ALLOWED_REQUEST_HEADERS);

export type AllowedHeaderName = "Content-Length" | "Content-Type" | "X-Turnstile-Token";

/** Read one allowlisted header. Other names are refused at compile time. */
export function readAllowedHeader(headers: Headers, name: AllowedHeaderName): string | null {
  if (!ALLOWED.has(name.toLowerCase())) {
    return null;
  }
  return headers.get(name);
}

export function isJsonContentType(value: string | null): boolean {
  if (value === null) {
    return false;
  }
  const media = value.split(";", 1)[0]?.trim().toLowerCase();
  return media === "application/json";
}
