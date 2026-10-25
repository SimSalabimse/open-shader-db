// SPDX-License-Identifier: MIT

//! Compare local SODB description hashes to a coverage index.

use crate::hash::DescriptionHash;
use crate::index::{CoverageIndex, UNSIGNED_INDEX_NOTE};
use crate::report::{CoverageReport, MatchRate};

/// Local build fields that can be derived from SODB `app_id` without an exe hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBuildHint {
    pub game_id: String,
    pub version: String,
}

/// Unique, sorted description hashes from a parsed SODB.
#[must_use]
pub fn local_description_hashes(summary: &sodb::SodbSummary) -> Vec<DescriptionHash> {
    let mut hashes: Vec<DescriptionHash> = summary
        .pipeline_states
        .iter()
        .map(|o| DescriptionHash::from_sodb(o.desc_hash))
        .chain(
            summary
                .state_objects
                .iter()
                .map(|o| DescriptionHash::from_sodb(o.desc_hash)),
        )
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

/// First `app_id` row, if it has a non-empty app name.
///
/// SODB does not store an exe content hash, so this is only a hint. Comparison
/// skips the build-id check when this returns `None`.
#[must_use]
pub fn local_build_hint(summary: &sodb::SodbSummary) -> Option<LocalBuildHint> {
    let row = summary.app_id.first()?;
    let game_id = row.app_name.as_str().trim();
    if game_id.is_empty() {
        return None;
    }
    Some(LocalBuildHint {
        game_id: game_id.to_string(),
        version: row.app_version.to_string(),
    })
}

/// Compare local hashes to the index for its single build.
///
/// `max_unmatched` caps [`CoverageReport::unmatched_hashes`]. The omitted count
/// is always recorded.
#[must_use]
pub fn compare(
    local: &sodb::SodbSummary,
    index: &CoverageIndex,
    max_unmatched: usize,
) -> CoverageReport {
    let local_hashes = local_description_hashes(local);
    let hint = local_build_hint(local);
    compare_hashes(&local_hashes, hint.as_ref(), index, max_unmatched)
}

/// Compare already-extracted local hashes to an index.
#[must_use]
pub fn compare_hashes(
    local_hashes: &[DescriptionHash],
    local_build: Option<&LocalBuildHint>,
    index: &CoverageIndex,
    max_unmatched: usize,
) -> CoverageReport {
    let unmatched: Vec<DescriptionHash> = local_hashes
        .iter()
        .copied()
        .filter(|hash| !index.hashes.contains(hash))
        .collect();
    let matched_count =
        u64::try_from(local_hashes.len().saturating_sub(unmatched.len())).unwrap_or(u64::MAX);
    let local_hash_count = u64::try_from(local_hashes.len()).unwrap_or(u64::MAX);
    let index_hash_count = u64::try_from(index.hashes.len()).unwrap_or(u64::MAX);
    let unmatched_count = u64::try_from(unmatched.len()).unwrap_or(u64::MAX);
    let index_only_count = index_hash_count.saturating_sub(matched_count);

    let omitted = unmatched.len().saturating_sub(max_unmatched);
    let shown = unmatched.into_iter().take(max_unmatched).collect();

    let build_id_mismatch = match local_build {
        Some(hint) => {
            hint.game_id != index.build_id.game_id.as_str()
                || hint.version != index.build_id.version.as_str()
        }
        None => false,
    };

    CoverageReport {
        local_hash_count,
        index_hash_count,
        matched_count,
        unmatched_count,
        index_only_count,
        match_rate: MatchRate::from_counts(matched_count, local_hash_count),
        unmatched_hashes: shown,
        unmatched_omitted: u64::try_from(omitted).unwrap_or(u64::MAX),
        build_id_mismatch,
        upload_count: index.upload_count,
        consensus_count: index.consensus_count,
        index_unsigned: true,
        index_verified: false,
        index_note: UNSIGNED_INDEX_NOTE,
    }
}
