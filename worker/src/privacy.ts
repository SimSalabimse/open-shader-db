// SPDX-License-Identifier: MIT

const ALLOWED = new Set(["content-length", "x-turnstile-token"]);

/** Read one allowlisted header. Other names are refused at compile time. */
export function readAllowedHeader(
  headers: Headers,
  name: "Content-Length" | "X-Turnstile-Token",
): string | null {
  if (!ALLOWED.has(name.toLowerCase())) {
    return null;
  }
  return headers.get(name);
}
