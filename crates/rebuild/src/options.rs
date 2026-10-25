// SPDX-License-Identifier: MIT

//! Rebuild options. Paths here are used only for same-file checks.

use std::path::PathBuf;

/// 8 GiB default cap on the rebuilt SODB.
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Options for [`crate::rebuild_sodb`].
#[derive(Debug, Clone)]
pub struct RebuildOptions {
    /// Overwrite an existing regular `--out` file.
    pub force: bool,
    /// Stop if the rebuilt file would exceed this many bytes.
    pub max_output_bytes: u64,
    /// Record-file path, used only to refuse writing onto an input.
    pub record_input: Option<PathBuf>,
}

impl Default for RebuildOptions {
    fn default() -> Self {
        Self {
            force: false,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            record_input: None,
        }
    }
}
