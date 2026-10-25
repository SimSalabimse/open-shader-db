// SPDX-License-Identifier: MIT

//! Resource caps for upload manifests.

/// Bytecode stages one record can contribute (`vs` … `cs`).
pub const BYTECODE_STAGES_PER_RECORD: u64 = 8;

/// Description hashes one record can contribute
/// (`root_signature`, `input_layout`, `stream_out`).
pub const DESCRIPTION_HASHES_PER_RECORD: u64 = 3;

/// Default hash-list cap: every record under the record-file cap can
/// contribute every bytecode stage.
pub const DEFAULT_MAX_HASHES_PER_LIST: u64 =
    osdb_records::Limits::DEFAULT.max_records * BYTECODE_STAGES_PER_RECORD;

/// Default JSON byte cap. Two full hash lists at
/// [`DEFAULT_MAX_HASHES_PER_LIST`] fit under this (same 64 MiB as record files).
pub const DEFAULT_MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;

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
    /// 64 MiB JSON, 400 000 hashes per list, nesting depth 8.
    ///
    /// The hash cap is `records.max_records * BYTECODE_STAGES_PER_RECORD` so
    /// any record file that passes the record cap can be encoded. Larger
    /// limits are library-only (`parse_with_limits` / `ManifestBuilder::limits`).
    pub const DEFAULT: Self = Self {
        max_json_bytes: DEFAULT_MAX_JSON_BYTES,
        max_hashes_per_list: DEFAULT_MAX_HASHES_PER_LIST,
        max_json_nesting: 8,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
