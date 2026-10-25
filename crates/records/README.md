# osdb-records

Local-only pipeline-state records for open-shader-db (Simsalabim).

The crate reads a SODB file the user already has and writes JSON
`PipelineStateRecord`s **without DXIL bytecode**. Each shader blob is
hashed (SHA-256, lowercase hex) and discarded. Root signatures are
sanitized (RTS0-only DXBC, or a size-capped raw blob) and stored once
in a file-level table keyed by SHA-256. Matching compares bytecode
hashes to a `HashSet` of local hashes. There is no upload, download,
hosting, or signing.

The implementation is written from this repository's architecture notes
and Microsoft's published SODB SQL schema. It does not reuse code from
SCSKiller.

## API

```rust
use std::collections::HashSet;
use osdb_records::{extract, load, match_local_hashes, BuildIdentity, Sha256};

let file = extract("game.sodb", BuildIdentity::empty())?;
let loaded = load("records.json")?;
let local: HashSet<Sha256> = HashSet::new();
let report = match_local_hashes(&loaded, &local);
```

On-disk JSON is documented in
[docs/RECORD_FORMAT.md](../../docs/RECORD_FORMAT.md)
(`format_version: 1`).

## Security limits

Record files are treated as untrusted:

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_file_bytes` | 64 MiB | Metadata check plus a capped read (`take(max+1)`). Extract uses the same cap and streams JSON. |
| `max_records` | 50 000 | Enforced in the JSON array visitor as soon as the cap is crossed |
| `max_json_nesting` | 8 | Reject deep JSON |
| `max_root_signature_bytes` | 1 MiB | Decoded table values |

Other hardening:

- Regular-file check on metadata and the opened handle
- Unix `O_NONBLOCK` open (FIFOs must not hang the process)
- Symbolic links rejected
- Unknown `format_version` rejected
- Unknown fields and duplicate JSON keys rejected
- Trailing tokens after JSON rejected; trailing whitespace allowed
- UTF-8 BOM rejected
- Lowercase hex only; odd-length hex rejected
- Descriptor table keys must equal SHA-256 of the content
- Record descriptor references must resolve
- `app_key` must be non-empty, unique, and sorted
- Typed errors (`thiserror`); Display text never includes paths
- `#![forbid(unsafe_code)]`; no `unwrap` / `expect` in non-test code

`app_key` is hex of the game-provided SODB key. The tool does not alter
those keys unless `--hash-app-keys` is passed (CLI) / `hash_app_keys`
(library), which replaces each key with SHA-256 of the raw bytes.
`SemanticName` is accepted only as `[A-Za-z_][A-Za-z0-9_]*` (max 128).
The file contains no filesystem paths, no machine identifiers, and no
timestamps as their own fields. `--game-id`, `--version`, and
`--exe-hash` are optional user-supplied labels.

## Memory bound (measured)

**Measured peak RSS** for library extract (release, Linux,
`RUSAGE` / `/proc/self/status` `VmRSS`):

| Case | Peak RSS | Output |
| --- | ---: | ---: |
| 200 pipeline states sharing one 8 MiB root signature | 40 607 744 bytes (38.7 MiB) | 16 891 692 bytes |
| 200 pipeline states sharing one 4000-element input layout | +3 567 616 bytes (3.4 MiB delta) | 1 049 370 bytes |
| 200 MiB of bytecode blobs (25 × 8 MiB) | 32 190 464 bytes (30.7 MiB) | hashes only |

Blobs are hashed with incremental SQLite blob I/O. Descriptors are
interned (`Arc`) in the parser and stored once per SHA-256 in the
record file. Extract streams JSON through a 64 MiB counting writer.
