// SPDX-License-Identifier: MIT

//! Typed failures. None of these variants panic.

/// Failure while reading a Steam text file or walking an install dir.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem error (missing path, permission denied).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The path exists but is not a regular file (directory, FIFO, device).
    #[error("path is not a regular file")]
    NotARegularFile,

    /// The file is larger than the VDF size cap (1 MiB).
    #[error("VDF file exceeds size limit of {max} bytes (got {got})")]
    FileTooLarge { max: u64, got: u64 },

    /// Text is not valid UTF-8.
    #[error("VDF text is not valid UTF-8")]
    InvalidUtf8,

    /// Unclosed quoted string.
    #[error("VDF has an unclosed quoted string")]
    UnclosedString,

    /// Unclosed `{` block.
    #[error("VDF has an unclosed block")]
    UnclosedBlock,

    /// Unclosed `/*` comment.
    #[error("VDF has an unclosed block comment")]
    UnclosedComment,

    /// Escape sequence that this parser does not accept.
    #[error("VDF contains an invalid escape sequence")]
    InvalidEscape,

    /// Nested objects exceeded [`crate::vdf::MAX_VDF_DEPTH`].
    #[error("VDF nesting exceeds limit of {max} (got {got})")]
    TooDeep { max: u32, got: u32 },

    /// Token count exceeded [`crate::vdf::MAX_VDF_TOKENS`].
    #[error("VDF token count exceeds limit of {max} (got {got})")]
    TooManyTokens { max: u32, got: u32 },

    /// Generic syntax problem (missing key, unexpected character).
    #[error("malformed VDF")]
    Syntax,
}
