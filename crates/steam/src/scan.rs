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
/// Default wall-clock budget for one install-dir walk (per game).
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

/// `true` when `canonicalize(path)` succeeds and is under `root`.
pub(crate) fn is_under_root(path: &Path, root: &Path) -> bool {
    match fs::canonicalize(path) {
        Ok(canon) => canon.starts_with(root),
        Err(_) => false,
    }
}

/// Find SODB files under `install_dir`.
///
/// Uses [`sodb::is_sodb`] (header only). Folder symlinks are followed only
/// when `canonicalize` succeeds and the target is still under the canonical
/// install dir. File symlinks are never followed. Every `read_dir` and file
/// probe re-checks [`is_under_root`].
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
            warnings: vec![Warning::new(
                WarningKind::UnreadableInstallDir,
                None,
                "game install directory could not be canonicalized",
            )],
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
        if !is_under_root(&dir, &root) {
            continue;
        }

        let read = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                warnings.push(Warning::new(
                    WarningKind::UnreadableInstallDir,
                    None,
                    if depth == 0 {
                        "game install directory could not be read"
                    } else {
                        "a subdirectory could not be read"
                    },
                ));
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
                if !is_under_root(&canon, &root) {
                    continue;
                }
                let Ok(meta) = fs::symlink_metadata(&canon) else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                if !mark_meta(&meta, &canon, &mut visited) {
                    continue;
                }
                if depth < limits.max_depth {
                    stack.push((canon, depth.saturating_add(1)));
                }
            } else if ft.is_dir() {
                if depth < limits.max_depth
                    && is_under_root(&path, &root)
                    && mark_visited(&path, &mut visited)
                {
                    stack.push((path, depth.saturating_add(1)));
                }
            } else if ft.is_file()
                && consider_file(
                    &path,
                    &root,
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
        }
    }

    if limited {
        warnings.push(Warning::new(
            WarningKind::SodbScanLimited,
            None,
            "stopped SODB scan after reaching a walk bound (files, entries, or time)",
        ));
    }

    files.sort_by(|a, b| a.as_path().cmp(b.as_path()));
    ScanOutcome { files, warnings }
}

/// Returns `true` when a file-count bound stopped the walk.
fn consider_file(
    path: &Path,
    root: &Path,
    considered: &mut u32,
    seen_files: &mut HashSet<FileId>,
    limits: ScanLimits,
    files: &mut Vec<LocalPath>,
    limited: &mut bool,
) -> bool {
    if !is_under_root(path, root) {
        return false;
    }
    if *considered >= limits.max_files {
        *limited = true;
        return true;
    }
    *considered = considered.saturating_add(1);
    let Ok(meta) = open_file_meta(path) else {
        return false;
    };
    if !seen_files.insert(file_id(&meta, path)) {
        return false;
    }
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
    match fs::symlink_metadata(path) {
        Ok(meta) => mark_meta(&meta, path, visited),
        Err(_) => false,
    }
}

fn mark_meta(meta: &Metadata, path: &Path, visited: &mut HashSet<FileId>) -> bool {
    visited.insert(file_id(meta, path))
}

/// Open a candidate file without following a symlink (Unix).
fn open_file_meta(path: &Path) -> std::io::Result<Metadata> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)?;
        let meta = file.metadata()?;
        if meta.is_file() {
            Ok(meta)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ))
        }
    }
    #[cfg(not(unix))]
    {
        let meta = fs::symlink_metadata(path)?;
        if meta.is_file() {
            Ok(meta)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ))
        }
    }
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
