// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { parseManifest } from "../src/parse.js";
import {
  listJsonFiles,
  repoRoot,
  type VectorFile,
  vectorPayload,
} from "./helpers.js";

interface VectorIndex {
  readonly count: number;
  readonly vectors: readonly {
    readonly path: string;
    readonly verdict: "accept" | "reject";
    readonly error_code: string | null;
  }[];
}

function loadIndex(): VectorIndex {
  const text = readFileSync(path.join(repoRoot(), "schemas", "vectors", "index.json"), "utf8");
  return JSON.parse(text) as VectorIndex;
}

test("every golden vector matches the Worker parser and the committed index", () => {
  const index = loadIndex();
  const files = listJsonFiles("schemas/vectors").filter(
    (file) => path.basename(file) !== "index.json",
  );
  const relative = files.map((file) =>
    path.relative(path.join(repoRoot(), "schemas", "vectors"), file).replaceAll("\\", "/"),
  );
  assert.equal(index.count, index.vectors.length, "index count must match the vector list");
  assert.deepEqual(
    relative,
    index.vectors.map((entry) => entry.path),
    "on-disk vectors must match schemas/vectors/index.json exactly",
  );
  for (const [i, file] of files.entries()) {
    const listed = index.vectors[i];
    assert.ok(listed);
    const value = JSON.parse(readFileSync(file, "utf8")) as VectorFile;
    assert.equal(value.verdict, listed.verdict, `${listed.path} verdict`);
    assert.equal(value.error_code, listed.error_code, `${listed.path} error_code`);
    const bytes = vectorPayload(value);
    const parsed = parseManifest(bytes);
    const label = listed.path;
    if (value.verdict === "accept") {
      assert.equal(value.error_code, null, `${label} accept must use null error_code`);
      assert.equal(parsed.ok, true, `${label} should accept`);
      if (parsed.ok && value.body !== undefined) {
        assert.deepEqual(parsed.storedBytes, bytes, `${label} body must be canonical`);
      }
    } else if (value.verdict === "reject") {
      assert.ok(value.error_code, `${label} reject needs error_code`);
      assert.equal(parsed.ok, false, `${label} should reject`);
      if (!parsed.ok) {
        assert.equal(parsed.code, value.error_code, `${label} code`);
      }
    } else {
      throw new Error(`${label} unknown verdict ${String(value.verdict)}`);
    }
  }
});
