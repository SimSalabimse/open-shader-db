// SPDX-License-Identifier: MIT

//! Typed parse failures. None of these variants panic.

use crate::TableId;
use rusqlite::ErrorCode;

/// Failure while opening or reading an SODB file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The path is not a readable SQLite SODB (empty, wrong magic, or too small).
    #[error("not an SODB file")]
    NotAnSodb,

    /// The path exists but is not a regular file (directory, FIFO, device).
    #[error("path is not a regular file")]
    NotARegularFile,

    /// `PRAGMA user_version` was not `2`.
    #[error("unsupported SODB user_version {found} (expected {expected})")]
    UnsupportedUserVersion { found: i64, expected: i32 },

    /// `PRAGMA application_id` was not `0xD3D50DB`.
    #[error("unsupported SODB application_id 0x{found:X} (expected 0x{expected:X})")]
    UnsupportedApplicationId { found: i64, expected: i32 },

    /// The file is not a readable SQLite database (truncated, corrupt, or random bytes).
    #[error("malformed, truncated, or unreadable SODB")]
    Malformed {
        #[source]
        source: rusqlite::Error,
    },

    /// SQLite returned an error while reading a known table.
    #[error("SQLite error while reading {table}: {source}")]
    Sqlite {
        table: TableId,
        #[source]
        source: rusqlite::Error,
    },

    /// Filesystem error (missing path, permission denied).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The file is larger than [`crate::Limits::max_file_bytes`].
    #[error("SODB file exceeds size limit of {max} bytes (got {got})")]
    FileTooLarge { max: u64, got: u64 },

    /// A BLOB exceeded [`crate::Limits::max_blob_bytes`].
    #[error("blob in {table} exceeds limit of {max} bytes (got {got})")]
    BlobTooLarge { table: TableId, max: u64, got: u64 },

    /// A key BLOB exceeded [`crate::Limits::max_key_bytes`].
    #[error("key in {table} exceeds limit of {max} bytes (got {got})")]
    KeyTooLarge { table: TableId, max: u64, got: u64 },

    /// A TEXT value exceeded [`crate::Limits::max_text_bytes`].
    #[error("text in {table} exceeds limit of {max} bytes (got {got})")]
    TextTooLarge { table: TableId, max: u64, got: u64 },

    /// A table had more rows than [`crate::Limits::max_rows_per_table`].
    #[error("row count in {table} exceeds limit of {max} (got {got})")]
    TooManyRows { table: TableId, max: u64, got: u64 },

    /// Rows read across all tables exceeded [`crate::Limits::max_total_rows`].
    #[error("total rows read exceed limit of {max} (got {got})")]
    TooManyTotalRows { max: u64, got: u64 },

    /// The same application key appeared twice in one table.
    #[error("duplicate key in {table}")]
    DuplicateKey { table: TableId },

    /// A table we need to index by key has no `Key` column.
    #[error("table {table} is missing a Key column")]
    MissingKeyColumn { table: TableId },

    /// A row that must be keyed had a NULL `Key`.
    #[error("null key in {table}")]
    NullKey { table: TableId },

    /// A column had a SQLite type that this parser does not accept.
    #[error("wrong column type in {table}.{column}: got {got}")]
    WrongColumnType {
        table: TableId,
        column: String,
        got: String,
    },

    /// An integer was negative, NULL, or otherwise not representable in the target type.
    #[error("invalid integer in {table}.{column}: {found}")]
    InvalidValue {
        table: TableId,
        column: String,
        found: i64,
    },

    /// Description-hash follows formed a cycle (for example `AddToStateObjectParent`).
    ///
    /// One cycle anywhere rejects the entire file.
    #[error("cycle detected while hashing {table}")]
    CycleDetected { table: TableId },

    /// A Windows path contained unpaired UTF-16 surrogates, so it cannot be
    /// converted to a SQLite URI without opening a different location.
    #[error("path contains unpaired UTF-16 surrogates")]
    InvalidWidePath,

    /// An interned key id was missing from the arena (internal inconsistency).
    #[error("internal key intern inconsistency")]
    InternInconsistency,
}

impl Error {
    pub(crate) fn from_open(source: rusqlite::Error) -> Self {
        if is_malformed(&source) {
            Self::Malformed { source }
        } else {
            Self::Sqlite {
                table: TableId::SqliteMaster,
                source,
            }
        }
    }

    pub(crate) fn from_sqlite(table: TableId, source: rusqlite::Error) -> Self {
        if is_malformed(&source) {
            Self::Malformed { source }
        } else if let rusqlite::Error::InvalidColumnType(_, name, ty) = &source {
            Self::WrongColumnType {
                table,
                column: sanitize_column_name(name),
                got: format!("{ty}").to_ascii_lowercase(),
            }
        } else if let rusqlite::Error::SqliteFailure(e, _) = &source {
            if e.code == ErrorCode::TooBig {
                Self::BlobTooLarge {
                    table,
                    max: 0,
                    got: 0,
                }
            } else {
                Self::Sqlite { table, source }
            }
        } else {
            Self::Sqlite { table, source }
        }
    }
}

fn sanitize_column_name(name: &str) -> String {
    if name.len() > 64 || name.contains('(') || name.contains(' ') {
        String::from("<generated>")
    } else {
        name.to_string()
    }
}

fn is_malformed(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(e, _) => matches!(
            e.code,
            ErrorCode::DatabaseCorrupt
                | ErrorCode::NotADatabase
                | ErrorCode::SystemIoFailure
                | ErrorCode::CannotOpen
        ),
        _ => false,
    }
}
