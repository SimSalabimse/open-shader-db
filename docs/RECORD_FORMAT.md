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
on the file (`build`) and a `records` array of pipeline states.

Each record holds:

- per-stage SHA-256 of `shader_bytecode.Bytecode` (lowercase hex)
- state descriptors taken from the published SODB SQL schema
- the application key and `groups.Version`

DXIL / bytecode bytes are **never** stored. The extractor hashes each blob
incrementally and discards it.

Signing is **not** part of this format.

## JSON schema (`format_version` 1)

Unknown top-level fields, unknown `build` fields, and unknown record
fields are rejected. Duplicate keys in those objects are rejected.
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
  "records": [
    {
      "app_key": "01",
      "version": 0,
      "bytecode_hashes": {
        "vs": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      "root_signature": "00",
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
| `records` | array | Sorted by `app_key` (byte order of the decoded key). Cap 50 000. |
| `records[].app_key` | string | Even-length lowercase hex of the SODB application key. |
| `records[].version` | integer | `groups.Version` for that PSO key, or `0` if none. |
| `records[].bytecode_hashes` | object | Keys from `vs`, `ps`, `hs`, `ds`, `gs`, `as`, `ms`, `cs` only. Values are 64-character lowercase hex. Stages are written in that order. |
| `records[].root_signature` | string or null | Lowercase hex of `root_signatures.value` (not DXIL). |
| descriptor objects | object or null | Integer / real / text columns from the matching SODB table. |
| integer fields | integer or null | JSON floats rejected. |

Output is deterministic: the same SODB bytes produce byte-identical JSON.

The file never contains game install paths, machine identifiers, or
timestamps.

## Verified versus unverified

**Verified against the published SODB SQL schema**
([State Object Database](https://microsoft.github.io/DirectX-Specs/d3d/StateObjectDatabase.html#sql-database-schema)):

- `pipeline_states` foreign keys `ByteCode_VS` … `ByteCode_CS` and
  integer columns `SampleDesc_Count`, `SampleDesc_Quality`, `SampleMask`,
  `IBStripCutValue`, `PrimitiveTopology`, `DSVFormat`, `NodeMask`, `Flags`
- `root_signatures.value` is a BLOB (emitted as hex)
- Column names and INTEGER/REAL/TEXT types for `depth_stencil_descs`,
  `depth_stencil_op_descs`, `blend_descs`, `render_target_blend_descs`,
  `rasterizer_descs`, `render_target_formats`, `view_instancing_descs`,
  `stream_out_descs`, `so_declarations`, `input_element_descs`

**UNVERIFIED**

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
| File size | 64 MiB | Reject huge payloads before JSON parse; the read is also capped |
| Record count | 50 000 | Enforced in the JSON array visitor as soon as the cap is crossed |
| JSON nesting | 8 | Reject deeply nested documents |

The loader opens only regular files. On Unix the file is opened
`O_NONBLOCK` and the opened handle is checked with `is_file`. Symbolic
links, directories, and FIFOs are rejected. Typed errors never include
filesystem paths. Larger record or file caps are configurable in the
library only (`load_with_limits`); the CLI uses the defaults.

## Matching

`osdb_records::match_local_hashes` is a pure function. Given a loaded
`RecordFile` and a `HashSet<Sha256>` of local bytecode hashes it reports
each record as:

- **fully covered** — every referenced stage hash is in the set (or the
  record has no stage hashes)
- **partially covered** — at least one stage hash is present and at least
  one is absent
- **missing shaders** — the record references hashes and none are present
