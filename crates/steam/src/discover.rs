// SPDX-License-Identifier: MIT

//! Locate Steam libraries and installed games from an injectable root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::scan::{find_sodb_files, ScanLimits};
use crate::types::{
    AppId, BuildId, Discovery, FolderScan, FolderScanKind, GameIdentity, GameName, InstalledGame,
    LocalPath, Note, Warning, WarningKind,
};
use crate::vdf::{parse_vdf_file, VdfDocument, VdfValue};
use crate::Error;

/// Sentence printed once when a Steam library has games and none of them
/// contain a `.sodb` file. Steam does not deliver those files today.
pub const NO_SODB_STEAM_NOTE: &str = "no SODB files found (expected for games from Steam today)";

/// Extra inputs for [`discover_with_options`].
#[derive(Clone, Debug, Default)]
pub struct DiscoverOptions {
    pub scan: ScanLimits,
    /// Folders of `.sodb` files captured outside a Steam install.
    pub sodb_dirs: Vec<PathBuf>,
    /// Folders the user named with `--xbox-games`. Each one is walked as given.
    /// Empty skips that scan. This crate does not probe drive letters.
    pub xbox_dirs: Vec<PathBuf>,
}

/// Discover games under `steam_root`.
///
/// A missing root yields an empty game list and a [`WarningKind::MissingSteamRoot`]
/// warning. Unreadable libraries and incomplete manifests produce warnings and
/// a partial list. Never panics.
///
/// Duplicate app ids: keep the higher [`BuildId`], then the earlier library
/// after numeric-key sort (`"2"` before `"10"`). A [`WarningKind::DuplicateApp`]
/// is emitted. Manifest `appid` wins over the filename when both parse as
/// `u32` and they differ.
#[must_use]
pub fn discover(steam_root: &Path) -> Discovery {
    discover_with_scan(steam_root, ScanLimits::default())
}

#[must_use]
pub fn discover_with_scan(steam_root: &Path, scan: ScanLimits) -> Discovery {
    discover_with_options(
        Some(steam_root),
        &DiscoverOptions {
            scan,
            ..DiscoverOptions::default()
        },
    )
}

/// Discover Steam games and any extra folders named in `options`.
///
/// `steam_root == None` skips Steam entirely (no missing-root warning). Callers
/// that tried the platform defaults and found nothing should add that warning
/// themselves. Never panics.
#[must_use]
pub fn discover_with_options(steam_root: Option<&Path>, options: &DiscoverOptions) -> Discovery {
    let mut warnings = Vec::new();
    let mut notes = Vec::new();
    let mut games = Vec::new();
    let mut stale_libraries = 0_u32;

    if let Some(steam_root) = steam_root {
        if !steam_root.is_dir() {
            warnings.push(Warning::new(
                WarningKind::MissingSteamRoot,
                None,
                "Steam root does not exist or is not a directory",
            ));
        } else {
            games = discover_steam(
                steam_root,
                options.scan,
                &mut warnings,
                &mut stale_libraries,
            );
        }
    }

    if stale_libraries > 0 {
        notes.push(Note::new(stale_library_message(stale_libraries)));
    }
    if !games.is_empty() && games.iter().all(|game| game.sodb_files.is_empty()) {
        notes.push(Note::new(NO_SODB_STEAM_NOTE));
    }

    let mut folder_scans = Vec::new();
    for dir in &options.sodb_dirs {
        folder_scans.push(scan_extra_folder(
            dir,
            FolderScanKind::ExplicitDir,
            options.scan,
            &mut warnings,
        ));
    }
    for dir in &options.xbox_dirs {
        folder_scans.push(scan_extra_folder(
            dir,
            FolderScanKind::XboxGames,
            options.scan,
            &mut warnings,
        ));
    }

    sort_warnings(&mut warnings);
    Discovery {
        games,
        warnings,
        notes,
        folder_scans,
    }
}

