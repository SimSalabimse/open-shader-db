// SPDX-License-Identifier: MIT

//! Strong types for Steam discovery.
//!
//! # Path isolation
//!
//! [`LocalPath`] is the only type that holds a filesystem path. It does **not**
//! implement [`serde::Serialize`] or [`std::fmt::Display`]. [`Debug`] prints
//! `LocalPath(<redacted>)` so logs cannot leak an install directory.
//!
//! Upload-safe data is [`GameIdentity`] (`app_id`, `build_id`, `name`) and
//! [`UploadSafeDiscovery`] (identities plus SODB **counts**). [`InstalledGame`]
//! and [`Discovery`] are intentionally not `Serialize`. A unit test serializes
//! every public `Serialize` type in this crate and asserts that the fixture
//! directory path and a fake username do not appear.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Steam application id (`appmanifest_*.acf` `appid`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
#[serde(transparent)]
pub struct AppId(pub u32);

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Steam depot build id (`appmanifest_*.acf` `buildid`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
#[serde(transparent)]
pub struct BuildId(pub u64);

impl fmt::Display for BuildId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Display name from the app manifest. Upload-safe.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(transparent)]
pub struct GameName(pub String);

impl GameName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Local filesystem path that must never appear in an upload payload.
///
/// Deliberately omits `Serialize` and `Display`. Use [`Self::as_path`] only
/// for local I/O (opening a file on this machine).
#[derive(Clone, PartialEq, Eq)]
pub struct LocalPath(PathBuf);

impl LocalPath {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// Borrow the real path for local use only. Do not serialize the result.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl fmt::Debug for LocalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalPath(<redacted>)")
    }
}

/// Upload-safe game identity. No filesystem paths.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct GameIdentity {
    pub app_id: AppId,
    pub build_id: BuildId,
    pub name: GameName,
}

/// One installed game. **Not** `Serialize` because it carries [`LocalPath`]s.
#[derive(Clone, Debug)]
pub struct InstalledGame {
    pub identity: GameIdentity,
    pub install_dir: LocalPath,
    pub sodb_files: Vec<LocalPath>,
}

/// Non-fatal discovery problem. Messages must not include filesystem paths.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Warning {
    pub kind: WarningKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<AppId>,
    pub message: String,
}

/// Class of a non-fatal discovery problem.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    MissingSteamRoot,
    UnreadableLibrary,
    MissingLibraryList,
    MalformedVdf,
    IncompleteManifest,
    UnreadableManifest,
    UnreadableInstallDir,
    SodbScanLimited,
}

/// Full local discovery result. **Not** `Serialize`.
#[derive(Clone, Debug)]
pub struct Discovery {
    pub games: Vec<InstalledGame>,
    pub warnings: Vec<Warning>,
}

impl Discovery {
    /// Identities and SODB counts only. Safe to write to an upload preview.
    #[must_use]
    pub fn upload_safe(&self) -> UploadSafeDiscovery {
        UploadSafeDiscovery {
            games: self.games.iter().map(GameListing::from).collect(),
            warnings: self.warnings.clone(),
        }
    }
}

/// One game as it may appear in JSON / upload previews (no paths).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct GameListing {
    pub identity: GameIdentity,
    pub sodb_file_count: usize,
}

impl From<&InstalledGame> for GameListing {
    fn from(game: &InstalledGame) -> Self {
        Self {
            identity: game.identity.clone(),
            sodb_file_count: game.sodb_files.len(),
        }
    }
}

/// Serialize-only view of a discovery pass. Contains no [`LocalPath`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct UploadSafeDiscovery {
    pub games: Vec<GameListing>,
    pub warnings: Vec<Warning>,
}
