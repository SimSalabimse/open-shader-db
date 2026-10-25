// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { canonicalize } from "../src/canonical.js";
import { parseManifest } from "../src/parse.js";
import { listJsonFiles, type VectorFile, vectorPayload } from "./helpers.js";

test("every golden vector matches the Worker parser", () => {
  const files = listJsonFiles("schemas/vectors");
  assert.ok(files.length >= 16, `expected a full set, found ${files.length}`);
  for (const file of files) {
    const value = JSON.parse(readFileSync(file, "utf8")) as VectorFile;
    const bytes = vectorPayload(value);
    const parsed = parseManifest(bytes);
    const label = path.basename(file);
    if (value.verdict === "accept") {
      assert.equal(value.error_code, null, `${label} accept must use null error_code`);
      assert.equal(parsed.ok, true, `${label} should accept`);
      if (parsed.ok && value.body !== undefined) {
        const canonical = canonicalize(parsed.manifest);
        assert.deepEqual(canonical, bytes, `${label} body must be canonical`);
      }
    } else {
      assert.ok(value.error_code, `${label} reject needs error_code`);
      assert.equal(parsed.ok, false, `${label} should reject`);
      if (!parsed.ok) {
        assert.equal(parsed.code, value.error_code, `${label} code`);
      }
    }
  }
});
