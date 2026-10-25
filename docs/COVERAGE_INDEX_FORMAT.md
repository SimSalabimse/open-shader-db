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
Duplicate keys in any object are rejected. Trailing bytes after the JSON
value are rejected. Nesting deeper than 8 is rejected.

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
| `upload_count` | integer | Non-negative. Informational. |
| `consensus_count` | integer | Non-negative. Informational. |
| `generated_at` | string | UTC ISO-8601 `YYYY-MM-DDTHH:MM:SSZ` only. |

Hash encoding matches `sodb::Sha256Hash::to_hex()` (lowercase, no `0x` prefix).

## Loader limits

Index files are untrusted (they will later arrive from the network):

| Limit | Default | Purpose |
| --- | ---: | --- |
| File size | 64 MiB | Reject huge payloads before JSON parse |
| Hash count | 5 000 000 | Bound the in-memory hash vector |
| JSON nesting | 8 | Reject deeply nested documents |

The loader opens only regular files. On Unix the file is opened
`O_NONBLOCK` and the opened handle is checked with `is_file`. Symbolic
links, directories, and FIFOs are rejected. Typed errors never include
filesystem paths.

A 5 million hash `IndexHashes` value (32 bytes per digest, sorted unique
`Vec`) measured **160 395 264 bytes (153.0 MiB)** extra RSS on Linux
debug. See [crates/coverage/README.md](../crates/coverage/README.md).
JSON text for 5 million hex strings cannot fit in the 64 MiB file cap;
the hash-count cap applies when a caller raises the file-size limit or
constructs the vector in memory.

## Comparison

Local description hashes are the unique sorted `desc_hash` values from
`sodb::open` (`pipeline_states` and `state_objects`). If the SODB has an
`app_id` row, its `app_name` and `app_version` are compared to
`build_id.game_id` and `build_id.version`. SODB does not store an
executable content hash, so `exe_hash` is not compared and the check is
skipped when `app_id` is absent.
