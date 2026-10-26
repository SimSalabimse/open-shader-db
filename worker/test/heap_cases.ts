// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";

import { handleUpload } from "../src/handler.js";
import { WIRE_MAX_JSON_BYTES } from "../src/limits.js";
import { testDeps, uploadRequest } from "./helpers.js";

const HEX = "0123456789abcdef";

function writeAscii(out: Uint8Array, offset: number, text: string): number {
  for (let i = 0; i < text.length; i += 1) {
    out[offset + i] = text.charCodeAt(i);
  }
  return offset + text.length;
}

function writeHex64(out: Uint8Array, offset: number, n: number): number {
  out[offset] = 0x22;
  for (let i = 0; i < 64; i += 1) {
    out[offset + 1 + i] = 0x30;
  }
  let value = n;
  let pos = offset + 64;
  while (value > 0) {
    out[pos] = HEX.charCodeAt(value & 0xf);
    value = Math.floor(value / 16);
    pos -= 1;
  }
  out[offset + 65] = 0x22;
  return offset + 66;
}

function maxValidManifest(target: number): Uint8Array {
  const exe = "0".repeat(62) + "ee";
  const prefix = `{"build_id":{"exe_hash":"${exe}","game_id":"synthetic","version":"1.0.0"},"bytecode_hashes":[`;
  const mid =
    '],"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[';
  const suffix = '],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"},"schema_version":1}';
  const overhead = prefix.length + mid.length + suffix.length;
  const perHash = 67;
  let count = Math.floor((target - overhead) / (2 * perHash));
  while (overhead + 2 * count * perHash - 2 > target && count > 0) {
    count -= 1;
  }
  const size = overhead + 2 * (count * 66 + Math.max(0, count - 1));
  const out = new Uint8Array(size);
  let offset = writeAscii(out, 0, prefix);
  for (let i = 1; i <= count; i += 1) {
    offset = writeHex64(out, offset, i);
    if (i < count) {
      out[offset] = 0x2c;
      offset += 1;
    }
  }
  offset = writeAscii(out, offset, mid);
  for (let i = 1; i <= count; i += 1) {
    offset = writeHex64(out, offset, i);
    if (i < count) {
      out[offset] = 0x2c;
      offset += 1;
    }
  }
  offset = writeAscii(out, offset, suffix);
  assert.equal(offset, size);
  assert.ok(size <= target);
  return out;
}

function prettyMaxValid(target: number): Uint8Array {
  const exe = "0".repeat(62) + "ee";
  const prefix = `{\n  "build_id": {\n    "exe_hash": "${exe}",\n    "game_id": "synthetic",\n    "version": "1.0.0"\n  },\n  "bytecode_hashes": [\n`;
  const mid =
    '\n  ],\n  "client_version": "0.1.0",\n  "collected_hour": "2026-10-25T14:00:00Z",\n  "description_hashes": [\n';
  const suffix =
    '\n  ],\n  "ownership": {\n    "kind": "unchallenged",\n    "reason": "no_server_challenge"\n  },\n  "schema_version": 1\n}';
  const entry = 72;
  const last = 70;
  const overhead = prefix.length + mid.length + suffix.length;
  let count = Math.floor((target - overhead - 2 * last) / (2 * entry)) + 1;
  while (count > 0) {
    const size = overhead + 2 * ((count - 1) * entry + last);
    if (size <= target) {
      const out = new Uint8Array(size);
      let offset = writeAscii(out, 0, prefix);
      for (let i = 1; i <= count; i += 1) {
        offset = writePrettyHash(out, offset, i, i < count);
      }
      offset = writeAscii(out, offset, mid);
      for (let i = 1; i <= count; i += 1) {
        offset = writePrettyHash(out, offset, i, i < count);
      }
      offset = writeAscii(out, offset, suffix);
      assert.equal(offset, size);
      return out;
    }
    count -= 1;
  }
  throw new Error("pretty max-valid does not fit the wire cap");
}

function writePrettyHash(out: Uint8Array, offset: number, n: number, comma: boolean): number {
  offset = writeAscii(out, offset, "    ");
  offset = writeHex64(out, offset, n);
  if (comma) {
    out[offset] = 0x2c;
    offset += 1;
    out[offset] = 0x0a;
    offset += 1;
  }
  return offset;
}

function hugeNumberToken(): Uint8Array {
  const prefix = '{"schema_version":';
  const suffix = "}";
  const out = new Uint8Array(WIRE_MAX_JSON_BYTES);
  let offset = writeAscii(out, 0, prefix);
  const digits = WIRE_MAX_JSON_BYTES - prefix.length - suffix.length;
  out.fill(0x31, offset, offset + digits);
  writeAscii(out, offset + digits, suffix);
  return out;
}

function fillPattern(prefix: string, unit: string, suffix: string, target: number): Uint8Array {
  const room = target - prefix.length - suffix.length;
  const n = Math.max(1, Math.floor(room / unit.length));
  const out = new Uint8Array(prefix.length + n * unit.length + suffix.length);
  let offset = writeAscii(out, 0, prefix);
  const u = new TextEncoder().encode(unit);
  for (let i = 0; i < n; i += 1) {
    out.set(u, offset);
    offset += u.length;
  }
  writeAscii(out, offset, suffix);
  return out;
}

