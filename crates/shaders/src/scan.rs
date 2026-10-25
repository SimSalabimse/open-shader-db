// SPDX-License-Identifier: MIT

//! Read-only scan of SODB files and loose DXBC trees.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use osdb_records::Sha256;
use sodb::{
    hash_shader_reader, parse_dxbc_header, starts_with_dxbc, DxbcHeaderError, DXBC_HEADER_LEN,
};

use crate::index::LocalShaderIndex;
use crate::types::{ByteLen, FileOffset, ShaderLocation, ShaderSourceKind};
use crate::{Error, Limits, ScanCap, ScanReport};

/// One user-supplied scan input.
#[derive(Debug, Clone)]
pub enum ScanInput {
    /// SODB file the user already has.
    Sodb(PathBuf),
    /// Directory tree of loose files.
    Directory(PathBuf),
}

/// Scan every input into one index. Report is counts only.
pub fn scan(inputs: &[ScanInput], limits: Limits) -> Result<(LocalShaderIndex, ScanReport), Error> {
    let mut index = LocalShaderIndex::new();
    let mut report = ScanReport::default();
    let mut hashed_bytes = 0_u64;
    for input in inputs {
        match input {
            ScanInput::Sodb(path) => {
                scan_sodb_into(path, limits, &mut index, &mut report, &mut hashed_bytes)?;
            }
            ScanInput::Directory(path) => {
                scan_dir_into(path, limits, &mut index, &mut report, &mut hashed_bytes)?;
            }
        }
        report.sources = report.sources.saturating_add(1);
    }
    Ok((index, report))
}

/// Scan one SODB with default-or-supplied limits.
pub fn scan_sodb(
    path: impl AsRef<Path>,
    limits: Limits,
) -> Result<(LocalShaderIndex, ScanReport), Error> {
    scan(&[ScanInput::Sodb(path.as_ref().to_path_buf())], limits)
}

/// Scan one directory tree with default-or-supplied limits.
pub fn scan_dir(
    path: impl AsRef<Path>,
    limits: Limits,
) -> Result<(LocalShaderIndex, ScanReport), Error> {
    scan(&[ScanInput::Directory(path.as_ref().to_path_buf())], limits)
}

fn scan_sodb_into(
    path: &Path,
    limits: Limits,
    index: &mut LocalShaderIndex,
    report: &mut ScanReport,
    hashed_bytes: &mut u64,
) -> Result<(), Error> {
    if !path.exists() {
        return Err(Error::MissingRoot);
    }
    let meta = fs::symlink_metadata(path).map_err(|_| Error::UnreadableRoot)?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(Error::NotARegularFile);
    }
    report.files_seen = report.files_seen.saturating_add(1);
    if meta.len() > limits.max_file_bytes {
        return Err(Error::CapExceeded {
            cap: ScanCap::PerFileBytes,
            max: limits.max_file_bytes,
            got: meta.len(),
        });
    }
    let refs = sodb::list_bytecode_blobs(path)?;
    let source = index.add_source(ShaderSourceKind::Sodb, path.to_path_buf());
    for blob in refs {
        charge_bytes(hashed_bytes, blob.len, limits)?;
        let hash = Sha256::from_sodb(blob.hash);
        let offset = FileOffset(blob.rowid.unwrap_or(0) as u64);
        index.insert(
            hash,
            ShaderLocation {
                source,
                offset,
                len: ByteLen(blob.len),
            },
            report,
            limits.max_index_entries,
        )?;
    }
    Ok(())
}

