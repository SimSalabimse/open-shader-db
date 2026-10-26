// SPDX-License-Identifier: MIT

//! Threshold over staged uploads.
//!
//! The agreement unit for pipeline content is `pipeline_hashes` (schema
//! version 2), not an application key. Bytecode hashes and descriptor keys
//! are tallied on their own. A version 1 manifest has no `pipeline_hashes`,
//! so it never enters the pipeline denominator.
//!
//! Nothing here claims the uploads came from different people. There are
//! no accounts.

use std::collections::BTreeMap;

use osdb_upload::{Sha256Hex, UploadManifest};

use crate::staging::{IngestPath, StagedUpload};
use crate::threshold::{hash_accepted, AgreementThreshold, CountingPolicy};
use crate::BuildId;

/// Which hash list a tally refers to.
///
/// Sort order is this declaration order, then the hash bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgreementUnit {
    /// `bytecode_hashes` on every manifest.
    BytecodeHash,
    /// `description_hashes` (descriptor content hashes, not app keys).
    DescriptorKey,
    /// `pipeline_hashes` on schema version 2 only.
    PipelineHash,
}

impl AgreementUnit {
    /// Stable name used in docs. Not a file path.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::BytecodeHash => "bytecode_hash",
            Self::DescriptorKey => "descriptor_key",
            Self::PipelineHash => "pipeline_hash",
        }
    }
}

/// Why a staged upload was left out of every tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HoldReason {
    /// Worker upload with `unchallenged` ownership, and the policy holds those.
    WorkerUnchallenged,
    /// Worker upload with `exe_binding_v1`. This crate does not verify it.
    WorkerExeBindingUnverified,
    /// Manifest `build_id` is not the build passed to [`build_consensus`].
    BuildMismatch,
}

impl HoldReason {
    /// Stable reason code. No path and no manifest body.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::WorkerUnchallenged => "worker_unchallenged",
            Self::WorkerExeBindingUnverified => "worker_exe_binding_unverified",
            Self::BuildMismatch => "build_id_mismatch",
        }
    }
}

/// One upload excluded from the tallies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldUpload {
    /// SHA-256 of the canonical manifest bytes.
    pub object_sha256: Sha256Hex,
    /// Why it was not counted.
    pub reason: HoldReason,
}

/// A hash that met the threshold, and how many counted uploads listed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashTally {
    /// Which list the hash came from.
    pub unit: AgreementUnit,
    /// The hash. Each counted upload contributes it at most once.
    pub hash: Sha256Hex,
    /// Counted uploads that carry this unit and list this hash.
    pub uploads: u32,
}

/// Deterministic consensus for one build.
///
/// Rejected hashes are counts per unit. The hashes themselves are not stored.
/// Repeated [`crate::StagedUpload::object_sha256`] values are one upload, so
/// `upload_count` is the number of distinct object hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusResult {
    /// The build the caller asked about.
    pub build: BuildId,
    /// Distinct object hashes in the input, saturating at `u32::MAX`.
    pub upload_count: u32,
    /// Distinct uploads that entered a tally. Held uploads are not included.
    pub counted_uploads: u32,
    /// Hashes that met the threshold. Sorted by unit, then hash.
    pub accepted: Vec<HashTally>,
    /// Distinct hashes that were tallied and did not meet the threshold.
    pub rejected_counts: BTreeMap<AgreementUnit, u64>,
    /// Uploads that were not tallied. Sorted by object hash, then reason.
    pub held: Vec<HeldUpload>,
}

/// Tally `uploads` for `build`.
///
/// `build` is an argument so an empty slice still has a build id, and so the
/// result does not depend on slice order. An upload whose manifest names a
/// different build is held with [`HoldReason::BuildMismatch`].
///
/// The same `object_sha256` is one upload no matter how many times it appears,
/// including once on each [`IngestPath`]. A copy that is counted wins over a
/// held copy of that hash. One object cannot meet `min_uploads` by itself.
///
/// The threshold is not reduced when `counted_uploads` is small.
#[must_use]
pub fn build_consensus(
    build: &BuildId,
    uploads: &[StagedUpload],
    threshold: AgreementThreshold,
    policy: CountingPolicy,
) -> ConsensusResult {
    let unique = unique_uploads(build, uploads, policy);
    let mut held = Vec::new();
    let mut counted: Vec<&StagedUpload> = Vec::new();
    for upload in unique {
        match disposition(upload, build, policy) {
            Disposition::Hold(reason) => held.push(HeldUpload {
                object_sha256: upload.object_sha256,
                reason,
            }),
            Disposition::Count => counted.push(upload),
        }
    }
    held.sort_by(|left, right| {
        left.object_sha256
            .cmp(&right.object_sha256)
            .then(left.reason.cmp(&right.reason))
    });

    let mut bytecode = UnitAccumulator::new(AgreementUnit::BytecodeHash);
    let mut descriptor = UnitAccumulator::new(AgreementUnit::DescriptorKey);
    let mut pipeline = UnitAccumulator::new(AgreementUnit::PipelineHash);
    for upload in &counted {
        tally_manifest(
            &upload.manifest,
            &mut bytecode,
            &mut descriptor,
            &mut pipeline,
        );
    }

    let mut accepted = Vec::new();
    let mut rejected_counts = BTreeMap::new();
    for unit in [bytecode, descriptor, pipeline] {
        let kind = unit.kind;
        let rejected = unit.finish(threshold, &mut accepted);
        if rejected > 0 {
            rejected_counts.insert(kind, rejected);
        }
    }
    accepted.sort_by(|left, right| left.unit.cmp(&right.unit).then(left.hash.cmp(&right.hash)));

    ConsensusResult {
        build: build.clone(),
        upload_count: saturate_u32(counted.len().saturating_add(held.len())),
        counted_uploads: saturate_u32(counted.len()),
        accepted,
        rejected_counts,
        held,
    }
}

