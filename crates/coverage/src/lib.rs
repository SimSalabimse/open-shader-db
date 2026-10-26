// SPDX-License-Identifier: MIT

//! Load an untrusted coverage index and compare it to a local SODB.
//!
//! Public report types hold **hashes and counts only**. Paths, usernames, and
//! shader bytecode never appear on [`CoverageReport`].

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod build;
mod compare;
mod error;
mod hash;
mod index;
mod limits;
mod report;

pub use build::{BuildId, ExeHash, GameBuild, GameId, GameVersion};
pub use compare::{compare, compare_hashes, local_description_hashes};
pub use error::Error;
pub use hash::DescriptionHash;
pub use index::{
    load, load_with_limits, CoverageIndex, GeneratedAt, IndexHashes, IndexThreshold, RedactedPath,
    FORMAT_VERSION, FORMAT_VERSION_V2, UNSIGNED_INDEX_LINE, UNSIGNED_INDEX_NOTE,
};
pub use limits::Limits;
pub use report::{CoverageReport, MatchRate, BUILD_IDENTITY_NOT_COMPARED, BUILD_IDENTITY_REASON};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
