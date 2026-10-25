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

    /// Serialized output exceeded the 64 MiB record-file cap.
    #[error("record file output exceeds limit of {max} bytes")]
    OutputTooLarge { max: u64 },

    /// A record referenced a descriptor hash that is not in the file tables.
    #[error("record references a missing descriptor")]
    UnresolvedDescriptor,

    /// A descriptor table key was not the SHA-256 of its content.
    #[error("descriptor table key does not match content hash")]
    DescriptorHashMismatch,

    /// `app_key` was empty.
    #[error("app_key is empty")]
    EmptyAppKey,

    /// `records` were not sorted by `app_key`.
    #[error("records are not sorted by app_key")]
    AppKeysNotSorted,

    /// A root-signature hex blob exceeded the 1 MiB cap.
    #[error("root signature exceeds limit of {max} bytes (got {got})")]
    RootSignatureTooLarge { max: u64, got: u64 },

    /// A root-signature hex blob failed structural parse or was not canonical.
    #[error("malformed root signature")]
    RootSignatureMalformed,

    /// `SemanticName` was not a C-like identifier.
    #[error("semantic name is not a valid identifier")]
    InvalidSemanticName,

    /// A descriptor table entry was not referenced by any record.
    #[error("descriptor table entry is not referenced by any record")]
    UnreferencedDescriptor,

    /// `unresolved_stages` listed the same stage twice.
    #[error("duplicate unresolved_stages entry")]
    DuplicateUnresolvedStage,

    /// `unresolved_stages` listed a stage that also has a bytecode hash.
    #[error("unresolved_stages overlaps a resolved bytecode hash")]
    OverlappingUnresolvedStage,

    /// `root_signature_skip` was not one of the documented reason codes.
    #[error("unknown root_signature_skip reason")]
    UnknownRootSignatureSkip,

    /// `root_signature_skip` was set together with a root-signature hash.
    #[error("root_signature_skip cannot be set with a root_signature hash")]
    RootSignatureSkipConflict,
}
