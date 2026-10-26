// SPDX-License-Identifier: MIT

//! Compare local SODB description hashes to a coverage index.

use crate::hash::DescriptionHash;
use crate::index::{CoverageIndex, FORMAT_VERSION, UNSIGNED_INDEX_NOTE};
use crate::report::{
    CoverageReport, MatchRate, BUILD_IDENTITY_NOT_COMPARED, BUILD_IDENTITY_REASON,
};
use crate::Error;

/// Unique, sorted description hashes from a parsed SODB.
#[must_use]
pub fn local_description_hashes(summary: &sodb::SodbSummary) -> Vec<DescriptionHash> {
    let hashes: Vec<DescriptionHash> = summary
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
    unique_sorted(hashes)
}

pub(crate) fn unique_sorted(mut hashes: Vec<DescriptionHash>) -> Vec<DescriptionHash> {
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

/// Compare local hashes to the index for its single build.
///
/// `max_unmatched` caps [`CoverageReport::unmatched_hashes`]. The omitted count
/// is always recorded. Build ids are not compared: an SODB has no build
/// identifier yet.
///
/// Format version 2 stores pipeline content hashes, not SODB description
/// hashes. Comparing those lists would report a false miss, so this function
/// refuses version 2.
pub fn compare(
    local: &sodb::SodbSummary,
    index: &CoverageIndex,
    max_unmatched: usize,
) -> Result<CoverageReport, Error> {
    let local_hashes = local_description_hashes(local);
    compare_hashes(&local_hashes, index, max_unmatched)
}

/// Compare already-extracted local hashes to an index.
pub fn compare_hashes(
    local_hashes: &[DescriptionHash],
    index: &CoverageIndex,
    max_unmatched: usize,
) -> Result<CoverageReport, Error> {
    if index.format_version != FORMAT_VERSION {
        return Err(Error::NotADescriptionIndex);
    }
    Ok(compare_hashes_v1(local_hashes, index, max_unmatched))
}

fn compare_hashes_v1(
    local_hashes: &[DescriptionHash],
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

    CoverageReport {
        local_hash_count,
        index_hash_count,
        matched_count,
        unmatched_count,
        index_only_count,
        match_rate: MatchRate::from_counts(matched_count, local_hash_count),
        unmatched_hashes: shown,
        unmatched_omitted: u64::try_from(omitted).unwrap_or(u64::MAX),
        build_identity: BUILD_IDENTITY_NOT_COMPARED,
        build_identity_reason: BUILD_IDENTITY_REASON,
        upload_count: index.upload_count,
        consensus_count: index.consensus_count,
        index_unsigned: true,
        index_verified: false,
        index_note: UNSIGNED_INDEX_NOTE,
    }
}
