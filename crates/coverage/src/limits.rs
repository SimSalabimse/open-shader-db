// SPDX-License-Identifier: MIT

//! Resource caps for untrusted coverage index files.

/// Default and overrideable limits applied while loading an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Reject the file if its size exceeds this many bytes.
    pub max_file_bytes: u64,
    /// Reject the file if it lists more description hashes than this.
    pub max_hashes: u64,
    /// Reject the file if `{` / `[` nesting exceeds this depth.
    pub max_json_nesting: u32,
}

impl Limits {
    /// 64 MiB file, 1 million hashes, nesting depth 8.
    ///
    /// One million compact hex strings fit under 64 MiB. Larger hash counts
    /// are library-only (`load_with_limits`).
    pub const DEFAULT: Self = Self {
        max_file_bytes: 64 * 1024 * 1024,
        max_hashes: 1_000_000,
        max_json_nesting: 8,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
