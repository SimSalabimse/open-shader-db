// SPDX-License-Identifier: MIT

//! Typed load and compare failures. Display text never includes filesystem paths.

/// Failure while loading a coverage index or comparing it to a local SODB.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The path exists but is not a regular file (directory, FIFO, device, symlink).
    #[error("path is not a regular file")]
    NotARegularFile,

    /// The file is larger than [`crate::Limits::max_file_bytes`].
    #[error("index file exceeds size limit of {max} bytes (got {got})")]
    FileTooLarge { max: u64, got: u64 },

    /// The index listed more hashes than [`crate::Limits::max_hashes`].
    #[error("index hash count exceeds limit of {max} (got {got})")]
    TooManyHashes { max: u64, got: u64 },

    /// `format_version` was not the supported value.
    #[error("unsupported coverage index format_version {found} (expected {expected})")]
    UnsupportedFormatVersion { found: u64, expected: u32 },

    /// A hash string contained a non-lowercase-hex character.
    #[error("malformed hex in coverage index")]
    MalformedHex,

    /// A hash string was not exactly 64 characters.
    #[error("hex value has length {got}, expected 64")]
    WrongHexLength { got: usize },

    /// The same object key appeared twice in one JSON object.
    #[error("duplicate JSON key")]
    DuplicateKey,

    /// A JSON object contained a field that is not in the documented schema.
    #[error("unknown field in coverage index")]
    UnknownField,

    /// Bytes remained after a complete JSON value.
    #[error("trailing data after coverage index JSON")]
    TrailingGarbage,

    /// The file is not a complete coverage-index JSON document.
    #[error("truncated or invalid coverage index JSON")]
    InvalidJson,

    /// `{` / `[` nesting exceeded the documented cap.
    #[error("JSON nesting exceeds limit of {max}")]
    NestingTooDeep { max: u32 },

    /// A build-id field was empty or contained a path separator.
    #[error("invalid build identifier")]
    InvalidBuildId,

    /// `generated_at` was not a UTC ISO-8601 timestamp.
    #[error("invalid generated_at timestamp")]
    InvalidTimestamp,

    /// Filesystem error. The message never includes the path.
    #[error("I/O error")]
    Io,

    /// Opening or hashing the local SODB failed.
    #[error(transparent)]
    Sodb(#[from] sodb::Error),
}
