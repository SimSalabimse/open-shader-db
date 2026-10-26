// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import {
  LOCAL_MAX_JSON_BYTES,
  MAX_BYTECODE_HASHES,
  MAX_DESCRIPTION_HASHES,
  MAX_JSON_NESTING,
  MAX_NUMBER_TOKEN_CHARS,
  PER_BUILD_UPLOAD_CAP,
  parsePerBuildCap,
  WIRE_MAX_JSON_BYTES,
} from "../src/limits.js";
import { DEFAULT_PARSE_LIMITS } from "../src/parse.js";
import { readUtf8, repoRoot } from "./helpers.js";

test("Worker constants match schemas/limits.v1.json", () => {
  const limits = JSON.parse(readUtf8("schemas/limits.v1.json")) as {
    local_max_json_bytes: number;
    wire_max_json_bytes: number;
    max_description_hashes: number;
    max_bytecode_hashes: number;
    max_json_nesting: number;
    max_number_token_chars: number;
    per_build_upload_cap: number;
  };
  assert.equal(LOCAL_MAX_JSON_BYTES, limits.local_max_json_bytes);
  assert.equal(WIRE_MAX_JSON_BYTES, limits.wire_max_json_bytes);
  assert.equal(MAX_DESCRIPTION_HASHES, limits.max_description_hashes);
  assert.equal(MAX_BYTECODE_HASHES, limits.max_bytecode_hashes);
  assert.equal(MAX_JSON_NESTING, limits.max_json_nesting);
  assert.equal(MAX_NUMBER_TOKEN_CHARS, limits.max_number_token_chars);
  assert.equal(PER_BUILD_UPLOAD_CAP, limits.per_build_upload_cap);
  assert.equal(WIRE_MAX_JSON_BYTES, 8 * 1024 * 1024);
  assert.equal(LOCAL_MAX_JSON_BYTES, 64 * 1024 * 1024);
  assert.equal(MAX_DESCRIPTION_HASHES, 150_000);
  assert.equal(MAX_BYTECODE_HASHES, 400_000);
  assert.equal(DEFAULT_PARSE_LIMITS.maxJsonBytes, LOCAL_MAX_JSON_BYTES);
  assert.equal(DEFAULT_PARSE_LIMITS.maxDescriptionHashes, MAX_DESCRIPTION_HASHES);
  assert.equal(DEFAULT_PARSE_LIMITS.maxBytecodeHashes, MAX_BYTECODE_HASHES);
});

test("per-build cap parser fails closed on bad or missing values", () => {
  assert.equal(parsePerBuildCap(undefined), PER_BUILD_UPLOAD_CAP);
  assert.equal(parsePerBuildCap(""), PER_BUILD_UPLOAD_CAP);
  assert.equal(parsePerBuildCap("abc"), PER_BUILD_UPLOAD_CAP);
  assert.equal(parsePerBuildCap("0"), PER_BUILD_UPLOAD_CAP);
  assert.equal(parsePerBuildCap("-1"), PER_BUILD_UPLOAD_CAP);
  assert.equal(parsePerBuildCap("1e30"), PER_BUILD_UPLOAD_CAP);
  assert.equal(parsePerBuildCap("NaN"), PER_BUILD_UPLOAD_CAP);
  for (const value of [undefined, "", "abc", "0", "-1", "1e30", "NaN"]) {
    const parsed = parsePerBuildCap(value);
    assert.equal(Number.isFinite(parsed), true, String(value));
    assert.equal(Number.isSafeInteger(parsed), true, String(value));
    assert.ok(parsed >= 1, String(value));
  }
  assert.equal(parsePerBuildCap("32"), 32);
  assert.equal(parsePerBuildCap("64"), 64);
});

test("every number-token length check uses the strict 16-character bound", () => {
  const source = readFileSync(path.join(repoRoot(), "worker", "src", "parse.ts"), "utf8");
  const start = source.indexOf("function parseNumber");
  const end = source.indexOf("function parseHex", start);
  assert.ok(start >= 0 && end > start);
  const fn = source.slice(start, end);
  const strict = fn.split("cur.pos - start > MAX_NUMBER_TOKEN_CHARS").length - 1;
  const loose = fn.split("cur.pos - start >= MAX_NUMBER_TOKEN_CHARS").length - 1;
  assert.equal(strict, 4, "integer, fraction, exponent, and final checks");
  assert.equal(loose, 0);
});
