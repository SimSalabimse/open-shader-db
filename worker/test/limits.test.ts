// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  LOCAL_MAX_JSON_BYTES,
  MAX_BYTECODE_HASHES,
  MAX_DESCRIPTION_HASHES,
  MAX_JSON_NESTING,
  PER_BUILD_UPLOAD_CAP,
  WIRE_MAX_JSON_BYTES,
} from "../src/limits.js";
import { DEFAULT_PARSE_LIMITS } from "../src/parse.js";
import { readUtf8 } from "./helpers.js";

test("Worker constants match schemas/limits.v1.json", () => {
  const limits = JSON.parse(readUtf8("schemas/limits.v1.json")) as {
    local_max_json_bytes: number;
    wire_max_json_bytes: number;
    max_description_hashes: number;
    max_bytecode_hashes: number;
    max_json_nesting: number;
    per_build_upload_cap: number;
  };
  assert.equal(LOCAL_MAX_JSON_BYTES, limits.local_max_json_bytes);
  assert.equal(WIRE_MAX_JSON_BYTES, limits.wire_max_json_bytes);
  assert.equal(MAX_DESCRIPTION_HASHES, limits.max_description_hashes);
  assert.equal(MAX_BYTECODE_HASHES, limits.max_bytecode_hashes);
  assert.equal(MAX_JSON_NESTING, limits.max_json_nesting);
  assert.equal(PER_BUILD_UPLOAD_CAP, limits.per_build_upload_cap);
  assert.equal(WIRE_MAX_JSON_BYTES, 8 * 1024 * 1024);
  assert.equal(LOCAL_MAX_JSON_BYTES, 64 * 1024 * 1024);
  assert.equal(MAX_DESCRIPTION_HASHES, 150_000);
  assert.equal(MAX_BYTECODE_HASHES, 400_000);
  assert.equal(DEFAULT_PARSE_LIMITS.maxJsonBytes, LOCAL_MAX_JSON_BYTES);
  assert.equal(DEFAULT_PARSE_LIMITS.maxDescriptionHashes, MAX_DESCRIPTION_HASHES);
  assert.equal(DEFAULT_PARSE_LIMITS.maxBytecodeHashes, MAX_BYTECODE_HASHES);
});
