// SPDX-License-Identifier: MIT

//! Read-only staging loader and upload-threshold consensus.
//!
//! This crate does not send bytes, write a coverage index, or publish
//! records. It reads a staging tree the caller already has and returns
//! hashes and counts. Paths and file bodies stay out of errors and reports.
//! See `docs/CONSENSUS.md`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod consensus;
mod error;
mod staging;
mod threshold;

pub use consensus::{
    build_consensus, AgreementUnit, ConsensusResult, HashTally, HeldUpload, HoldReason,
};
pub use error::Error;
pub use osdb_upload::{BuildId, Sha256Hex, UploadManifest};
pub use staging::{
    list_builds, load_build, IngestPath, LoadLimits, LoadReport, SkipReason, StagedUpload,
    StagingRoot, DEFAULT_MAX_ENTRIES_SCANNED, DEFAULT_MAX_FILES_PER_BUILD, DEFAULT_MAX_FILE_BYTES,
    WALK_DEPTH,
};
pub use threshold::{AgreementThreshold, CountingPolicy};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
