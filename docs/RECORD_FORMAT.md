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

## `pipeline_content_hash`

`osdb_records::pipeline_content_hash(file, record)` is the SHA-256 of one
pipeline-state record. A later consensus step can agree on this value
(full description plus shader hashes) without agreeing on `app_key`.

The owning `RecordFile` is not an input. `format_version`, `build`, and
the descriptor tables are not hashed. `root_signature`, `input_layout`,
and `stream_out` stay the 64-character lowercase content hashes already
stored on the record (or JSON `null`). Table bodies are not read.

This is **not** the SODB CanonicalHasher `desc_hash` used by coverage
indexes. It is only the encoding below.

### Preimage bytes

The preimage is the UTF-8 bytes of one JSON object, with no BOM, no
whitespace, and no trailing newline. SHA-256 covers those bytes, not
their hex encoding. There is no length prefix and no domain separator.

`osdb_records` writes these bytes with its own encoder. It does not call
`serde_json` to format the preimage, and it does not build a
`serde_json` map, so the crate feature `preserve_order` cannot change a
byte. Keys below are written in UTF-8 lexicographic order on purpose.

1. Delete `app_key` and `version`. No other field is renamed or removed
   except the omissions below. `version` here is `groups.Version` on the
   record, not `build.version` and not a root-signature version code.
2. Object keys are sorted by raw UTF-8 byte order at every object,
   including nested descriptor objects and `bytecode_hashes`. Stage keys
   therefore appear in lexicographic order (`as` before `ps` before
   `vs`), not in spec order.
3. Arrays keep the record's element order. They are not sorted.
   `unresolved_stages` stays in the order stored on the record.
4. `bytecode_hashes` is always present. A stage key is omitted when that
   stage has no hash. The amplification stage key is `as`.
5. `unresolved_stages` is omitted when the array is empty.
6. `root_signature_skip` is omitted when absent. When present it is the
   documented reason string (`unsupported_version`, `invalid`, or
   `unsupported_container`).
7. Every other record field is present. An absent optional value is JSON
   `null` (not omitted).
8. Integers (`i64` fields, including those inside descriptor objects)
   are decimal JSON numbers with no leading zero, fraction, or exponent.
   `0` is `0`. A negative value has a leading `-`. The most negative
   `i64` is `-9223372036854775808`.
9. Finite floats are spelled from the IEEE-754 binary64 bits already
   stored on the record, by the procedure in the next section. A value
   that is an integer in magnitude but stored as `f64` still has a
   fraction: `0.0`, `1.0`, `1.5`. Negative zero is the distinct token
   `-0.0`. This is not `serde_json`'s formatter.
