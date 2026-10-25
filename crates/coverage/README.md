# osdb-coverage

Load an untrusted community coverage index and compare it to description
hashes from a local SODB. Used by `osdb-cli coverage` (Simsalabim).
The same binary also provides `inspect` (SODB hashes), `discover`
(Steam games, counts only in JSON), `records extract` (local
pipeline-state records without bytecode), and `upload prepare`
(anonymized manifest preview; nothing is sent).

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
accepted and labelled **unsigned index, not verified**. Build ids are
stored on the index but not compared: the report sets
`build_identity` to `not_compared` because an SODB has no build
identifier yet.

On-disk JSON is documented in
[docs/COVERAGE_INDEX_FORMAT.md](../../docs/COVERAGE_INDEX_FORMAT.md)
(`format_version: 1`). Hashes are 64-character lowercase hex SHA-256
values identical to `sodb` description hashes. Lists are sorted and
deduplicated at load.

## Security limits

Index files are treated as untrusted:

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_file_bytes` | 64 MiB | Metadata check plus a capped read (`take(max+1)`) |
| `max_hashes` | 1 000 000 | Enforced in the JSON visitor as soon as the cap is crossed |
| `max_json_nesting` | 8 | Reject deep JSON |

One million compact hex strings fit under the 64 MiB file cap. Larger
limits are library-only (`load_with_limits`); the CLI uses the defaults.

Other hardening:

- Regular-file check on metadata and the opened handle
- Unix `O_NONBLOCK` open (FIFOs must not hang the process)
- Symbolic links rejected
- Unknown `format_version` rejected
- Unknown fields and duplicate JSON keys rejected
- Trailing tokens after JSON rejected; trailing whitespace allowed
- UTF-8 BOM rejected
- Lowercase hex only; wrong lengths rejected
- Typed errors (`thiserror`); Display text never includes paths
- `Debug` of [`RedactedPath`](src/index.rs) is always `<redacted>`
- `#![forbid(unsafe_code)]`; no `unwrap` / `expect` in non-test code

## Memory bound (measured)

**Measured peak RSS** for `osdb-cli coverage` (release, Linux) at the
default cap (1 000 000 hashes in a 67 000 233 byte index):

**103 845 888 bytes (101 412 KiB, 99.0 MiB)** — `RUSAGE_CHILDREN.ru_maxrss`
over three runs (101 324–101 412 KiB). This is process peak RSS, not
the size of the final hash vector alone.

Override defaults with `load_with_limits` in library callers only.
