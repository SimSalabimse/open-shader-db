// SPDX-License-Identifier: MIT

//! Bounded walk of a game install dir for SODB files.
//!
//! The walk is breadth-first: the install root, then every depth-1 directory,
//! then every depth-2 directory, before anything deeper. Only names that end
//! in `.sodb` are opened. On a Steam install walk, directory names that are
//! usually large asset trees are not entered. That skip is a heuristic
//! (inferred), not a claim that an SODB cannot live there. A folder the user
//! named (`--sodb-dir` or `--xbox-games`) is walked in full, still inside the
//! same depth, file, entry, and time bounds.

use std::collections::{HashSet, VecDeque};
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

/// Which walk cap stopped a scan. Messages name the cap and never a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkBound {
    Files,
    Entries,
    Time,
}

impl WalkBound {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Files => "stopped SODB scan after reaching the files walk bound",
            Self::Entries => "stopped SODB scan after reaching the entries walk bound",
            Self::Time => "stopped SODB scan after reaching the time walk bound",
        }
    }
}

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

/// Single path-component names that are not entered.
///
/// Heuristic only (inferred): these trees are often huge asset dumps. Skipping
/// them is not a claim that they are free of `.sodb` files. A directory on the
/// way to the game executable is still entered.
const HUGE_ASSET_DIR_NAMES: &[&str] = &[
    "content",
    "paks",
    "movies",
    "videos",
    "audio",
    "sounds",
    "textures",
    "streaming",
];

/// `true` when `canonicalize(path)` succeeds and is under `root`.
pub(crate) fn is_under_root(path: &Path, root: &Path) -> bool {
    match fs::canonicalize(path) {
        Ok(canon) => canon.starts_with(root),
        Err(_) => false,
    }
}

/// Find SODB files under `install_dir`.
///
/// `exe_dir` is an optional directory relative to the install root (the folder
/// that holds the game's main executable). That folder is visited before other
/// folders at the same depth. Uses [`sodb::is_sodb`] (header only) and only on
/// files whose extension is `.sodb`. Folder symlinks are followed only when
/// `canonicalize` succeeds and the target is still under the canonical install
/// dir. File symlinks are never followed. Every `read_dir` and `.sodb` probe
/// re-checks [`is_under_root`].
///
/// `skip_huge_asset_dirs` is for an automatically discovered Steam install.
/// User-named roots pass `false` and are walked fully, inside `limits`.
pub(crate) fn find_sodb_files(
    install_dir: &Path,
    limits: ScanLimits,
    exe_dir: Option<&Path>,
    skip_huge_asset_dirs: bool,
) -> ScanOutcome {
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut visited = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut considered = 0_u32;
    let mut entries = 0_u32;
    let mut bound: Option<WalkBound> = None;
    let mut scanned_exe_files = false;
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

    let mut queue = VecDeque::new();
    queue.push_back((root.clone(), 0_u32));
    mark_visited(&root, &mut visited);

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if started.elapsed() >= limits.max_duration {
            bound = Some(WalkBound::Time);
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

        let mut children = Vec::new();
        for entry in read {
            if started.elapsed() >= limits.max_duration {
                bound = Some(WalkBound::Time);
                break 'walk;
            }
            if entries >= limits.max_entries {
                bound = Some(WalkBound::Entries);
                break 'walk;
            }
            let Ok(entry) = entry else {
                continue;
            };
            entries = entries.saturating_add(1);
            if entries > limits.max_entries {
                bound = Some(WalkBound::Entries);
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
                if depth < limits.max_depth
                    && !skip_huge_asset_dir(&canon, &root, exe_dir, skip_huge_asset_dirs)
                {
                    children.push(canon);
                }
            } else if ft.is_dir() {
                if depth < limits.max_depth
                    && is_under_root(&path, &root)
                    && mark_visited(&path, &mut visited)
                    && !skip_huge_asset_dir(&path, &root, exe_dir, skip_huge_asset_dirs)
                {
                    children.push(path);
                }
            } else if ft.is_file()
                && !(scanned_exe_files && dir_is_exe(&dir, &root, exe_dir))
                && consider_file(
                    &path,
                    &root,
                    &mut considered,
                    &mut seen_files,
                    limits,
                    &mut files,
                )
            {
                bound = Some(WalkBound::Files);
                break 'walk;
            }
        }

        children.sort_by(|a, b| {
            exe_rank(a, &root, exe_dir)
                .cmp(&exe_rank(b, &root, exe_dir))
                .then_with(|| a.file_name().cmp(&b.file_name()))
        });
        // Root first, then the executable's own folder, then breadth-first.
        if depth == 0 {
            if let Some(rel) = exe_dir {
                let mut walk = WalkCounters {
                    limits,
                    started: &started,
                    entries: &mut entries,
                    considered: &mut considered,
                    seen_files: &mut seen_files,
                    files: &mut files,
                };
                let exe_scan = scan_exe_dir_files(&root, rel, &mut walk);
                scanned_exe_files = exe_scan.scanned;
                if let Some(hit) = exe_scan.bound {
                    bound = Some(hit);
                    break;
                }
            }
        }
        let next_depth = depth.saturating_add(1);
        for child in children {
            queue.push_back((child, next_depth));
        }
    }

    if let Some(bound) = bound {
        warnings.push(Warning::new(
            WarningKind::SodbScanLimited,
            None,
            bound.message(),
        ));
    }

    files.sort_by(|a, b| a.as_path().cmp(b.as_path()));
    ScanOutcome { files, warnings }
}

