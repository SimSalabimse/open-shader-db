# osdb-rebuild

Rebuild a local State Object Database from pipeline-state records plus
shaders already on this machine (Simsalabim).

The crate joins records to a [`osdb_shaders::LocalShaderIndex`] by
bytecode SHA-256. Shader bytes are copied only into the rebuilt SODB the
user asked for. Records, reports, and errors carry hashes and counts
only.

The rebuilt file is **for local use only** and must not be shared or
uploaded. It contains game bytecode.

## What is written

`PRAGMA user_version = 2` and `PRAGMA application_id = 0xD3D50DB`, plus
the tables in [`sodb::SODB_WRITER_SCHEMA_SQL`]. That schema is the
column set `crates/sodb` already reads for pipeline states, descriptors,
and `shader_bytecode`. Tables that format-1 records cannot populate
(`state_objects`, `app_id`, raytracing, work graphs) are omitted.

Root signatures are re-parsed and re-serialized with the same verifier
as `records extract`. They are never copied raw. A non-canonical stored
signature skips that record (`skipped_noncanonical_root_signature`)
instead of aborting. The residual numeric capacity documented in
[docs/RECORD_FORMAT.md](../../docs/RECORD_FORMAT.md) still applies.

## What is not rebuilt

A record is skipped (and counted) when:

- a referenced stage hash is missing from the local index
- `unresolved_stages` is non-empty
- `root_signature_skip` is set (`root_signature` is null because the
  blob was skipped)
- the stored root signature does not match its canonical serialization

Nothing is fabricated.

## Output safety

The writer uses on-disk SQLite temps, `VACUUM INTO` a sibling snapshot,
then rename (copy fallback on every platform if rename fails). On
non-Windows, snapshot fsync errors are returned and the parent
directory is synced. Exclusive sibling names refuse an existing path.
The output must not be a record input, an SODB input, a symlink, or
inside a scanned shader root. A missing parent or a non-UTF-8 output
path is a typed error. `--force` is required to overwrite; a failed
publish leaves the previous file. Default and hard-maximum size cap is
8 GiB (`--max-output-bytes`).
