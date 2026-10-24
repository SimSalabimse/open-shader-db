// SPDX-License-Identifier: MIT

//! Locate Steam libraries and installed games from an injectable root.

use std::fs;
use std::path::{Path, PathBuf};

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
        return Discovery {
            games: Vec::new(),
            warnings,
        };
    }

    let libraries = collect_libraries(steam_root, &mut warnings);
    let mut games = Vec::new();
    let mut seen_apps = std::collections::BTreeSet::new();

    for library in libraries {
        let steamapps = library.join("steamapps");
        if !steamapps.is_dir() {
            warnings.push(Warning {
                kind: WarningKind::UnreadableLibrary,
                app_id: None,
                message: String::from("a library folder has no readable steamapps directory"),
            });
            continue;
        }

        let manifests = match fs::read_dir(&steamapps) {
            Ok(rd) => rd,
            Err(_) => {
                warnings.push(Warning {
                    kind: WarningKind::UnreadableLibrary,
                    app_id: None,
                    message: String::from("a steamapps directory could not be read"),
                });
                continue;
            }
        };

        for entry in manifests {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if !is_app_manifest(&path) {
                continue;
            }
            match load_game(&path, &steamapps, scan) {
                Ok(LoadOutcome::Game(game)) => {
                    if seen_apps.insert(game.installed.identity.app_id) {
                        warnings.extend(game_scan_warnings(&game));
                        games.push(game.installed);
                    }
                }
                Ok(LoadOutcome::Skipped(extra)) => warnings.extend(extra),
                Err(err) => warnings.push(vdf_warning(err, WarningKind::UnreadableManifest, None)),
            }
        }
    }

    games.sort_by_key(|g| g.identity.app_id);
    Discovery { games, warnings }
}

struct LoadedGame {
    installed: InstalledGame,
    extra_warnings: Vec<Warning>,
}

enum LoadOutcome {
    Game(LoadedGame),
    Skipped(Vec<Warning>),
}

fn game_scan_warnings(game: &LoadedGame) -> Vec<Warning> {
    game.extra_warnings
        .iter()
        .map(|w| Warning {
            kind: w.kind,
            app_id: Some(game.installed.identity.app_id),
            message: w.message.clone(),
        })
        .collect()
}

fn collect_libraries(steam_root: &Path, warnings: &mut Vec<Warning>) -> Vec<PathBuf> {
    let mut libraries = Vec::new();
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
                libraries.extend(libraries_from_vdf(&doc, steam_root, warnings));
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

    libraries.push(steam_root.to_path_buf());
    dedup_library_paths(libraries)
}

fn libraries_from_vdf(
    doc: &VdfDocument,
    steam_root: &Path,
    warnings: &mut Vec<Warning>,
) -> Vec<PathBuf> {
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
                    out.push(path);
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
    key.chars().all(|c| c.is_ascii_digit())
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
    let parsed_app = table
        .get("appid")
        .and_then(VdfValue::as_str)
        .and_then(|s| s.parse::<u32>().ok());

    let Some(raw_app) = parsed_app.or(filename_app) else {
        return Ok(LoadOutcome::Skipped(vec![Warning {
            kind: WarningKind::IncompleteManifest,
            app_id: None,
            message: String::from("appmanifest is missing appid"),
        }]));
    };
    let app_id = AppId(raw_app);

    let mut extra_warnings = Vec::new();

    if parsed_app.is_none() {
        extra_warnings.push(Warning {
            kind: WarningKind::IncompleteManifest,
            app_id: Some(app_id),
            message: String::from("appmanifest is missing appid; used the filename"),
        });
    }

    let name = match table.get("name").and_then(VdfValue::as_str) {
        Some(n) if !n.is_empty() => GameName(n.to_string()),
        _ => {
            extra_warnings.push(Warning {
                kind: WarningKind::IncompleteManifest,
                app_id: Some(app_id),
                message: String::from("appmanifest is missing name"),
            });
            GameName(String::from("unknown"))
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
            identity: GameIdentity {
                app_id,
                build_id,
                name,
            },
            install_dir: LocalPath::new(install_dir),
            sodb_files: scan_out.files,
        },
        extra_warnings,
    }))
}

fn app_id_from_filename(path: &Path) -> Option<u32> {
    let name = path.file_stem()?.to_str()?;
    let digits = name.strip_prefix("appmanifest_")?;
    digits.parse().ok()
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
