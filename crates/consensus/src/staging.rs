// SPDX-License-Identifier: MIT

//! Read-only loader for a staging tree.
//!
//! Layout, under the staging root (the `staging/` directory for a reviewed
//! pull request, or the worker prefix root):
//!
//! ```text
//! <game_id>/<version>--<exe_hash>/<64 lowercase hex>.json
//! ```
//!
//! `exe_hash` is 64 lowercase hex, so the last `--` splits the build
//! directory. That is the same rule as `worker/src/keys.ts`. The walk never
//! follows a symlink, never writes, and never deletes. Every directory entry
//! is hostile input.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use osdb_upload::{parse_with_limits, BuildId, ExeHash, GameId, GameVersion, Sha256Hex};
use sha2::{Digest, Sha256};

use crate::Error;

/// Default objects opened per build directory. Matches the worker per-build cap.
pub const DEFAULT_MAX_FILES_PER_BUILD: u64 = osdb_upload::DEFAULT_PER_BUILD_UPLOAD_CAP;

/// Default per-file read cap. Matches the local manifest byte cap (64 MiB),
/// so a reviewed pull request can hold a full manifest that does not fit the
/// worker wire cap.
pub const DEFAULT_MAX_FILE_BYTES: u64 = osdb_upload::DEFAULT_MAX_JSON_BYTES;

/// Default names visited while listing or loading. The walk stops with an
/// error instead of returning a shorter list. Raise this explicitly for a
/// tree with more names than the default.
pub const DEFAULT_MAX_ENTRIES_SCANNED: u64 = 8_192;

/// Levels under the root: `game_id`, then `version--exe_hash`.
/// Files sit in the build directory. The walk does not go deeper.
pub const WALK_DEPTH: u32 = 2;

const HEX_LEN: usize = 64;
const OBJECT_SUFFIX: &str = ".json";

/// Staging root. [`Debug`] is always `<redacted>` so a log cannot grow a path.
#[derive(Clone)]
pub struct StagingRoot(PathBuf);

impl StagingRoot {
    /// Wrap a directory the caller already chose. The path is not touched yet.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// Borrow the path for a caller that opens it. Do not write it into a report.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Debug for StagingRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Caps for one load or list. Manifest caps are the `osdb-upload` limits
/// passed to `parse_with_limits`.
///
/// A zero `max_files_per_build` counts every candidate as
/// [`SkipReason::OverFileCap`] and opens nothing. A zero
/// `max_entries_scanned` is an error as soon as a directory has one name.
/// An empty directory still succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadLimits {
    /// How many candidate files to open in one build directory.
    /// Extra candidates are [`SkipReason::OverFileCap`] and are not read.
    /// Selection is by filename (the object hash), not directory order.
    pub max_files_per_build: u64,
    /// Metadata and read cap for one object. Larger files are
    /// [`SkipReason::TooLarge`] and the body is not read.
    pub max_file_bytes: u64,
    /// Names `read_dir` may yield. Crossing this returns
    /// [`Error::TooManyEntries`] rather than a partial report.
    /// [`list_builds`] counts game-level and build-level names.
    /// [`load_build`] counts names inside the one build directory.
    pub max_entries_scanned: u64,
    /// Caps forwarded to manifest parse and canonical re-encode.
    pub manifest: osdb_upload::Limits,
}

impl LoadLimits {
    /// 64 files, 64 MiB each, 8192 directory names, and
    /// [`osdb_upload::Limits::DEFAULT`].
    pub const DEFAULT: Self = Self {
        max_files_per_build: DEFAULT_MAX_FILES_PER_BUILD,
        max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        max_entries_scanned: DEFAULT_MAX_ENTRIES_SCANNED,
        manifest: osdb_upload::Limits::DEFAULT,
    };
}

impl Default for LoadLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Where the caller says these bytes came from. The loader does not infer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IngestPath {
    /// Object layout produced by the ingest worker. Not deployed.
    WorkerStaging,
    /// A maintainer-merged tree under `staging/`.
    ReviewedPullRequest,
}

