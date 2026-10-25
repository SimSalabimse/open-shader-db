# osdb-upload

Local anonymized upload-manifest types and preview for open-shader-db
(Simsalabim).

The crate builds an [`UploadManifest`] from a `records extract` file
and shows a preview of the **exact bytes** that would later be sent.
There is **no network client**, no HTTP crate, and no submit command.
A later Worker (issue #12) will validate the same schema.

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
(`schema_version: 1`). The machine-readable schema is
[schemas/upload_manifest.v1.json](../../schemas/upload_manifest.v1.json).
A drift test fails if the hand-maintained schema and that file disagree.

## What is hashed

From format-1 record files, the builder copies only:

- `bytecode_hashes` — unique SHA-256 values already stored on
  `records[].bytecode_hashes`
- `description_hashes` — unique SHA-256 values already stored on
  `records[].root_signature`, `input_layout`, and `stream_out`

`app_key`, inline integer descriptors, root-signature bytes, paths, and
Steam install paths are not read into the payload. Format-1 records do
**not** store the SODB CanonicalHasher description hash; that gap is
documented and UNVERIFIED for consensus.

## Security limits

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_json_bytes` | 64 MiB | Reject oversized wire JSON (fits two 400 000-hash lists) |
| `max_hashes_per_list` | 400 000 | `50 000` records × 8 bytecode stages |
| `max_json_nesting` | 8 | Reject deep JSON |

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
