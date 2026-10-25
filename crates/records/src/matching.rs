// SPDX-License-Identifier: MIT

//! Local bytecode-hash matching against a loaded record file.

use std::collections::HashSet;

use crate::hash::Sha256;
use crate::types::{AppKeyHex, PipelineStateRecord, ShaderStage};
use crate::RecordFile;

/// How a record's stage hashes compare to a local hash set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageKind {
    /// At least one resolved stage hash is present locally, every resolved
    /// stage hash is present, and the record has no unresolved stages.
    FullyCovered,
    /// At least one resolved stage hash is present and at least one resolved
    /// hash is absent or a stage is unresolved.
    PartiallyCovered,
    /// No resolved stage hash is present locally (including empty records).
    MissingShaders,
}

/// One record's match result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordMatch {
    pub app_key: AppKeyHex,
    pub kind: CoverageKind,
    pub present_stages: Vec<ShaderStage>,
    pub missing_stages: Vec<ShaderStage>,
}

/// Partition of a record file against local bytecode hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchReport {
    pub fully_covered: Vec<RecordMatch>,
    pub partially_covered: Vec<RecordMatch>,
    pub missing_shaders: Vec<RecordMatch>,
}

/// Report which records are fully covered, partially covered, or missing shaders.
///
/// `local` is the set of SHA-256 hashes of bytecode the user already has.
/// This function does not read files or talk to the network.
#[must_use]
pub fn match_local_hashes(file: &RecordFile, local: &HashSet<Sha256>) -> MatchReport {
    let mut fully_covered = Vec::new();
    let mut partially_covered = Vec::new();
    let mut missing_shaders = Vec::new();
    for record in &file.records {
        let classified = classify_record(record, local);
        match classified.kind {
            CoverageKind::FullyCovered => fully_covered.push(classified),
            CoverageKind::PartiallyCovered => partially_covered.push(classified),
            CoverageKind::MissingShaders => missing_shaders.push(classified),
        }
    }
    MatchReport {
        fully_covered,
        partially_covered,
        missing_shaders,
    }
}

fn classify_record(record: &PipelineStateRecord, local: &HashSet<Sha256>) -> RecordMatch {
    let mut present_stages = Vec::new();
    let mut missing_stages = Vec::new();
    for (stage, hash) in record.bytecode_hashes.iter() {
        if local.contains(&hash) {
            present_stages.push(stage);
        } else {
            missing_stages.push(stage);
        }
    }
    let unresolved = !record.unresolved_stages.is_empty();
    let kind = if unresolved {
        if present_stages.is_empty() {
            CoverageKind::MissingShaders
        } else {
            CoverageKind::PartiallyCovered
        }
    } else if missing_stages.is_empty() && !present_stages.is_empty() {
        CoverageKind::FullyCovered
    } else if present_stages.is_empty() {
        CoverageKind::MissingShaders
    } else {
        CoverageKind::PartiallyCovered
    };
    RecordMatch {
        app_key: record.app_key.clone(),
        kind,
        present_stages,
        missing_stages,
    }
}
