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
    pub skipped_noncanonical_root_signature: u64,
    pub unique_shaders: u64,
    pub unique_root_signatures: u64,
    /// Post-publish cleanup or dest fsync problems. The output is already in place.
    pub publish_warnings: u64,
}

impl RebuildReport {
    /// Human-readable counts. Never includes paths.
    #[must_use]
    pub fn format_human(&self) -> String {
        format!(
            "rebuilt={} skipped_missing_stage={} skipped_unresolved_stages={} \
             skipped_root_signature={} skipped_noncanonical_root_signature={} \
             unique_shaders={} unique_root_signatures={} publish_warnings={}\n",
            self.rebuilt,
            self.skipped_missing_stage,
            self.skipped_unresolved_stages,
            self.skipped_root_signature,
            self.skipped_noncanonical_root_signature,
            self.unique_shaders,
            self.unique_root_signatures,
            self.publish_warnings
        )
    }
}
