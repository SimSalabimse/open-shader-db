// SPDX-License-Identifier: MIT

//! Rebuild a local SODB from records plus local shaders.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod error;
mod options;
mod report;
mod write;

pub use error::Error;
pub use options::RebuildOptions;
pub use report::RebuildReport;
pub use write::{rebuild_sodb, LOCAL_USE_NOTICE};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
