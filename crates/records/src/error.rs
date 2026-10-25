// SPDX-License-Identifier: MIT

//! Typed load and extract failures. Display text never includes filesystem paths.

/// Failure while extracting records from an SODB or loading a record file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The path exists but is not a regular file (directory, FIFO, device, symlink).
    #[error("path is not a regular file")]
    NotARegularFile,

    /// The file is larger than [`crate::Limits::max_file_bytes`].
    #[error("record file exceeds size limit of {max} bytes (got {got})")]
    FileTooLarge { max: u64, got: u64 },

    /// The file listed more records than [`crate::Limits::max_records`].
    #[error("record count exceeds limit of {max} (got {got})")]
    TooManyRecords { max: u64, got: u64 },

    /// `format_version` was not the supported value.
    #[error("unsupported record file format_version {found} (expected {expected})")]
    UnsupportedFormatVersion { found: u64, expected: u32 },

    /// A hex string contained a non-lowercase-hex character.
    #[error("malformed hex in record file")]
    MalformedHex,

    /// A SHA-256 hex string was not exactly 64 characters.
    #[error("hex value has length {got}, expected 64")]
    WrongHexLength { got: usize },

    /// A descriptor hex string had an odd number of digits.
    #[error("hex value has odd length {got}")]
    OddHexLength { got: usize },

    /// The same object key appeared twice in one JSON object.
    #[error("duplicate JSON key")]
    DuplicateKey,

    /// A JSON object contained a field that is not in the documented schema.
    #[error("unknown field in record file")]
    UnknownField,

    /// Bytes remained after a complete JSON value.
    #[error("trailing data after record file JSON")]
    TrailingGarbage,

    /// The file is not a complete record-file JSON document.
    #[error("truncated or invalid record file JSON")]
    InvalidJson,

    /// `{` / `[` nesting exceeded the documented cap.
    #[error("JSON nesting exceeds limit of {max}")]
    NestingTooDeep { max: u32 },

    /// A build-id field was empty, too long, or not a safe identifier.
    #[error("invalid build identifier")]
    InvalidBuildId,

    /// The file began with a UTF-8 byte-order mark.
    #[error("record file begins with a UTF-8 byte order mark")]
    ByteOrderMark,

    /// An integer field was a JSON float or otherwise not an integer.
    #[error("invalid integer in record file")]
    InvalidInteger,

    /// Filesystem error. The message never includes the path.
    #[error("I/O error")]
    Io,

    /// Opening or extracting the local SODB failed.
    #[error(transparent)]
    Sodb(#[from] sodb::Error),
}