fn discover_steam(
    steam_root: &Path,
    scan: ScanLimits,
    warnings: &mut Vec<Warning>,
    stale_libraries: &mut u32,
) -> Vec<InstalledGame> {
    let libraries = collect_libraries(steam_root, warnings, stale_libraries);
    let mut chosen: BTreeMap<AppId, ChosenGame> = BTreeMap::new();

    for (library_index, library) in libraries.iter().enumerate() {
        let steamapps = library.join("steamapps");
        if !steamapps.is_dir() {
            warnings.push(Warning::new(
                WarningKind::UnreadableLibrary,
                None,
                "a library folder has no readable steamapps directory",
            ));
            continue;
        }

        let mut manifests = match list_manifests(&steamapps) {
            Ok(paths) => paths,
            Err(()) => {
                warnings.push(Warning::new(
                    WarningKind::UnreadableLibrary,
                    None,
                    "a steamapps directory could not be read",
                ));
                continue;
            }
        };
        manifests.sort();

        for path in manifests {
            match load_game(&path, &steamapps, scan) {
                Ok(LoadOutcome::Game(game)) => {
                    record_game(&mut chosen, warnings, game, library_index);
                }
                Ok(LoadOutcome::Skipped(extra)) => warnings.extend(extra),
                Err(err) => warnings.push(vdf_warning(err, WarningKind::UnreadableManifest, None)),
            }
        }
    }

    let mut games: Vec<InstalledGame> = chosen.into_values().map(|c| c.installed).collect();
    games.sort_by_key(|g| g.identity.app_id());
    games
}

fn stale_library_message(count: u32) -> String {
    if count == 1 {
        String::from("1 library path listed in VDF is not a directory")
    } else {
        format!("{count} library paths listed in VDF are not directories")
    }
}

fn scan_extra_folder(
    dir: &Path,
    kind: FolderScanKind,
    scan: ScanLimits,
    warnings: &mut Vec<Warning>,
) -> FolderScan {
    if let Some(message) = extra_folder_problem(kind, dir) {
        warnings.push(Warning::new(
            WarningKind::UnreadableInstallDir,
            None,
            message,
        ));
        return FolderScan {
            kind,
            root: LocalPath::new(dir.to_path_buf()),
            sodb_files: Vec::new(),
        };
    }
    // User-named roots are not Steam installs, so the asset-directory skip
    // does not apply. Depth, file, entry, and time bounds still do.
    let outcome = find_sodb_files(dir, scan, None, false);
    warnings.extend(outcome.warnings);
    FolderScan {
        kind,
        root: LocalPath::new(dir.to_path_buf()),
        sodb_files: outcome.files,
    }
}

/// Path-free reason a user-named folder cannot be walked. `None` when `dir` is a directory.
fn extra_folder_problem(kind: FolderScanKind, dir: &Path) -> Option<&'static str> {
    if dir.is_dir() {
        return None;
    }
    Some(match (kind, dir.is_file()) {
        (FolderScanKind::ExplicitDir, true) => "--sodb-dir must be a folder",
        (FolderScanKind::ExplicitDir, false) => "--sodb-dir folder not found or not a directory",
        (FolderScanKind::XboxGames, true) => "--xbox-games must be a folder",
        (FolderScanKind::XboxGames, false) => "--xbox-games folder not found or not a directory",
    })
}

/// Count of walk-bound stops, with no paths and no per-game lines.
///
/// `None` when no scan hit a bound. The wording names files, entries, and time.
#[must_use]
pub fn scan_bound_summary(warnings: &[Warning]) -> Option<String> {
    let mut files = 0_u32;
    let mut entries = 0_u32;
    let mut time = 0_u32;
    for warning in warnings {
        if warning.kind() != WarningKind::SodbScanLimited {
            continue;
        }
        match warning.message() {
            msg if msg == crate::scan::WalkBound::Files.message() => {
                files = files.saturating_add(1);
            }
            msg if msg == crate::scan::WalkBound::Entries.message() => {
                entries = entries.saturating_add(1);
            }
            msg if msg == crate::scan::WalkBound::Time.message() => {
                time = time.saturating_add(1);
            }
            _ => {}
        }
    }
    let total = files.saturating_add(entries).saturating_add(time);
    if total == 0 {
        None
    } else {
        Some(format!(
            "stopped SODB scan after a walk bound (files={files}, entries={entries}, time={time})"
        ))
    }
}

struct ChosenGame {
    installed: InstalledGame,
    library_index: usize,
}

struct LoadedGame {
    installed: InstalledGame,
    extra_warnings: Vec<Warning>,
}

enum LoadOutcome {
    Game(LoadedGame),
    Skipped(Vec<Warning>),
}

