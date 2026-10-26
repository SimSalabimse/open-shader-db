// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
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
    readonly stored?: string;
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
  const storedDir = path.join(repoRoot(), "schemas", "vectors", "stored");
  const listedStored = index.vectors
    .filter((entry) => entry.verdict === "accept")
    .map((entry) => {
      assert.ok(entry.stored, `${entry.path} accept entry must list stored bytes`);
      return entry.stored;
    });
  const onDiskStored: string[] = [];
  const walkStored = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walkStored(full);
        continue;
      }
      if (entry.name.endsWith(".json")) {
        onDiskStored.push(path.relative(path.join(repoRoot(), "schemas", "vectors"), full).replaceAll("\\", "/"));
      }
    }
  };
  walkStored(storedDir);
  onDiskStored.sort();
  assert.deepEqual(
    onDiskStored,
    listedStored,
    "on-disk stored-bytes files must match accept index entries exactly",
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
      assert.ok(listed.stored, `${label} accept must list stored bytes`);
      const expected = new Uint8Array(
        readFileSync(path.join(repoRoot(), "schemas", "vectors", listed.stored)),
      );
      if (parsed.ok) {
        if (value.body !== undefined) {
          assert.deepEqual(parsed.storedBytes, bytes, `${label} body must be canonical`);
        }
        assert.deepEqual(parsed.storedBytes, expected, `${label} stored bytes`);
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
