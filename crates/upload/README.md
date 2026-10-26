# osdb-upload

Local anonymized upload-manifest types and preview for open-shader-db
(Simsalabim).

The crate builds an [`UploadManifest`] from a `records extract` file
and shows a preview of the **exact bytes** that would later be sent.
There is **no network client**, no HTTP crate, and no submit command.
The Worker in `worker/` validates the same rules (built and tested,
not deployed).

The implementation is written from this repository's architecture notes.
It does not reuse code from SCSKiller.

## API

```rust
use osdb_records::RecordFile;
use osdb_upload::{ManifestBuilder, UploadPreview, FixedClock};

let records = RecordFile::load("records.json")?;
let manifest = ManifestBuilder::new(&records)
    .game_id(osdb_upload::GameId::parse("synthetic")?)
    .clock(&clock)?
    .build()?;
let preview = UploadPreview::from_manifest(&manifest)?;
// preview.canonical_bytes() is the payload; nothing is sent
```

On-wire JSON is documented in
[docs/UPLOAD_MANIFEST.md](../../docs/UPLOAD_MANIFEST.md)
(`schema_version` 1 and 2; `prepare` emits 2). The machine-readable
schemas are
[schemas/upload_manifest.v1.json](../../schemas/upload_manifest.v1.json)
and
[schemas/upload_manifest.v2.json](../../schemas/upload_manifest.v2.json).
A drift test fails if a hand-maintained schema and its file disagree.

## What is hashed

From format-1 record files, the builder copies only:

- `bytecode_hashes` — unique SHA-256 values already stored on
  `records[].bytecode_hashes`
- `description_hashes` — unique SHA-256 values already stored on
  `records[].root_signature`, `input_layout`, and `stream_out`
- `pipeline_hashes` — one `osdb_records::pipeline_content_hash` per
  record (sorted unique). `app_key` and `version` are not inputs.
  Descriptor references stay content hashes.

`app_key`, inline integer descriptors, root-signature bytes, paths, and
Steam install paths are not copied into the payload.
`pipeline_content_hash` is not the SODB CanonicalHasher description
hash; that gap is documented and UNVERIFIED for consensus. Version 1
manifests still parse and carry no `pipeline_hashes`.

## Security limits

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_json_bytes` | 64 MiB | Local preview. Max valid v2 is bounded at 40 800 349 bytes. |
| `WIRE_MAX_JSON_BYTES` | 8 MiB | Worker request body, before parse. A full v2 does not fit. |
| `max_description_hashes` | 150 000 | `50 000` records × 3 description keys |
| `max_bytecode_hashes` | 400 000 | `50 000` records × 8 bytecode stages |
| `max_pipeline_hashes` | 50 000 | one `pipeline_content_hash` per record |
| `max_json_nesting` | 8 | Reject deep JSON. Version 2 adds no nesting level. |

Other hardening:

- Unknown fields and duplicate JSON keys rejected
- Trailing tokens after JSON rejected
- UTF-8 BOM rejected
- Lowercase hex only
- `collected_hour` accepts only `YYYY-MM-DDTHH:00:00Z`
- Typed errors (`thiserror`); Display text never includes paths or bytes
- `#![forbid(unsafe_code)]`; no `unwrap` / `expect` in non-test code

Ownership is `unchallenged` / `no_server_challenge` until a server
nonce exists. That field proves nothing. UNVERIFIED.