/// Why one directory entry was not returned as an upload.
///
/// Counts only. The reason does not carry a path or any file bytes.
/// [`SkipReason::Parse`] stores [`osdb_upload::Error::code`], which is a
/// fixed string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    /// Directory, fifo, device, or other non-regular file.
    NotRegularFile,
    /// Symlink. Not followed.
    Symlink,
    /// Name is not `<64 lowercase hex>.json`, or it is not a single path component.
    BadFileName,
    /// The name is 64 lowercase hex but is not the SHA-256 of the file bytes.
    NameHashMismatch,
    /// Parsed, but the bytes are not the canonical encoding.
    NotCanonical,
    /// `parse_with_limits` or canonical encode failed. The string is the upload error code.
    Parse(&'static str),
    /// Canonical manifest `build_id` is not the directory's build.
    BuildIdMismatch,
    /// File is larger than [`LoadLimits::max_file_bytes`]. Body not kept.
    TooLarge,
    /// Past [`LoadLimits::max_files_per_build`] after sorting names. Not opened.
    OverFileCap,
}

impl SkipReason {
    /// Short stable label. [`SkipReason::Parse`] returns the upload error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotRegularFile => "not_regular_file",
            Self::Symlink => "symlink",
            Self::BadFileName => "bad_file_name",
            Self::NameHashMismatch => "name_hash_mismatch",
            Self::NotCanonical => "not_canonical",
            Self::Parse(code) => code,
            Self::BuildIdMismatch => "build_id_mismatch",
            Self::TooLarge => "too_large",
            Self::OverFileCap => "over_file_cap",
        }
    }
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(code) => write!(f, "parse:{code}"),
            other => f.write_str(other.code()),
        }
    }
}

/// One manifest that passed every staging check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedUpload {
    /// SHA-256 of the canonical file bytes. Also the file name without `.json`.
    pub object_sha256: Sha256Hex,
    /// Parsed manifest. The loader does not keep the original bytes.
    pub manifest: osdb_upload::UploadManifest,
    /// Caller-supplied origin. Stamped on every object from this load.
    pub via: IngestPath,
}

/// Manifests loaded for one build, plus skip counts.
///
/// `loaded` is sorted by [`StagedUpload::object_sha256`]. Identical canonical
/// bytes appear once. `skipped` is counts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadReport {
    /// Build the caller asked for.
    pub build: BuildId,
    /// Accepted objects, sorted by object hash.
    pub loaded: Vec<StagedUpload>,
    /// Skip counts. Keys are reasons, never paths.
    pub skipped: BTreeMap<SkipReason, u64>,
}

/// Load every accepted manifest for `build`.
///
/// Missing game or build directories are an empty report, not an error.
/// A symlink at the root, the game component, or the build component is
/// [`Error::Symlink`]. The directory is not followed.
///
/// Check order for a build-directory entry:
/// symlink, non-regular file, bad name, too large, over the file cap,
/// then open (regular file, capped read), content hash, parse, canonical
/// bytes, build id. The first matching reason is the one counted.
pub fn load_build(
    root: &StagingRoot,
    build: &BuildId,
    via: IngestPath,
    limits: LoadLimits,
) -> Result<LoadReport, Error> {
    let mut report = LoadReport {
        build: build.clone(),
        loaded: Vec::new(),
        skipped: BTreeMap::new(),
    };
    let Some(build_dir) = locate_build_dir(root.as_path(), build)? else {
        return Ok(report);
    };
    let names = read_child_names(&build_dir, limits.max_entries_scanned)?;
    let mut candidates: Vec<(Sha256Hex, PathBuf)> = Vec::new();
    for name in names {
        match classify_cheap(&build_dir, &name, limits.max_file_bytes) {
            Cheap::Skip(reason) => bump(&mut report.skipped, reason),
            Cheap::Candidate(hash, path) => candidates.push((hash, path)),
        }
    }
    candidates.sort_by_key(|candidate| candidate.0);
    let mut opened: Vec<(Sha256Hex, PathBuf)> = Vec::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        if u64::try_from(index).unwrap_or(u64::MAX) >= limits.max_files_per_build {
            bump(&mut report.skipped, SkipReason::OverFileCap);
        } else {
            opened.push(candidate);
        }
    }
    let mut loaded: BTreeMap<Sha256Hex, StagedUpload> = BTreeMap::new();
    for (name_hash, path) in opened {
        match load_candidate(&path, name_hash, build, via, limits) {
            Ok(upload) => {
                loaded.entry(upload.object_sha256).or_insert(upload);
            }
            Err(CandidateFail::Skip(reason)) => bump(&mut report.skipped, reason),
            Err(CandidateFail::Fatal(err)) => return Err(err),
        }
    }
    report.loaded = loaded.into_values().collect();
    Ok(report)
}

