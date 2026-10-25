# Upload manifest format

This is the on-wire JSON schema for the anonymized upload payload
produced by `osdb-cli upload prepare` and parsed by `osdb-upload`.

**Schema version:** `schema_version: 1`  
**Scope:** local preview and (later) Worker ingest. This crate does
not send bytes. There is no HTTP client in this repository.

**Data licence:** published staging objects (when that exists later)
would be dedicated to the public domain under
[CC0 1.0 Universal](../DATA-LICENSE.md). Test fixtures in this
repository are synthetic and CC0.

## Relationship to ARCHITECTURE.md

[ARCHITECTURE.md](../ARCHITECTURE.md) sketches a logical
`UploadManifest` with `descriptions` objects and a string
`proofOfOwnership`. Schema version 1 is the allowlist that the client
and the future Worker both check:

- one integer `schema_version`
- one structured `build_id` (`game_id`, `version`, `exe_hash`)
- `client_version` (`major.minor.patch`)
- `collected_hour` (UTC, minutes and seconds always `00`)
- two sorted unique SHA-256 arrays: `description_hashes`,
  `bytecode_hashes`
- `ownership` (versioned object; see [Proof of ownership](#proof-of-ownership))

The payload never includes a game path, username, IP, hostname,
hardware id, `app_key`, or a full timestamp.

## JSON schema (`schema_version` 1)

Unknown top-level fields, unknown `build_id` fields, and unknown
`ownership` fields are rejected. Duplicate keys are rejected.
Trailing tokens after the JSON value are rejected. A UTF-8 BOM
(`EF BB BF`) is rejected. Nesting deeper than 8 is rejected.

The machine-readable schema is
[schemas/upload_manifest.v1.json](../schemas/upload_manifest.v1.json).
`osdb-upload` builds the same document from its Rust types. A drift
test fails if either side changes.

Canonical encoding is compact JSON with object keys sorted
lexicographically. The same validated input always produces the same
bytes. The preview and `--out` file are those bytes.

```json
{
  "bytecode_hashes": [
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  ],
  "build_id": {
    "exe_hash": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    "game_id": "synthetic",
    "version": "1.0.0"
  },
  "client_version": "0.1.0",
  "collected_hour": "2026-10-25T14:00:00Z",
  "description_hashes": [
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  ],
  "ownership": {
    "kind": "unchallenged",
    "reason": "no_server_challenge"
  },
  "schema_version": 1
}
```

| Field | Type | Rules |
| --- | --- | --- |
| `schema_version` | integer | Must be `1`. |
| `build_id.game_id` | string | 1–128 ASCII graphic characters. No `/`, `\`, `:`, or whitespace. |
| `build_id.version` | string | Same character rules as `game_id`. |
| `build_id.exe_hash` | string | 64-character lowercase hex SHA-256. |
| `client_version` | string | `major.minor.patch` of ASCII digits. |
| `collected_hour` | string | `YYYY-MM-DDTHH:00:00Z` only. The constructor floors a clock instant to the UTC hour and discards the original instant. |
| `description_hashes` | array | Sorted unique 64-character lowercase hex. Cap 50 000. |
| `bytecode_hashes` | array | Same rules as `description_hashes`. |
| `ownership` | object | `unchallenged` or `exe_binding_v1` (see below). |

## Which record fields become hashes

`ManifestBuilder` reads a format-1 [`RecordFile`](RECORD_FORMAT.md)
and copies only hashes that the record file already stores:

| Manifest array | Record fields used |
| --- | --- |
| `bytecode_hashes` | `records[].bytecode_hashes` stage values (`vs` … `cs`) |
| `description_hashes` | `records[].root_signature`, `input_layout`, `stream_out` |

The builder does not read `app_key`, `version`, `unresolved_stages`,
`root_signature_skip`, inline integer descriptors, or table values
(root-signature bytes, input-layout objects, stream-out objects).
It never opens shader bytecode.

### What format-1 records do not carry

The SODB parser's CanonicalHasher description hash (used by coverage
indexes) is **not** stored on a format-1 record. This crate does not
invent a substitute hash of the inline descriptors. Consensus that
needs those SODB description hashes is **UNVERIFIED** until a later
record format or a documented derivation exists.

Steam discovery (`osdb-steam`) can supply a `GameIdentity`. That
identity is upload-safe (no paths) but is still **not** copied into
the manifest: Steam's depot build id is a different type from
`build_id`, and the display name is not a game id.

## Preview

`UploadPreview` is rendered from the canonical bytes, not from a
second struct. The text lists every JSON field that will be sent
(with array counts), states that those exact bytes will be sent,
states that nothing is sent by this command, and then prints the
full JSON. `--out` writes the same bytes.

## Proof of ownership

`ownership` is a versioned enum so the Worker can validate one schema.

### `unchallenged`

Emitted by `osdb-cli upload prepare` today. Fields:

- `kind`: `unchallenged`
- `reason`: `no_server_challenge`

There is no ingest server and no nonce. The client must still emit a
well-formed manifest, so the missing-challenge state is explicit.
The preview prints that this field proves nothing.

### `exe_binding_v1`

Defined for issues #12 (Worker) and #16 (server-side checks). Fields:

- `kind`: `exe_binding_v1`
- `challenge`: 64 lowercase hex (server nonce, 32 bytes)
- `response`: 64 lowercase hex

Expected flow (not implemented; no server exists):

1. Worker issues a fresh 32-byte nonce (the challenge).
2. Client streams the installed game executable and computes
   SHA-256(`challenge` bytes || exe file bytes). The exe bytes are
   never stored, logged, or placed in the manifest.
3. Worker verifies `response` against a copy of that exe (or an
   equivalent precomputed function of the same bytes and nonce).

An attacker who does not have the game files cannot produce a valid
`response` for a fresh nonce **if** the server verifies against the
real exe bytes. That property is **UNVERIFIED**: there is no server,
no nonce issuer, no exe catalog, and no test that a party without
the exe fails verification.

`SHA-256(nonce || exe_hash)` is **not** this design. Anyone who
knows the published `build_id.exe_hash` could forge that value.

The prepare command never reads an exe and never emits
`exe_binding_v1`.

## UNVERIFIED

These claims are **not** made by the code or tests:

- That `unchallenged` ownership resists Sybil uploads (it does not)
- That `exe_binding_v1` will be verified by a server, or that the
  project will host exe bytes to verify against
- That SHA-256(nonce || exe bytes) is a sufficient anti-poisoning
  control
- That `description_hashes` from record descriptor keys match SODB
  CanonicalHasher hashes or a future consensus index
- Real game record files, real Steam installs, or a network path
- Cloudflare ingest, Turnstile, rate limits, or IP-log omission
  (those belong to later M2 issues)

## Privacy

The following must not appear in the struct, the JSON, the preview,
error `Display` / `Debug`, or CLI stderr:

- game install paths
- usernames
- IP addresses
- hostnames
- hardware ids
- full timestamps (minutes and seconds)

`collected_hour` is the only time field and is always an hour
boundary. Typed errors never echo rejected field values.

## Limits

| Limit | Default | Purpose |
| --- | ---: | --- |
| JSON bytes | 4 MiB | Reject huge payloads before or after encode |
| Hashes per list | 50 000 | Matches the record-file record cap |
| JSON nesting | 8 | Reject deep documents |

Larger caps are library-only.

## CLI

```
osdb-cli upload prepare --records <file> --build-id <id>
osdb-cli upload prepare --records <file> --build-id <id> --version <ver> --exe-hash <hex>
osdb-cli upload prepare --records <file> --build-id <id> --out manifest.json --yes-i-reviewed [--force]
```

`--build-id` is the `game_id`. `version` and `exe_hash` come from
the record file when present, or from the flags. `--out` requires
`--yes-i-reviewed`. `--force` overwrites an existing regular file.
`--out` uses the same atomic helper as the other commands
(same-file refusal, symlink refusal, temp file, fsync, rename).
The command prints the preview and a notice that nothing is sent.
