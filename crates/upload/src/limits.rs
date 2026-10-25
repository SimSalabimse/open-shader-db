// SPDX-License-Identifier: MIT

//! Resource caps for upload manifests.

/// Bytecode stages one record can contribute (`vs` … `cs`).
pub const BYTECODE_STAGES_PER_RECORD: u64 = 8;

/// Description hashes one record can contribute
/// (`root_signature`, `input_layout`, `stream_out`).
pub const DESCRIPTION_HASHES_PER_RECORD: u64 = 3;

/// Default bytecode-hash-list cap: every record under the record-file
/// cap can contribute every bytecode stage.
pub const DEFAULT_MAX_BYTECODE_HASHES: u64 =
    osdb_records::Limits::DEFAULT.max_records * BYTECODE_STAGES_PER_RECORD;

/// Default description-hash-list cap: every record under the record-file
/// cap can contribute every description key. Real SODB description
/// lists cannot exceed this (`50 000 × 3`).
pub const DEFAULT_MAX_DESCRIPTION_HASHES: u64 =
    osdb_records::Limits::DEFAULT.max_records * DESCRIPTION_HASHES_PER_RECORD;

/// Historical name for [`DEFAULT_MAX_BYTECODE_HASHES`].
pub const DEFAULT_MAX_HASHES_PER_LIST: u64 = DEFAULT_MAX_BYTECODE_HASHES;

/// Default local-client JSON byte cap. Two full hash lists at the
/// bytecode cap fit under this (same 64 MiB as record files).
pub const DEFAULT_MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;

/// Wire request-body cap for the ingest Worker. Enforced before parse.
/// A 64 MiB document is not safe in a 128 MB isolate; see
/// `docs/WORKER_DESIGN.md`.
pub const WIRE_MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;

/// Staging objects accepted per `(game_id, build_id)` prefix. Approximate
/// under races; documented in `docs/WORKER_DESIGN.md`.
pub const DEFAULT_PER_BUILD_UPLOAD_CAP: u64 = 64;

/// Maximum characters in a JSON number token (`schema_version` is one digit).
/// Longer tokens are rejected before conversion.
pub const MAX_NUMBER_TOKEN_CHARS: usize = 16;

/// Checked-in numbers shared with the Worker.
pub const LIMITS_JSON: &str = include_str!("../../../schemas/limits.v1.json");

/// Default and overrideable limits applied while building or parsing a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Reject the payload if its UTF-8 JSON exceeds this many bytes.
    pub max_json_bytes: u64,
    /// Reject `description_hashes` if it has more unique entries than this.
    pub max_description_hashes: u64,
    /// Reject `bytecode_hashes` if it has more unique entries than this.
    pub max_bytecode_hashes: u64,
    /// Reject the document if `{` / `[` nesting exceeds this depth.
    pub max_json_nesting: u32,
}

impl Limits {
    /// 64 MiB JSON, 150 000 description hashes, 400 000 bytecode hashes,
    /// nesting depth 8.
    ///
    /// Larger limits are library-only (`parse_with_limits` /
    /// `ManifestBuilder::limits`). The Worker uses [`WIRE_MAX_JSON_BYTES`]
    /// for the HTTP body and the same list and nesting caps.
    pub const DEFAULT: Self = Self {
        max_json_bytes: DEFAULT_MAX_JSON_BYTES,
        max_description_hashes: DEFAULT_MAX_DESCRIPTION_HASHES,
        max_bytecode_hashes: DEFAULT_MAX_BYTECODE_HASHES,
        max_json_nesting: 8,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
