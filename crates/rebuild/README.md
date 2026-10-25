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
as `records extract`. They are never copied raw. The residual numeric
capacity documented in [docs/RECORD_FORMAT.md](../../docs/RECORD_FORMAT.md)
still applies.

## What is not rebuilt

A record is skipped (and counted) when:

- a referenced stage hash is missing from the local index
- `unresolved_stages` is non-empty
- `root_signature_skip` is set (`root_signature` is null because the
  blob was skipped)

Nothing is fabricated.

## Output safety

Atomic temp-file + fsync + rename. The output must not be a record
input, an SODB input, or inside a scanned shader root. Non-regular
targets are refused. `--force` is required to overwrite. Default size
cap is 8 GiB.
