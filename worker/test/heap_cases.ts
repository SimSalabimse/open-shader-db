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

function sampleMemory(): { heapUsed: number; arrayBuffers: number; rss: number } {
  const mem = process.memoryUsage();
  return { heapUsed: mem.heapUsed, arrayBuffers: mem.arrayBuffers, rss: mem.rss };
}

async function runCase(name: string, body: Uint8Array, expectedStatus: number): Promise<void> {
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
  const response = await handleUpload(uploadRequest(body), testDeps());
  const after = sampleMemory();
  clearInterval(timer);
  peakHeap = Math.max(peakHeap, after.heapUsed);
  peakExternal = Math.max(peakExternal, after.heapUsed + after.arrayBuffers);
  peakRss = Math.max(peakRss, after.rss);
  assert.equal(response.status, expectedStatus, name);
  const heapMb = (peakHeap / (1024 * 1024)).toFixed(1);
  const totalMb = (peakExternal / (1024 * 1024)).toFixed(1);
  const rssMb = (peakRss / (1024 * 1024)).toFixed(1);
  console.log(
    `HEAP_CASE ${name} status=${response.status} peak_heap_mb=${heapMb} peak_heap_plus_ab_mb=${totalMb} peak_rss_mb=${rssMb} bytes=${body.byteLength}`,
  );
}

const CASES = [
  "max_valid",
  "many_objects",
  "empty_arrays",
  "deep_nesting",
  "huge_string",
  "number_arrays",
] as const;

async function main(): Promise<void> {
  const body = maxValidManifest(WIRE_MAX_JSON_BYTES);
  assert.ok(body.byteLength <= WIRE_MAX_JSON_BYTES);
  assert.ok(body.byteLength > WIRE_MAX_JSON_BYTES - 200);
  await runCase("max_valid", body, 200);
  await runCase("many_objects", fillPattern('{"x":[', "{},", "{}]}", WIRE_MAX_JSON_BYTES), 400);
  await runCase("empty_arrays", fillPattern('{"x":[', "[],", "[]]}", WIRE_MAX_JSON_BYTES), 400);
  const depth = 64;
  const nest = `${"[".repeat(depth)}0${"]".repeat(depth)}`;
  await runCase("deep_nesting", fillPattern(nest, " ", "", WIRE_MAX_JSON_BYTES), 400);
  await runCase("huge_string", fillPattern('{"x":"', "a", '"}', WIRE_MAX_JSON_BYTES), 400);
  await runCase("number_arrays", fillPattern('{"x":[', "0,", "1]}", WIRE_MAX_JSON_BYTES), 400);
  console.log(`HEAP_CHILD_OK cases=${CASES.length}`);
}

await main();