fn record_game(
    chosen: &mut BTreeMap<AppId, ChosenGame>,
    warnings: &mut Vec<Warning>,
    game: LoadedGame,
    library_index: usize,
) {
    let app_id = game.installed.identity.app_id();
    let extras = game_scan_warnings(&game);
    if let Some(existing) = chosen.get(&app_id) {
        let new_build = game.installed.identity.build_id();
        let old_build = existing.installed.identity.build_id();
        let replace = new_build > old_build
            || (new_build == old_build && library_index < existing.library_index);
        warnings.push(Warning::new(
            WarningKind::DuplicateApp,
            Some(app_id),
            "duplicate app id; kept the higher build id, then the earlier library",
        ));
        if replace {
            chosen.insert(
                app_id,
                ChosenGame {
                    installed: game.installed,
                    library_index,
                },
            );
            warnings.extend(extras);
        }
        return;
    }
    warnings.extend(extras);
    chosen.insert(
        app_id,
        ChosenGame {
            installed: game.installed,
            library_index,
        },
    );
}

fn game_scan_warnings(game: &LoadedGame) -> Vec<Warning> {
    game.extra_warnings
        .iter()
        .cloned()
        .map(|w| w.with_app_id(game.installed.identity.app_id()))
        .collect()
}

fn list_manifests(steamapps: &Path) -> Result<Vec<PathBuf>, ()> {
    let rd = fs::read_dir(steamapps).map_err(|_| ())?;
    let mut out = Vec::new();
    for entry in rd {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if is_app_manifest(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

fn collect_libraries(
    steam_root: &Path,
    warnings: &mut Vec<Warning>,
    stale_libraries: &mut u32,
) -> Vec<PathBuf> {
    let mut keyed = Vec::new();
    let candidates = [
        steam_root.join("steamapps").join("libraryfolders.vdf"),
        steam_root.join("config").join("libraryfolders.vdf"),
    ];

    let mut parsed_any = false;
    for vdf_path in &candidates {
        if !vdf_path.exists() {
            continue;
        }
        match parse_vdf_file(vdf_path) {
            Ok(doc) => {
                parsed_any = true;
                keyed.extend(libraries_from_vdf(
                    &doc,
                    steam_root,
                    warnings,
                    stale_libraries,
                ));
            }
            Err(err) => {
                warnings.push(vdf_warning(err, WarningKind::MalformedVdf, None));
            }
        }
    }

    if !parsed_any {
        warnings.push(Warning::new(
            WarningKind::MissingLibraryList,
            None,
            "libraryfolders.vdf was missing or unreadable; using steamapps on the Steam root",
        ));
    }

    keyed.sort_by_key(|a| library_sort_key(&a.0));
    let mut libraries: Vec<PathBuf> = keyed.into_iter().map(|(_, p)| p).collect();
    libraries.push(steam_root.to_path_buf());
    dedup_library_paths(libraries)
}

fn library_sort_key(key: &str) -> (u8, u32, String) {
    match key.parse::<u32>() {
        Ok(n) => (0, n, key.to_string()),
        Err(_) => (1, 0, key.to_string()),
    }
}

fn libraries_from_vdf(
    doc: &VdfDocument,
    steam_root: &Path,
    warnings: &mut Vec<Warning>,
    stale_libraries: &mut u32,
) -> Vec<(String, PathBuf)> {
    let Some(table) = doc.root.as_table() else {
        warnings.push(Warning::new(
            WarningKind::MalformedVdf,
            None,
            "libraryfolders.vdf root is not a table",
        ));
        return Vec::new();
    };

    let mut out = Vec::new();
    for (key, value) in table {
        if !looks_like_library_key(key) {
            continue;
        }
        match library_path(value) {
            Some(raw) => {
                let path = resolve_library_path(steam_root, &raw);
                if path.is_dir() {
                    out.push((key.clone(), path));
                } else {
                    *stale_libraries = stale_libraries.saturating_add(1);
                }
            }
            None => warnings.push(Warning::new(
                WarningKind::MalformedVdf,
                None,
                "a library entry is missing a path",
            )),
        }
    }
    out
}

fn looks_like_library_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_digit())
}

fn library_path(value: &VdfValue) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    value
        .get("path")
        .and_then(VdfValue::as_str)
        .map(ToOwned::to_owned)
}

fn resolve_library_path(steam_root: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        steam_root.join(path)
    }
}

fn dedup_library_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for path in paths {
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.iter().any(|s| s == &key) {
            continue;
        }
        seen.push(key);
        out.push(path);
    }
    out
}

fn is_app_manifest(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.starts_with("appmanifest_") && lower.ends_with(".acf")
}

