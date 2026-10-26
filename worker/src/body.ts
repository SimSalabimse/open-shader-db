// SPDX-License-Identifier: MIT

import type { ErrorCode } from "./errors.js";
import { readAllowedHeader } from "./privacy.js";

export type DeclaredLength =
  | { readonly kind: "missing" }
  | { readonly kind: "ok"; readonly value: number }
  | { readonly kind: "too_large" };

export function declaredContentLength(request: Request): DeclaredLength {
  return parseContentLength(readAllowedHeader(request.headers, "Content-Length"));
}

export function declaredBodyTooLarge(request: Request, maxBytes: number): boolean {
  const declared = declaredContentLength(request);
  if (declared.kind === "too_large") {
    return true;
  }
  return declared.kind === "ok" && declared.value > maxBytes;
}

export async function readBodyCapped(
  request: Request,
  maxBytes: number,
): Promise<{ ok: true; bytes: Uint8Array } | { ok: false; code: ErrorCode }> {
  const declared = declaredContentLength(request);
  if (declared.kind === "too_large" || (declared.kind === "ok" && declared.value > maxBytes)) {
    return { ok: false, code: "BODY_TOO_LARGE" };
  }
  const stream = request.body;
  if (stream === null) {
    return { ok: true, bytes: new Uint8Array() };
  }
  const capacity = declared.kind === "ok" ? declared.value : maxBytes;
  const reader = stream.getReader();
  const buf = new Uint8Array(capacity);
  let offset = 0;
  for (;;) {
    const step = await reader.read();
    if (step.done) {
      break;
    }
    const chunk = step.value;
    if (offset + chunk.byteLength > maxBytes || offset + chunk.byteLength > buf.length) {
      await reader.cancel();
      return { ok: false, code: "BODY_TOO_LARGE" };
    }
    buf.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return { ok: true, bytes: buf.subarray(0, offset) };
}

function parseContentLength(value: string | null): DeclaredLength {
  if (value === null) {
    return { kind: "missing" };
  }
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return { kind: "missing" };
  }
  if (!/^[0-9]+$/.test(trimmed)) {
    return { kind: "too_large" };
  }
  if (trimmed.length > 15) {
    return { kind: "too_large" };
  }
  const parsed = Number(trimmed);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    return { kind: "too_large" };
  }
  return { kind: "ok", value: parsed };
}
