// SPDX-License-Identifier: MIT

//! Typed rebuild failures. Display text never includes paths or bytecode.

/// Failure while writing a rebuilt SODB.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `--out` is the same file as a record input, an SODB input, or a scanned file.
    #[error("output path is the same file as an input")]
    SameAsInput,

    /// `--out` is inside a scanned shader directory.
    #[error("output path is inside a scanned shader root")]
    InsideScanRoot,

    /// `--out` exists and is not a regular file.
    #[error("output path is not a regular file")]
    NotRegular,

    /// `--out` exists as a regular file and `--force` was not set.
    #[error("output file already exists (pass --force to overwrite)")]
    AlreadyExists,

    /// Written bytes exceeded [`crate::RebuildOptions::max_output_bytes`].
    #[error("rebuilt SODB exceeds size limit of {max} bytes (got {got})")]
    OutputTooLarge { max: u64, got: u64 },

    /// `--max-output-bytes` is above the documented hard maximum.
    #[error("max-output-bytes {got} exceeds hard maximum of {max}")]
    MaxOutputBytesTooLarge { max: u64, got: u64 },

    /// `--out` parent directory does not exist.
    #[error("output parent directory is missing")]
    OutputParentMissing,

    /// `--out` cannot be represented as UTF-8 for SQLite.
    #[error("output path is not valid UTF-8")]
    OutputPathNotUtf8,

    /// A temp sibling could not be created exclusively.
    #[error("rebuild temp file already exists")]
    TempFileExists,

    /// A record table hash did not match its payload.
    #[error("record descriptor hash mismatch")]
    DescriptorHashMismatch,

    /// A stored root signature failed re-parse or was not canonical.
    #[error("malformed root signature")]
    RootSignatureMalformed,

    /// Hex in a record could not be decoded.
    #[error("malformed hex in record file")]
    MalformedHex,

    /// Filesystem error. The message never includes the path.
    #[error("I/O error")]
    Io,

    /// SQLite error while writing the rebuilt file.
    #[error("SQLite error while writing rebuilt SODB")]
    Sqlite,

    /// Reopening a local shader failed.
    #[error(transparent)]
    Shader(#[from] osdb_shaders::Error),

    /// Record-file helper failed.
    #[error(transparent)]
    Records(#[from] osdb_records::Error),

    /// SODB helper failed.
    #[error(transparent)]
    Sodb(#[from] sodb::Error),
}

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Sqlite
    }
}
