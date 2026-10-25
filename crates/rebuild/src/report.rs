// SPDX-License-Identifier: MIT

//! Rebuild report: counts only, never paths or bytecode.

use serde::Serialize;

/// Outcome of a local SODB rebuild. Counts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct RebuildReport {
    pub rebuilt: u64,
    pub skipped_missing_stage: u64,
    pub skipped_unresolved_stages: u64,
    pub skipped_root_signature: u64,
    pub unique_shaders: u64,
    pub unique_root_signatures: u64,
}

impl RebuildReport {
    /// Human-readable counts. Never includes paths.
    #[must_use]
    pub fn format_human(&self) -> String {
        format!(
            "rebuilt={} skipped_missing_stage={} skipped_unresolved_stages={} \
             skipped_root_signature={} unique_shaders={} unique_root_signatures={}\n",
            self.rebuilt,
            self.skipped_missing_stage,
            self.skipped_unresolved_stages,
            self.skipped_root_signature,
            self.unique_shaders,
            self.unique_root_signatures
        )
    }
}
