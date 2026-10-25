// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { canonicalize } from "../src/canonical.js";
import { handleUpload } from "../src/handler.js";
import { parseManifest } from "../src/parse.js";
import { MemoryStore } from "../src/storage.js";
import { createTurnstileClient } from "../src/turnstile.js";
import type { UploadReceipt } from "../src/types.js";
import { listJsonFiles, testDeps, uploadRequest, type VectorFile, vectorPayload } from "./helpers.js";

const RECEIPT_KEYS = [
  "accepted",
  "idempotent",
  "ownership_state",
  "schema_version",
  "build_id",
  "collected_hour",
  "description_hash_count",
  "bytecode_hash_count",
  "canonical_sha256",
  "object_key",
].sort();

function minimalBytes(): Uint8Array {
  const files = listJsonFiles("schemas/vectors/valid");
  const minimal = files.find((f) => f.endsWith("minimal.json"));
  assert.ok(minimal);
  const value = JSON.parse(readFileSync(minimal, "utf8")) as VectorFile;
  return vectorPayload(value);
}

async function readJson(response: Response): Promise<unknown> {
  return response.json();
}

test("valid upload stores only canonical manifest bytes", async () => {
  const store = new MemoryStore();
  const deps = testDeps({ store });
  const bytes = minimalBytes();
  const response = await handleUpload(uploadRequest(bytes), deps);
  assert.equal(response.status, 200);
  const body = (await readJson(response)) as UploadReceipt;
  assert.deepEqual(Object.keys(body).sort(), RECEIPT_KEYS);
  assert.equal(body.accepted, true);
  assert.equal(body.idempotent, false);
  assert.equal(body.ownership_state, "UNVERIFIED");
  assert.equal(body.schema_version, 1);
  const stored = await store.get(body.object_key);
  assert.ok(stored);
  const parsed = parseManifest(stored);
  assert.equal(parsed.ok, true);
  if (parsed.ok) {
    assert.deepEqual(stored, canonicalize(parsed.manifest));
    assert.deepEqual([...Object.keys(parsed.manifest)].sort(), [
      "build_id",
      "bytecode_hashes",
      "client_version",
      "collected_hour",
      "description_hashes",
      "ownership",
      "schema_version",
    ]);
  }
  const storedText = new TextDecoder().decode(stored);
  assert.equal(storedText.includes("CF-Connecting-IP"), false);
  assert.equal(JSON.stringify(body).includes("203.0.113"), false);
});

test("same canonical bytes are idempotent and do not use a second slot", async () => {
  const store = new MemoryStore();
  const deps = testDeps({ store, perBuildCap: 1 });
  const bytes = minimalBytes();
  const first = await handleUpload(uploadRequest(bytes), deps);
  const second = await handleUpload(uploadRequest(bytes), deps);
  assert.equal(first.status, 200);
  assert.equal(second.status, 200);
  const a = (await first.json()) as UploadReceipt;
  const b = (await second.json()) as UploadReceipt;
  assert.equal(a.idempotent, false);
  assert.equal(b.idempotent, true);
  assert.equal(a.object_key, b.object_key);
  assert.equal(await store.countPrefix(a.object_key.slice(0, a.object_key.lastIndexOf("/") + 1), 8), 1);
});

test("per-build cap rejects a new object under the same prefix", async () => {
  const store = new MemoryStore();
  const deps = testDeps({ store, perBuildCap: 1 });
  const firstBytes = minimalBytes();
  const first = await handleUpload(uploadRequest(firstBytes), deps);
  assert.equal(first.status, 200);
  const mutated = JSON.parse(new TextDecoder().decode(firstBytes)) as {
    client_version: string;
  };
  mutated.client_version = "0.1.1";
  const secondBytes = new TextEncoder().encode(JSON.stringify(mutated));
  const second = await handleUpload(uploadRequest(secondBytes), deps);
  assert.equal(second.status, 429);
  const err = (await second.json()) as { code: string; message: string };
  assert.equal(err.code, "BUILD_CAP_EXCEEDED");
  assert.equal(err.message.includes("0.1.1"), false);
});

test("Content-Length over the wire cap is refused before parse", async () => {
  const deps = testDeps({ wireMaxBytes: 32 });
  const bytes = new Uint8Array(40);
  const response = await handleUpload(
    uploadRequest(bytes, { "Content-Length": "40" }),
    deps,
  );
  assert.equal(response.status, 413);
  const err = (await response.json()) as { code: string };
  assert.equal(err.code, "BODY_TOO_LARGE");
});

test("streaming counter catches a missing or lying Content-Length", async () => {
  const deps = testDeps({ wireMaxBytes: 16 });
  const bytes = new Uint8Array(32);
  const missing = await handleUpload(
    new Request("https://example.invalid/v1/upload", {
      method: "POST",
      headers: { "X-Turnstile-Token": "ok" },
      body: bytes,
    }),
    deps,
  );
  assert.equal(missing.status, 413);
  const lying = await handleUpload(
    new Request("https://example.invalid/v1/upload", {
      method: "POST",
      headers: {
        "X-Turnstile-Token": "ok",
        "Content-Length": "8",
      },
      body: bytes,
    }),
    deps,
  );
  assert.equal(lying.status, 413);
});

test("Turnstile failures and rate denials do not store objects", async () => {
  const store = new MemoryStore();
  const missing = await handleUpload(
    uploadRequest(minimalBytes(), { "X-Turnstile-Token": "" }),
    testDeps({ store }),
  );
  assert.equal(missing.status, 403);
  const failed = await handleUpload(
    uploadRequest(minimalBytes(), { "X-Turnstile-Token": "nope" }),
    testDeps({ store }),
  );
  assert.equal(failed.status, 403);
  const limited = await handleUpload(
    uploadRequest(minimalBytes()),
    testDeps({ store, rate: { allow: async () => false } }),
  );
  assert.equal(limited.status, 429);
  assert.equal(await store.countPrefix("", 8), 0);
});

test("siteverify is called without a client address field", async () => {
  let posted = "";
  const client = createTurnstileClient(
    "placeholder-secret",
    "https://example.invalid/siteverify",
    async (_input, init) => {
      posted = init.body;
      return { ok: true, json: async () => ({ success: true }) };
    },
  );
  assert.equal(await client.verify("token-value"), true);
  assert.match(posted, /secret=/);
  assert.match(posted, /response=/);
  assert.equal(posted.includes("remoteip"), false);
  assert.equal(posted.includes("token-value") || posted.includes("token"), true);
});

test("error messages never echo request content", async () => {
  const hostile = '{"schema_version":1,"username":"user-alice-fixture"}';
  const response = await handleUpload(uploadRequest(hostile), testDeps());
  assert.equal(response.status, 400);
  const err = (await response.json()) as { code: string; message: string };
  assert.equal(err.code, "UNKNOWN_FIELD");
  assert.equal(err.message.includes("user-alice-fixture"), false);
  assert.equal(err.message.includes("username"), false);
});

test("wrong method and path are rejected", async () => {
  const get = await handleUpload(
    new Request("https://example.invalid/v1/upload", { method: "GET" }),
    testDeps(),
  );
  assert.equal(get.status, 405);
  const other = await handleUpload(
    new Request("https://example.invalid/v1/other", { method: "POST" }),
    testDeps(),
  );
  assert.equal(other.status, 404);
});
