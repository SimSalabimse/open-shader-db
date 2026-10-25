# Ingest Worker design (issues #12, #13)

Built and tested TypeScript Worker plus staging config. **Not deployed.**
This repository has no Cloudflare account, token, zone, domain, real URL,
or secret. Config files use placeholders only.

This document is the contract for `worker/`. Implementation must match it.

## What is in scope

- `POST /v1/upload` accepts UploadManifest v1 JSON
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
type SchemaVersion = 1

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
  canonical_sha256: Sha256Hex
  object_key: string
}

interface ErrorBody {
  code: ErrorCode
  message: string // short, stable; never echoes request content
}

interface StagingStore {
  get(key: string): Promise<Uint8Array | null>
  putIfAbsent(key: string, bytes: Uint8Array): Promise<"created" | "exists">
  countPrefix(prefix: string, limit: number): Promise<number>
}

interface TurnstileClient {
  verify(token: string): Promise<boolean>
}

interface RateGate {
  allow(): Promise<boolean> // constant key only; never an IP or fingerprint
}

interface InjectedFetch {
  (input: string, init: { method: string; headers: Record<string, string>; body: string }): Promise<{
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
| JSON nesting | 8 | both validators |
| Per-build upload objects | 64 | Worker staging prefix only |

64 MiB stays the local cap so a full record file can still be previewed.
An 8 MiB Worker isolate budget cannot parse a 53-64 MiB document
(128 MB isolate; see [Memory](#memory)). A full 150 000 + 400 000 hash
payload does not fit on the wire; those contributors use the reviewed-PR
path.

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
    body.ts        // Content-Length + streaming byte counter
    turnstile.ts   // siteverify via injected fetch; omit remoteip
    storage.ts     // R2 adapter behind StagingStore
    rate_limit.ts  // RateGate adapter; constant key only
    privacy.ts     // allowlisted header reads only
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
3. Read the Turnstile token from header `X-Turnstile-Token`. Missing:
   `TURNSTILE_MISSING`. Verify via injected fetch. Failure: `TURNSTILE_FAILED`.
   The token is dropped after the call. `remoteip` is omitted.
4. Enforce the 8 MiB wire cap **before** parse:
   - If `Content-Length` is a valid non-negative integer and is greater
     than 8 MiB, reject (`BODY_TOO_LARGE`) without reading the body.
   - Stream the body through a byte counter. Crossing 8 MiB rejects even
     when `Content-Length` is missing, non-numeric, or too small.
5. Parse bytes: BOM, nesting, UTF-8, JSON, deny unknown fields, typed
   fields, sorted unique hashes, hour-rounded real calendar timestamp,
   identifier rules, ownership shape. Same rules as `crates/upload`.
6. Re-encode canonical bytes (compact, sorted keys). SHA-256 those bytes.
7. Build object key (see [Staging](#staging)). Re-check key components.
8. `get(key)`: if the object exists, return a receipt with
   `idempotent: true`. No second put. No cap increment.
9. `countPrefix(game_id/build_id/, cap)`. If the count is already `cap`,
   reject (`BUILD_CAP_EXCEEDED`). The count is approximate under races;
   that is acceptable (see below).
10. `putIfAbsent` the **canonical** bytes only. Receipt fields are
    manifest-derived plus `idempotent` and `ownership_state: "UNVERIFIED"`.

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
| `TURNSTILE_FAILED` | 403 | siteverify returned unsuccessful |
| `BODY_TOO_LARGE` | 413 | wire cap (Content-Length or stream) |
| `MANIFEST_TOO_LARGE` | 413 | parser JSON byte cap |
| `BYTE_ORDER_MARK` | 400 | UTF-8 BOM |
| `TRAILING_GARBAGE` | 400 | tokens after one JSON value |
| `INVALID_JSON` | 400 | truncated, non-UTF-8, or not JSON |
| `NESTING_TOO_DEEP` | 400 | `{` / `[` depth over 8 |
| `DUPLICATE_KEY` | 400 | repeated object key |
| `UNKNOWN_FIELD` | 400 | field outside the allowlist |
| `MISSING_FIELD` | 400 | required field absent |
| `UNSUPPORTED_SCHEMA_VERSION` | 400 | `schema_version` is not 1 |
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
for identifiers, no `/`, `\`, `:`, NUL, whitespace, `.`, `..`, or a
leading dash-dot mix that could traverse. A hostile `game_id` or
`BuildId` cannot escape its prefix or collide with another prefix.

Stored bytes are the validated canonical manifest only. No request
headers, no server clock, no IP, no Turnstile token.

**90-day expiry:** `worker/config/r2-lifecycle.json` (placeholder bucket).
A test asserts the committed rule is 90 days.

**Access policy (intended, not deployed):**

- The Worker binding can `put` / `get` / `list` prefix counts
- Maintainers with the Cloudflare account can read for review
- No public bucket listing
- No public unauthenticated read URL
- Logpush and analytics must omit `ClientIP` (see [Privacy](#privacy))

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

**Configured only in a future Cloudflare dashboard (not in this repo):**

- A zone-level rate-limit or WAF rule in front of the Worker
- Turnstile sitekey and secret (secret binding `TURNSTILE_SECRET`)
- R2 bucket name, account id, Logpush destination

**Not configured, and must not be added:**

- Counters in R2 / KV / Durable Objects keyed by IP or fingerprint
- Reading hop-identifying or client-hint headers to feed a limiter
- Storing the Turnstile token

## Privacy

The Worker source must not read:

- `CF-Connecting-IP`, `X-Forwarded-For`, `User-Agent`
- `request.cf` or any `cf.*` request property
- Geo or ASN fields

A lint-style test fails if `worker/src/` mentions those tokens.

The stored object is canonical manifest bytes. The HTTP response is
`UploadReceipt` or `ErrorBody`. A runtime test asserts both contain
only manifest-derived fields (plus `accepted`, `idempotent`,
`ownership_state`).

Logging: outcome code, hash-list counts, object-key **prefix**
(`game_id/build_id/`), and `idempotent`. No request body, no headers,
no token, no IP.

**Logpush:** `worker/config/logpush-fields.json` lists Worker trace
fields **without** `ClientIP`. A test or script check fails if that
name appears. Operators who later enable Logpush must use this field
list.

Turnstile: POST to the public siteverify endpoint with `secret` and
`response` only. Do not send `remoteip`. Do not log the token or the
raw siteverify body.

## Memory

A Cloudflare Worker isolate has 128 MB. The wire cap is 8 MiB so
parsing stays well below that:

- Request buffer: 8 MiB
- UTF-8 string copy: 8 MiB
- `JSON.parse` object graph for hex-heavy JSON: about 2-3× the source
  (string data plus array pointers), so about 16-24 MiB
- Canonical re-encode: 8 MiB
- SHA-256 and key strings: small

Worst case around 50 MiB plus isolate overhead, leaving a margin under
128 MB. A 64 MiB local preview document would not be safe to parse here
(64 × 3 is already over 128 MB), which is why the wire cap is separate.

## Shared golden vectors

`schemas/vectors/` holds valid and invalid fixtures. Each file has a
`verdict` (`accept` or `reject`) and an `error_code` (or `null`).
`crates/upload` tests and Worker tests both load every file. Valid
bodies are produced by the Rust crate (canonical bytes). Invalid
bodies are hand-written hostile cases.

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
