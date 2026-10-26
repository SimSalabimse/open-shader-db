// SPDX-License-Identifier: MIT

//! Staging loader, upload-threshold consensus, and unsigned publish writers.
//!
//! The loader reads a staging tree the caller already has. `publish` writes
//! an unsigned local staging directory (coverage format v2, a count report,
//! and an optional record set). It does not sign, upload, or open a network
//! connection. Paths and file bodies stay out of errors and reports.
//! See `docs/CONSENSUS.md`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod consensus;
mod error;
mod publish;
mod staging;
mod threshold;

pub use consensus::{
    build_consensus, AgreementUnit, ConsensusResult, HashTally, HeldUpload, HoldReason,
};
pub use error::Error;
pub use osdb_upload::{BuildId, Sha256Hex, UploadManifest};
pub use publish::{
    consensus_record_bytes, format_counts, has_consensus_pipeline, write_index, write_record_set,
    PublishMeta, PublishOutcome, PublishStagingDir, COVERAGE_FILE_NAME, RECORDS_FILE_NAME,
    REPORT_FILE_NAME, UNSIGNED_STAGING_LINE, WAITING_FOR_DATA_LINE,
};
pub use staging::{
    list_builds, load_build, IngestPath, LoadLimits, LoadReport, SkipReason, StagedUpload,
    StagingRoot, DEFAULT_MAX_ENTRIES_SCANNED, DEFAULT_MAX_FILES_PER_BUILD, DEFAULT_MAX_FILE_BYTES,
    WALK_DEPTH,
};
pub use threshold::{AgreementThreshold, CountingPolicy};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
