# osdb-records

Local-only pipeline-state records for open-shader-db (Simsalabim).

The crate reads a SODB file the user already has and writes JSON
`PipelineStateRecord`s **without DXIL bytecode**. Each shader blob is
hashed (SHA-256, lowercase hex) and discarded. Matching compares those
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
| `max_file_bytes` | 64 MiB | Metadata check plus a capped read (`take(max+1)`) |
| `max_records` | 50 000 | Enforced in the JSON array visitor as soon as the cap is crossed |
| `max_json_nesting` | 8 | Reject deep JSON |

Other hardening:

- Regular-file check on metadata and the opened handle
- Unix `O_NONBLOCK` open (FIFOs must not hang the process)
- Symbolic links rejected
- Unknown `format_version` rejected
- Unknown fields and duplicate JSON keys rejected
- Trailing tokens after JSON rejected; trailing whitespace allowed
- UTF-8 BOM rejected
- Lowercase hex only
- Typed errors (`thiserror`); Display text never includes paths
- `#![forbid(unsafe_code)]`; no `unwrap` / `expect` in non-test code

Output JSON contains no filesystem paths, no machine identifiers, and
no timestamps. `--game-id`, `--version`, and `--exe-hash` are optional
user-supplied labels.

## Memory bound (measured)

**Measured peak RSS** for `osdb-cli records extract` (release, Linux)
on a synthetic SODB with **200 MiB of bytecode blobs** (25 × 8 MiB;
database file 209 956 864 bytes):

**49 405 952 bytes (48 248 KiB, 47.1 MiB)** —
`RUSAGE_CHILDREN.ru_maxrss` of the extract process. Blobs are hashed
with incremental SQLite blob I/O and are not retained.
