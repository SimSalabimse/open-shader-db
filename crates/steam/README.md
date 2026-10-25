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
**counts**). Those structs use private fields. Warnings are built only
inside this crate from `'static` messages. `VdfValue` `Debug` redacts
string values (they can hold library paths). A unit test serializes
every public `Serialize` type from a fixture whose directory name
contains a fake username and asserts that neither the temp path nor
that username appears.

Game install paths must never appear in anything meant for upload.

`installdir` from an app manifest must be a non-empty relative name
made only of `Normal` path components. Absolute paths, `..`, empty
names, and Windows drive / verbatim prefixes (`C:\x`, `\\?\C:\x`) are
rejected on every OS. **Symlinked install directories are not
allowed** (a link under `steamapps/common` to a folder outside the
library, or to another game, is skipped). The canonical install dir
must stay under the canonical `steamapps/common` directory.

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

Libraries are ordered by numeric VDF key (`"2"` before `"10"`), then
text. Manifest files are sorted by path before load. Duplicate app ids
keep the higher build id, then the earlier library in that order, and
emit `WarningKind::DuplicateApp`. Warnings are sorted by
`(app_id, kind, message)`. When a manifest `appid` and the filename
disagree, the manifest value is used.

## SODB detection

Candidate files are regular files whose SQLite header is SODB v2
(`user_version=2`, `application_id=0xD3D50DB`). Confirmation uses
`sodb::is_sodb` (100-byte header only). This crate does not parse
pipeline tables or retain bytecode.

The install-dir walk is bounded by **depth** (default 4), **regular
files considered** (default 10 000), **every directory entry**
(default 50 000), and **wall-clock time per game** (default 2 s; not
per whole `discover` run). Folder symlinks inside an install dir are
followed only when `canonicalize` succeeds and the target stays under
the canonical install dir. File symlinks are never followed. Loops
(`self -> .`, `a -> b -> a`) are skipped after the first visit. A
target reached by two paths is counted once.

On Unix, candidate files are opened with `O_NOFOLLOW | O_NONBLOCK |
O_NOCTTY`. The opened handle's metadata must report a regular file;
FIFOs and devices are opened non-blocking, then rejected.

## CLI

`osdb-cli` exposes three commands:

- `inspect <file.sodb>` — print SODB hashes as JSON
- `discover` — list Steam games and SODB counts (this crate)
- `coverage --sodb <file> --index <file>` — compare local hashes to an
  unsigned coverage index

## What is not verified

- Real Steam installs and real game folders
- Windows registry lookups (`winreg` is compiled for Windows CI but
  not exercised against a live Steam key)
- Flatpak or macOS Application Support trees on this project's
  Ubuntu fixtures (paths are constructed; existence is not assumed)
- Windows symlink behaviour (escape / loop / internal-link / race
  tests are Unix-only)
- Creating 100 000 real folders in CI (the entry-count test uses a
  smaller configurable cap)
