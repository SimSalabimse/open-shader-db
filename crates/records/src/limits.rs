// SPDX-License-Identifier: MIT

//! Resource caps for untrusted record files.

/// Default and overrideable limits applied while loading a record file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Reject the file if its size exceeds this many bytes.
    pub max_file_bytes: u64,
    /// Reject the file if it lists more pipeline-state records than this.
    pub max_records: u64,
    /// Reject the file if `{` / `[` nesting exceeds this depth.
    pub max_json_nesting: u32,
}

impl Limits {
    /// 64 MiB file, 50 000 records, nesting depth 8.
    ///
    /// The record cap matches the SODB per-table row cap used by `sodb`.
    /// Larger limits are library-only (`load_with_limits`).
    pub const DEFAULT: Self = Self {
        max_file_bytes: 64 * 1024 * 1024,
        max_records: 50_000,
        max_json_nesting: 8,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
