// SPDX-License-Identifier: MIT

//! Read-only local shader scanner. Paths stay in memory; reports are counts.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod error;
mod index;
mod limits;
mod matching;
mod report;
mod scan;
mod types;

pub use error::{Error, ScanCap};
pub use index::LocalShaderIndex;
pub use limits::Limits;
pub use matching::match_records;
pub use osdb_records::{CoverageKind, MatchReport, RecordMatch, Sha256};
pub use report::ScanReport;
pub use scan::{scan, scan_dir, scan_sodb, ScanInput};
pub use types::{ByteLen, FileOffset, ShaderLocation, ShaderSourceKind, SourceId};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
