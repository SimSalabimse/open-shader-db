# osdb-consensus

Read-only staging loader and upload-threshold consensus for open-shader-db
(Simsalabim).

The crate reads a directory of canonical upload manifests and reports which
hashes meet a documented threshold. It does not send bytes, write a coverage
index, publish records, or talk to the Worker. Those steps are not in this
crate.

The implementation is written from this repository's architecture notes.
It does not reuse code from SCSKiller.

## API

```rust
use osdb_consensus::{
    build_consensus, list_builds, load_build, AgreementThreshold, CountingPolicy,
    IngestPath, LoadLimits, StagingRoot,
};

let root = StagingRoot::new("staging");
let builds = list_builds(&root, LoadLimits::default())?;
let report = load_build(&root, &builds[0], IngestPath::ReviewedPullRequest, LoadLimits::default())?;
let consensus = build_consensus(
    &builds[0],
    &report.loaded,
    AgreementThreshold::default(),
    CountingPolicy::default(),
);
// consensus holds hashes and counts only
```

`StagingRoot`'s `Debug` output is always `<redacted>`.

On-disk rules, the threshold, and the counting policy are documented in
[docs/CONSENSUS.md](../../docs/CONSENSUS.md).

## Security limits

| Limit | Default | Purpose |
| --- | ---: | --- |
| `max_files_per_build` | 64 | Objects opened per build. Matches the worker per-build cap |
| `max_file_bytes` | 64 MiB | Metadata check plus a capped read. Same local cap as upload manifests |
| `max_entries_scanned` | 8192 | Names visited. Crossing the cap is an error, not a shorter result |
| `manifest` | `osdb_upload` defaults | Parse and canonical re-encode caps |
| Walk depth | 2 | `game_id` / `version--exe_hash` only |

Other hardening:

- Regular-file check on metadata and the opened handle
- Unix `O_NOFOLLOW`, `O_NONBLOCK`, and `O_NOCTTY` (FIFOs must not hang; symlinks are not followed)
- Directory symlinks are not entered
- File name must be the SHA-256 of the file bytes
- Bytes must parse and re-encode to the same canonical JSON
- Manifest `build_id` must match the directory
- Skip results are counts only
- Typed errors (`thiserror`); Display text never includes paths or file bytes
- `#![forbid(unsafe_code)]`; no `unwrap` / `expect` in non-test code

The default threshold is 3 counted uploads and 500 per mille. The library
does not lower it for a small build. Worker uploads are held unless a
counting flag is turned on. That flag does not verify ownership.
