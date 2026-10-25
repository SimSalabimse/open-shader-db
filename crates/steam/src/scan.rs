// SPDX-License-Identifier: MIT

//! Bounded walk of a game install dir for SODB files.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::types::{LocalPath, Warning, WarningKind};

/// Default subdirectory depth from the game install dir (install dir is 0).
pub const DEFAULT_SODB_DEPTH: u32 = 4;
/// Default cap on regular files considered during a walk.
pub const DEFAULT_MAX_FILES: u32 = 10_000;
/// Default cap on every directory entry (files, folders, links, other).
pub const DEFAULT_MAX_ENTRIES: u32 = 50_000;
/// Default wall-clock budget for one install-dir walk.
pub const DEFAULT_MAX_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_depth: u32,
    pub max_files: u32,
    pub max_entries: u32,
    pub max_duration: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_SODB_DEPTH,
            max_files: DEFAULT_MAX_FILES,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_duration: DEFAULT_MAX_DURATION,
        }
    }
}

pub(crate) struct ScanOutcome {
    pub files: Vec<LocalPath>,
    pub warnings: Vec<Warning>,
}

/// Find SODB files under `install_dir`.
///
/// Uses [`sodb::is_sodb`] (header only). Symlinks are followed only when
/// `canonicalize` succeeds and the target is still under the canonical
/// install dir. Symlink loops are detected via device/inode (Unix) or
/// canonical paths (other OS). Every directory entry counts toward
/// `max_entries`.
pub(crate) fn find_sodb_files(install_dir: &Path, limits: ScanLimits) -> ScanOutcome {
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut visited = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut considered = 0_u32;
    let mut entries = 0_u32;
    let mut limited = false;
    let started = Instant::now();

    let Ok(root) = fs::canonicalize(install_dir) else {
        return ScanOutcome {
            files: Vec::new(),
            warnings: vec![Warning {
                kind: WarningKind::UnreadableInstallDir,
                app_id: None,
                message: String::from("game install directory could not be canonicalized"),
            }],
        };
    };

    let mut stack = vec![(root.clone(), 0_u32)];
    mark_visited(&root, &mut visited);

    'walk: while let Some((dir, depth)) = stack.pop() {
        if started.elapsed() >= limits.max_duration {
            limited = true;
            stack.clear();
            break;
        }

        let read = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                warnings.push(Warning {
                    kind: WarningKind::UnreadableInstallDir,
                    app_id: None,
                    message: if depth == 0 {
                        String::from("game install directory could not be read")
                    } else {
                        String::from("a subdirectory could not be read")
                    },
                });
                continue;
            }
        };

        for entry in read {
            if started.elapsed() >= limits.max_duration || entries >= limits.max_entries {
                limited = true;
                stack.clear();
                break 'walk;
            }
            let Ok(entry) = entry else {
                continue;
            };
            entries = entries.saturating_add(1);
            if entries > limits.max_entries {
                limited = true;
                stack.clear();
                break 'walk;
            }

            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };

            if ft.is_symlink() {
                let Some(canon) = canonicalize_inside(&root, &path) else {
                    continue;
                };
                let Ok(meta) = fs::metadata(&canon) else {
                    continue;
                };
                if !mark_meta(&meta, &canon, &mut visited) {
                    continue;
                }
                if meta.is_dir() {
                    if depth < limits.max_depth {
                        stack.push((canon, depth.saturating_add(1)));
                    }
                } else if meta.is_file()
                    && consider_file(
                        &canon,
                        &meta,
                        &mut considered,
                        &mut seen_files,
                        limits,
                        &mut files,
                        &mut limited,
                    )
                {
                    stack.clear();
                    break 'walk;
                }
            } else if ft.is_dir() {
                if depth < limits.max_depth && mark_visited(&path, &mut visited) {
                    stack.push((path, depth.saturating_add(1)));
                }
            } else if ft.is_file() {
                let Ok(meta) = fs::metadata(&path) else {
                    continue;
                };
                if consider_file(
                    &path,
                    &meta,
                    &mut considered,
                    &mut seen_files,
                    limits,
                    &mut files,
                    &mut limited,
                ) {
                    stack.clear();
                    break 'walk;
                }
            }
        }
    }

    if limited {
        warnings.push(Warning {
            kind: WarningKind::SodbScanLimited,
            app_id: None,
            message: String::from(
                "stopped SODB scan after reaching a walk bound (files, entries, or time)",
            ),
        });
    }

    files.sort_by(|a, b| a.as_path().cmp(b.as_path()));
    ScanOutcome { files, warnings }
}

/// Returns `true` when a file-count bound stopped the walk.
fn consider_file(
    path: &Path,
    meta: &Metadata,
    considered: &mut u32,
    seen_files: &mut HashSet<FileId>,
    limits: ScanLimits,
    files: &mut Vec<LocalPath>,
    limited: &mut bool,
) -> bool {
    if *considered >= limits.max_files {
        *limited = true;
        return true;
    }
    *considered = considered.saturating_add(1);
    if !seen_files.insert(file_id(meta, path)) {
        return false;
    }
    // Header-only SODB check. Unix `probe` opens with O_NONBLOCK so a FIFO
    // swap cannot hang; this walk already skipped non-regular lstat types.
    if sodb::is_sodb(path) {
        files.push(LocalPath::new(path.to_path_buf()));
    }
    false
}

fn canonicalize_inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let canon = fs::canonicalize(path).ok()?;
    if canon.starts_with(root) {
        Some(canon)
    } else {
        None
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
