// SPDX-License-Identifier: MIT

import type { ErrorCode } from "./errors.js";
import { readAllowedHeader } from "./privacy.js";

export async function readBodyCapped(
  request: Request,
  maxBytes: number,
): Promise<{ ok: true; bytes: Uint8Array } | { ok: false; code: ErrorCode }> {
  const declared = parseContentLength(readAllowedHeader(request.headers, "Content-Length"));
  if (declared !== null && declared > maxBytes) {
    return { ok: false, code: "BODY_TOO_LARGE" };
  }
  const stream = request.body;
  if (stream === null) {
    return { ok: true, bytes: new Uint8Array() };
  }
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const step = await reader.read();
    if (step.done) {
      break;
    }
    const chunk = step.value;
    total += chunk.byteLength;
    if (total > maxBytes) {
      await reader.cancel();
      return { ok: false, code: "BODY_TOO_LARGE" };
    }
    chunks.push(chunk);
  }
  return { ok: true, bytes: concat(chunks, total) };
}

function parseContentLength(value: string | null): number | null {
  if (value === null) {
    return null;
  }
  const trimmed = value.trim();
  if (trimmed.length === 0 || !/^[0-9]+$/.test(trimmed)) {
    return null;
  }
  const parsed = Number(trimmed);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    return null;
  }
  return parsed;
}

function concat(chunks: readonly Uint8Array[], total: number): Uint8Array {
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return out;
}
