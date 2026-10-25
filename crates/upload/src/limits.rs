// SPDX-License-Identifier: MIT

//! Resource caps for upload manifests.

/// Default and overrideable limits applied while building or parsing a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Reject the payload if its UTF-8 JSON exceeds this many bytes.
    pub max_json_bytes: u64,
    /// Reject each hash list if it has more unique entries than this.
    pub max_hashes_per_list: u64,
    /// Reject the document if `{` / `[` nesting exceeds this depth.
    pub max_json_nesting: u32,
}

impl Limits {
    /// 4 MiB JSON, 50 000 hashes per list, nesting depth 8.
    ///
    /// The hash cap matches the record-file record cap. Larger limits are
    /// library-only (`parse_with_limits` / `ManifestBuilder::limits`).
    pub const DEFAULT: Self = Self {
        max_json_bytes: 4 * 1024 * 1024,
        max_hashes_per_list: 50_000,
        max_json_nesting: 8,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
