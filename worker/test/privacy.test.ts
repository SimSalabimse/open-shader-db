// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { ALLOWED_REQUEST_HEADERS } from "../src/privacy.js";
import { repoRoot } from "./helpers.js";
import { lintTypeScript } from "./privacy_lint.js";

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

test("Worker source never reads hop-identifying fields or request.cf", () => {
  const src = path.join(repoRoot(), "worker", "src");
  const files = listSourceFiles(src);
  assert.ok(files.length > 0);
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    const errors = lintTypeScript(path.relative(repoRoot(), file), text);
    assert.deepEqual(errors, [], `${file}: ${errors.join("; ")}`);
  }
});

test("privacy lint rejects the seven bypass variants", () => {
  const variants: { name: string; file: string; source: string }[] = [
    {
      name: "lowercase header name",
      file: "mutant-lowercase.ts",
      source: `
        export function read(headers: Headers): string | null {
          return headers.get("cf-connecting-ip");
        }
      `,
    },
    {
      name: "concatenated header name",
      file: "mutant-concat.ts",
      source: `
        export function read(headers: Headers): string | null {
          return headers.get("CF-" + "Connecting-IP");
        }
      `,
    },
    {
      name: "request[cf]",
      file: "mutant-index-cf.ts",
      source: `
        export function geo(request: Request): unknown {
          return request["cf"];
        }
      `,
    },
    {
      name: "destructuring cf",
      file: "mutant-destructure-cf.ts",
      source: `
        export function geo(request: Request): unknown {
          const { cf } = request;
          return cf;
        }
      `,
    },
    {
      name: "logging all headers",
      file: "mutant-log-headers.ts",
      source: `
        export function leak(request: Request): void {
          console.log([...request.headers]);
        }
      `,
    },
    {
      name: "logging the token",
      file: "mutant-log-token.ts",
      source: `
        export function leak(token: string): void {
          console.log(token);
        }
      `,
    },
    {
      name: "user-agent on the allowed set",
      file: "src/privacy.ts",
      source: `
        export const ALLOWED_REQUEST_HEADERS = [
          "content-length",
          "content-type",
          "x-turnstile-token",
          "user-agent",
        ] as const;
      `,
    },
  ];
  for (const variant of variants) {
    const errors = lintTypeScript(variant.file, variant.source);
    assert.ok(errors.length > 0, `${variant.name} must fail the privacy lint`);
  }
});

test("allowed request headers are exactly the three the Worker reads", () => {
  assert.deepEqual([...ALLOWED_REQUEST_HEADERS].sort(), [
    "content-length",
    "content-type",
    "x-turnstile-token",
  ]);
});
