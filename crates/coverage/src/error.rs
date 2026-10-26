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

    /// `format_version` was not 1 or 2.
    #[error("unsupported coverage index format_version {found}")]
    UnsupportedFormatVersion { found: u64 },

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

    /// A build-id field was empty, too long, or not a safe identifier.
    #[error("invalid build identifier")]
    InvalidBuildId,

    /// `generated_at` was not a valid UTC ISO-8601 timestamp.
    #[error("invalid generated_at timestamp")]
    InvalidTimestamp,

    /// The file began with a UTF-8 byte-order mark.
    #[error("coverage index begins with a UTF-8 byte order mark")]
    ByteOrderMark,

    /// Format version 2 `hashes` were not strictly ascending.
    #[error("hashes are not sorted")]
    UnsortedHashes,

    /// Format version 2 `hashes` listed the same value twice.
    #[error("duplicate hash")]
    DuplicateHash,

    /// A threshold field was outside the documented range.
    #[error("invalid agreement threshold")]
    InvalidThreshold,

    /// A count or threshold field was not a JSON integer in range.
    #[error("invalid integer")]
    InvalidInteger,

    /// Canonical encoding failed.
    #[error("coverage index JSON is not in canonical form")]
    CanonicalJson,

    /// A raw JSON string is longer than any field this format allows.
    ///
    /// The check runs before the string is copied. See issue #29.
    #[error("json string exceeds limit")]
    StringTooLong,

    /// `osdb-cli coverage` compares SODB description hashes, which are format version 1.
    #[error("coverage comparison accepts format_version 1 description hashes only")]
    NotADescriptionIndex,

    /// Filesystem error. The message never includes the path.
    #[error("I/O error")]
    Io,

    /// Opening or hashing the local SODB failed.
    #[error(transparent)]
    Sodb(#[from] sodb::Error),
}
