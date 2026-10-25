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
incrementally and discards it. Root signatures are copied only after
structural validation and canonical re-serialization (DXC
`SimpleSerializer` order of the D3D12 1.0 / 1.1 blob, identical to an
RTS0 part payload). A DXBC container may also hold other parts (a root
signature embedded in a shader); only the RTS0 payload is parsed and
other parts are ignored and never copied. Two RTS0 parts, a container
with no RTS0, or a structurally invalid blob do **not** abort the
extract: that record stores `root_signature: null` plus a
`root_signature_skip` reason code. Version 1.2 (`Version = 3`,
`D3D12_ROOT_SIGNATURE_DESC2`) is `unsupported_version` — DXC has no
serializer for that layout, so this tool does not guess it.

Signing is **not** part of this format. Record files are untrusted input:
any consumer that accepts records from others (the upload server, the
consensus builder, the signing step) MUST re-validate every record with
the same parser.

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
    "eaf7ecdeba4ab59a9e3f0dbd5e1aef2e4f6f143cabf1df4217159ed1045a7206": "010000000000000018000000000000001800000000000000"
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
      "root_signature": "eaf7ecdeba4ab59a9e3f0dbd5e1aef2e4f6f143cabf1df4217159ed1045a7206",
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
| `root_signatures` | object | Map from SHA-256 (64 lowercase hex) to lowercase hex of the **canonical** serialized root signature. Each key must equal the SHA-256 of the decoded value. The loader re-parses every entry and rejects bytes that are not that exact structure (`serialize()` of the parse must equal the stored bytes). Missing table is treated as empty. Unreferenced entries are rejected. |
| `input_layouts` | object | Map from SHA-256 of compact JSON of the element array to that array. Key must match the content hash. |
| `stream_outputs` | object | Map from SHA-256 of compact JSON of the stream-out object to that object. Key must match the content hash. |
| `records` | array | Sorted by `app_key` (hex string order). Unique, non-empty keys. Cap 50 000. |
| `records[].app_key` | string | Even-length lowercase hex of the SODB application key, or SHA-256 of that key when `--hash-app-keys` is used. |
| `records[].version` | integer | `groups.Version` for that PSO key, or `0` if none. |
| `records[].bytecode_hashes` | object | Keys from `vs`, `ps`, `hs`, `ds`, `gs`, `as`, `ms`, `cs` only. Values are 64-character lowercase hex. Stages are written in that order. |
| `records[].unresolved_stages` | array | Stage names whose `ByteCode_*` key was non-NULL but missing from `shader_bytecode`. Omitted when empty. |
| `records[].root_signature` | string or null | SHA-256 of an entry in `root_signatures`. Must resolve when non-null. Null when the PSO had no root signature or the referenced blob was skipped. |
| `records[].root_signature_skip` | string | Omitted unless the referenced blob was skipped. JSON `null` is rejected. Documented codes only: `unsupported_version` (well-formed 24-byte header whose version is not 1.0/1.1), `invalid` (truncated, garbage, or structurally invalid), `unsupported_container`. Never includes bytes or content. Must not be set together with a `root_signature` hash. |
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

| Table field | Source | What is stored |
| --- | --- | --- |
| `root_signatures` values | `root_signatures.value` | Copied only after structural validation and canonical re-serialization of D3D12 root signature 1.0 (`Version = 1`) or 1.1 (`Version = 2`) as DXC / `D3D12SerializeRootSignature` write it (same bytes as a DXBC RTS0 part payload). Layout: 24-byte little-endian header (`Version`, `NumParameters`, `RootParametersOffset`, `NumStaticSamplers`, `StaticSamplersOffset`, `Flags`); then `NumParameters` × 12-byte `{ParameterType, ShaderVisibility, PayloadOffset}` records at `RootParametersOffset`; then type-specific payloads at each absolute `PayloadOffset` (descriptor table = 8-byte `{NumDescriptorRanges, DescriptorRangesOffset}` plus 20-byte 1.0 or 24-byte 1.1 ranges; 32-bit constants = 12 bytes; CBV/SRV/UAV = 8 bytes in 1.0 or 12 in 1.1); then 52-byte static samplers at `StaticSamplersOffset`. Every offset and count is bounds-checked; spans must not overlap; every byte of the blob must be exactly the header + parameter array + payloads + ranges + samplers (no gaps, no trailing bytes). Parameter type, shader visibility, range type, filter (including Agility `MINIMUM`/`MAXIMUM` `MIN_MAG_ANISOTROPIC_MIP_POINT` `0x54` / `0xd4` / `0x154` / `0x1d4`), address mode, and border color (including `OpaqueWhiteUint = 4`) are allow-listed. `ComparisonFunc` is checked only for `COMPARISON_*` filters (`StaticSamplerVerifier::Verify`). Root-signature flags must be a subset of DXC `ValidFlags` `0x80000fff` (bits 0–11 plus `0x80000000`; official `CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED` / `SAMPLER_HEAP_DIRECTLY_INDEXED` are `0x400` / `0x800`). Version 1.1 range flags ⊆ `0x1000f`; root-descriptor flags ⊆ `0xe`. Sampler `MinLOD` / `MaxLOD` follow `StaticSamplerVerifier::Verify`: every non-NaN value is accepted, including ±INF and the D3D11 default `-FLT_MAX`; `MipLODBias` ∈ `[-16, 15.99]` (NaN and INF rejected). `-0.0` is canonicalised to `+0.0`. Binding rules match DXC `DescriptorTableVerifier::Verify` and `RootSignatureVerifier::AddRegisterRange` / `VerifyRootSignature`: `NumDescriptors != 0` (except `UINT_MAX` unbounded), reserved register spaces `≥ 0xfffffff0` rejected, `base + num` overflow rejected, overlapping register bindings in the same space and visibility rejected (shader visibility `ALL` overlaps a specific visibility). Caps: ≤ 64 parameters, ≤ 1024 ranges per table, ≤ 4096 ranges total, ≤ 2048 static samplers, ≤ 1 MiB. The extract emits the **canonical DXC-order re-serialization**, never the input bytes. The loader re-parses every table value and rejects it unless `serialize() == stored`. If the blob starts with `DXBC`, only the RTS0 part is extracted; other parts are ignored and never copied. Two RTS0 parts, no RTS0 part, or a structurally invalid payload skip that record (`root_signature: null` plus `invalid`). An unwalkable DXBC header skips as `unsupported_container`. Version 1.2 (`Version = 3`) skips as `unsupported_version`. One skipped signature does not abort the extract; the extractor prints the unique skipped-key count to stderr only after a successful write. **Free-form capacity:** register indices / spaces, non-zero range counts, table offsets, leftover root-parameter numeric fields, the three sampler floats (LOD any non-NaN; bias range-checked), and `ComparisonFunc` for every non-`COMPARISON_*` filter can still hold arbitrary in-range numbers. Worst case after the verifier rules is **115 168 loose bytes** per signature (4096 ranges × 16 + 60 leftover parameters × 8 + 2048 samplers × 24) inside a structured blob of at most 206 344 bytes (24 + 64×12 + 4×(8+1024×24) + 60×12 + 2048×52). This is not shader bytecode, and a normal file never contains such a payload, but the format cannot rule out an adversarial file. Any consumer that accepts records from others (the upload server, the consensus builder, the signing step) MUST re-validate records with the same parser and treat them as untrusted. |

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
