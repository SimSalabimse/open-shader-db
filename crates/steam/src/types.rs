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
//! [`UploadSafeDiscovery`] (identities plus SODB **counts**). Their fields are
//! private so a path string cannot be assigned by accident. [`InstalledGame`]
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
pub struct GameName(String);

impl GameName {
    /// Strip control and Unicode format characters (bidi overrides, zero-width, BOM).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().chars().filter(|c| keep_name_char(*c)).collect())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn keep_name_char(c: char) -> bool {
    if c.is_control() {
        return false;
    }
    !matches!(
        c,
        '\u{200B}'..='\u{200D}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
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
    app_id: AppId,
    build_id: BuildId,
    name: GameName,
}

impl GameIdentity {
    #[must_use]
    pub fn new(app_id: AppId, build_id: BuildId, name: GameName) -> Self {
        Self {
            app_id,
            build_id,
            name,
        }
    }

    #[must_use]
    pub fn app_id(&self) -> AppId {
        self.app_id
    }

    #[must_use]
    pub fn build_id(&self) -> BuildId {
        self.build_id
    }

    #[must_use]
    pub fn name(&self) -> &GameName {
        &self.name
    }
}

/// One installed game. **Not** `Serialize` because it carries [`LocalPath`]s.
#[derive(Clone, Debug)]
pub struct InstalledGame {
    pub identity: GameIdentity,
    pub install_dir: LocalPath,
    pub sodb_files: Vec<LocalPath>,
}

/// Non-fatal discovery problem. Messages must not include filesystem paths.
///
/// Constructed only inside this crate from `'static` text so a caller cannot
/// inject a filesystem path.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Warning {
    kind: WarningKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_id: Option<AppId>,
    message: String,
}

impl Warning {
    pub(crate) fn new(kind: WarningKind, app_id: Option<AppId>, message: &'static str) -> Self {
        Self {
            kind,
            app_id,
            message: String::from(message),
        }
    }

    #[must_use]
    pub fn kind(&self) -> WarningKind {
        self.kind
    }

    #[must_use]
    pub fn app_id(&self) -> Option<AppId> {
        self.app_id
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn with_app_id(mut self, app_id: AppId) -> Self {
        self.app_id = Some(app_id);
        self
    }
}

/// Class of a non-fatal discovery problem.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    MissingSteamRoot,
    UnreadableLibrary,
    MissingLibraryList,
    MalformedVdf,
    IncompleteManifest,
    InvalidInstallDir,
    UnreadableManifest,
    UnreadableInstallDir,
    DuplicateApp,
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
        UploadSafeDiscovery::new(
            self.games.iter().map(GameListing::from).collect(),
            self.warnings.clone(),
        )
    }
}

/// One game as it may appear in JSON / upload previews (no paths).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct GameListing {
    identity: GameIdentity,
    sodb_file_count: usize,
}

impl GameListing {
    #[must_use]
    pub(crate) fn new(identity: GameIdentity, sodb_file_count: usize) -> Self {
        Self {
            identity,
            sodb_file_count,
        }
    }

    #[must_use]
    pub fn identity(&self) -> &GameIdentity {
        &self.identity
    }

    #[must_use]
    pub fn sodb_file_count(&self) -> usize {
        self.sodb_file_count
    }
}

impl From<&InstalledGame> for GameListing {
    fn from(game: &InstalledGame) -> Self {
        Self::new(game.identity.clone(), game.sodb_files.len())
    }
}

/// Serialize-only view of a discovery pass. Contains no [`LocalPath`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct UploadSafeDiscovery {
    games: Vec<GameListing>,
    warnings: Vec<Warning>,
}

impl UploadSafeDiscovery {
    #[must_use]
    pub(crate) fn new(games: Vec<GameListing>, warnings: Vec<Warning>) -> Self {
        Self { games, warnings }
    }

    #[must_use]
    pub fn games(&self) -> &[GameListing] {
        &self.games
    }

    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }
}