fn scan_dir_into(
    path: &Path,
    limits: Limits,
    index: &mut LocalShaderIndex,
    report: &mut ScanReport,
    hashed_bytes: &mut u64,
) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(Error::MissingRoot),
        Err(_) => return Err(Error::UnreadableRoot),
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(Error::UnreadableRoot);
            }
            if !meta.file_type().is_dir() {
                return Err(Error::NotADirectory);
            }
        }
    }
    let root = fs::canonicalize(path).map_err(|_| Error::UnreadableRoot)?;
    index.directory_roots.push(root.clone());
    let mut stack = vec![(path.to_path_buf(), 0_u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > limits.max_depth {
            return Err(Error::CapExceeded {
                cap: ScanCap::Depth,
                max: u64::from(limits.max_depth),
                got: u64::from(depth),
            });
        }
        let mut entries = match fs::read_dir(&dir) {
            Ok(rd) => match rd.collect::<Result<Vec<_>, _>>() {
                Ok(v) => v,
                Err(_) => {
                    if depth == 0 {
                        return Err(Error::UnreadableRoot);
                    }
                    report.skipped_unreadable = report.skipped_unreadable.saturating_add(1);
                    continue;
                }
            },
            Err(_) => {
                if depth == 0 {
                    return Err(Error::UnreadableRoot);
                }
                report.skipped_unreadable = report.skipped_unreadable.saturating_add(1);
                continue;
            }
        };
        entries.sort_by_key(|a| a.file_name());
        let mut subdirs = Vec::new();
        for entry in entries {
            let child = entry.path();
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => {
                    report.skipped_unreadable = report.skipped_unreadable.saturating_add(1);
                    continue;
                }
            };
            if ft.is_symlink() {
                report.skipped_symlink = report.skipped_symlink.saturating_add(1);
                continue;
            }
            if ft.is_dir() {
                if !is_under_root(&child, &root) {
                    report.skipped_symlink = report.skipped_symlink.saturating_add(1);
                    continue;
                }
                subdirs.push(child);
                continue;
            }
            consider_file(&child, &root, limits, index, report, hashed_bytes)?;
        }
        // Push later names first so we pop in sorted order (deterministic).
        subdirs.sort();
        for sub in subdirs.into_iter().rev() {
            stack.push((sub, depth.saturating_add(1)));
        }
    }
    Ok(())
}

fn consider_file(
    path: &Path,
    root: &Path,
    limits: Limits,
    index: &mut LocalShaderIndex,
    report: &mut ScanReport,
    hashed_bytes: &mut u64,
) -> Result<(), Error> {
    if !is_under_root(path, root) {
        report.skipped_symlink = report.skipped_symlink.saturating_add(1);
        return Ok(());
    }
    if report.files_seen >= limits.max_files {
        return Err(Error::CapExceeded {
            cap: ScanCap::TotalFiles,
            max: limits.max_files,
            got: report.files_seen.saturating_add(1),
        });
    }
    report.files_seen = report.files_seen.saturating_add(1);
    let mut file = match open_regular_nonblocking(path) {
        Ok(f) => f,
        Err(Error::NotARegularFile) => {
            report.skipped_special_file = report.skipped_special_file.saturating_add(1);
            return Ok(());
        }
        Err(_) => {
            report.skipped_unreadable = report.skipped_unreadable.saturating_add(1);
            return Ok(());
        }
    };
    let meta = match file.metadata() {
        Ok(m) => m,
        Err(_) => {
            report.skipped_unreadable = report.skipped_unreadable.saturating_add(1);
            return Ok(());
        }
    };
    if !meta.is_file() {
        report.skipped_special_file = report.skipped_special_file.saturating_add(1);
        return Ok(());
    }
    let initial_len = meta.len();
    if initial_len == 0 {
        report.skipped_corrupt = report.skipped_corrupt.saturating_add(1);
        report.corrupt_empty = report.corrupt_empty.saturating_add(1);
        return Ok(());
    }
    if initial_len > limits.max_file_bytes {
        report.skipped_too_large = report.skipped_too_large.saturating_add(1);
        return Ok(());
    }
    match hash_loose_file(&mut file, initial_len, limits) {
        Ok(Some((hash, declared))) => {
            charge_bytes(hashed_bytes, declared, limits)?;
            let source = index.add_source(ShaderSourceKind::LooseFile, path.to_path_buf());
            index.insert(
                hash,
                ShaderLocation {
                    source,
                    offset: FileOffset(0),
                    len: ByteLen(declared),
                },
                report,
                limits.max_index_entries,
            )?;
        }
        Ok(None) => {}
        Err(FileOutcome::CorruptEmpty) => {
            report.skipped_corrupt = report.skipped_corrupt.saturating_add(1);
            report.corrupt_empty = report.corrupt_empty.saturating_add(1);
        }
        Err(FileOutcome::CorruptTruncated) => {
            report.skipped_corrupt = report.skipped_corrupt.saturating_add(1);
            report.corrupt_truncated = report.corrupt_truncated.saturating_add(1);
        }
        Err(FileOutcome::CorruptSize) => {
            report.skipped_corrupt = report.skipped_corrupt.saturating_add(1);
            report.corrupt_size_exceeds_file = report.corrupt_size_exceeds_file.saturating_add(1);
        }
        Err(FileOutcome::TooLarge) => {
            report.skipped_too_large = report.skipped_too_large.saturating_add(1);
        }
        Err(FileOutcome::NotDxbc) => {}
        Err(FileOutcome::Unreadable) => {
            report.skipped_unreadable = report.skipped_unreadable.saturating_add(1);
        }
    }
    Ok(())
}