/// List build directories under `root`.
///
/// A directory is listed when its game component and its `version--exe_hash`
/// component both match the worker key rules. Files are not opened. Symlinks
/// are not followed and are not listed. Hostile names are ignored. The result
/// is sorted and deduplicated.
///
/// `max_entries_scanned` counts every name at game level and build level.
/// Names inside a build directory are not visited (`WALK_DEPTH`).
pub fn list_builds(root: &StagingRoot, limits: LoadLimits) -> Result<Vec<BuildId>, Error> {
    ensure_real_directory(root.as_path())?;
    let mut seen = 0_u64;
    let game_names =
        read_child_names_counted(root.as_path(), &mut seen, limits.max_entries_scanned)?;
    let mut builds = Vec::new();
    for game_name in game_names {
        let Some(game) = game_name.to_str() else {
            continue;
        };
        if !is_single_component(game) || !is_safe_ident(game) {
            continue;
        }
        let game_path = root.as_path().join(game);
        match directory_component(&game_path)? {
            DirState::Missing | DirState::Skip => continue,
            DirState::NotDir => continue,
            DirState::Dir => {}
        }
        let children = read_child_names_counted(&game_path, &mut seen, limits.max_entries_scanned)?;
        for child in children {
            let Some(build_name) = child.to_str() else {
                continue;
            };
            if let Some(build) = build_from_components(game, build_name) {
                builds.push(build);
            }
        }
    }
    builds.sort();
    builds.dedup();
    Ok(builds)
}

enum DirState {
    Missing,
    /// Symlink or otherwise not a real directory we will enter.
    Skip,
    NotDir,
    Dir,
}

fn locate_build_dir(root: &Path, build: &BuildId) -> Result<Option<PathBuf>, Error> {
    ensure_real_directory(root)?;
    let game = build.game_id.as_str();
    let build_name = build_dir_name(build)?;
    if !is_single_component(game) || !is_single_component(&build_name) {
        return Err(Error::InvalidBuildId);
    }
    let game_path = root.join(game);
    match directory_component(&game_path)? {
        DirState::Missing => return Ok(None),
        DirState::Skip => return Err(Error::Symlink),
        DirState::NotDir => return Err(Error::NotADirectory),
        DirState::Dir => {}
    }
    let build_path = game_path.join(&build_name);
    match directory_component(&build_path)? {
        DirState::Missing => Ok(None),
        DirState::Skip => Err(Error::Symlink),
        DirState::NotDir => Err(Error::NotADirectory),
        DirState::Dir => Ok(Some(build_path)),
    }
}

fn directory_component(path: &Path) -> Result<DirState, Error> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(DirState::Skip),
        Ok(meta) if meta.is_dir() => Ok(DirState::Dir),
        Ok(_) => Ok(DirState::NotDir),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(DirState::Missing),
        Err(_) => Err(Error::Io),
    }
}

fn ensure_real_directory(path: &Path) -> Result<(), Error> {
    match directory_component(path)? {
        DirState::Dir => Ok(()),
        DirState::Missing => Err(Error::Io),
        DirState::Skip => Err(Error::Symlink),
        DirState::NotDir => Err(Error::NotADirectory),
    }
}

fn read_child_names(dir: &Path, max_entries: u64) -> Result<Vec<std::ffi::OsString>, Error> {
    let mut seen = 0_u64;
    read_child_names_counted(dir, &mut seen, max_entries)
}

