// SPDX-License-Identifier: MIT

//! Scan report: counts only, never paths or bytecode.

use serde::Serialize;

/// Outcome of a local shader scan. Counts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ScanReport {
    pub files_seen: u64,
    pub shaders_indexed: u64,
    pub unique_hashes: u64,
    pub duplicates: u64,
    pub skipped_symlink: u64,
    pub skipped_special_file: u64,
    pub skipped_too_large: u64,
    pub skipped_corrupt: u64,
    pub skipped_unreadable: u64,
    pub sources: u64,
    pub corrupt_empty: u64,
    pub corrupt_truncated: u64,
    pub corrupt_size_exceeds_file: u64,
}

impl ScanReport {
    /// Human-readable counts. Never includes paths.
    #[must_use]
    pub fn format_human(&self) -> String {
        format!(
            "files_seen={} shaders_indexed={} unique_hashes={} duplicates={}\n\
             skipped: symlink={} special_file={} too_large={} corrupt={} unreadable={}\n\
             corrupt: empty={} truncated={} size_exceeds_file={}\n",
            self.files_seen,
            self.shaders_indexed,
            self.unique_hashes,
            self.duplicates,
            self.skipped_symlink,
            self.skipped_special_file,
            self.skipped_too_large,
            self.skipped_corrupt,
            self.skipped_unreadable,
            self.corrupt_empty,
            self.corrupt_truncated,
            self.corrupt_size_exceeds_file
        )
    }
}
