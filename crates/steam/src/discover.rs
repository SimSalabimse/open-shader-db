// SPDX-License-Identifier: MIT

//! Locate Steam libraries and installed games from an injectable root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::scan::{find_sodb_files, ScanLimits};
use crate::types::{
    AppId, BuildId, Discovery, GameIdentity, GameName, InstalledGame, LocalPath, Warning,
    WarningKind,
};
use crate::vdf::{parse_vdf_file, VdfDocument, VdfValue};
use crate::Error;

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
    let mut warnings = Vec::new();

    if !steam_root.is_dir() {
        warnings.push(Warning {
            kind: WarningKind::MissingSteamRoot,
            app_id: None,
            message: String::from("Steam root does not exist or is not a directory"),
        });
        sort_warnings(&mut warnings);
        return Discovery {
            games: Vec::new(),
            warnings,
        };
    }

    let libraries = collect_libraries(steam_root, &mut warnings);
    let mut chosen: BTreeMap<AppId, ChosenGame> = BTreeMap::new();

    for (library_index, library) in libraries.iter().enumerate() {
        let steamapps = library.join("steamapps");
        if !steamapps.is_dir() {
            warnings.push(Warning {
                kind: WarningKind::UnreadableLibrary,
                app_id: None,
                message: String::from("a library folder has no readable steamapps directory"),
            });
            continue;
        }

        let mut manifests = match list_manifests(&steamapps) {
            Ok(paths) => paths,
            Err(()) => {
                warnings.push(Warning {
                    kind: WarningKind::UnreadableLibrary,
                    app_id: None,
                    message: String::from("a steamapps directory could not be read"),
                });
                continue;
            }
        };
        manifests.sort();

        for path in manifests {
            match load_game(&path, &steamapps, scan) {
                Ok(LoadOutcome::Game(game)) => {
                    record_game(&mut chosen, &mut warnings, game, library_index);
                }
                Ok(LoadOutcome::Skipped(extra)) => warnings.extend(extra),
                Err(err) => warnings.push(vdf_warning(err, WarningKind::UnreadableManifest, None)),
            }
        }
    }

    let mut games: Vec<InstalledGame> = chosen.into_values().map(|c| c.installed).collect();
    games.sort_by_key(|g| g.identity.app_id());
    sort_warnings(&mut warnings);
    Discovery { games, warnings }
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
        warnings.push(Warning {
            kind: WarningKind::DuplicateApp,
            app_id: Some(app_id),
            message: String::from(
                "duplicate app id; kept the higher build id, then the earlier library",
            ),
        });
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
        .map(|w| Warning {
            kind: w.kind,
            app_id: Some(game.installed.identity.app_id()),
            message: w.message.clone(),
        })
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

