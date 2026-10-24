# sodb

Typed parser for Microsoft State Object Database (SODB) files, used by
open-shader-db (Simsalabim).

An SODB is a SQLite database with `PRAGMA user_version = 2` and
`PRAGMA application_id = 0xD3D50DB`. This crate opens those files
**read-only**, reads application identity and object keys, and emits
**SHA-256 hashes only**. DXIL / bytecode bytes are never stored on the
public output types.

The implementation is written from Microsoft's published SODB schema. It
does not reuse code from SCSKiller.

## API

```rust
use sodb::{open, open_with_limits, Limits, SodbSummary};

let summary: SodbSummary = open("game.sodb")?;
// summary.pipeline_states[*].desc_hash
// summary.pipeline_states[*].bytecode_hashes
// summary.shader_bytecode[*].bytecode_hash
// no bytecode fields exist on these types
```

`SodbSummary` contains:

- `app_id` rows (exe / app / engine names and version integers)
- `pipeline_states` and `state_objects` with app-defined keys, object
  versions, description hashes, and referenced bytecode hashes
- `shader_bytecode` keys mapped to SHA-256 of each blob
- `missing_tables` for known optional tables that were absent
- `wal_ignored` when a WAL-mode file was opened and the `-wal` sibling
  next to the opened path is larger than the 32-byte WAL header
  (header-only WAL files are not treated as ignored data)

Unknown `user_version` or `application_id` values return a typed error
and do not parse further. Empty files, non-SQLite magic, directories,
and FIFOs fail with `NotAnSodb` or `NotARegularFile`.

Table and column names live in `schema.rs`. Assumptions that are **not**
in Microsoft's spec (version `0` when `groups` has no row; WAL files
opened with `file:<path>?immutable=1` so unflushed WAL pages are ignored
and no `-wal`/`-shm` files are created; a sibling `-journal` rollback
file is not applied) are documented there.

## Description hash coverage

A description hash covers the object's own non-bytecode columns plus
followed descriptor and association content loaded once into memory:

- PSO: root signature, blend / rasterizer / depth-stencil (including
  `depth_stencil_op_descs` and `render_target_blend_descs`), render
  target formats, view instancing, stream-out plus
  `so_declarations`, and `input_layout_to_input_element_associations`
  → `input_element_descs`
- State object: every `so_to_*` association and the immediate target
  row (root signatures, exports, hit groups, RT config, generic
  programs and their descriptors, work graphs and their nodes)

**Limitation:** `string_associations` is not included. Its
`OwningTableKey` may be NULL and is not a clean per-object foreign key.
Bytecode blobs are never part of a description hash (DXIL library
associations contribute the bytecode **key** only).

## CLI

The `osdb-cli` crate exposes `inspect <file.sodb>`, which prints
`SodbSummary` as JSON.

## Security limits

Files are treated as untrusted:

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_file_bytes` | 256 MiB | Reject huge files before SQLite opens them |
| `max_blob_bytes` | 8 MiB | Cap bytecode and descriptor BLOBs |
| `max_key_bytes` | 4 KiB | Cap application-defined keys |
| `max_text_bytes` | 4 KiB | Cap `app_id` TEXT columns |
| `max_rows_per_table` | 50 000 | Cap rows read per table |
| `max_total_rows` | 500 000 | Total-work budget across tables |

Other hardening:

- Regular-file check on the opened handle (directories and FIFOs rejected)
- SQLite header inspected before `PRAGMA` (empty / non-SODB files)
- WAL databases opened with URI `immutable=1` (no sidecar files created;
  `wal_ignored` is set if the `-wal` next to the opened path has frames)
- A hot rollback journal (`-journal`) is not applied; the main file is
  read as stored
- `SQLITE_LIMIT_LENGTH` set to `max(caps, 1 MiB) + 1` so spec CREATE SQL
  (over 1 KiB) still loads under tiny test caps
- Sizes measured with `octet_length()` (bytes, not UTF-8 characters)
- `SQLITE_OPEN_READ_ONLY` (URI only for WAL `immutable=1`; no extensions)
- `PRAGMA trusted_schema = OFF`
- Groups, descriptor tables, and `so_to_*` associations loaded once and
  reduced to 32-byte sub-hashes (followed by an iterative, memoized walk)
- Typed errors (`thiserror`); no `unwrap` / `expect` in non-test code
- `#![forbid(unsafe_code)]`

Keys are interned once (`u32` ids). Association follows are added only
when the owner actually appears in that table, and follow-miss hashes
are computed inline (not cached). One cycle in the follow graph rejects
the whole file. `sqlite_master` rows are not charged against
`max_total_rows`.

Peak RSS is measured in tests (Linux debug RSS delta, two runs):

- 50k-row shared-layout fixture: **98–104 MiB**
- 50k × 4 KiB keys with 11 empty `so_to_*` tables: **398–408 MiB**
  (under the 512 MiB test bound)

Override defaults with `open_with_limits`.
