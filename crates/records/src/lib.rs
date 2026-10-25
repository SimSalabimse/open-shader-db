// SPDX-License-Identifier: MIT

//! Local pipeline-state records without DXIL bytecode.
//!
//! Extraction reads a SODB the user already has. Matching is a pure function
//! over hashes. This crate has no network client.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod build;
mod error;
mod extract;
mod file;
mod hash;
mod limits;
mod matching;
mod types;

pub use build::{BuildIdentity, ExeHash, GameId, GameVersion};
pub use error::Error;
pub use extract::{
    encode_record_file, encode_record_file_to_writer, extract, extract_with_options,
    extract_with_sodb_limits,
};
pub use file::{load, load_with_limits, RecordFile, RedactedPath, FORMAT_VERSION};
pub use hash::Sha256;
pub use limits::Limits;
pub use matching::{match_local_hashes, CoverageKind, MatchReport, RecordMatch};
pub use sodb::RootSignatureSkipReason;
pub use types::{
    AppKeyHex, BlendDesc, DepthStencilDesc, DepthStencilOpDesc, HexBytes, InputElementDesc,
    PipelineStateRecord, RasterizerDesc, RenderTargetBlendDesc, RenderTargetFormats, SampleDesc,
    ShaderStage, SoDeclaration, StageHashes, StreamOutDesc, ViewInstancingDesc,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
