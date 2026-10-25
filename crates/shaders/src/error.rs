// SPDX-License-Identifier: MIT

//! Typed scan and reopen failures. Display text never includes paths or bytes.

/// Which documented resource cap was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCap {
    /// One file or DXBC container exceeded [`crate::Limits::max_file_bytes`].
    PerFileBytes,
    /// More files were visited than [`crate::Limits::max_files`].
    TotalFiles,
    /// Directory nesting exceeded [`crate::Limits::max_depth`].
    Depth,
    /// Hashed bytes exceeded [`crate::Limits::max_total_bytes`].
    TotalBytes,
    /// Stored locations exceeded [`crate::Limits::max_index_entries`].
    IndexEntries,
}

impl ScanCap {
    /// Stable name used in errors and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PerFileBytes => "per_file_bytes",
            Self::TotalFiles => "total_files",
            Self::Depth => "depth",
            Self::TotalBytes => "total_bytes",
            Self::IndexEntries => "index_entries",
        }
    }
}

/// Failure while scanning shaders or reopening a scanned location.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The scan root does not exist.
    #[error("shader scan root is missing")]
    MissingRoot,

    /// The scan root exists but could not be read.
    #[error("shader scan root is unreadable")]
    UnreadableRoot,

    /// A file source was not a regular file.
    #[error("path is not a regular file")]
    NotARegularFile,

    /// A directory source was not a directory.
    #[error("path is not a directory")]
    NotADirectory,

    /// A documented resource cap was exceeded.
    #[error("scan exceeded {cap} cap of {max} (got {got})", cap = cap.as_str())]
    CapExceeded { cap: ScanCap, max: u64, got: u64 },

    /// The file at a scanned location is gone or its length changed.
    #[error("shader source changed after scan")]
    ShaderChanged,

    /// The bytes at a scanned location no longer hash to the indexed digest.
    #[error("shader hash mismatch after scan")]
    ShaderHashMismatch,

    /// A reopened loose file no longer starts with a usable DXBC header.
    #[error("shader is not a DXBC container")]
    ShaderNotDxbc,

    /// Filesystem error. The message never includes the path.
    #[error("I/O error")]
    Io,

    /// Opening or reading a local SODB failed.
    #[error(transparent)]
    Sodb(#[from] sodb::Error),
}

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
