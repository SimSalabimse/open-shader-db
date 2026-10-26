// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { parseManifest } from "../src/parse.js";
import { repoRoot, type VectorFile, vectorPayload } from "./helpers.js";

const SKIP_CASES = [
  "invalid/v2-pipeline-hashes-number-before-version.json",
  "invalid/v2-pipeline-hashes-two-numbers-before-version.json",
  "invalid/v2-pipeline-hashes-fraction-before-version.json",
  "invalid/v2-pipeline-hashes-exponent-before-version.json",
  "invalid/v2-pipeline-hashes-object-keys-before-version.json",
  "invalid/v2-pipeline-hashes-leading-zero-before-version.json",
  "invalid/v2-pipeline-hashes-object-before-version.json",
  "invalid/v2-pipeline-hashes-array-before-version.json",
  "invalid/v2-pipeline-hashes-literal-before-version.json",
  "invalid/v2-pipeline-hashes-bad-literal-before-version.json",
  "invalid/v2-pipeline-hashes-bad-escape-before-version.json",
  "invalid/v2-pipeline-hashes-nested-escape-before-version.json",
  "invalid/v2-pipeline-hashes-unterminated-before-version.json",
  "valid-text/v2-pipeline-hash-escape-before-version.json",
] as const;

function loadVector(rel: string): { bytes: Uint8Array; file: VectorFile; stored?: Uint8Array } {
  const full = path.join(repoRoot(), "schemas", "vectors", rel);
  const file = JSON.parse(readFileSync(full, "utf8")) as VectorFile;
  const bytes = vectorPayload(file);
  if (file.verdict === "accept") {
    const index = JSON.parse(
      readFileSync(path.join(repoRoot(), "schemas", "vectors", "index.json"), "utf8"),
    ) as { vectors: { path: string; stored?: string }[] };
    const listed = index.vectors.find((entry) => entry.path === rel);
    assert.ok(listed?.stored, `${rel} stored path`);
    const stored = new Uint8Array(
      readFileSync(path.join(repoRoot(), "schemas", "vectors", listed.stored)),
    );
    return { bytes, file, stored };
  }
  return { bytes, file };
}

// Timeout kills a mutant that drops `cur.consume()` in skipNumber's digit,
// fraction, or exponent loop (parse.ts). Those loops run only when a
// `pipeline_hashes` value is skipped before `schema_version`.
test(
  "pipeline_hashes before schema_version skips numbers, nests, literals, and escapes",
  { timeout: 2_000 },
  () => {
    for (const rel of SKIP_CASES) {
      const { bytes, file, stored } = loadVector(rel);
      const parsed = parseManifest(bytes);
      if (file.verdict === "reject") {
        assert.equal(parsed.ok, false, rel);
        if (!parsed.ok) {
          assert.equal(parsed.code, "INVALID_JSON", rel);
        }
        continue;
      }
      assert.equal(parsed.ok, true, rel);
      if (parsed.ok && stored) {
        assert.deepEqual(parsed.storedBytes, stored, rel);
        assert.equal(parsed.pipelineHashCount, 1, rel);
      }
    }
  },
);