fn read_child_names_counted(
    dir: &Path,
    seen: &mut u64,
    max_entries: u64,
) -> Result<Vec<std::ffi::OsString>, Error> {
    // Re-check immediately before `read_dir`. `read_dir` has no `O_NOFOLLOW`
    // without unsafe, which this crate forbids. A symlink swap in between is
    // the same window the other crates accept for a single file open.
    ensure_real_directory(dir)?;
    let rd = std::fs::read_dir(dir).map_err(|_| Error::Io)?;
    let mut names = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|_| Error::Io)?;
        *seen = seen.saturating_add(1);
        if *seen > max_entries {
            return Err(Error::TooManyEntries { max: max_entries });
        }
        names.push(entry.file_name());
    }
    names.sort();
    Ok(names)
}

enum Cheap {
    Skip(SkipReason),
    Candidate(Sha256Hex, PathBuf),
}

fn classify_cheap(dir: &Path, name: &std::ffi::OsString, max_file_bytes: u64) -> Cheap {
    let Some(text) = name.to_str() else {
        return Cheap::Skip(SkipReason::BadFileName);
    };
    if !is_single_component(text) {
        return Cheap::Skip(SkipReason::BadFileName);
    }
    let path = dir.join(text);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(_) => return Cheap::Skip(SkipReason::NotRegularFile),
    };
    if meta.file_type().is_symlink() {
        return Cheap::Skip(SkipReason::Symlink);
    }
    if !meta.file_type().is_file() {
        return Cheap::Skip(SkipReason::NotRegularFile);
    }
    let Some(hash) = object_hash_from_name(text) else {
        return Cheap::Skip(SkipReason::BadFileName);
    };
    if meta.len() > max_file_bytes {
        return Cheap::Skip(SkipReason::TooLarge);
    }
    Cheap::Candidate(hash, path)
}

enum CandidateFail {
    Skip(SkipReason),
    Fatal(Error),
}

fn load_candidate(
    path: &Path,
    name_hash: Sha256Hex,
    build: &BuildId,
    via: IngestPath,
    limits: LoadLimits,
) -> Result<StagedUpload, CandidateFail> {
    // Stat again so a swap to a symlink or fifo is not opened as a normal file.
    let meta = std::fs::symlink_metadata(path).map_err(|_| CandidateFail::Fatal(Error::Io))?;
    if meta.file_type().is_symlink() {
        return Err(CandidateFail::Skip(SkipReason::Symlink));
    }
    if !meta.file_type().is_file() {
        return Err(CandidateFail::Skip(SkipReason::NotRegularFile));
    }
    if meta.len() > limits.max_file_bytes {
        return Err(CandidateFail::Skip(SkipReason::TooLarge));
    }
    let mut file = match open_regular_nonblocking(path) {
        Ok(file) => file,
        // `O_NOFOLLOW` reports a raced symlink as `ELOOP` on Unix. Other
        // targets refuse symlinks with the `symlink_metadata` check above.
        #[cfg(unix)]
        Err(OpenFail::Symlink) => return Err(CandidateFail::Skip(SkipReason::Symlink)),
        Err(OpenFail::NotRegular) => return Err(CandidateFail::Skip(SkipReason::NotRegularFile)),
        Err(OpenFail::Io) => return Err(CandidateFail::Fatal(Error::Io)),
    };
    let bytes = match read_capped(&mut file, limits.max_file_bytes) {
        Ok(bytes) => bytes,
        Err(ReadFail::TooLarge) => return Err(CandidateFail::Skip(SkipReason::TooLarge)),
        Err(ReadFail::Io) => return Err(CandidateFail::Fatal(Error::Io)),
    };
    let digest = sha256_of(&bytes);
    if digest != name_hash {
        return Err(CandidateFail::Skip(SkipReason::NameHashMismatch));
    }
    let manifest = match parse_with_limits(&bytes, limits.manifest) {
        Ok(manifest) => manifest,
        Err(err) => return Err(CandidateFail::Skip(SkipReason::Parse(err.code()))),
    };
    let canonical = match manifest.to_canonical_bytes_with_limits(limits.manifest) {
        Ok(canonical) => canonical,
        Err(err) => return Err(CandidateFail::Skip(SkipReason::Parse(err.code()))),
    };
    if canonical != bytes {
        return Err(CandidateFail::Skip(SkipReason::NotCanonical));
    }
    if manifest.build_id != *build {
        return Err(CandidateFail::Skip(SkipReason::BuildIdMismatch));
    }
    Ok(StagedUpload {
        object_sha256: digest,
        manifest,
        via,
    })
}

