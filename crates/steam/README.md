# osdb-steam

Steam library discovery for open-shader-db (Simsalabim).

This crate finds Steam library folders and installed games, and looks for
local SODB files **inside each game install directory**. It is written
from scratch (Valve KeyValues text parser included). It does not scrape
or reuse any third-party shader-cache project.

## Path isolation

`LocalPath` wraps an install or SODB path and does **not** implement
`serde::Serialize` or `Display`. Its `Debug` output is a redacted
placeholder. Upload-safe data lives on `GameIdentity` (`app_id`,
`build_id`, `name`) and `UploadSafeDiscovery` (identities plus SODB
**counts**). A unit test serializes every public `Serialize` type from
a fixture whose directory name contains a fake username and asserts
that neither the temp path nor that username appears.

Game install paths must never appear in anything meant for upload.

## API

```rust
use osdb_steam::{discover, UploadSafeDiscovery};

// Tests and tools must pass a Steam root. This never touches the
// real user Steam directory unless the caller passes it.
let found = discover(steam_root);
let upload: UploadSafeDiscovery = found.upload_safe();
```

`discover` never panics on a missing Steam root, an unreadable library,
or a partial app manifest. Those cases produce an empty or partial
game list plus a typed `Warning` list.

## SODB detection

Candidate files are `*.sodb` or other regular files whose SQLite
header is SODB v2 (`user_version=2`, `application_id=0xD3D50DB`).
Confirmation uses `sodb::is_sodb` (100-byte header only). This crate
does not parse pipeline tables or retain bytecode.

The install-dir walk is bounded (default depth 4) and does not follow
symlink loops. Non-regular files (directories, FIFOs, devices) are
skipped without opening them.

## What is not verified

- Real Steam installs and real game folders
- Windows registry lookups (`winreg` is compiled for Windows CI but
  not exercised against a live Steam key)
- Flatpak or macOS Application Support trees on this project's
  Ubuntu fixtures (paths are constructed; existence is not assumed)
