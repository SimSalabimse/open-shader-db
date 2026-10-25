# osdb-coverage

Load an untrusted community coverage index and compare it to description
hashes from a local SODB. Used by `osdb-cli coverage` (Simsalabim).
The same binary also provides `inspect` (SODB hashes) and `discover`
(Steam games, counts only in JSON).

The crate is written from this repository's architecture notes. It does
not reuse code from SCSKiller.

## API

```rust
use osdb_coverage::{compare, load, CoverageReport};

let summary = sodb::open("game.sodb")?;
let index = load("index.json")?;
let report: CoverageReport = compare(&summary, &index, 100);
// report holds hashes and counts only — never paths or bytecode
```

`CoverageReport` is `Serialize`. Human output ends with:

`Index: unsigned (signature verification arrives in a later milestone)`

Signature verification is not implemented (M4). Unsigned local files are
accepted and labelled **unsigned index, not verified**.

On-disk JSON is documented in
[docs/COVERAGE_INDEX_FORMAT.md](../../docs/COVERAGE_INDEX_FORMAT.md)
(`format_version: 1`). Hashes are 64-character lowercase hex SHA-256
values identical to `sodb` description hashes. Lists are sorted and
deduplicated at load.

## Security limits

Index files are treated as untrusted:

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_file_bytes` | 64 MiB | Reject huge files before parse |
| `max_hashes` | 5 000 000 | Cap the in-memory hash vector |
| `max_json_nesting` | 8 | Reject deep JSON |

Other hardening:

- Regular-file check on metadata and the opened handle
- Unix `O_NONBLOCK` open (FIFOs must not hang the process)
- Symbolic links rejected
- Unknown `format_version` rejected
- Unknown fields and duplicate JSON keys rejected
- Trailing bytes after JSON rejected
- Lowercase hex only; wrong lengths rejected
- Typed errors (`thiserror`); Display text never includes paths
- `Debug` of [`RedactedPath`](src/index.rs) is always `<redacted>`
- `#![forbid(unsafe_code)]`; no `unwrap` / `expect` in non-test code

## Memory bound (measured)

A 5 000 000-hash `IndexHashes` value (32-byte digests, sorted, unique)
was constructed in the `five_million_hashes_stay_within_measured_bound`
test on Linux debug.

**Measured extra RSS:** 160 395 264 bytes (153.0 MiB) on Linux debug
(`rustc` 1.99.1, `cargo test` unoptimized). The test asserts the delta
stays under 384 MiB.

JSON text for five million hex strings cannot fit under the 64 MiB file
cap; the figure above is the in-memory hash vector, which is what a
5M-hash index becomes after a successful load.

Override defaults with `load_with_limits`.