fn collect_libraries(steam_root: &Path, warnings: &mut Vec<Warning>) -> Vec<PathBuf> {
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
                keyed.extend(libraries_from_vdf(&doc, steam_root, warnings));
            }
            Err(err) => {
                warnings.push(vdf_warning(err, WarningKind::MalformedVdf, None));
            }
        }
    }

    if !parsed_any {
        warnings.push(Warning {
            kind: WarningKind::MissingLibraryList,
            app_id: None,
            message: String::from(
                "libraryfolders.vdf was missing or unreadable; using steamapps on the Steam root",
            ),
        });
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
) -> Vec<(String, PathBuf)> {
    let Some(table) = doc.root.as_table() else {
        warnings.push(Warning {
            kind: WarningKind::MalformedVdf,
            app_id: None,
            message: String::from("libraryfolders.vdf root is not a table"),
        });
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
                    warnings.push(Warning {
                        kind: WarningKind::UnreadableLibrary,
                        app_id: None,
                        message: String::from("a library path listed in VDF is not a directory"),
                    });
                }
            }
            None => warnings.push(Warning {
                kind: WarningKind::MalformedVdf,
                app_id: None,
                message: String::from("a library entry is missing a path"),
            }),
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
                extra_warnings.push(Warning {
                    kind: WarningKind::IncompleteManifest,
                    app_id: None,
                    message: String::from("appid is not a valid u32"),
                });
                return Ok(LoadOutcome::Skipped(extra_warnings));
            }
        },
    };

    let Some(raw_app) = parsed_app.or(filename_app) else {
        extra_warnings.push(Warning {
            kind: WarningKind::IncompleteManifest,
            app_id: None,
            message: String::from("appmanifest is missing appid"),
        });
        return Ok(LoadOutcome::Skipped(extra_warnings));
    };
    let app_id = AppId(raw_app);

    if parsed_app.is_none() && filename_app.is_some() {
        extra_warnings.push(Warning {
            kind: WarningKind::IncompleteManifest,
            app_id: Some(app_id),
            message: String::from("appmanifest is missing appid; used the filename"),
        });
    }

    if let (Some(from_file), Some(from_name)) = (parsed_app, filename_app) {
        if from_file != from_name {
            extra_warnings.push(Warning {
                kind: WarningKind::IncompleteManifest,
                app_id: Some(app_id),
                message: String::from(
                    "manifest appid differs from the filename; using the manifest value",
                ),
            });
        }
    }

    let name = match table.get("name").and_then(VdfValue::as_str) {
        Some(n) if !n.is_empty() => GameName::new(n),
        _ => {
            extra_warnings.push(Warning {
                kind: WarningKind::IncompleteManifest,
                app_id: Some(app_id),
                message: String::from("appmanifest is missing name"),
            });
            GameName::new("unknown")
        }
    };

    let build_id = match table.get("buildid").and_then(VdfValue::as_str) {
        Some(s) => match s.parse::<u64>() {
            Ok(n) => BuildId(n),
            Err(_) => {
                extra_warnings.push(Warning {
                    kind: WarningKind::IncompleteManifest,
                    app_id: Some(app_id),
                    message: String::from("appmanifest buildid is not a number"),
                });
                BuildId(0)
            }
        },
        None => {
            extra_warnings.push(Warning {
                kind: WarningKind::IncompleteManifest,
                app_id: Some(app_id),
                message: String::from("appmanifest is missing buildid"),
            });
            BuildId(0)
        }
    };

    let Some(installdir) = table.get("installdir").and_then(VdfValue::as_str) else {
        extra_warnings.push(Warning {
            kind: WarningKind::IncompleteManifest,
            app_id: Some(app_id),
            message: String::from("appmanifest is missing installdir; game skipped"),
        });
        return Ok(LoadOutcome::Skipped(extra_warnings));
    };

    if !is_plain_relative_installdir(installdir) {
        extra_warnings.push(Warning {
            kind: WarningKind::InvalidInstallDir,
            app_id: Some(app_id),
            message: String::from(
                "appmanifest installdir is not a plain relative name; game skipped",
            ),
        });
        return Ok(LoadOutcome::Skipped(extra_warnings));
    }

    let install_dir = steamapps.join("common").join(installdir);
    let scan_out = if install_dir.is_dir() {
        find_sodb_files(&install_dir, scan)
    } else {
        crate::scan::ScanOutcome {
            files: Vec::new(),
            warnings: vec![Warning {
                kind: WarningKind::UnreadableInstallDir,
                app_id: None,
                message: String::from("game install directory is missing"),
            }],
        }
    };
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

fn sort_warnings(warnings: &mut [Warning]) {
    warnings.sort_by(|a, b| {
        a.app_id
            .cmp(&b.app_id)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.message.cmp(&b.message))
    });
}

fn vdf_warning(err: Error, kind: WarningKind, app_id: Option<AppId>) -> Warning {
    let message = match err {
        Error::FileTooLarge { .. } => String::from("VDF file exceeds the 1 MiB size limit"),
        Error::TooDeep { .. } => String::from("VDF nesting exceeds the depth limit"),
        Error::TooManyTokens { .. } => String::from("VDF token count exceeds the limit"),
        Error::NotARegularFile => String::from("VDF path is not a regular file"),
        Error::UnclosedString
        | Error::UnclosedBlock
        | Error::UnclosedComment
        | Error::InvalidEscape
        | Error::InvalidUtf8
        | Error::Syntax => String::from("VDF text is malformed"),
        Error::Io(_) => String::from("VDF file could not be read"),
    };
    Warning {
        kind,
        app_id,
        message,
    }
}
