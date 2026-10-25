// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { handleUpload } from "../src/handler.js";
import { TURNSTILE_TIMEOUT_MS, WIRE_MAX_JSON_BYTES } from "../src/limits.js";
import { parseManifest } from "../src/parse.js";
import { MemoryStore } from "../src/storage.js";
import { createTurnstileClient } from "../src/turnstile.js";
import type { R2Bucket, UploadReceipt } from "../src/types.js";
import { R2StagingStore } from "../src/storage.js";
import {
  listJsonFiles,
  testDeps,
  uploadRequest,
  type VectorFile,
  vectorPayload,
} from "./helpers.js";

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
  const stored = await store.read(body.object_key);
  assert.ok(stored);
  const parsed = parseManifest(stored);
  assert.equal(parsed.ok, true);
  if (parsed.ok) {
    assert.deepEqual(stored, parsed.storedBytes);
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

test("Content-Length over the wire cap is refused before siteverify", async () => {
  let called = false;
  const deps = testDeps({
    turnstile: {
      verify: async () => {
        called = true;
        return true;
      },
    },
  });
  const response = await handleUpload(
    uploadRequest(minimalBytes(), { "Content-Length": String(WIRE_MAX_JSON_BYTES + 1) }),
    deps,
  );
  assert.equal(response.status, 413);
  const err = (await response.json()) as { code: string };
  assert.equal(err.code, "BODY_TOO_LARGE");
  assert.equal(called, false);
});

test("body of exactly the wire cap is read; one extra byte is 413", async () => {
  const atCap = await handleUpload(
    uploadRequest(new Uint8Array(WIRE_MAX_JSON_BYTES).fill(0x20)),
    testDeps(),
  );
  assert.equal(atCap.status, 400);
  const over = await handleUpload(
    uploadRequest(new Uint8Array(WIRE_MAX_JSON_BYTES + 1).fill(0x20)),
    testDeps(),
  );
  assert.equal(over.status, 413);
  const err = (await over.json()) as { code: string };
  assert.equal(err.code, "BODY_TOO_LARGE");
});

test("non-JSON Content-Type is 415 and stores nothing", async () => {
  const store = new MemoryStore();
  const response = await handleUpload(
    uploadRequest(minimalBytes(), { "content-type": "text/plain" }),
    testDeps({ store }),
  );
  assert.equal(response.status, 415);
  const err = (await response.json()) as { code: string };
  assert.equal(err.code, "UNSUPPORTED_MEDIA_TYPE");
  assert.equal(await store.countPrefix("", 8), 0);
});

test("Content-Length above the safe-integer range is 413", async () => {
  const bytes = minimalBytes();
  for (const length of ["1e30", "1000000000000000000000000000000", "9007199254740992"]) {
    const response = await handleUpload(
      uploadRequest(bytes, { "Content-Length": length }),
      testDeps(),
    );
    assert.equal(response.status, 413, length);
    const err = (await response.json()) as { code: string };
    assert.equal(err.code, "BODY_TOO_LARGE", length);
  }
});

test("streaming counter catches a missing or lying Content-Length", async () => {
  const deps = testDeps({ wireMaxBytes: 16 });
  const bytes = new Uint8Array(32);
  const missing = await handleUpload(
    new Request("https://example.invalid/v1/upload", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "X-Turnstile-Token": "ok",
      },
      body: bytes,
    }),
    deps,
  );
  assert.equal(missing.status, 413);
  const lying = await handleUpload(
    new Request("https://example.invalid/v1/upload", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "X-Turnstile-Token": "ok",
        "Content-Length": "8",
      },
      body: bytes,
    }),
    deps,
  );
  assert.equal(lying.status, 413);
});

