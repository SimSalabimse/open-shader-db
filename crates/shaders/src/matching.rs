// SPDX-License-Identifier: MIT

//! Record matching against a local shader index (hashes only).

use std::collections::HashSet;

use osdb_records::{match_local_hashes, MatchReport, RecordFile, Sha256};

use crate::LocalShaderIndex;

/// Classify each record as fully covered, partially covered, or uncovered.
///
/// Uses the same rules as [`osdb_records::match_local_hashes`], including
/// unresolved stages. Matching ignores which stage a hash came from.
#[must_use]
pub fn match_records(records: &RecordFile, shaders: &LocalShaderIndex) -> MatchReport {
    let local: HashSet<Sha256> = shaders.hashes().collect();
    match_local_hashes(records, &local)
}
