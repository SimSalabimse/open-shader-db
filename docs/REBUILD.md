# Local shader scan and SODB rebuild

This document describes `osdb-cli shaders scan` and `osdb-cli rebuild`.
Both commands are **local and offline**. They never upload, never open a
network socket, never attach to a game process, and never modify game
files. They only read files the user points at.

**Maintainer:** Simsalabim

The rebuilt SODB contains game bytecode. It is for **local use only**
and must not be shared or uploaded.

## Commands

### Scan

```
osdb-cli shaders scan --dir <path>
osdb-cli shaders scan --sodb <file>
osdb-cli shaders scan --dir <path> --json --out report.json [--force]
```

The report is counts only: files seen, shaders indexed, unique hashes,
duplicates, and skips (symlink, special file, too large, corrupt,
unreadable). It never contains paths or bytecode.

`--out` uses the same atomic write helper as `records extract`
(temp file, fsync, rename; refuse overwrite without `--force`; refuse
the same file as the input).

### Rebuild

```
osdb-cli rebuild --records records.json --sodb game.sodb --out rebuilt.sodb [--force]
osdb-cli rebuild --records records.json --dir shaders/ --out rebuilt.sodb [--json]
osdb-cli rebuild ... --out rebuilt.sodb --max-output-bytes 1048576
```

`--dir` and `--sodb` may be repeated and mixed. The output must not be
a record input, an SODB input, a symlink, or inside a scanned shader
root. `--force` overwrites an existing regular file; the previous file
is left in place if publish fails. `--max-output-bytes` caps the
rebuilt file (default 8 GiB, hard maximum 8 GiB). A missing output
parent is a typed error. The output path must be valid UTF-8 (SQLite
needs a UTF-8 filename).

The writer builds in a temp database (SQLite temp files on disk, not
`temp_store=MEMORY`), then `VACUUM INTO` a sibling snapshot so the live
work file is never renamed. Exclusive `create_new` names the siblings
so a planted file or symlink is never written through. On non-Windows,
fsync errors on the snapshot are returned and the parent directory is
synced. On Windows, fsync errors are ignored. If the first rename
fails, a copy-then-rename fallback runs on every platform. On success
the CLI prints a one-line local-use notice.

## What the scanner supports

1. An SODB file the user already has. `shader_bytecode.Bytecode` blobs
   are hashed by streaming. The hash is SHA-256 of the whole blob, the
   same definition `records extract` writes.
2. A directory tree of loose files whose first four bytes are `DXBC`
   (`.dxbc`, `.dxil`, `.cso`, or any other extension). The container
   length is the DXBC `FileSize` field. Trailing bytes past that field
   are not hashed.

Packed engine archives are **unsupported**. The scanner does not carve
DXBC out of other containers.

## What is guaranteed (covered by tests)

- Same bytes hashed through `sodb::hash_shader_container`,
  `records extract`, SODB scan, and loose-file scan produce the same
  SHA-256.
- Symlinks are skipped and counted (not followed outside the root).
- FIFOs, devices, and sockets are skipped without blocking.
- Hashed loose-file bytes are bounded by the DXBC `FileSize` field, the
  size observed at open, and the per-file cap. Trailing bytes past
  `FileSize` are not hashed. A header `FileSize` larger than the file
  is `corrupt`.
- Caps (per-file 256 MiB, 1 000 000 files, depth 16, 64 GiB total,
  1 000 000 index entries) produce a typed error that names the cap, or
  a `too_large` skip for a single oversized loose file.
- Zero-length files and truncated DXBC headers (including a `FileSize`
  larger than the file) are `corrupt` with a typed reason.
- Duplicate shaders are stored as extra locations and counted.
- Directory order is sorted, so the same tree yields the same report.
- A missing or unreadable scan root is a typed error.
- Reopen re-hashes and re-checks the DXBC header; a changed file is
  refused.
- Rebuild writes `PRAGMA user_version = 2` and
  `PRAGMA application_id = 0xD3D50DB` using
  `sodb::SODB_WRITER_SCHEMA_SQL`.
- Only fully rebuildable records are written. Missing stages,
  unresolved stages, `root_signature_skip` records, and records whose
  stored root signature is not canonical (`skipped_noncanonical_root_signature`)
  are counted and omitted. Nothing is fabricated.
- Root signatures are re-parsed and re-serialized. They are never
  copied raw. A non-canonical stored signature is not rewritten into
  the output; that record is skipped. The residual numeric capacity in
  [RECORD_FORMAT.md](RECORD_FORMAT.md) still applies.
- A non-DXBC regular file under `--dir` counts in `files_seen` and is
  not indexed. It is not added to any skip bucket.
- The same inputs produce a byte-identical SODB (sorted inserts, fixed
  pragmas, no timestamps).
- A fixture SODB can be extracted, scanned, rebuilt, and extracted
  again with equal records (modulo records the report counted as
  skipped).
- The rebuilt file opens with `inspect`.

## UNVERIFIED

These claims are **not** made by the code or tests:

- Real game SODBs, real installed game trees, or storefront layouts
- Engines other than a user-supplied SODB and loose DXBC files
- Carving shaders out of packed archives (Unreal pak, Unity data,
  RedEngine, proprietary caches, …)
- Whether a bytecode SHA-256 alone is enough to match shaders extracted
  from those archives
- Whether a rebuilt SODB is accepted by a real Microsoft runtime, a
  storefront compiler, or a game's anti-cheat
- `state_objects`, raytracing, work graphs, `app_id` (not written;
  format-1 records do not carry them)
- Original descriptor-row `Key` blobs (records store content hashes)

## Engines and formats left as future work

UNVERIFIED, not implemented:

- Unreal Engine pak / iostore / derived data cache
- Unity data files and shader caches
- RedEngine / other publisher archives
- Vendor or driver shader caches
- Any format that requires attaching to, hooking, or reading a game
  process
