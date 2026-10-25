# Pipeline-state record file format

This is the on-disk JSON schema for local pipeline-state records produced by
`osdb-cli records extract` and consumed by `osdb-records`.

**Format version:** `format_version: 1`  
**Scope:** local files on the user's PC. This format is not an upload
payload. There is no network, hosting, or signing in this crate.

**Data licence:** published record files (when that exists later) would be
dedicated to the public domain under [CC0 1.0 Universal](../DATA-LICENSE.md).
Test fixtures in this repository are synthetic and CC0.

## Relationship to ARCHITECTURE.md

[ARCHITECTURE.md](../ARCHITECTURE.md) sketches a logical
`PipelineStateRecord` that also carries a per-record `buildId`. Format
version 1 stores **one** optional [`BuildIdentity`](../ARCHITECTURE.md#buildid)
on the file (`build`), top-level descriptor tables, and a `records` array
of pipeline states.

Each record holds:

- per-stage SHA-256 of `shader_bytecode.Bytecode` (lowercase hex)
- SHA-256 references into the file-level descriptor tables
- the application key and `groups.Version`
- `unresolved_stages` when a `ByteCode_*` key does not resolve

DXIL / bytecode bytes are **never** stored. The extractor hashes each blob
incrementally and discards it. Root signatures are structurally parsed as
D3D12 version 1.0 or 1.1 and **re-serialized canonically**, so only
structured root-signature fields are carried. A DXBC container used as a
root signature must contain only an RTS0 part whose payload is that same
structure; any other part (DXIL, SHEX, SHDR, ILDB, …) rejects the extract.

Signing is **not** part of this format.

## JSON schema (`format_version` 1)

Unknown top-level fields, unknown `build` fields, and unknown record
fields are rejected. Duplicate keys in those objects (including
`bytecode_hashes` and the descriptor tables) are rejected.
Trailing **tokens** after the JSON value are rejected. Trailing ASCII
whitespace is allowed. A UTF-8 BOM (`EF BB BF`) is rejected. Nesting
deeper than 8 is rejected.

```json
{
  "format_version": 1,
  "build": {
    "game_id": "synthetic",
    "version": "1.0.0",
    "exe_hash": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "root_signatures": {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": "010000000000000000000000000000000000000000000000"
  },
  "input_layouts": {},
  "stream_outputs": {},
  "records": [
    {
      "app_key": "01",
      "version": 0,
      "bytecode_hashes": {
        "vs": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
      },
      "unresolved_stages": [],
      "root_signature": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "input_layout": null,
      "depth_stencil": null,
      "blend": null,
      "rasterizer": null,
      "render_target_formats": null,
      "view_instancing": null,
      "stream_out": null,
      "sample_desc": { "count": 1, "quality": null },
      "sample_mask": 4294967295,
      "ib_strip_cut_value": null,
      "primitive_topology": null,
      "dsv_format": null,
      "node_mask": null,
      "flags": 0
    }
  ]
}
```

| Field | Type | Rules |
| --- | --- | --- |
| `format_version` | integer | Must be `1`. Any other value is rejected. |
| `build.game_id` | string or null | Optional. 1–128 ASCII graphic characters. No `/`, `\`, `:`, or whitespace. |
| `build.version` | string or null | Same character rules as `game_id`. |
| `build.exe_hash` | string or null | Optional. 64-character lowercase hex SHA-256. |
| `root_signatures` | object | Map from SHA-256 (64 lowercase hex) to lowercase hex of the **canonical** serialized root signature. Each key must equal the SHA-256 of the decoded value. The loader re-parses every entry and rejects bytes that are not that exact structure. Missing table is treated as empty. Unreferenced entries are rejected. |
| `input_layouts` | object | Map from SHA-256 of compact JSON of the element array to that array. Key must match the content hash. |
| `stream_outputs` | object | Map from SHA-256 of compact JSON of the stream-out object to that object. Key must match the content hash. |
| `records` | array | Sorted by `app_key` (hex string order). Unique, non-empty keys. Cap 50 000. |
| `records[].app_key` | string | Even-length lowercase hex of the SODB application key, or SHA-256 of that key when `--hash-app-keys` is used. |
| `records[].version` | integer | `groups.Version` for that PSO key, or `0` if none. |
| `records[].bytecode_hashes` | object | Keys from `vs`, `ps`, `hs`, `ds`, `gs`, `as`, `ms`, `cs` only. Values are 64-character lowercase hex. Stages are written in that order. |
| `records[].unresolved_stages` | array | Stage names whose `ByteCode_*` key was non-NULL but missing from `shader_bytecode`. Omitted when empty. |
| `records[].root_signature` | string or null | SHA-256 of an entry in `root_signatures`. Must resolve. |
| `records[].input_layout` | string or null | SHA-256 of an entry in `input_layouts`. Must resolve. |
| `records[].stream_out` | string or null | SHA-256 of an entry in `stream_outputs`. Must resolve. |
| other descriptor objects | object or null | Integer / real / text columns stored inline (small). |
| integer fields | integer or null | JSON floats rejected. |
| real fields | finite number | NaN / Inf are rejected at extract so the loader never sees them. |

Output is deterministic: the same SODB bytes produce byte-identical JSON.
Extract encodes JSON in memory up to the 64 MiB cap and writes the complete
document only on success (stdout cannot observe a partial document). File
output (`--out`) is atomic (temp file, fsync, rename).

## Which fields are copied as raw bytes

Only **one** field family is stored as raw bytes (lowercase hex):

| Table field | Source | Why it cannot carry bytecode |
| --- | --- | --- |
| `root_signatures` values | `root_signatures.value` | The blob is parsed as a D3D12 root signature 1.0 or 1.1 (header, parameters, descriptor ranges, static samplers; enums and flags validated; every offset bounds-checked; every byte accounted for). The extract emits the **canonical re-serialization** of the parsed fields, never the input bytes. If the blob starts with `DXBC`, the container must contain only an RTS0 part (plus the standard checksum header); the RTS0 payload is parsed the same way. Any other part (DXIL, SHEX, SHDR, ILDB, STAT, …), a malformed container, a gap/trailing byte, or a non-canonical leftover fails the extract with a typed error that names no content. Size cap: 1 MiB by default. |

Every other descriptor is structured JSON of INTEGER / REAL / TEXT columns
from the published SQL schema. None of those columns is a bytecode BLOB:

- `input_layouts` — `input_element_descs` integers plus `SemanticName`
  (validated as `[A-Za-z_][A-Za-z0-9_]*`, max 128)
- `stream_outputs` — `stream_out_descs` integers plus `so_declarations`
  (same `SemanticName` rule)
- inline `depth_stencil`, `blend`, `rasterizer`, `render_target_formats`,
  `view_instancing` — integers and finite floats only

`bytecode_hashes` stores SHA-256 of `shader_bytecode.Bytecode`, never the
bytes. `RecordFile`'s `Debug` implementation redacts descriptor hex
(prints key + length only).

## Privacy

The tool does **not** rewrite game-provided application keys by default.
`app_key` is lowercase hex of the SODB `Key` blob. A game that stored a
path-shaped key will see that key's hex in the file. `--hash-app-keys`
replaces each key with the SHA-256 of the raw key bytes (64 hex chars).
The hash is **unsalted**: low-entropy keys can be guessed by brute force.
Hashing hides the original bytes and makes them unrecoverable; matching or
debugging by the original key is then impossible.

`SemanticName` is validated as an identifier and rejected (without
echoing the value) when it is not. Filesystem paths, home directories,
and machine identifiers are not collected as their own fields. The file
contains no install paths, no machine identifiers, and no timestamps.

## Verified versus unverified

**Verified against the published SODB SQL schema**
([State Object Database](https://microsoft.github.io/DirectX-Specs/d3d/StateObjectDatabase.html#sql-database-schema)):

- `pipeline_states` foreign keys `ByteCode_VS` … `ByteCode_CS` and
  integer columns `SampleDesc_Count`, `SampleDesc_Quality`, `SampleMask`,
  `IBStripCutValue`, `PrimitiveTopology`, `DSVFormat`, `NodeMask`, `Flags`
- `root_signatures.value` is a BLOB (sanitized, then stored once in the
  file-level table)
- Column names and INTEGER/REAL/TEXT types for `depth_stencil_descs`,
  `depth_stencil_op_descs`, `blend_descs`, `render_target_blend_descs`,
  `rasterizer_descs`, `render_target_formats`, `view_instancing_descs`,
  `stream_out_descs`, `so_declarations`, `input_element_descs`

### Unverified

- Whether these SQL column values are sufficient to rebuild a game-usable
  SODB (descriptor row `Key` blobs are not preserved)
- Whether bytecode hash alone is enough to match shaders extracted from
  installed game files (this slice hashes blobs already inside a local SODB)
- Real game SODBs; non-SODB engines
- `state_objects` / raytracing / work graphs (not extracted in format 1)
- Mapping of INTEGER enum columns to D3D12 C struct field names beyond
  the SQL identifiers

## Loader limits

Record files are untrusted:

| Limit | Default | Purpose |
| --- | ---: | --- |
| File size | 64 MiB | Reject huge payloads before JSON parse; extract uses the same cap |
| Record count | 50 000 | Enforced in the JSON array visitor as soon as the cap is crossed |
| JSON nesting | 8 | Reject deeply nested documents |
| Root-signature bytes | 1 MiB | Enforced when decoding `root_signatures` table values |

The loader opens only regular files. On Unix the file is opened
`O_NONBLOCK` and the opened handle is checked with `is_file`. Symbolic
links, directories, and FIFOs are rejected. Typed errors never include
filesystem paths. Larger record or file caps are configurable in the
library only (`load_with_limits`); the CLI uses the defaults.

Table keys must equal the SHA-256 of the content. Every
`root_signature` / `input_layout` / `stream_out` reference on a record
must resolve. `app_key` values must be non-empty, unique, and sorted.

## Matching

`osdb_records::match_local_hashes` is a pure function. Given a loaded
`RecordFile` and a `HashSet<Sha256>` of local bytecode hashes it reports
each record as:

- **fully covered** — the record has at least one resolved stage hash,
  every resolved stage hash is in the set, and `unresolved_stages` is
  empty
- **partially covered** — at least one resolved stage hash is present and
  at least one resolved hash is absent or a stage is unresolved
- **missing shaders** — no resolved stage hash is present locally
  (including empty records)

Matching ignores which stage a hash came from. The local set is untyped:
a hash present locally covers a record stage if and only if that stage's
stored hash equals it, regardless of whether the local copy was a VS, PS,
or any other stage.

A NULL `ByteCode_*` column is an absent stage. A non-NULL key that is
missing from `shader_bytecode` is listed in `unresolved_stages` and
prevents `FullyCovered`.
