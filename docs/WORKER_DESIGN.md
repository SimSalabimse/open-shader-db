# Ingest Worker design (issues #12, #13)

Built and tested TypeScript Worker plus staging config. **Not deployed.**
This repository has no Cloudflare account, token, zone, domain, real URL,
or secret. Config files use placeholders only.

This document is the contract for `worker/`. Implementation must match it.

## What is in scope

- `POST /v1/upload` accepts UploadManifest v1 and v2 JSON
- The same validation rules as `crates/upload` (plus a lower wire body cap)
- Staging object layout on R2 (or a documented reviewed-PR alternative)
- Turnstile verification, edge rate limiting, per-build upload cap
- No IP or fingerprint persistence

## What is explicitly not in this design

- Deploying anything, filling placeholders, or shipping secrets
- A client `submit` command or any client network send
- Ownership verification (the Worker records a claim as `UNVERIFIED`)
- The anti-poisoning shipped-hash-list check (later; do not stub)
- Consensus, publishing, or accounts

## Types

```typescript
type SchemaVersion = 1 | 2

type Sha256Hex = string // 64 lowercase hex characters

interface BuildId {
  game_id: string       // 1-128 ASCII graphic; no `/`, `\`, `:`, whitespace
  version: string       // same identifier rules as game_id
  exe_hash: Sha256Hex
}

type OwnershipProof =
  | { kind: "unchallenged"; reason: "no_server_challenge" }
  | { kind: "exe_binding_v1"; challenge: Sha256Hex; response: Sha256Hex }

interface UploadManifest {
  schema_version: SchemaVersion
  build_id: BuildId
  client_version: string // major.minor.patch of ASCII digits, max 32
  collected_hour: string // YYYY-MM-DDTHH:00:00Z, real calendar, hour only
  description_hashes: Sha256Hex[] // sorted unique; cap 150 000
  bytecode_hashes: Sha256Hex[]    // sorted unique; cap 400 000
  pipeline_hashes?: Sha256Hex[]   // version 2 only; sorted unique; cap 50 000
  ownership: OwnershipProof
}

interface UploadReceipt {
  accepted: true
  idempotent: boolean
  ownership_state: "UNVERIFIED"
  schema_version: SchemaVersion
  build_id: BuildId
  collected_hour: string
  description_hash_count: number
  bytecode_hash_count: number
  pipeline_hash_count: number // 0 on version 1
  canonical_sha256: Sha256Hex
  object_key: string
}

interface ErrorBody {
  code: ErrorCode
  message: string // short, stable; never echoes request content
}

interface StagingStore {
  head(key: string): Promise<boolean>
  put(key: string, bytes: Uint8Array): Promise<void>
  countPrefix(prefix: string, limit: number): Promise<number>
}

interface TurnstileClient {
  verify(token: string): Promise<boolean>
}

interface RateGate {
  allow(): Promise<boolean> // constant key only; never an IP or fingerprint
}

interface InjectedFetch {
  (input: string, init: { method: string; headers: Record<string, string>; body: string; signal?: AbortSignal }): Promise<{
    ok: boolean
    json(): Promise<unknown>
  }>
}
```

`unknown` at the HTTP body becomes `UploadManifest` or a typed `ErrorCode`.
No `any`. Parse, do not validate-and-cast.

## Caps (shared numbers)

Pinned in `schemas/limits.v1.json`, `crates/upload` `Limits`, and
`worker/src/limits.ts`. A test in each language reads the JSON file.

| Cap | Value | Where it applies |
| --- | ---: | --- |
| Local client JSON | 64 MiB | `osdb-upload` parse and encode |
| Wire request body | 8 MiB | Worker, before JSON parse |
| Description hashes | 150 000 | both validators (`50 000` records × 3 keys) |
| Bytecode hashes | 400 000 | both validators (`50 000` records × 8 stages) |
| Pipeline hashes | 50 000 | both validators (version 2; one hash per record). No extra nesting level. |
| JSON nesting | 8 | both validators |
| JSON number token | 16 characters | `schema_version` only |
| Per-build upload objects | 64 | Worker staging prefix only |

The 16-character cap can only affect `schema_version`. Counts, timestamps, and hashes are strings or array lengths, so no other field can reach it.