10. Strings escape `"` as `\"` and `\` as `\\`. Solidus is not escaped.
    Each byte in `0x00`..=`0x1F` is `\u00` plus two lowercase hex digits
    (`\n` is `\u000a`, not a short escape). No other byte is escaped.
    The golden vectors need no escapes.
11. A non-finite float (`NaN`, `+Inf`, `-Inf`) on any of the three
    rasterizer fields cannot be encoded. There is no object preimage.
    The hash is SHA-256 of the four bytes `null` (`6e756c6c`), which is
    `74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b`.
    A file the loader accepted does not contain a non-finite float.

Two records that differ only in `app_key` and `version` have the same
preimage and the same hash. Filling a descriptor table does not change
the hash.

### Finite `f64` spelling

The only `f64` fields on a pipeline record are `rasterizer.depth_bias`,
`rasterizer.depth_bias_clamp`, and `rasterizer.slope_scaled_depth_bias`.
The encoder reads those bits. It does not parse a JSON number.

**In-memory value, not the file text.** Extract stores the SQLite REAL,
which is already binary64. A D3D bias is often binary32; widening
(`f32 as f64`) is exact, and that exact binary64 is what extract holds.
The record **file** writer still emits JSON with `serde_json`. The
record **loader** parses those decimals with `serde_json`'s concise
parser, which is not correctly rounded: for some widened values it
returns a binary64 one ulp away from the widened bits. That is a
pre-existing loader limitation (the hash function does not change it).
`pipeline_content_hash` hashes the bits on the struct it is given — the
exact widened value on an extracted record, the loader's value on a
loaded record. An outside tool reproduces the hash from those bits
using the rules below. Re-parsing the decimal with a different
converter does not.

Pinned misread (also the widened golden vector below): f32 bits
`3005d789` widen to f64 bits `3e00baf120000000`. Both this encoder and
the record-file writer spell that value `4.869141201346849e-10`. The
loader parses that token as bits `3e00baf11fffffff`, which this encoder
spells `4.869141201346848e-10`.

Procedure for one finite value. An implementation that follows it and
checks candidates with a correctly rounded round-ties-to-even
binary64 parser (IEEE-754 default) produces the same token.

1. Bits `0x0000000000000000` (`+0.0`) spell `0.0`. Bits
   `0x8000000000000000` (`-0.0`) spell `-0.0`. Stop.
2. Remember the sign. Continue with the absolute value.
3. Integer significand and binary power. `exp` is bits 62..=52,
   `frac` is bits 51..=0.
   - `exp == 0` (subnormal): significand = `frac`, power = `-1074`.
   - otherwise: significand = `frac + 2^52`, power = `exp - 1075`
     (`1075 = 1023 + 52`).
4. Exact decimal digits.
   - power ≥ 0: `N = significand × 2^power`. The value is the integer `N`.
   - power < 0: `N = significand × 5^(-power)`. The value is `N × 10^power`.
   - Write `N` in decimal with no leading zeros. `F` is that digit count.
     `T` is the number of trailing zeros, with `T < F`. Drop them. The
     remaining digits are `d0…d(L-1)` with `d0 ≠ 0`.
   - Scientific exponent `k` is `F - 1` when power ≥ 0, and
     `F - 1 + power` when power < 0. The value is `d0.d1… × 10^k`.
     Dropping trailing zeros does not change `k`.
5. Shortest round trip. For length `L` from 1 through `min(17, digit count)`,
   round the exact digits to `L` digits, half to even:
   - first dropped digit `< 5`: truncate;
   - `> 5`, or `= 5` with any later non-zero digit: round up;
   - `= 5` with every later digit zero: round up if and only if the last
     kept digit is odd.
   - A carry out of `d0` becomes the digit `1` followed by `L - 1` zeros,
     and `k` increases by 1.
   - Spell that candidate (step 6). Accept the smallest `L` whose spelling
     converts back to the original bits, sign bit included. A finite
     binary64 always has such an `L` of at most 17.
6. Spelling of digits `d0…d(L-1)` and exponent `k`. A negative original
   value gets one leading `-`.
   - Plain form when `-4 ≤ k ≤ 15`:
     - `k ≥ 0` and `k ≥ L - 1`: digits, then `k - (L - 1)` zeros, then
       `.0`. Examples: `1.0`, `10.0`, `1000000000000000.0` (`10^15`).
     - `k ≥ 0` and `k < L - 1`: digits `0` through `k`, `.`, the rest.
       Example: `1.5`.
     - `k < 0`: `0.`, then `-k - 1` zeros, then the digits.
       Example: `0.0001` (`10^-4`, `k = -4`).
   - Exponent form otherwise: `d0`, then `.` and the remaining digits
     only when `L > 1`, then `e`, then `+` when `k ≥ 0` and `-` when
     `k < 0`, then `|k|` in decimal with no leading zeros. A one-digit
     significand has no decimal point. Examples: `1e+16`, `1e-5`,
     `1e-7`, `4.869141201346849e-10`.

`10^15` is plain (`1000000000000000.0`). `10^16` is exponent (`1e+16`).
`10^-4` is plain (`0.0001`). `10^-5` and `10^-7` are exponent (`1e-5`,
`1e-7`), for the binary64 values of those decimals (each of those four
tokens round-trips to the bits of the Rust `f64` literal).

### Golden vectors

Each preimage hex decodes to the JSON on the line above it. The SHA-256
is of those decoded bytes. The Rust test
`golden_vectors_match_the_record_format_doc` checks the same strings.

#### Minimal

`app_key` `01`, `version` `0`, no stages, every optional field null.
The same preimage is produced for `app_key` `ff` and `version` `7`.

```json
{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":null,"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}
```

```text
7b22626c656e64223a6e756c6c2c2262797465636f64655f686173686573223a7b7d2c2264657074685f7374656e63696c223a6e756c6c2c226473765f666f726d6174223a6e756c6c2c22666c616773223a6e756c6c2c2269625f73747269705f6375745f76616c7565223a6e756c6c2c22696e7075745f6c61796f7574223a6e756c6c2c226e6f64655f6d61736b223a6e756c6c2c227072696d69746976655f746f706f6c6f6779223a6e756c6c2c22726173746572697a6572223a6e756c6c2c2272656e6465725f7461726765745f666f726d617473223a6e756c6c2c22726f6f745f7369676e6174757265223a6e756c6c2c2273616d706c655f64657363223a6e756c6c2c2273616d706c655f6d61736b223a6e756c6c2c2273747265616d5f6f7574223a6e756c6c2c22766965775f696e7374616e63696e67223a6e756c6c7d
```

SHA-256: `d2bf3066e02ce21bd8e7af9d601299f921793901a6df3aa775fade455884e615`

#### Stages and descriptor references

`vs` / `ps` / `as` bytecode hashes, `root_signature` and `input_layout`
references, `sample_desc.count` 1, `sample_mask` 4294967295, `flags` 0.
`stream_out` stays null. Stage keys are sorted (`as`, `ps`, `vs`).

```json
{"blend":null,"bytecode_hashes":{"as":"3333333333333333333333333333333333333333333333333333333333333333","ps":"2222222222222222222222222222222222222222222222222222222222222222","vs":"1111111111111111111111111111111111111111111111111111111111111111"},"depth_stencil":null,"dsv_format":null,"flags":0,"ib_strip_cut_value":null,"input_layout":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","node_mask":null,"primitive_topology":null,"rasterizer":null,"render_target_formats":null,"root_signature":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sample_desc":{"count":1,"quality":null},"sample_mask":4294967295,"stream_out":null,"view_instancing":null}
```

```text
7b22626c656e64223a6e756c6c2c2262797465636f64655f686173686573223a7b226173223a2233333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333222c227073223a2232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232222c227673223a2231313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131227d2c2264657074685f7374656e63696c223a6e756c6c2c226473765f666f726d6174223a6e756c6c2c22666c616773223a302c2269625f73747269705f6375745f76616c7565223a6e756c6c2c22696e7075745f6c61796f7574223a2262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262626262222c226e6f64655f6d61736b223a6e756c6c2c227072696d69746976655f746f706f6c6f6779223a6e756c6c2c22726173746572697a6572223a6e756c6c2c2272656e6465725f7461726765745f666f726d617473223a6e756c6c2c22726f6f745f7369676e6174757265223a2261616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161222c2273616d706c655f64657363223a7b22636f756e74223a312c227175616c697479223a6e756c6c7d2c2273616d706c655f6d61736b223a343239343936373239352c2273747265616d5f6f7574223a6e756c6c2c22766965775f696e7374616e63696e67223a6e756c6c7d
```

SHA-256: `26809b705494bb1915b52f1e9fe3a36ff9a21ea1eb23cef1137280d8ed79518a`

#### Nested objects, skip reason, unresolved stages

`root_signature_skip` `invalid`, `unresolved_stages` `["vs","ps"]` in
that order (not sorted), one depth-stencil front face, and blend render
target 0. Nested object keys are sorted. The other seven render targets
are JSON `null`.

```json
{"blend":{"alpha_to_coverage_enable":0,"independent_blend_enable":1,"render_targets":[{"blend_enable":0,"blend_op":3,"blend_op_alpha":6,"dest_blend":2,"dest_blend_alpha":5,"logic_op":7,"logic_op_enable":0,"render_target_write_mask":15,"src_blend":1,"src_blend_alpha":4},null,null,null,null,null,null,null]},"bytecode_hashes":{},"depth_stencil":{"back_face":null,"depth_bounds_test_enable":0,"depth_enable":1,"depth_func":2,"depth_write_mask":1,"front_face":{"stencil_depth_fail_op":2,"stencil_fail_op":1,"stencil_func":4,"stencil_pass_op":3,"stencil_read_mask":255,"stencil_write_mask":255},"stencil_enable":0},"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":null,"render_target_formats":null,"root_signature":null,"root_signature_skip":"invalid","sample_desc":null,"sample_mask":null,"stream_out":null,"unresolved_stages":["vs","ps"],"view_instancing":null}
```

```text
7b22626c656e64223a7b22616c7068615f746f5f636f7665726167655f656e61626c65223a302c22696e646570656e64656e745f626c656e645f656e61626c65223a312c2272656e6465725f74617267657473223a5b7b22626c656e645f656e61626c65223a302c22626c656e645f6f70223a332c22626c656e645f6f705f616c706861223a362c22646573745f626c656e64223a322c22646573745f626c656e645f616c706861223a352c226c6f6769635f6f70223a372c226c6f6769635f6f705f656e61626c65223a302c2272656e6465725f7461726765745f77726974655f6d61736b223a31352c227372635f626c656e64223a312c227372635f626c656e645f616c706861223a347d2c6e756c6c2c6e756c6c2c6e756c6c2c6e756c6c2c6e756c6c2c6e756c6c2c6e756c6c5d7d2c2262797465636f64655f686173686573223a7b7d2c2264657074685f7374656e63696c223a7b226261636b5f66616365223a6e756c6c2c2264657074685f626f756e64735f746573745f656e61626c65223a302c2264657074685f656e61626c65223a312c2264657074685f66756e63223a322c2264657074685f77726974655f6d61736b223a312c2266726f6e745f66616365223a7b227374656e63696c5f64657074685f6661696c5f6f70223a322c227374656e63696c5f6661696c5f6f70223a312c227374656e63696c5f66756e63223a342c227374656e63696c5f706173735f6f70223a332c227374656e63696c5f726561645f6d61736b223a3235352c227374656e63696c5f77726974655f6d61736b223a3235357d2c227374656e63696c5f656e61626c65223a307d2c226473765f666f726d6174223a6e756c6c2c22666c616773223a6e756c6c2c2269625f73747269705f6375745f76616c7565223a6e756c6c2c22696e7075745f6c61796f7574223a6e756c6c2c226e6f64655f6d61736b223a6e756c6c2c227072696d69746976655f746f706f6c6f6779223a6e756c6c2c22726173746572697a6572223a6e756c6c2c2272656e6465725f7461726765745f666f726d617473223a6e756c6c2c22726f6f745f7369676e6174757265223a6e756c6c2c22726f6f745f7369676e61747572655f736b6970223a22696e76616c6964222c2273616d706c655f64657363223a6e756c6c2c2273616d706c655f6d61736b223a6e756c6c2c2273747265616d5f6f7574223a6e756c6c2c22756e7265736f6c7665645f737461676573223a5b227673222c227073225d2c22766965775f696e7374616e63696e67223a6e756c6c7d
```

SHA-256: `54ec9d9e5ea052ddb266be20aa9e27840374cba3b2bcffcd8444e6a6cd9baf7d`

#### Finite floats, including negative zero

Rasterizer `depth_bias` is `1.5`, `depth_bias_clamp` is `-0.0`, and
`slope_scaled_depth_bias` is `0.0`.

```json
{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":{"conservative_raster":0,"cull_mode":1,"depth_bias":1.5,"depth_bias_clamp":-0.0,"depth_clip_enable":1,"fill_mode":3,"forced_sample_count":0,"front_counter_clockwise":0,"line_rasterization_mode":0,"slope_scaled_depth_bias":0.0},"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}
```

```text
7b22626c656e64223a6e756c6c2c2262797465636f64655f686173686573223a7b7d2c2264657074685f7374656e63696c223a6e756c6c2c226473765f666f726d6174223a6e756c6c2c22666c616773223a6e756c6c2c2269625f73747269705f6375745f76616c7565223a6e756c6c2c22696e7075745f6c61796f7574223a6e756c6c2c226e6f64655f6d61736b223a6e756c6c2c227072696d69746976655f746f706f6c6f6779223a6e756c6c2c22726173746572697a6572223a7b22636f6e7365727661746976655f726173746572223a302c2263756c6c5f6d6f6465223a312c2264657074685f62696173223a312e352c2264657074685f626961735f636c616d70223a2d302e302c2264657074685f636c69705f656e61626c65223a312c2266696c6c5f6d6f6465223a332c22666f726365645f73616d706c655f636f756e74223a302c2266726f6e745f636f756e7465725f636c6f636b77697365223a302c226c696e655f726173746572697a6174696f6e5f6d6f6465223a302c22736c6f70655f7363616c65645f64657074685f62696173223a302e307d2c2272656e6465725f7461726765745f666f726d617473223a6e756c6c2c22726f6f745f7369676e6174757265223a6e756c6c2c2273616d706c655f64657363223a6e756c6c2c2273616d706c655f6d61736b223a6e756c6c2c2273747265616d5f6f7574223a6e756c6c2c22766965775f696e7374616e63696e67223a6e756c6c7d
```

SHA-256: `e1f347fd625cc1c8755fa8f10cb5dc74da72208bbf501aee594b32586742fae0`

The four preimages above are unchanged by the explicit encoder: they
contain no exponent, and `1.5`, `0.0`, and `-0.0` spell the same way.

#### Exponents, including negative zero

`depth_bias` is the binary64 value of `10^16` (`1e+16`),
`depth_bias_clamp` is the binary64 value of `10^-7` (`1e-7`), and
`slope_scaled_depth_bias` is `-0.0`. Every integer rasterizer field is
`0`.

```json
{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":{"conservative_raster":0,"cull_mode":0,"depth_bias":1e+16,"depth_bias_clamp":1e-7,"depth_clip_enable":0,"fill_mode":0,"forced_sample_count":0,"front_counter_clockwise":0,"line_rasterization_mode":0,"slope_scaled_depth_bias":-0.0},"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}
```

```text
7b22626c656e64223a6e756c6c2c2262797465636f64655f686173686573223a7b7d2c2264657074685f7374656e63696c223a6e756c6c2c226473765f666f726d6174223a6e756c6c2c22666c616773223a6e756c6c2c2269625f73747269705f6375745f76616c7565223a6e756c6c2c22696e7075745f6c61796f7574223a6e756c6c2c226e6f64655f6d61736b223a6e756c6c2c227072696d69746976655f746f706f6c6f6779223a6e756c6c2c22726173746572697a6572223a7b22636f6e7365727661746976655f726173746572223a302c2263756c6c5f6d6f6465223a302c2264657074685f62696173223a31652b31362c2264657074685f626961735f636c616d70223a31652d372c2264657074685f636c69705f656e61626c65223a302c2266696c6c5f6d6f6465223a302c22666f726365645f73616d706c655f636f756e74223a302c2266726f6e745f636f756e7465725f636c6f636b77697365223a302c226c696e655f726173746572697a6174696f6e5f6d6f6465223a302c22736c6f70655f7363616c65645f64657074685f62696173223a2d302e307d2c2272656e6465725f7461726765745f666f726d617473223a6e756c6c2c22726f6f745f7369676e6174757265223a6e756c6c2c2273616d706c655f64657363223a6e756c6c2c2273616d706c655f6d61736b223a6e756c6c2c2273747265616d5f6f7574223a6e756c6c2c22766965775f696e7374616e63696e67223a6e756c6c7d
```

SHA-256: `1cf3ea8044bef955acd52a0f9ed19eabf6a4a1f9742ad3b9e023be859e29e16d`

#### Widened f32, exact bits and the loader's bits

`depth_bias` is f32 `3005d789` widened to f64 `3e00baf120000000`
(`4.869141201346849e-10`). `depth_bias_clamp` is the value the record
loader returns for that token, f64 `3e00baf11fffffff`
(`4.869141201346848e-10`). `slope_scaled_depth_bias` is `0.0`. The hash
is of those in-memory bits.

```json
{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":{"conservative_raster":0,"cull_mode":0,"depth_bias":4.869141201346849e-10,"depth_bias_clamp":4.869141201346848e-10,"depth_clip_enable":0,"fill_mode":0,"forced_sample_count":0,"front_counter_clockwise":0,"line_rasterization_mode":0,"slope_scaled_depth_bias":0.0},"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}
```

```text
7b22626c656e64223a6e756c6c2c2262797465636f64655f686173686573223a7b7d2c2264657074685f7374656e63696c223a6e756c6c2c226473765f666f726d6174223a6e756c6c2c22666c616773223a6e756c6c2c2269625f73747269705f6375745f76616c7565223a6e756c6c2c22696e7075745f6c61796f7574223a6e756c6c2c226e6f64655f6d61736b223a6e756c6c2c227072696d69746976655f746f706f6c6f6779223a6e756c6c2c22726173746572697a6572223a7b22636f6e7365727661746976655f726173746572223a302c2263756c6c5f6d6f6465223a302c2264657074685f62696173223a342e383639313431323031333436383439652d31302c2264657074685f626961735f636c616d70223a342e383639313431323031333436383438652d31302c2264657074685f636c69705f656e61626c65223a302c2266696c6c5f6d6f6465223a302c22666f726365645f73616d706c655f636f756e74223a302c2266726f6e745f636f756e7465725f636c6f636b77697365223a302c226c696e655f726173746572697a6174696f6e5f6d6f6465223a302c22736c6f70655f7363616c65645f64657074685f62696173223a302e307d2c2272656e6465725f7461726765745f666f726d617473223a6e756c6c2c22726f6f745f7369676e6174757265223a6e756c6c2c2273616d706c655f64657363223a6e756c6c2c2273616d706c655f6d61736b223a6e756c6c2c2273747265616d5f6f7574223a6e756c6c2c22766965775f696e7374616e63696e67223a6e756c6c7d
```

SHA-256: `762d76dbfc66de83ea604a198a728bdd8f1b5c711abd98190c55c739cd486332`

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
  packed engine archives (loose DXBC and SODB blobs are hashed; archives
  are unsupported)
- Real game SODBs; engines other than SODB and loose DXBC
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