test("Turnstile failure paths return 403 and store nothing", async () => {
  const bytes = minimalBytes();
  const cases: { name: string; request: Request; deps: ReturnType<typeof testDeps> }[] = [];
  const store = new MemoryStore();
  cases.push({
    name: "missing token",
    request: uploadRequest(bytes, { "X-Turnstile-Token": "" }),
    deps: testDeps({ store }),
  });
  cases.push({
    name: "oversized token",
    request: uploadRequest(bytes, { "X-Turnstile-Token": "t".repeat(2049) }),
    deps: testDeps({ store }),
  });
  const failedClient = createTurnstileClient(
    "placeholder-secret",
    "https://example.invalid/siteverify",
    async () => ({ ok: true, json: async () => ({ success: false }) }),
  );
  cases.push({
    name: "success false",
    request: uploadRequest(bytes),
    deps: testDeps({ store, turnstile: failedClient }),
  });
  const serverError = createTurnstileClient(
    "placeholder-secret",
    "https://example.invalid/siteverify",
    async () => ({ ok: false, json: async () => ({ success: true }) }),
  );
  cases.push({
    name: "5xx",
    request: uploadRequest(bytes),
    deps: testDeps({ store, turnstile: serverError }),
  });
  const malformed = createTurnstileClient(
    "placeholder-secret",
    "https://example.invalid/siteverify",
    async () => ({
      ok: true,
      json: async () => {
        throw new Error("not json");
      },
    }),
  );
  cases.push({
    name: "malformed JSON",
    request: uploadRequest(bytes),
    deps: testDeps({ store, turnstile: malformed }),
  });
  for (const item of cases) {
    const response = await handleUpload(item.request, item.deps);
    assert.equal(response.status, 403, item.name);
    assert.equal(await store.countPrefix("", 8), 0, item.name);
  }
});

test("siteverify client mutants fail closed", async () => {
  const url = "https://example.invalid/siteverify";
  const secret = "placeholder-secret";
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => ({ success: false }) })).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: false, json: async () => ({ success: true }) })).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => { throw new Error("net"); }).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => "true" })).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => 1 })).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => null })).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => [true] })).verify("tok"), false);
  assert.equal(await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => ({ success: true }) })).verify(""), false);
  assert.equal(
    await createTurnstileClient(secret, url, async () => ({ ok: true, json: async () => ({ success: true }) })).verify("t".repeat(2049)),
    false,
  );
  const hostnameClient = createTurnstileClient({
    secret,
    siteverifyUrl: url,
    expectedHostname: "example.invalid",
    fetchImpl: async () => ({ ok: true, json: async () => ({ success: true, hostname: "other.invalid" }) }),
  });
  assert.equal(await hostnameClient.verify("tok"), false);
  const actionClient = createTurnstileClient({
    secret,
    siteverifyUrl: url,
    expectedAction: "upload",
    fetchImpl: async () => ({ ok: true, json: async () => ({ success: true, action: "other" }) }),
  });
  assert.equal(await actionClient.verify("tok"), false);
  const okClient = createTurnstileClient({
    secret,
    siteverifyUrl: url,
    expectedHostname: "example.invalid",
    expectedAction: "upload",
    fetchImpl: async () => ({
      ok: true,
      json: async () => ({ success: true, hostname: "example.invalid", action: "upload" }),
    }),
  });
  assert.equal(await okClient.verify("tok"), true);
  assert.equal(
    await createTurnstileClient(secret, url, async () => ({
      ok: true,
      json: async () => ({ success: "true" }),
    })).verify("tok"),
    false,
  );
  assert.equal(
    await createTurnstileClient(secret, url, async () => ({
      ok: true,
      json: async () => ({ success: 1 }),
    })).verify("tok"),
    false,
  );
  let fetched2048 = false;
  assert.equal(
    await createTurnstileClient(secret, url, async () => {
      fetched2048 = true;
      return { ok: true, json: async () => ({ success: true }) };
    }).verify("t".repeat(2048)),
    true,
  );
  assert.equal(fetched2048, true);
  let fetched2049 = false;
  assert.equal(
    await createTurnstileClient(secret, url, async () => {
      fetched2049 = true;
      return { ok: true, json: async () => ({ success: true }) };
    }).verify("t".repeat(2049)),
    false,
  );
  assert.equal(fetched2049, false);
  const unsetHostAction = createTurnstileClient({
    secret,
    siteverifyUrl: url,
    fetchImpl: async () => ({ ok: true, json: async () => ({ success: true }) }),
  });
  assert.equal(await unsetHostAction.verify("tok"), true);
});

