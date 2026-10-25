// SPDX-License-Identifier: MIT

//! Coverage report: hashes and counts only.

use serde::Serialize;

use crate::hash::DescriptionHash;
use crate::index::{UNSIGNED_INDEX_LINE, UNSIGNED_INDEX_NOTE};

/// JSON / human value while SODB has no build identifier.
pub const BUILD_IDENTITY_NOT_COMPARED: &str = "not_compared";

/// Why [`BUILD_IDENTITY_NOT_COMPARED`] is reported.
pub const BUILD_IDENTITY_REASON: &str = "SODB carries no build identifier yet";

/// Exact match-rate fraction plus a rounded percentage (no floating point).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MatchRate {
    pub numerator: u64,
    pub denominator: u64,
    pub percent_rounded: u8,
}

impl MatchRate {
    /// `numerator / denominator`, rounded to the nearest percent.
    ///
    /// An empty local set is 0/0 and 100% (every local hash is present).
    /// If any local hash is unmatched, the rounded value is at most 99.
    #[must_use]
    pub fn from_counts(matched: u64, total: u64) -> Self {
        let percent_rounded = if total == 0 || matched >= total {
            100
        } else {
            let rounded = (u128::from(matched) * 100 + u128::from(total) / 2) / u128::from(total);
            u8::try_from(rounded.min(99)).unwrap_or(99)
        };
        Self {
            numerator: matched,
            denominator: total,
            percent_rounded,
        }
    }

    /// `N/D (P%)`.
    #[must_use]
    pub fn display_text(&self) -> String {
        format!(
            "{}/{} ({}%)",
            self.numerator, self.denominator, self.percent_rounded
        )
    }
}

/// Comparison result. Serialized form contains hashes and counts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageReport {
    pub local_hash_count: u64,
    pub index_hash_count: u64,
    pub matched_count: u64,
    pub unmatched_count: u64,
    pub index_only_count: u64,
    pub match_rate: MatchRate,
    pub unmatched_hashes: Vec<DescriptionHash>,
    pub unmatched_omitted: u64,
    pub build_identity: &'static str,
    pub build_identity_reason: &'static str,
    pub upload_count: u64,
    pub consensus_count: u64,
    pub index_unsigned: bool,
    pub index_verified: bool,
    pub index_note: &'static str,
}

impl CoverageReport {
    /// Human-readable report. The last line is always [`UNSIGNED_INDEX_LINE`].
    #[must_use]
    pub fn format_human(&self) -> String {
        let mut out = String::new();
        out.push_str("Coverage report\n");
        push_count(&mut out, "Local description hashes", self.local_hash_count);
        push_count(&mut out, "Index description hashes", self.index_hash_count);
        push_count(&mut out, "Matched", self.matched_count);
        push_count(&mut out, "Unmatched", self.unmatched_count);
        push_count(
            &mut out,
            "Index hashes absent locally",
            self.index_only_count,
        );
        out.push_str("Match rate: ");
        out.push_str(&self.match_rate.display_text());
        out.push('\n');
        out.push_str("Unmatched description hashes:\n");
        if self.unmatched_hashes.is_empty() {
            out.push_str("(none)\n");
        } else {
            for hash in &self.unmatched_hashes {
                out.push_str("  ");
                out.push_str(&hash.to_hex());
                out.push('\n');
            }
        }
        out.push_str(&omitted_line(self.unmatched_omitted));
        out.push('\n');
        out.push_str("Build identity: ");
        out.push_str(self.build_identity);
        out.push_str(" (");
        out.push_str(self.build_identity_reason);
        out.push_str(")\n");
        out.push_str("Index status: ");
        out.push_str(UNSIGNED_INDEX_NOTE);
        out.push('\n');
        out.push_str(UNSIGNED_INDEX_LINE);
        out.push('\n');
        out
    }
}

fn omitted_line(omitted: u64) -> String {
    if omitted == 1 {
        String::from("1 unmatched hash omitted")
    } else {
        format!("{omitted} unmatched hashes omitted")
    }
}

fn push_count(out: &mut String, label: &str, value: u64) {
    out.push_str(label);
    out.push_str(": ");
    out.push_str(&value.to_string());
    out.push('\n');
}
