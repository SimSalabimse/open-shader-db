# Coverage index file format

This is the on-disk JSON schema for M0 coverage indexes consumed by
`osdb-coverage` and `osdb-cli coverage`.

**Format version:** `format_version: 1`  
**Data licence:** published index files are dedicated to the public domain
under [CC0 1.0 Universal](../DATA-LICENSE.md). Test fixtures in this
repository are synthetic and CC0.

## Relationship to ARCHITECTURE.md

[ARCHITECTURE.md](../ARCHITECTURE.md) defines the logical `CoverageIndex`
and `DescriptionHash` records (including bytecode hashes and app keys).
For M0 comparison the client only needs description hashes. The smallest
consistent on-disk reading is therefore:

- one index file describes **one** [`BuildId`](../ARCHITECTURE.md#buildid)
- `descriptions[].descHash` is stored as a single sorted, unique `hashes`
  array of 64-character lowercase hex SHA-256 values
- those hashes are exactly the description hashes produced by the `sodb`
  crate (`HashedObject.desc_hash`)
- full `DescriptionHash` objects (bytecode hashes, app keys, object type)
  are not present in format version 1

Signing is **not** part of this format. Local unsigned files are allowed.
The CLI always states that the index is unsigned and not verified.
Signature verification is [M4 Signed releases](../ROADMAP.md#m4-signed-releases).

## JSON schema (`format_version` 1)

Unknown top-level fields and unknown `build_id` fields are rejected.
Duplicate keys in any object are rejected. Trailing **tokens** after the
JSON value are rejected. Trailing ASCII whitespace is allowed. A UTF-8
BOM (`EF BB BF`) is rejected. Nesting deeper than 8 is rejected. JSON
`\u` escapes in strings (including hashes) are accepted and decoded
before hex validation.

```json
{
  "format_version": 1,
  "build_id": {
    "game_id": "synthetic",
    "version": "1.0.0",
    "exe_hash": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "hashes": [
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  ],
  "upload_count": 3,
  "consensus_count": 2,
  "generated_at": "2026-10-25T00:00:00Z"
}
```

| Field | Type | Rules |
| --- | --- | --- |
| `format_version` | integer | Must be `1`. Any other value is rejected. |
| `build_id.game_id` | string | 1–128 ASCII graphic characters. No `/`, `\`, `:`, or whitespace. |
| `build_id.version` | string | Same character rules as `game_id`. |
| `build_id.exe_hash` | string | 64-character lowercase hex SHA-256 of the game executable. |
| `hashes` | array of strings | Each entry is 64-character lowercase hex SHA-256. Sorted and deduplicated at load. |
| `upload_count` | integer | Inclusive range `0`..=`2^64-1`. Informational. Floats rejected. |
| `consensus_count` | integer | Inclusive range `0`..=`2^64-1`. Informational. Floats rejected. |
| `generated_at` | string | UTC `YYYY-MM-DDTHH:MM:SSZ` only. Month 1–12, day valid for the month (leap years), hour 0–23, minute and second 0–59. Offsets (`Z` only) are rejected. |

Hash encoding matches `sodb::Sha256Hash::to_hex()` (lowercase, no `0x` prefix).

## Loader limits

Index files are untrusted (they will later arrive from the network):

| Limit | Default | Purpose |
| --- | ---: | --- |
| File size | 64 MiB | Reject huge payloads before JSON parse; the read is also capped |
| Hash count | 1 000 000 | Enforced in the JSON array visitor as soon as the cap is crossed |
| JSON nesting | 8 | Reject deeply nested documents |

The loader opens only regular files. On Unix the file is opened
`O_NONBLOCK` and the opened handle is checked with `is_file`. Symbolic
links, directories, and FIFOs are rejected. Typed errors never include
filesystem paths. Larger hash or file caps are configurable in the
library only (`load_with_limits`); the CLI uses the defaults.

Peak RSS for a default-cap load via the release CLI was measured at
**103 845 888 bytes (99.0 MiB)** on Linux. See
[crates/coverage/README.md](../crates/coverage/README.md).

## Comparison

Local description hashes are the unique sorted `desc_hash` values from
`sodb::open` (`pipeline_states` and `state_objects`). The index still
carries a typed `build_id`. The SODB format has no build identifier
yet, so the report sets `build_identity` to `not_compared` and does
not warn about a mismatch.