fn load_game(
    manifest_path: &Path,
    steamapps: &Path,
    scan: ScanLimits,
) -> Result<LoadOutcome, Error> {
    let doc = parse_vdf_file(manifest_path)?;
    let table = doc.root.as_table().ok_or(Error::Syntax)?;

    let filename_app = app_id_from_filename(manifest_path);
    let mut extra_warnings = Vec::new();

    let parsed_app = match table.get("appid").and_then(VdfValue::as_str) {
        None => None,
        Some(s) => match s.parse::<u32>() {
            Ok(n) => Some(n),
            Err(_) => {
                extra_warnings.push(Warning::new(
                    WarningKind::IncompleteManifest,
                    None,
                    "appid is not a valid u32",
                ));
                return Ok(LoadOutcome::Skipped(extra_warnings));
            }
        },
    };

    let Some(raw_app) = parsed_app.or(filename_app) else {
        extra_warnings.push(Warning::new(
            WarningKind::IncompleteManifest,
            None,
            "appmanifest is missing appid",
        ));
        return Ok(LoadOutcome::Skipped(extra_warnings));
    };
    let app_id = AppId(raw_app);

    if parsed_app.is_none() && filename_app.is_some() {
        extra_warnings.push(Warning::new(
            WarningKind::IncompleteManifest,
            Some(app_id),
            "appmanifest is missing appid; used the filename",
        ));
    }

    if let (Some(from_file), Some(from_name)) = (parsed_app, filename_app) {
        if from_file != from_name {
            extra_warnings.push(Warning::new(
                WarningKind::IncompleteManifest,
                Some(app_id),
                "manifest appid differs from the filename; using the manifest value",
            ));
        }
    }

    let name = match table.get("name").and_then(VdfValue::as_str) {
        Some(n) if !n.is_empty() => GameName::new(n),
        _ => {
            extra_warnings.push(Warning::new(
                WarningKind::IncompleteManifest,
                Some(app_id),
                "appmanifest is missing name",
            ));
            GameName::new("unknown")
        }
    };

    let build_id = match table.get("buildid").and_then(VdfValue::as_str) {
        Some(s) => match s.parse::<u64>() {
            Ok(n) => BuildId(n),
            Err(_) => {
                extra_warnings.push(Warning::new(
                    WarningKind::IncompleteManifest,
                    Some(app_id),
                    "appmanifest buildid is not a number",
                ));
                BuildId(0)
            }
        },
        None => {
            extra_warnings.push(Warning::new(
                WarningKind::IncompleteManifest,
                Some(app_id),
                "appmanifest is missing buildid",
            ));
            BuildId(0)
        }
    };

    let Some(installdir) = table.get("installdir").and_then(VdfValue::as_str) else {
        extra_warnings.push(Warning::new(
            WarningKind::IncompleteManifest,
            Some(app_id),
            "appmanifest is missing installdir; game skipped",
        ));
        return Ok(LoadOutcome::Skipped(extra_warnings));
    };

    if !is_plain_relative_installdir(installdir) {
        extra_warnings.push(Warning::new(
            WarningKind::InvalidInstallDir,
            Some(app_id),
            "appmanifest installdir is not a plain relative name; game skipped",
        ));
        return Ok(LoadOutcome::Skipped(extra_warnings));
    }

    let install_dir = match confine_install_dir(steamapps, installdir, app_id, &mut extra_warnings)
    {
        Confine::Skip => return Ok(LoadOutcome::Skipped(extra_warnings)),
        Confine::Missing(path) => {
            extra_warnings.push(Warning::new(
                WarningKind::UnreadableInstallDir,
                Some(app_id),
                "game install directory is missing",
            ));
            return Ok(LoadOutcome::Game(LoadedGame {
                installed: InstalledGame {
                    identity: GameIdentity::new(app_id, build_id, name),
                    install_dir: LocalPath::new(path),
                    sodb_files: Vec::new(),
                },
                extra_warnings,
            }));
        }
        Confine::Ready(path) => path,
    };
    let exe_dir = table
        .get("Executable")
        .or_else(|| table.get("executable"))
        .and_then(VdfValue::as_str)
        .and_then(exe_parent_dir);
    let scan_out = find_sodb_files(&install_dir, scan, exe_dir.as_deref(), true);
    extra_warnings.extend(scan_out.warnings);

    Ok(LoadOutcome::Game(LoadedGame {
        installed: InstalledGame {
            identity: GameIdentity::new(app_id, build_id, name),
            install_dir: LocalPath::new(install_dir),
            sodb_files: scan_out.files,
        },
        extra_warnings,
    }))
}

