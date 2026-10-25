// SPDX-License-Identifier: MIT

//! Rebuild a local SODB from records plus local shaders.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod error;
mod options;
mod report;
mod write;

pub use error::Error;
pub use options::{RebuildOptions, DEFAULT_MAX_OUTPUT_BYTES, HARD_MAX_OUTPUT_BYTES};
pub use report::RebuildReport;
pub use write::{inject_fault, rebuild_sodb, InjectedFault, LOCAL_USE_NOTICE};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
