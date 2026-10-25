// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { repoRoot } from "./helpers.js";

const FORBIDDEN = [
  "CF-Connecting-IP",
  "X-Forwarded-For",
  "User-Agent",
  "request.cf",
  "X-Real-IP",
  "ClientIP",
  "cf-ipcountry",
  "asn",
  "latitude",
  "longitude",
];

function listSourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...listSourceFiles(full));
      continue;
    }
    if (entry.name.endsWith(".ts")) {
      out.push(full);
    }
  }
  return out;
}

test("Worker source never references hop-identifying or geo fields", () => {
  const src = path.join(repoRoot(), "worker", "src");
  const files = listSourceFiles(src);
  assert.ok(files.length > 0);
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    for (const token of FORBIDDEN) {
      assert.equal(
        text.includes(token),
        false,
        `${path.relative(repoRoot(), file)} mentions ${token}`,
      );
    }
  }
});
