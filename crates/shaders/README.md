# osdb-shaders

Read-only local shader scanner for open-shader-db (Simsalabim).

The crate builds a [`LocalShaderIndex`] of shaders available on this
machine. It never writes shader bytes, never follows a game process, and
never modifies game files. Absolute paths stay inside the scanner object
and are never serialized or printed.

## Supported sources

1. A State Object Database (SODB) the user already has. Bytecode blobs
   are hashed by streaming through the same helper `records extract`
   uses (`sodb::hash_shader_reader`).
2. A directory tree of loose files whose first four bytes are the DXBC
   container magic (`DXBC`). Extension does not matter.

Packed engine archives are **unsupported**. Carving DXBC out of those
formats is **UNVERIFIED** future work (see [docs/REBUILD.md](../../docs/REBUILD.md)).

## Hash

SHA-256 over the container bytes exactly as stored. For an SODB blob
that is the whole `shader_bytecode.Bytecode` value. For a loose file
that is the leading `FileSize` bytes declared in the DXBC header.

## Limits

| Limit | Default | On exceed |
| --- | ---: | --- |
| Per-file size | 256 MiB | skip `too_large` or cap error when the tree budget is hit |
| Files visited | 1 000 000 | typed cap error |
| Directory depth | 16 | typed cap error |
| Total bytes hashed | 64 GiB | typed cap error |
| Index entries (locations) | 1 000 000 | typed cap error |

Symlinks are skipped and counted (they are not followed outside the
root). FIFOs, devices, and sockets are skipped without blocking
(`O_NONBLOCK` on Unix; `is_file` on the opened handle). A file that
grows while being read is bounded by the DXBC `FileSize` and the
per-file cap.

## Matching

`match_records` classifies each pipeline-state record as fully covered,
partially covered, or uncovered using the same rules as
`osdb_records::match_local_hashes`.
