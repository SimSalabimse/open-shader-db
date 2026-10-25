// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const CASES = [
  "max_valid",
  "many_objects",
  "empty_arrays",
  "deep_nesting",
  "huge_string",
  "number_arrays",
];

test("handler stays under a 96 MB old-space limit for max valid and hostile 8 MiB bodies", () => {
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
  assert.match(output, /HEAP_CHILD_OK cases=6/);
  const ran = [...output.matchAll(/HEAP_CASE ([a-z_]+) /g)].map((m) => m[1]);
  assert.deepEqual(ran, CASES, "child must run exactly the intended cases");
});
