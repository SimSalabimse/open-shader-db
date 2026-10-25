// SPDX-License-Identifier: MIT

//! Typed parser for Microsoft State Object Database (SODB) files.
//!
//! Public output types hold **hashes and keys only**. DXIL / bytecode bytes are
//! never stored on [`SodbSummary`] or any type it contains.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod dxbc;
mod error;
mod extract;
mod hash;
mod limits;
mod parser;
mod rootsig;
mod schema;
mod types;

pub use dxbc::sanitize_root_signature;
pub use error::Error;
pub use extract::{
    validate_semantic_name, DescriptorBlob, ExtractedBlend, ExtractedDepthStencil,
    ExtractedDepthStencilOp, ExtractedInputElement, ExtractedPipelineState, ExtractedRasterizer,
    ExtractedRenderTargetBlend, ExtractedRenderTargetFormats, ExtractedSoDeclaration,
    ExtractedSodb, ExtractedStage, ExtractedStageHashes, ExtractedStreamOut,
    ExtractedViewInstancing,
};
pub use limits::Limits;
pub use parser::{
    extract_pipeline_states, extract_pipeline_states_with_limits, is_sodb, open, open_with_limits,
    probe,
};
pub use rootsig::{
    empty_root_signature_v1_0, empty_root_signature_v1_1, one_cbv_root_signature_v1_0,
    one_table_root_signature_v1_1, parse_root_signature, CanonicalRootSignature,
};
pub use schema::{SODB_APPLICATION_ID, SODB_USER_VERSION};
pub use types::{
    AppIdentity, AppKey, AppText, HashedBytecode, HashedObject, ObjectKind, ObjectVersion,
    Sha256Hash, SodbHeader, SodbSummary, TableId,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
