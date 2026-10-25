// SPDX-License-Identifier: MIT

//! Resource caps for untrusted directory trees and SODB shader sources.

/// Default and overrideable limits applied while scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Reject or skip a single file larger than this many bytes.
    pub max_file_bytes: u64,
    /// Stop when more than this many files have been considered.
    pub max_files: u64,
    /// Directory nesting from the scan root (root is depth 0).
    pub max_depth: u32,
    /// Stop when hashed container bytes exceed this total.
    pub max_total_bytes: u64,
    /// Stop when the index would store more locations than this.
    pub max_index_entries: u64,
}

impl Limits {
    /// 256 MiB per file, 1_000_000 files, depth 16, 64 GiB total, 1_000_000 entries.
    pub const DEFAULT: Self = Self {
        max_file_bytes: 256 * 1024 * 1024,
        max_files: 1_000_000,
        max_depth: 16,
        max_total_bytes: 64 * 1024 * 1024 * 1024,
        max_index_entries: 1_000_000,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
