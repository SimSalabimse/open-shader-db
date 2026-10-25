// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

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
];

test("handler stays under a 96 MB old-space limit for compact, pretty, chunked, and hostile bodies", () => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const child = path.join(here, "heap_cases.js");
  const result = spawnSync(
    process.execPath,
    ["--max-old-space-size=96", "--max-semi-space-size=1", child],
    { encoding: "utf8", env: { ...process.env, OSDB_HEAP_CHILD: "1" } },
  );
  assert.equal(
    result.status,
    0,
    `heap child exited ${String(result.status)}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
  );
  const output = `${result.stdout}\n${result.stderr}`;
  for (const name of CASES) {
    assert.match(output, new RegExp(`HEAP_CASE ${name} `), `child must run ${name}`);
  }
  assert.match(output, /HEAP_CASE max_valid status=200 peak_heap_mb=/);
  assert.match(output, /HEAP_CASE max_valid_pretty status=200 peak_heap_mb=/);
  assert.match(output, /HEAP_CASE max_valid_chunk_1 status=200 peak_heap_mb=/);
  assert.match(output, /HEAP_CASE max_valid_pretty_chunk_1 status=200 peak_heap_mb=/);
  assert.match(output, /HEAP_CASE huge_number_token status=400 peak_heap_mb=/);
  assert.match(output, /HEAP_CHILD_OK cases=10/);
  const ran = [...output.matchAll(/HEAP_CASE ([a-z0-9_]+) /g)].map((m) => m[1]);
  assert.deepEqual(ran, CASES, "child must run exactly the intended cases");
});
