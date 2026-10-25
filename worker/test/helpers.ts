// SPDX-License-Identifier: MIT

import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { HandlerDeps } from "../src/types.js";
import { MemoryStore } from "../src/storage.js";

export function repoRoot(): string {
  let dir = path.dirname(fileURLToPath(import.meta.url));
  for (let i = 0; i < 8; i += 1) {
    if (existsSync(path.join(dir, "schemas", "vectors"))) {
      return dir;
    }
    dir = path.dirname(dir);
  }
  throw new Error("schemas/vectors not found from test location");
}

export function readUtf8(rel: string): string {
  return readFileSync(path.join(repoRoot(), rel), "utf8").replace(/\r\n/g, "\n");
}

export function listJsonFiles(rel: string): string[] {
  const dir = path.join(repoRoot(), rel);
  const out: string[] = [];
  visit(dir, out);
  out.sort();
  return out;
}

function visit(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      visit(full, out);
      continue;
    }
    if (entry.name.endsWith(".json")) {
      out.push(full);
    }
  }
}

export function decodeHex(text: string): Uint8Array {
  if (text.length % 2 !== 0) {
    throw new Error("odd hex length");
  }
  const out = new Uint8Array(text.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export interface VectorFile {
  readonly verdict: "accept" | "reject";
  readonly error_code: string | null;
  readonly body?: unknown;
  readonly body_text?: string;
  readonly body_hex?: string;
}

export function vectorPayload(value: VectorFile): Uint8Array {
  const kinds = [value.body, value.body_text, value.body_hex].filter((v) => v !== undefined);
  if (kinds.length !== 1) {
    throw new Error("vector must have exactly one payload field");
  }
  if (value.body_hex !== undefined) {
    return decodeHex(value.body_hex);
  }
  if (value.body_text !== undefined) {
    return new TextEncoder().encode(value.body_text);
  }
  return new TextEncoder().encode(stableStringify(value.body));
}

function stableStringify(value: unknown): string {
  return JSON.stringify(sortJson(value));
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJson);
  }
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(record).sort()) {
      const child = record[key];
      if (child !== undefined) {
        out[key] = sortJson(child);
      }
    }
    return out;
  }
  return value;
}

export function testDeps(overrides: Partial<HandlerDeps> = {}): HandlerDeps {
  return {
    store: new MemoryStore(),
    rate: { allow: async () => true },
    turnstile: { verify: async (token) => token === "ok" },
    perBuildCap: 64,
    wireMaxBytes: 8 * 1024 * 1024,
    ...overrides,
  };
}

export function uploadRequest(
  body: Uint8Array | string,
  headers: Record<string, string> = {},
): Request {
  const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
  return new Request("https://example.invalid/v1/upload", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Turnstile-Token": "ok",
      "Content-Length": String(bytes.byteLength),
      ...headers,
    },
    body: bytes,
  });
}
