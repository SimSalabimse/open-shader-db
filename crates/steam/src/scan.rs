// SPDX-License-Identifier: MIT

//! Bounded walk of a game install dir for SODB files.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;

use crate::types::{LocalPath, Warning, WarningKind};

/// Default subdirectory depth from the game install dir (install dir is 0).
pub const DEFAULT_SODB_DEPTH: u32 = 4;
/// Default cap on regular files considered during a walk.
pub const DEFAULT_MAX_FILES: u32 = 10_000;

#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_depth: u32,
    pub max_files: u32,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_SODB_DEPTH,
            max_files: DEFAULT_MAX_FILES,
        }
    }
}

pub(crate) struct ScanOutcome {
    pub files: Vec<LocalPath>,
    pub warnings: Vec<Warning>,
}

/// Find `*.sodb` and other regular files whose SQLite header is SODB v2.
///
/// Does not parse tables. Uses [`sodb::is_sodb`] (header only). Skips FIFOs
/// and other non-regular files without opening them. Symlink loops are
/// detected via device/inode (Unix) or canonical paths (other OS).
pub(crate) fn find_sodb_files(install_dir: &Path, limits: ScanLimits) -> ScanOutcome {
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut visited = HashSet::new();
    let mut considered = 0_u32;
    let mut limited = false;

    let mut stack = vec![(install_dir.to_path_buf(), 0_u32)];
    mark_visited(install_dir, &mut visited);

    while let Some((dir, depth)) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                if depth == 0 {
                    warnings.push(Warning {
                        kind: WarningKind::UnreadableInstallDir,
                        app_id: None,
                        message: String::from("game install directory could not be read"),
                    });
                }
                continue;
            }
        };

        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };

            if ft.is_symlink() {
                let Ok(meta) = fs::metadata(&path) else {
                    continue;
                };
                if !mark_meta(&meta, &path, &mut visited) {
                    continue;
                }
                if meta.is_dir() {
                    if depth < limits.max_depth {
                        stack.push((path, depth.saturating_add(1)));
                    }
                } else if meta.is_file() {
                    consider_file(&path, &mut considered, limits, &mut files, &mut limited);
                }
            } else if ft.is_dir() {
                if depth < limits.max_depth && mark_visited(&path, &mut visited) {
                    stack.push((path, depth.saturating_add(1)));
                }
            } else if ft.is_file() {
                consider_file(&path, &mut considered, limits, &mut files, &mut limited);
            }
        }
    }

    if limited {
        warnings.push(Warning {
            kind: WarningKind::SodbScanLimited,
            app_id: None,
            message: String::from("stopped SODB scan after reaching the file-count bound"),
        });
    }

    files.sort_by(|a, b| a.as_path().cmp(b.as_path()));
    ScanOutcome { files, warnings }
}

fn consider_file(
    path: &Path,
    considered: &mut u32,
    limits: ScanLimits,
    files: &mut Vec<LocalPath>,
    limited: &mut bool,
) {
    if *considered >= limits.max_files {
        *limited = true;
        return;
    }
    *considered = considered.saturating_add(1);
    // Header-only SODB check (`sodb::is_sodb`). Non-SQLite files return false
    // after a 100-byte read. FIFOs never reach this function.
    if sodb::is_sodb(path) {
        files.push(LocalPath::new(path.to_path_buf()));
    }
}

fn mark_visited(path: &Path, visited: &mut HashSet<FileId>) -> bool {
    match fs::metadata(path) {
        Ok(meta) => mark_meta(&meta, path, visited),
        Err(_) => false,
    }
}

fn mark_meta(meta: &Metadata, path: &Path, visited: &mut HashSet<FileId>) -> bool {
    visited.insert(file_id(meta, path))
}

#[cfg(unix)]
type FileId = (u64, u64);

#[cfg(unix)]
fn file_id(meta: &Metadata, _path: &Path) -> FileId {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
type FileId = PathBuf;

#[cfg(not(unix))]
fn file_id(_meta: &Metadata, path: &Path) -> FileId {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