64 MiB stays the local cap so a full record file can still be previewed.
The largest version 2 manifest this tree builds (150 000 description
hashes, 400 000 bytecode hashes, and 50 000 pipeline hashes) is
40 200 346 bytes from the empty synthetic manifest. The closed-form
upper bound `empty + slots * (per_hash + 1)` is 40 800 349 bytes (it
counts a comma for every hash, including the first of each list).
Both are under 64 MiB and over 8 MiB.
The Worker **wire cap** is 8 MiB (not an isolate-size budget). A 53-64
MiB document is refused before parse; see [Memory](#memory) for measured
peaks under that wire cap. A full version 2 payload does not fit on the
wire; those contributors use the reviewed-PR path.

## Module layout

```
worker/
  src/
    index.ts       // Cloudflare fetch export; wires bindings
    handler.ts     // request flow; depends on injected interfaces
    parse.ts       // unknown bytes -> UploadManifest | ErrorCode
    canonical.ts   // compact JSON, lexicographic object keys
    limits.ts      // numbers (must match schemas/limits.v1.json)
    errors.ts      // ErrorCode + safe messages
    keys.ts        // R2 key layout + component checks
    body.ts        // Content-Length + one preallocated body buffer
    turnstile.ts   // siteverify via injected fetch; omit remoteip
    storage.ts     // R2 adapter behind StagingStore
    rate_limit.ts  // RateGate adapter; constant key only
    privacy.ts     // allowlisted header reads only
    log.ts         // the only module allowed to touch console
  test/            // node:test; in-memory fakes; no network
  config/          // lifecycle, Logpush field list (placeholders)
  wrangler.toml    // placeholder names; NOT DEPLOYED comment
```

Zero runtime dependencies. DevDependencies: `typescript` (typecheck and
emit) and `@types/node` (`node:test`, `node:assert`, `node:fs`,
`node:path`, `node:crypto`). Lockfile committed. Tests run on the
emitted JS.

## Request flow

1. Method `POST` and path `/v1/upload` only. Other methods: `METHOD_NOT_ALLOWED`.
   Other paths: `NOT_FOUND`.
2. `RateGate.allow()` with the constant key `v1-upload`. Denial: `RATE_LIMITED`.
   No request-derived key.
3. Require `Content-Type: application/json` (optional `charset`). Other
   types: `UNSUPPORTED_MEDIA_TYPE` (415).
4. If `Content-Length` is a valid non-negative integer and is greater
   than 8 MiB, reject (`BODY_TOO_LARGE`) without calling siteverify and
   without reading the body.
5. Read the Turnstile token from header `X-Turnstile-Token`. Missing:
   `TURNSTILE_MISSING`. Tokens longer than 2048 characters, siteverify
   failure, timeout, or a non-`success: true` body: `TURNSTILE_FAILED`.
   The token is dropped after the call. `remoteip` is omitted.
   `hostname` and `action` are checked only when
   `TURNSTILE_EXPECTED_HOSTNAME` / `TURNSTILE_EXPECTED_ACTION` are
   non-empty (this repository has no site hostname to commit).
6. Stream the body through a byte counter. Crossing 8 MiB rejects even
   when `Content-Length` is missing, non-numeric, or too small.
7. Parse bytes with a schema-directed walk: reject at the first unknown
   key, wrong type, depth over 8, list over its cap, string over its
   max length, or duplicate key. Same rules as `crates/upload`.
8. SHA-256 the stored bytes. If the request was already compact
   canonical JSON, those bytes are the original buffer (no
   re-serialize).
9. Build object key (see [Staging](#staging)). Re-check key components.
10. `head(key)` once: if the object exists, return a receipt with
    `idempotent: true`. No second existence check. No cap increment.
11. `countPrefix(game_id/build_id/, cap)`. If the count is already `cap`,
    reject (`BUILD_CAP_EXCEEDED`). The count is approximate under races;
    that is acceptable (see below). Anyone can fill the 64 slots under
    one prefix (for example by varying `client_version`) and block
    other contributors on that build for 90 days. That is a known
    tradeoff; mitigations are a later step. New `exe_hash` values are
    unbounded except by the rate gate.
12. `put` the stored canonical bytes only. Receipt fields are
    manifest-derived plus `idempotent` and `ownership_state: "UNVERIFIED"`.
    `pipeline_hash_count` is on the receipt (0 for version 1). It is
    not written to the accept log.

The Worker never verifies `exe_binding_v1` and never claims that
`unchallenged` is a proof. Both are stored as `UNVERIFIED`.

## Error taxonomy

Stable `code` plus a short `message`. Messages do not include hashes,
header values, IPs, paths, or any request substring.

| Code | HTTP | Meaning |
| --- | ---: | --- |
| `METHOD_NOT_ALLOWED` | 405 | not POST |
| `NOT_FOUND` | 404 | path is not `/v1/upload` |
| `RATE_LIMITED` | 429 | edge / binding gate denied |
| `TURNSTILE_MISSING` | 403 | token header absent |
| `TURNSTILE_FAILED` | 403 | siteverify unsuccessful, timeout, or oversized token |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | `Content-Type` is not `application/json` |
| `BODY_TOO_LARGE` | 413 | wire cap (Content-Length or stream) |
| `MANIFEST_TOO_LARGE` | 413 | parser JSON byte cap |
| `BYTE_ORDER_MARK` | 400 | UTF-8 BOM |
| `TRAILING_GARBAGE` | 400 | tokens after one JSON value |
| `INVALID_JSON` | 400 | truncated, non-UTF-8, or not JSON |
| `NESTING_TOO_DEEP` | 400 | `{` / `[` depth over 8 |
| `DUPLICATE_KEY` | 400 | repeated object key |
| `UNKNOWN_FIELD` | 400 | field outside the allowlist |
| `MISSING_FIELD` | 400 | required field absent |
| `UNSUPPORTED_SCHEMA_VERSION` | 400 | `schema_version` is not 1 or 2 |
| `INVALID_INTEGER` | 400 | integer field is not an integer token |
| `MALFORMED_HEX` | 400 | non-lowercase-hex in a hash |
| `WRONG_HEX_LENGTH` | 400 | hash is not 64 characters |
| `UNSORTED_OR_DUPLICATE_HASHES` | 400 | list not strictly sorted unique |
| `TOO_MANY_HASHES` | 400 | a list exceeds its cap |
| `INVALID_BUILD_ID` | 400 | identifier empty, long, or unsafe |
| `INVALID_TIMESTAMP` | 400 | not an hour-boundary real UTC date |
| `INVALID_CLIENT_VERSION` | 400 | not `major.minor.patch` of digits |
| `UNKNOWN_OWNERSHIP_KIND` | 400 | `ownership.kind` not documented |
| `UNKNOWN_OWNERSHIP_REASON` | 400 | `unchallenged` reason not documented |
| `BUILD_CAP_EXCEEDED` | 429 | per-build object cap |

`crates/upload::Error` exposes the same parse codes via `Error::code()`.
Shared golden vectors in `schemas/vectors/` name one `error_code` each.

## Staging

Key:

```text
<game_id>/<build_id>/<sha256 of canonical manifest bytes>.json
```

`build_id` on the path is `version--<exe_hash>`. `exe_hash` is always 64
lowercase hex, so the last `--` plus 64 hex digits is unambiguous even
if `version` contains `--`.

Every component is re-checked before use: non-empty, at most 128 bytes
for identifiers, no `/`, `\`, `:`, NUL, whitespace, `.`, or `..`. A
hostile `game_id` or `BuildId` cannot escape its prefix or collide
with another prefix. `.` and `..` are rejected by both language
validators.

Stored bytes are the validated canonical manifest only. No request
headers, no server clock, no IP, no Turnstile token.

**90-day expiry:** `worker/config/r2-lifecycle.json` (placeholder bucket).
A test asserts the committed rule is 90 days.

**Access policy (intended, not deployed):**

- The Worker binding can `put` / `get` / `list` prefix counts
- Maintainers with the Cloudflare account can read for review
- No public bucket listing
- No public unauthenticated read URL
- Logpush, if enabled later, must use the exact field allowlist in
  `worker/config/logpush-fields.json` (see [Privacy](#privacy))

**Per-build cap:** count objects under `<game_id>/<build_id>/` and refuse
a new key when the count is already 64. Two concurrent first-time puts
can both observe a count below the cap and both succeed. That race is
accepted: the cap is a cheap Sybil brake, not a linearizable quota.
Idempotent retries of the same canonical bytes use one key and do not
consume an extra slot.

### Reviewed-pull-request path (if R2 is not used)

A contributor who cannot use the Worker (no Turnstile, payload over 8
MiB, or operators have not deployed R2) can open a pull request that
adds a canonical manifest under `staging/<game_id>/<build_id>/` with
the same filename rule. A maintainer reviews and merges. The 90-day
clock is then the git history plus a documented sweep, not R2
lifecycle. This path is manual and is the trust anchor until R2 is
actually deployed.

## Rate limiting (what is configured, what is not)

**Configured in this repo (placeholders only):**

- `wrangler.toml` `[[ratelimits]]` binding `UPLOAD_RATE_LIMITER`
  (30 events / 60 seconds, namespace id placeholder)
- Worker calls `limit({ key: "v1-upload" })` — a constant, never an IP
- Cloudflare documents this binding as **approximate and counted per
  location**. Bursts can exceed 30/60s. The single global key means
  one client can use the whole budget for everyone at that location.

**Not in this repository (operator deployment choices):**

- Turnstile sitekey and secret (secret binding `TURNSTILE_SECRET`)
- Optional `TURNSTILE_EXPECTED_HOSTNAME` / `TURNSTILE_EXPECTED_ACTION`
- R2 bucket name, account id, Logpush destination
- A paid Workers plan or a lower wire cap (see [CPU time](#cpu-time))

**Not configured, and must not be added:**

- Counters in R2 / KV / Durable Objects keyed by IP or fingerprint
- Reading hop-identifying or client-hint headers to feed a limiter
- Storing the Turnstile token

## Privacy

The Worker source must not read:

- `CF-Connecting-IP`, `X-Forwarded-For`, `User-Agent`
- `request.cf` or any `cf.*` request property
- Geo or ASN fields

A TypeScript-compiler lint is a **best-effort** guard on those reads,
on any `.cf` access, on `headers.get` / header iteration outside the
allowlist (`Content-Length`, `Content-Type`, `X-Turnstile-Token`), on
concatenated or computed property names on `request` / `headers` /
`env` (including a one-level alias), on `Object.entries` /
`fromEntries` / `getOwnPropertyDescriptor` / `assign` / spread of those
objects, on `Function` and `eval`, and on passing `token` / `secret` /
`turnstileToken` (or a one-level rename of them) to anything except
the Turnstile client call. It does not prove the absence of a leak.
Code review is the real control.

The stored object is canonical manifest bytes. The HTTP response is
`UploadReceipt` or `ErrorBody`. A runtime test asserts both contain
only manifest-derived fields (plus `accepted`, `idempotent`,
`ownership_state`).

Logging: outcome code, hash-list counts, object-key **prefix**
(`game_id/build_id/`), and `idempotent`. No request body, no headers,
no token, no IP.

**Logpush:** `worker/config/logpush-fields.json` is an **exact**
allowlist: `EventType`, `Outcome`, `ScriptName`, `Logs`, `CPUTimeMs`,
`WallTimeMs`, `EventTimestampMs`. It does not include `Event` or
`Exceptions`. `workers_trace_events` has no `ClientIP` field;
omitting that name proves nothing. `Event` would carry request
headers (including `X-Turnstile-Token` in cleartext) and Cloudflare
`cf` data (country, ASN, TLS). A test pins this allowlist.

`[observability] enabled = false` in `wrangler.toml` so Workers Logs
do not store request data.

Cloudflare itself may still log request metadata at the edge (access
logs, bot scores, and similar platform logs). That is outside this
repository's control.

Turnstile: POST to the public siteverify endpoint with `secret` and
`response` only. Do not send `remoteip`. Do not log the token or the
raw siteverify body. Abort after a few seconds. Cap the token at 2048
characters.

## Memory

A Cloudflare Worker isolate has 128 MB. The **wire cap is 8 MiB**;
that is not an isolate budget. The parser is schema-directed and
fails at the first violation, so hostile 8 MiB bodies do not build a
generic JSON tree.

Measured on one Node 22 machine. The figures below are approximate
(another machine measured about 0-20% more CPU). `heapUsed` is V8
old-space; ArrayBuffers for the 8 MiB body sit beside it
(`heapUsed + arrayBuffers`). CPU is `process.cpuUsage` (user + system):

| 8 MiB input | Peak heapUsed | heapUsed + ArrayBuffers | CPU |
| --- | ---: | ---: | ---: |
| compact max-valid (62 599 + 62 599 hashes) | 7.1 MB | 45.2 MB | 105.5 ms |
| pretty-printed max-valid (indented JSON, 58 251 hashes) | 24.9 MB | 60.4 MB | 188.5 ms |
| compact max-valid in 1-byte chunks | 8.0 MB | 46.1 MB | 3208.1 ms |
| compact max-valid in 16-byte chunks | 7.6 MB | 67.7 MB | 309.6 ms |
| pretty-printed max-valid in 1-byte chunks | 15.9 MB | 54.8 MB | 3559.8 ms |
| pretty-printed max-valid in 16-byte chunks | 24.1 MB | 59.6 MB | 351.5 ms |
| braces inside a string field | 15.5 MB | 78.5 MB | 21.2 ms |
| huge string inside a hash field | 15.5 MB | 78.5 MB | 16.2 ms |
| nesting-depth guard (64 brackets) | 5.8 MB | 45.8 MB | 16.2 ms |
| 8 MiB `schema_version` number token | 5.8 MB | 53.9 MB | 19.2 ms |

The ten rows above are the earlier measurement and are left as recorded.
This machine (Node 22) added one case and re-ran the heap child under
`--max-old-space-size=64`. Compact max-valid v2 fills the wire cap with
three equal hash lists (41 732 hashes each, 8 388 478 bytes, under the
50 000 pipeline cap):

| 8 MiB input | Peak heapUsed | heapUsed + ArrayBuffers | CPU |
| --- | ---: | ---: | ---: |
| compact max-valid v2 (41 732 + 41 732 + 41 732 hashes) | 5.9 MB | 53.0 MB | 103.1 ms |

CI runs these eleven cases in a child process under a 64 MB
old-space limit. An uncapped 8 MiB number token runs out of memory
there. The pretty-printed max-valid path used to peak at
223 MB heap when `hexEncode32` built cons-string ropes; a 256-entry
lookup table plus one `join` brings it to the 24.9 MB above. Tiny
chunks used to keep one array entry each and ran out of memory;
`body.ts` now copies into one preallocated `Uint8Array`. An 8 MiB
number token used to cost 178-200 ms and 78.7 MB heap because the
parser converted the whole token; both languages now cap the token at
16 characters before conversion (19.2 ms / 5.8 MB here).

A 64 MiB local preview document is still not parsed here, which is
why the wire cap is separate.

## CPU time

The table above is approximate. Compact max-valid is about
106 ms; pretty-printed is about 189 ms; a 1-byte-chunk stream of the
compact body is about 3.2 s and of the pretty body about 3.6 s;
16-byte chunks are about 310-352 ms. Hostile in-field rejects are
31-52 ms on the reviewer's harness and 16-21 ms here. An uncapped 8
MiB number token was 178-200 ms; after the length cap it is 19.2 ms.
The Workers Free plan allows 10 ms of CPU per request, which is far
below any parse of a real upload. A deployment therefore needs a
paid plan or smaller caps. That is a deployment-time decision for
Simsalabim; this repository does not pick one.

## Shared golden vectors

`schemas/vectors/` holds valid and invalid fixtures. Each file has a
`verdict` (`accept` or `reject`) and an `error_code` (or `null`).
`crates/upload` tests and Worker tests both load every file and
require `index.json` to list each path, verdict, and error code.
Every accept vector also has a `stored/` file with the Rust
canonicalizer's compact JSON; both languages compare stored bytes
byte-for-byte. Deleting a vector or its stored-bytes file fails.
Valid bodies are produced by the Rust crate (canonical bytes).
Invalid bodies are hand-written hostile cases.

The JSON Schema in `schemas/upload_manifest.v1.json` is looser than
these checks (no sort order, no real calendar dates, no byte/nesting
enforcement, no canonical form). The Worker must not use the schema
alone.

## Safeguards (unchanged)

No bytecode and no hash keys of bytecode as publishable content. Opt-in
only. No accounts. No IPs. No game paths. Upload preview stays a client
concern. Offline extraction only. Never hook anti-cheat processes. No
Microsoft redistributables or trademarks as branding. No scraping or
reuse of any competitor database or of SCSKiller.