enum OpenFail {
    Io,
    NotRegular,
    #[cfg(unix)]
    Symlink,
}

fn open_regular_nonblocking(path: &Path) -> Result<File, OpenFail> {
    let file = open_file(path)?;
    let opened = file.metadata().map_err(|_| OpenFail::Io)?;
    if !opened.is_file() {
        return Err(OpenFail::NotRegular);
    }
    Ok(file)
}

fn open_file(path: &Path) -> Result<File, OpenFail> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        match File::options()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)
        {
            Ok(file) => Ok(file),
            Err(err) if err.raw_os_error() == Some(libc::ELOOP) => Err(OpenFail::Symlink),
            Err(_) => Err(OpenFail::Io),
        }
    }
    #[cfg(not(unix))]
    {
        File::open(path).map_err(|_| OpenFail::Io)
    }
}

enum ReadFail {
    Io,
    TooLarge,
}

fn read_capped(file: &mut File, max_file_bytes: u64) -> Result<Vec<u8>, ReadFail> {
    let mut bytes = Vec::new();
    let limit = max_file_bytes.saturating_add(1);
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadFail::Io)?;
    let got = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if got > max_file_bytes {
        return Err(ReadFail::TooLarge);
    }
    Ok(bytes)
}

pub(crate) fn sha256_of(bytes: &[u8]) -> Sha256Hex {
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    Sha256Hex::from_bytes(out)
}

fn bump(skipped: &mut BTreeMap<SkipReason, u64>, reason: SkipReason) {
    let entry = skipped.entry(reason).or_insert(0);
    *entry = entry.saturating_add(1);
}

/// Worker `isSafeIdent`: 1..=128 bytes, not `.` or `..`, ASCII graphic except
/// `/`, `\`, and `:`.
pub(crate) fn is_safe_ident(text: &str) -> bool {
    if text.is_empty() || text.len() > 128 || text == "." || text == ".." {
        return false;
    }
    text.chars().all(|ch| {
        let code = u32::from(ch);
        code != 0x2f && code != 0x5c && code != 0x3a && code > 0x20 && code <= 0x7e
    })
}

pub(crate) fn is_single_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name
        .bytes()
        .any(|byte| byte == 0 || byte == b'/' || byte == b'\\')
    {
        return false;
    }
    let path = Path::new(name);
    !path.is_absolute() && path.components().count() == 1
}

/// Last `--` plus 64 lowercase hex is the exe hash. `version` is the prefix.
pub(crate) fn split_build_dir(name: &str) -> Option<(&str, &str)> {
    if !is_single_component(name) {
        return None;
    }
    let idx = name.rfind("--")?;
    let version = &name[..idx];
    let exe = &name[idx + 2..];
    if exe.len() != HEX_LEN || !exe.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    if !is_safe_ident(version) {
        return None;
    }
    Some((version, exe))
}

pub(crate) fn object_hash_from_name(name: &str) -> Option<Sha256Hex> {
    let hex = name.strip_suffix(OBJECT_SUFFIX)?;
    if !is_single_component(name) {
        return None;
    }
    Sha256Hex::parse(hex).ok()
}

fn build_dir_name(build: &BuildId) -> Result<String, Error> {
    let version = build.version.as_str();
    let exe = build.exe_hash.to_hex();
    if !is_safe_ident(build.game_id.as_str()) || !is_safe_ident(version) {
        return Err(Error::InvalidBuildId);
    }
    let name = format!("{version}--{exe}");
    let (parsed_version, parsed_exe) = split_build_dir(&name).ok_or(Error::InvalidBuildId)?;
    if parsed_version != version || parsed_exe != exe {
        return Err(Error::InvalidBuildId);
    }
    Ok(name)
}

fn build_from_components(game: &str, build_dir: &str) -> Option<BuildId> {
    if !is_safe_ident(game) || !is_single_component(game) {
        return None;
    }
    let (version, exe) = split_build_dir(build_dir)?;
    let game_id = GameId::parse(game).ok()?;
    let version = GameVersion::parse(version).ok()?;
    let exe_hash = ExeHash::parse(exe).ok()?;
    Some(BuildId::new(game_id, version, exe_hash))
}