enum Disposition {
    Count,
    Hold(HoldReason),
}

fn disposition(upload: &StagedUpload, build: &BuildId, policy: CountingPolicy) -> Disposition {
    if upload.manifest.build_id != *build {
        return Disposition::Hold(HoldReason::BuildMismatch);
    }
    match hold_reason(upload, policy) {
        Some(reason) => Disposition::Hold(reason),
        None => Disposition::Count,
    }
}

/// One row per object hash. A counted copy is kept when any copy is counted.
/// Ties break by hold reason, then ingest path, then canonical bytes, so the
/// slice order does not matter.
fn unique_uploads<'a>(
    build: &BuildId,
    uploads: &'a [StagedUpload],
    policy: CountingPolicy,
) -> Vec<&'a StagedUpload> {
    let mut chosen: BTreeMap<Sha256Hex, &'a StagedUpload> = BTreeMap::new();
    for upload in uploads {
        let replace = match chosen.get(&upload.object_sha256) {
            Some(incumbent) => replaces(upload, incumbent, build, policy),
            None => true,
        };
        if replace {
            chosen.insert(upload.object_sha256, upload);
        }
    }
    chosen.into_values().collect()
}

fn replaces(
    candidate: &StagedUpload,
    incumbent: &StagedUpload,
    build: &BuildId,
    policy: CountingPolicy,
) -> bool {
    match (
        disposition(candidate, build, policy),
        disposition(incumbent, build, policy),
    ) {
        (Disposition::Count, Disposition::Hold(_)) => true,
        (Disposition::Hold(_), Disposition::Count) => false,
        (Disposition::Hold(left), Disposition::Hold(right)) if left != right => left < right,
        _ => {
            if candidate.via != incumbent.via {
                return candidate.via < incumbent.via;
            }
            encoded_less(candidate, incumbent)
        }
    }
}

fn encoded_less(candidate: &StagedUpload, incumbent: &StagedUpload) -> bool {
    match (
        candidate.manifest.to_canonical_bytes(),
        incumbent.manifest.to_canonical_bytes(),
    ) {
        (Ok(left), Ok(right)) => left < right,
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}

fn hold_reason(upload: &StagedUpload, policy: CountingPolicy) -> Option<HoldReason> {
    if upload.via != IngestPath::WorkerStaging {
        return None;
    }
    match &upload.manifest.ownership {
        osdb_upload::OwnershipProof::Unchallenged { .. } if !policy.count_worker_unchallenged => {
            Some(HoldReason::WorkerUnchallenged)
        }
        osdb_upload::OwnershipProof::ExeBindingV1 { .. } if !policy.count_worker_exe_binding => {
            Some(HoldReason::WorkerExeBindingUnverified)
        }
        _ => None,
    }
}

fn tally_manifest(
    manifest: &UploadManifest,
    bytecode: &mut UnitAccumulator,
    descriptor: &mut UnitAccumulator,
    pipeline: &mut UnitAccumulator,
) {
    // Every manifest carries these two lists, including an empty list.
    bytecode.observe(manifest.bytecode_hashes.as_slice());
    descriptor.observe(manifest.description_hashes.as_slice());
    // Version 1 leaves the field absent. Absence is not an empty vote.
    if let Some(hashes) = &manifest.pipeline_hashes {
        pipeline.observe(hashes.as_slice());
    }
}

struct UnitAccumulator {
    kind: AgreementUnit,
    denominator: u32,
    counts: BTreeMap<Sha256Hex, u32>,
}

impl UnitAccumulator {
    fn new(kind: AgreementUnit) -> Self {
        Self {
            kind,
            denominator: 0,
            counts: BTreeMap::new(),
        }
    }

    /// One counted upload that carries this unit. Each hash is once per
    /// upload because [`osdb_upload::HashSet`] is sorted and unique.
    fn observe(&mut self, hashes: &[Sha256Hex]) {
        self.denominator = self.denominator.saturating_add(1);
        for hash in hashes {
            let entry = self.counts.entry(*hash).or_insert(0);
            *entry = entry.saturating_add(1);
        }
    }

    /// Push accepted hashes and return how many distinct hashes were rejected.
    fn finish(self, threshold: AgreementThreshold, accepted: &mut Vec<HashTally>) -> u64 {
        let mut rejected = 0_u64;
        for (hash, uploads) in self.counts {
            if hash_accepted(uploads, self.denominator, threshold) {
                accepted.push(HashTally {
                    unit: self.kind,
                    hash,
                    uploads,
                });
            } else {
                rejected = rejected.saturating_add(1);
            }
        }
        rejected
    }
}

fn saturate_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}
