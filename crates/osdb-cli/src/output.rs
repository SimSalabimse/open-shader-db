// SPDX-License-Identifier: MIT

//! Atomic, non-clobbering writes for CLI `--out` paths.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Failure while writing an output file. Display text never includes paths.
#[derive(Debug)]
pub(crate) enum OutputError {
    SameAsInput,
    AlreadyExists,
    NotRegular,
    Io,
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SameAsInput => f.write_str("output path is the same file as the input SODB"),
            Self::AlreadyExists => {
                f.write_str("output file already exists (pass --force to overwrite)")
            }
            Self::NotRegular => f.write_str("output path is not a regular file"),
            Self::Io => f.write_str("I/O error"),
        }
    }
}

impl std::error::Error for OutputError {}

impl From<io::Error> for OutputError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

/// Write `write_fn` to `dest` via a same-directory temp file, then rename.
///
/// Refuses when `dest` is the same file as `input` (device + inode), a
/// symlink, or any non-regular existing path. Existing regular files require
/// `force`. The temp file is removed on any error.
pub(crate) fn write_atomic(
    dest: &Path,
    input: &Path,
    force: bool,
    write_fn: impl FnOnce(&mut File) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if paths_are_same(dest, input)? {
        return Err(Box::new(OutputError::SameAsInput));
    }
    match fs::symlink_metadata(dest) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.file_type().is_file() {
                return Err(Box::new(OutputError::NotRegular));
            }
            if same_file::is_same_file(dest, input).map_err(|_| OutputError::Io)? {
                return Err(Box::new(OutputError::SameAsInput));
            }
            if !force {
                return Err(Box::new(OutputError::AlreadyExists));
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Box::new(OutputError::Io)),
    }

    let tmp = temp_sibling(dest);
    let mut guard = TempGuard {
        path: tmp.clone(),
        persist: false,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|_| OutputError::Io)?;
    write_fn(&mut file)?;
    file.flush().map_err(|_| OutputError::Io)?;
    file.sync_all().map_err(|_| OutputError::Io)?;
    drop(file);
    fs::rename(&tmp, dest).map_err(|_| OutputError::Io)?;
    guard.persist = true;
    Ok(())
}

fn paths_are_same(dest: &Path, input: &Path) -> Result<bool, OutputError> {
    if dest == input {
        return Ok(true);
    }
    match (fs::symlink_metadata(dest), fs::symlink_metadata(input)) {
        (Ok(d), Ok(i)) => {
            if d.file_type().is_symlink() || i.file_type().is_symlink() {
                return Ok(false);
            }
            if d.file_type().is_file() && i.file_type().is_file() {
                return same_file::is_same_file(dest, input).map_err(|_| OutputError::Io);
            }
            Ok(false)
        }
        (Err(err), _) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        (_, Err(err)) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        _ => Err(OutputError::Io),
    }
}

fn temp_sibling(dest: &Path) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("out"));
    dest.with_file_name(format!(".{name}.osdb-{}-{n}.tmp", std::process::id()))
}

struct TempGuard {
    path: PathBuf,
    persist: bool,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if !self.persist {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn temp_file_is_removed_when_write_fails() {
        let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dest =
            std::env::temp_dir().join(format!("osdb-out-cleanup-{}-{n}.json", std::process::id()));
        let input =
            std::env::temp_dir().join(format!("osdb-in-cleanup-{}-{n}.sodb", std::process::id()));
        std::fs::write(&input, b"sodb").unwrap();
        let _ = fs::remove_file(&dest);
        let err = write_atomic(&dest, &input, false, |_| Err("boom".into()));
        assert!(err.is_err());
        assert!(!dest.exists(), "dest must not be created on failure");
        let stem = dest.file_name().unwrap().to_string_lossy();
        let parent = dest.parent().unwrap();
        for entry in fs::read_dir(parent).unwrap() {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with(&format!(".{stem}.osdb-")) || !name.ends_with(".tmp"),
                "temp file left behind: {name}"
            );
        }
        let _ = fs::remove_file(&input);
    }
}
