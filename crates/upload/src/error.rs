// SPDX-License-Identifier: MIT

//! Typed manifest failures. Display text never includes paths, bytes, or field values.

/// Failure while building or parsing an upload manifest.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Serialized manifest exceeded [`crate::Limits::max_json_bytes`].
    #[error("upload manifest exceeds size limit of {max} bytes (got {got})")]
    ManifestTooLarge { max: u64, got: u64 },

    /// A hash list exceeded [`crate::Limits::max_hashes_per_list`].
    #[error("hash count exceeds limit of {max} (got {got})")]
    TooManyHashes { max: u64, got: u64 },

    /// `schema_version` was not the supported value.
    #[error("unsupported upload manifest schema_version {found} (expected {expected})")]
    UnsupportedSchemaVersion { found: u64, expected: u32 },

    /// A hex string contained a non-lowercase-hex character.
    #[error("malformed hex in upload manifest")]
    MalformedHex,

    /// A SHA-256 hex string was not exactly 64 characters.
    #[error("hex value has length {got}, expected 64")]
    WrongHexLength { got: usize },

    /// The same object key appeared twice in one JSON object.
    #[error("duplicate JSON key")]
    DuplicateKey,

    /// A JSON object contained a field that is not in the documented schema.
    #[error("unknown field in upload manifest")]
    UnknownField,

    /// A required schema field was absent.
    #[error("missing field in upload manifest")]
    MissingField,

    /// Bytes remained after a complete JSON value.
    #[error("trailing data after upload manifest JSON")]
    TrailingGarbage,

    /// The input is not a complete upload-manifest JSON document.
    #[error("truncated or invalid upload manifest JSON")]
    InvalidJson,

    /// `{` / `[` nesting exceeded the documented cap.
    #[error("JSON nesting exceeds limit of {max}")]
    NestingTooDeep { max: u32 },

    /// A build-id field was empty, too long, or not a safe identifier.
    #[error("invalid build identifier")]
    InvalidBuildId,

    /// `game_id`, `version`, or `exe_hash` was missing after merge.
    #[error("build identity is incomplete")]
    IncompleteBuildId,

    /// The document began with a UTF-8 byte-order mark.
    #[error("upload manifest begins with a UTF-8 byte order mark")]
    ByteOrderMark,

    /// An integer field was a JSON float or otherwise not an integer.
    #[error("invalid integer in upload manifest")]
    InvalidInteger,

    /// `collected_hour` was not an hour-boundary UTC timestamp.
    #[error("invalid collected_hour timestamp")]
    InvalidTimestamp,

    /// The clock instant cannot be represented as a UTC hour in range.
    #[error("timestamp is out of range")]
    TimestampOutOfRange,

    /// `client_version` was not `major.minor.patch` of digits.
    #[error("invalid client version")]
    InvalidClientVersion,

    /// `ownership.kind` was not a documented variant.
    #[error("unknown ownership proof kind")]
    UnknownOwnershipKind,

    /// Canonical JSON encoding failed.
    #[error("failed to encode upload manifest JSON")]
    CanonicalJson,

    /// Opening or parsing the record file failed.
    #[error(transparent)]
    Records(#[from] osdb_records::Error),
}