test("siteverify hard deadline uses fake timers when fetch or json hang", async () => {
  const fakeSchedule = ((fn: () => void, _ms?: number) => {
    queueMicrotask(fn);
    return 0 as unknown as ReturnType<typeof setTimeout>;
  }) as typeof setTimeout;
  const hangingFetch = createTurnstileClient({
    secret: "placeholder-secret",
    siteverifyUrl: "https://example.invalid/siteverify",
    timeoutMs: TURNSTILE_TIMEOUT_MS,
    schedule: fakeSchedule,
    fetchImpl: () => new Promise(() => {}),
  });
  assert.equal(await hangingFetch.verify("tok"), false);
  const hangingJson = createTurnstileClient({
    secret: "placeholder-secret",
    siteverifyUrl: "https://example.invalid/siteverify",
    timeoutMs: TURNSTILE_TIMEOUT_MS,
    schedule: fakeSchedule,
    fetchImpl: () =>
      Promise.resolve({
        ok: true,
        json: () => new Promise(() => {}),
      }),
  });
  assert.equal(await hangingJson.verify("tok"), false);
});

test("empty Turnstile token is MISSING and 2048-character tokens reach verify", async () => {
  const empty = await handleUpload(
    uploadRequest(minimalBytes(), { "X-Turnstile-Token": "" }),
    testDeps(),
  );
  assert.equal(empty.status, 403);
  const emptyBody = (await empty.json()) as { code: string };
  assert.equal(emptyBody.code, "TURNSTILE_MISSING");
  let seen = "";
  const atCap = await handleUpload(
    uploadRequest(minimalBytes(), { "X-Turnstile-Token": "t".repeat(2048) }),
    testDeps({
      turnstile: {
        verify: async (value) => {
          seen = value;
          return value.length === 2048;
        },
      },
    }),
  );
  assert.equal(seen.length, 2048);
  assert.equal(atCap.status, 200);
  const over = await handleUpload(
    uploadRequest(minimalBytes(), { "X-Turnstile-Token": "t".repeat(2049) }),
    testDeps({
      turnstile: {
        verify: async () => {
          throw new Error("oversized token must not reach verify");
        },
      },
    }),
  );
  assert.equal(over.status, 403);
  const overBody = (await over.json()) as { code: string };
  assert.equal(overBody.code, "TURNSTILE_FAILED");
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

test("hash-list cap accepts max and refuses max plus one", () => {
  const base = JSON.parse(new TextDecoder().decode(minimalBytes())) as {
    bytecode_hashes: string[];
  };
  const hex = (n: number): string => n.toString(16).padStart(64, "0");
  base.bytecode_hashes = [hex(1), hex(2)];
  const two = parseManifest(new TextEncoder().encode(JSON.stringify(base)), {
    maxJsonBytes: 64 * 1024 * 1024,
    maxDescriptionHashes: 2,
    maxBytecodeHashes: 2,
  });
  assert.equal(two.ok, true);
  base.bytecode_hashes = [hex(1), hex(2), hex(3)];
  const three = parseManifest(new TextEncoder().encode(JSON.stringify(base)), {
    maxJsonBytes: 64 * 1024 * 1024,
    maxDescriptionHashes: 2,
    maxBytecodeHashes: 2,
  });
  assert.equal(three.ok, false);
  if (!three.ok) {
    assert.equal(three.code, "TOO_MANY_HASHES");
  }
});

test("R2 staging existence uses one head and never get", async () => {
  const calls: string[] = [];
  const objects = new Map<string, Uint8Array>();
  const bucket: R2Bucket = {
    async head(key) {
      calls.push(`head:${key}`);
      return objects.has(key) ? { key } : null;
    },
    async get() {
      throw new Error("get must not be used for existence");
    },
    async put(key, value) {
      calls.push(`put:${key}`);
      const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
      objects.set(key, bytes);
      return {};
    },
    async list() {
      return { objects: [...objects.keys()].map((key) => ({ key })), truncated: false };
    },
  };
  const store = new R2StagingStore(bucket);
  const deps = testDeps({ store });
  const first = await handleUpload(uploadRequest(minimalBytes()), deps);
  assert.equal(first.status, 200);
  const second = await handleUpload(uploadRequest(minimalBytes()), deps);
  assert.equal(second.status, 200);
  assert.equal(calls.filter((c) => c.startsWith("head:")).length, 2);
  assert.equal(calls.filter((c) => c.startsWith("put:")).length, 1);
});