enum FileOutcome {
    CorruptEmpty,
    CorruptTruncated,
    CorruptSize,
    TooLarge,
    NotDxbc,
    Unreadable,
}

fn hash_loose_file(
    file: &mut File,
    initial_len: u64,
    limits: Limits,
) -> Result<Option<(Sha256, u64)>, FileOutcome> {
    let mut header = [0_u8; DXBC_HEADER_LEN];
    let n = file
        .read(&mut header)
        .map_err(|_| FileOutcome::Unreadable)?;
    if n == 0 {
        return Err(FileOutcome::CorruptEmpty);
    }
    if n < 4 || !starts_with_dxbc(&header[..n]) {
        return Err(FileOutcome::NotDxbc);
    }
    if n < DXBC_HEADER_LEN {
        return Err(FileOutcome::CorruptTruncated);
    }
    let parsed = match parse_dxbc_header(&header) {
        Ok(h) => h,
        Err(DxbcHeaderError::TruncatedHeader) => return Err(FileOutcome::CorruptTruncated),
        Err(DxbcHeaderError::SizeBelowHeader) => return Err(FileOutcome::CorruptTruncated),
    };
    let declared = u64::from(parsed.declared_size);
    if declared > initial_len {
        return Err(FileOutcome::CorruptSize);
    }
    if declared > limits.max_file_bytes {
        return Err(FileOutcome::TooLarge);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| FileOutcome::Unreadable)?;
    // Bound a file that grows after the size snapshot: never read past
    // min(declared, initial_len, max_file_bytes).
    let bound = declared.min(initial_len).min(limits.max_file_bytes);
    let mut limited = file.take(bound);
    let hash = hash_shader_reader(&mut limited).map_err(|_| FileOutcome::Unreadable)?;
    Ok(Some((Sha256::from_sodb(hash), declared)))
}

fn charge_bytes(hashed_bytes: &mut u64, add: u64, limits: Limits) -> Result<(), Error> {
    let next = hashed_bytes.saturating_add(add);
    if next > limits.max_total_bytes {
        return Err(Error::CapExceeded {
            cap: ScanCap::TotalBytes,
            max: limits.max_total_bytes,
            got: next,
        });
    }
    *hashed_bytes = next;
    Ok(())
}

fn is_under_root(path: &Path, root: &Path) -> bool {
    match fs::canonicalize(path) {
        Ok(canon) => canon.starts_with(root),
        Err(_) => false,
    }
}

/// Open for read. Unix uses `O_NOFOLLOW | O_NONBLOCK | O_NOCTTY`.
pub(crate) fn open_regular_nonblocking(path: &Path) -> Result<File, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)
            .map_err(|_| Error::Io)?;
        if !file.metadata().map_err(|_| Error::Io)?.is_file() {
            return Err(Error::NotARegularFile);
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let meta = fs::symlink_metadata(path).map_err(|_| Error::Io)?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(Error::NotARegularFile);
        }
        let file = File::open(path).map_err(|_| Error::Io)?;
        if !file.metadata().map_err(|_| Error::Io)?.is_file() {
            return Err(Error::NotARegularFile);
        }
        Ok(file)
    }
}