struct ExeDirScan {
    scanned: bool,
    bound: Option<WalkBound>,
}

/// Read files in the executable's directory before the rest of the tree.
///
/// Subdirectories are left for the breadth-first walk. Missing directories
/// are ignored (`scanned` stays false).
struct WalkCounters<'a> {
    limits: ScanLimits,
    started: &'a Instant,
    entries: &'a mut u32,
    considered: &'a mut u32,
    seen_files: &'a mut HashSet<FileId>,
    files: &'a mut Vec<LocalPath>,
}

fn scan_exe_dir_files(root: &Path, exe_dir: &Path, walk: &mut WalkCounters<'_>) -> ExeDirScan {
    let depth = exe_dir.components().count() as u32;
    if depth == 0 || depth > walk.limits.max_depth {
        return ExeDirScan {
            scanned: false,
            bound: None,
        };
    }
    let joined = root.join(exe_dir);
    let Ok(canon) = fs::canonicalize(&joined) else {
        return ExeDirScan {
            scanned: false,
            bound: None,
        };
    };
    if !canon.starts_with(root) {
        return ExeDirScan {
            scanned: false,
            bound: None,
        };
    }
    let Ok(read) = fs::read_dir(&canon) else {
        return ExeDirScan {
            scanned: false,
            bound: None,
        };
    };
    for entry in read {
        if walk.started.elapsed() >= walk.limits.max_duration {
            return ExeDirScan {
                scanned: true,
                bound: Some(WalkBound::Time),
            };
        }
        if *walk.entries >= walk.limits.max_entries {
            return ExeDirScan {
                scanned: true,
                bound: Some(WalkBound::Entries),
            };
        }
        let Ok(entry) = entry else {
            continue;
        };
        *walk.entries = walk.entries.saturating_add(1);
        if *walk.entries > walk.limits.max_entries {
            return ExeDirScan {
                scanned: true,
                bound: Some(WalkBound::Entries),
            };
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_file()
            && consider_file(
                &entry.path(),
                root,
                walk.considered,
                walk.seen_files,
                walk.limits,
                walk.files,
            )
        {
            return ExeDirScan {
                scanned: true,
                bound: Some(WalkBound::Files),
            };
        }
    }
    ExeDirScan {
        scanned: true,
        bound: None,
    }
}

fn dir_is_exe(dir: &Path, root: &Path, exe_dir: Option<&Path>) -> bool {
    let Some(exe_dir) = exe_dir else {
        return false;
    };
    dir.strip_prefix(root)
        .ok()
        .is_some_and(|rel| rel == exe_dir)
}

/// Returns `true` when the file-count bound stopped the walk.
fn consider_file(
    path: &Path,
    root: &Path,
    considered: &mut u32,
    seen_files: &mut HashSet<FileId>,
    limits: ScanLimits,
    files: &mut Vec<LocalPath>,
) -> bool {
    if *considered >= limits.max_files {
        return true;
    }
    if !has_sodb_extension(path) {
        *considered = considered.saturating_add(1);
        return false;
    }
    if !is_under_root(path, root) {
        return false;
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

fn has_sodb_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sodb"))
}

fn skip_huge_asset_dir(
    dir: &Path,
    root: &Path,
    exe_dir: Option<&Path>,
    skip_huge_asset_dirs: bool,
) -> bool {
    if !skip_huge_asset_dirs {
        return false;
    }
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if !HUGE_ASSET_DIR_NAMES
        .iter()
        .any(|huge| name.eq_ignore_ascii_case(huge))
    {
        return false;
    }
    // Still walk the executable's own folder, and its parents, so a capture
    // that sits next to the exe is not dropped because of this heuristic.
    !leads_toward_exe(dir, root, exe_dir)
}

fn leads_toward_exe(dir: &Path, root: &Path, exe_dir: Option<&Path>) -> bool {
    let Some(exe_dir) = exe_dir else {
        return false;
    };
    let Ok(rel) = dir.strip_prefix(root) else {
        return false;
    };
    if rel.as_os_str().is_empty() {
        return false;
    }
    // `rel` is the exe directory, or a parent of it. Descendants stay subject
    // to the asset-directory heuristic.
    exe_dir.starts_with(rel)
}

/// Lower rank is visited first among siblings. The exe directory (and its
/// parents) sort ahead of every other name at that depth.
fn exe_rank(dir: &Path, root: &Path, exe_dir: Option<&Path>) -> u8 {
    if leads_toward_exe(dir, root, exe_dir) {
        0
    } else {
        1
    }
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