function chunkedRequest(bytes: Uint8Array, chunkSize: number): Request {
  let offset = 0;
  const stream = new ReadableStream<Uint8Array>({
    pull(controller) {
      if (offset >= bytes.length) {
        controller.close();
        return;
      }
      const end = Math.min(offset + chunkSize, bytes.length);
      controller.enqueue(bytes.subarray(offset, end));
      offset = end;
    },
  });
  return new Request("https://example.invalid/v1/upload", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Turnstile-Token": "ok",
      "Content-Length": String(bytes.byteLength),
    },
    body: stream,
    duplex: "half",
  } as RequestInit);
}

function sampleMemory(): { heapUsed: number; arrayBuffers: number; rss: number } {
  const mem = process.memoryUsage();
  return { heapUsed: mem.heapUsed, arrayBuffers: mem.arrayBuffers, rss: mem.rss };
}

async function runCase(
  name: string,
  request: Request,
  expectedStatus: number,
): Promise<void> {
  let peakHeap = 0;
  let peakExternal = 0;
  let peakRss = 0;
  const timer = setInterval(() => {
    const mem = sampleMemory();
    peakHeap = Math.max(peakHeap, mem.heapUsed);
    peakExternal = Math.max(peakExternal, mem.heapUsed + mem.arrayBuffers);
    peakRss = Math.max(peakRss, mem.rss);
  }, 5);
  timer.unref();
  const cpuStart = process.cpuUsage();
  const wallStart = process.hrtime.bigint();
  const response = await handleUpload(request, testDeps());
  const cpu = process.cpuUsage(cpuStart);
  const wallMs = Number(process.hrtime.bigint() - wallStart) / 1e6;
  const after = sampleMemory();
  clearInterval(timer);
  peakHeap = Math.max(peakHeap, after.heapUsed);
  peakExternal = Math.max(peakExternal, after.heapUsed + after.arrayBuffers);
  peakRss = Math.max(peakRss, after.rss);
  assert.equal(response.status, expectedStatus, name);
  let code = "";
  if (response.status !== 200) {
    const err = (await response.json()) as { code?: string };
    code = err.code ?? "";
  }
  const heapMb = (peakHeap / (1024 * 1024)).toFixed(1);
  const totalMb = (peakExternal / (1024 * 1024)).toFixed(1);
  const rssMb = (peakRss / (1024 * 1024)).toFixed(1);
  const cpuMs = ((cpu.user + cpu.system) / 1000).toFixed(1);
  console.log(
    `HEAP_CASE ${name} status=${response.status} code=${code} peak_heap_mb=${heapMb} peak_heap_plus_ab_mb=${totalMb} peak_rss_mb=${rssMb} cpu_ms=${cpuMs} wall_ms=${wallMs.toFixed(1)} bytes=${request.headers.get("content-length") ?? "?"}`,
  );
}

const CASES = [
  "max_valid",
  "max_valid_pretty",
  "max_valid_chunk_1",
  "max_valid_chunk_16",
  "max_valid_pretty_chunk_1",
  "max_valid_pretty_chunk_16",
  "in_field_braces",
  "in_field_huge",
  "deep_nesting",
  "huge_number_token",
] as const;

async function main(): Promise<void> {
  const compact = maxValidManifest(WIRE_MAX_JSON_BYTES);
  assert.ok(compact.byteLength <= WIRE_MAX_JSON_BYTES);
  assert.ok(compact.byteLength > WIRE_MAX_JSON_BYTES - 200);
  const pretty = prettyMaxValid(WIRE_MAX_JSON_BYTES);
  assert.ok(pretty.byteLength <= WIRE_MAX_JSON_BYTES);
  assert.ok(pretty.byteLength > WIRE_MAX_JSON_BYTES - 200);
  assert.equal(pretty[0], 0x7b);
  assert.equal(pretty[1], 0x0a);
  await runCase("max_valid", uploadRequest(compact), 200);
  await runCase("max_valid_pretty", uploadRequest(pretty), 200);
  await runCase("max_valid_chunk_1", chunkedRequest(compact, 1), 200);
  await runCase("max_valid_chunk_16", chunkedRequest(compact, 16), 200);
  await runCase("max_valid_pretty_chunk_1", chunkedRequest(pretty, 1), 200);
  await runCase("max_valid_pretty_chunk_16", chunkedRequest(pretty, 16), 200);
  await runCase(
    "in_field_braces",
    uploadRequest(fillPattern('{"x":"', "{", '"}', WIRE_MAX_JSON_BYTES)),
    400,
  );
  await runCase(
    "in_field_huge",
    uploadRequest(fillPattern('{"description_hashes":["', "a", '"]}', WIRE_MAX_JSON_BYTES)),
    400,
  );
  const depth = 64;
  const nest = `${"[".repeat(depth)}0${"]".repeat(depth)}`;
  await runCase("deep_nesting", uploadRequest(fillPattern(nest, " ", "", WIRE_MAX_JSON_BYTES)), 400);
  await runCase("huge_number_token", uploadRequest(hugeNumberToken()), 400);
  console.log(`HEAP_CHILD_OK cases=${CASES.length}`);
}

await main();