/// Parent directory of a manifest `Executable`, relative to the install dir.
///
/// `game.exe` has no parent folder (the install root is already scanned first).
/// Backslashes are normalized so a Windows manifest is testable on Linux.
/// Absolute paths and `..` are ignored.
fn exe_parent_dir(raw: &str) -> Option<PathBuf> {
    let normalized = raw.replace('\\', "/");
    if !is_plain_relative_installdir(&normalized) {
        return None;
    }
    let path = Path::new(&normalized);
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        None
    } else {
        Some(parent.to_path_buf())
    }
}

/// `installdir` must be a non-empty relative name made only of `Normal`
/// components. Absolute paths, `..`, empty, and Windows drive / verbatim
/// prefixes (`C:\x`, `\\?\C:\x`) are rejected on every OS.
pub(crate) fn is_plain_relative_installdir(installdir: &str) -> bool {
    if installdir.is_empty() {
        return false;
    }
    if installdir.starts_with('\\') || installdir.starts_with('/') {
        return false;
    }
    if looks_like_windows_prefix(installdir) {
        return false;
    }
    let path = Path::new(installdir);
    if path.is_absolute() {
        return false;
    }
    let mut any = false;
    for component in path.components() {
        match component {
            Component::Normal(name) if !name.is_empty() => any = true,
            _ => return false,
        }
    }
    any
}

fn looks_like_windows_prefix(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    raw.starts_with("\\\\") || raw.starts_with("//")
}

fn app_id_from_filename(path: &Path) -> Option<u32> {
    let name = path.file_stem()?.to_str()?;
    let digits = name.strip_prefix("appmanifest_")?;
    digits.parse().ok()
}

enum Confine {
    Ready(PathBuf),
    Missing(PathBuf),
    Skip,
}

fn confine_install_dir(
    steamapps: &Path,
    installdir: &str,
    app_id: AppId,
    extra_warnings: &mut Vec<Warning>,
) -> Confine {
    let common = steamapps.join("common");
    let install_dir = common.join(installdir);
    match fs::symlink_metadata(&install_dir) {
        Err(_) => return Confine::Missing(install_dir),
        Ok(meta) if meta.file_type().is_symlink() => {
            extra_warnings.push(Warning::new(
                WarningKind::InvalidInstallDir,
                Some(app_id),
                "install dir resolves outside the library; game skipped",
            ));
            return Confine::Skip;
        }
        Ok(meta) if !meta.is_dir() => return Confine::Missing(install_dir),
        Ok(_) => {}
    }
    let Ok(common_root) = fs::canonicalize(&common) else {
        extra_warnings.push(Warning::new(
            WarningKind::UnreadableInstallDir,
            Some(app_id),
            "game install directory could not be canonicalized",
        ));
        return Confine::Skip;
    };
    let Ok(canon) = fs::canonicalize(&install_dir) else {
        extra_warnings.push(Warning::new(
            WarningKind::UnreadableInstallDir,
            Some(app_id),
            "game install directory could not be canonicalized",
        ));
        return Confine::Skip;
    };
    if !canon.starts_with(&common_root) {
        extra_warnings.push(Warning::new(
            WarningKind::InvalidInstallDir,
            Some(app_id),
            "install dir resolves outside the library; game skipped",
        ));
        return Confine::Skip;
    }
    Confine::Ready(install_dir)
}

fn sort_warnings(warnings: &mut [Warning]) {
    warnings.sort_by(|a, b| {
        a.app_id()
            .cmp(&b.app_id())
            .then_with(|| a.kind().cmp(&b.kind()))
            .then_with(|| a.message().cmp(b.message()))
    });
}

fn vdf_warning(err: Error, kind: WarningKind, app_id: Option<AppId>) -> Warning {
    let message = match err {
        Error::FileTooLarge { .. } => "VDF file exceeds the 1 MiB size limit",
        Error::TooDeep { .. } => "VDF nesting exceeds the depth limit",
        Error::TooManyTokens { .. } => "VDF token count exceeds the limit",
        Error::NotARegularFile => "VDF path is not a regular file",
        Error::UnclosedString
        | Error::UnclosedBlock
        | Error::UnclosedComment
        | Error::InvalidEscape
        | Error::InvalidUtf8
        | Error::Syntax => "VDF text is malformed",
        Error::Io(_) => "VDF file could not be read",
    };
    Warning::new(kind, app_id, message)
}
