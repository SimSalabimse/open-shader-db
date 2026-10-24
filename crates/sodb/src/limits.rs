// SPDX-License-Identifier: MIT

//! Documented resource caps for untrusted SODB files.

/// Default and overrideable limits applied while parsing an SODB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Reject the file if its size exceeds this many bytes.
    pub max_file_bytes: u64,
    /// Maximum size of any BLOB value copied into Rust (bytecode, descriptors).
    pub max_blob_bytes: usize,
    /// Maximum size of an application-defined key BLOB.
    pub max_key_bytes: usize,
    /// Maximum size of a TEXT value (exe name, app name, engine name).
    pub max_text_bytes: usize,
    /// Maximum rows read from any one table.
    pub max_rows_per_table: u64,
    /// Maximum rows read across all tables (total-work budget).
    pub max_total_rows: u64,
}

impl Limits {
    /// 256 MiB file, 8 MiB blobs, 4 KiB keys/text, 50_000 rows per table,
    /// 500_000 rows total.
    pub const DEFAULT: Self = Self {
        max_file_bytes: 256 * 1024 * 1024,
        max_blob_bytes: 8 * 1024 * 1024,
        max_key_bytes: 4 * 1024,
        max_text_bytes: 4 * 1024,
        max_rows_per_table: 50_000,
        max_total_rows: 500_000,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
