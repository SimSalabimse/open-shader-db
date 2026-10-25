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

pub use dxbc::{
    parse_dxbc_header, sanitize_root_signature, starts_with_dxbc, DxbcHeader, DxbcHeaderError,
    RootSignatureSkipReason, SanitizedRootSignature, DXBC_HEADER_LEN, DXBC_MAGIC,
};
pub use error::Error;
pub use extract::{
    validate_semantic_name, DescriptorBlob, ExtractedBlend, ExtractedDepthStencil,
    ExtractedDepthStencilOp, ExtractedInputElement, ExtractedPipelineState, ExtractedRasterizer,
    ExtractedRenderTargetBlend, ExtractedRenderTargetFormats, ExtractedSoDeclaration,
    ExtractedSodb, ExtractedStage, ExtractedStageHashes, ExtractedStreamOut,
    ExtractedViewInstancing,
};
pub use hash::{
    hash_shader_container, hash_shader_reader, StreamingShaderHasher, SHADER_HASH_BUFFER,
};
pub use limits::Limits;
pub use parser::{
    extract_pipeline_states, extract_pipeline_states_with_limits, is_sodb, list_bytecode_blobs,
    list_bytecode_blobs_with_limits, open, open_with_limits, probe, read_bytecode_blob,
    read_bytecode_blob_by_key,
};
pub use rootsig::{
    empty_root_signature_v1_0, empty_root_signature_v1_0_heap_flag, empty_root_signature_v1_1,
    one_cbv_root_signature_v1_0, one_cbv_root_signature_v1_1, one_cbv_v1_0_payload_before_params,
    one_constants_root_signature_v1_0, one_sampler_border4_v1_0, one_table_root_signature_v1_1,
    parse_root_signature, table_three_ranges_v1_0, table_two_ranges_v1_1,
    worst_case_loose_numeric_bytes, CanonicalRootSignature, WORST_CASE_LOOSE_NUMERIC_BYTES,
};
pub use schema::{
    SODB_APPLICATION_ID, SODB_USER_VERSION, SODB_WRITER_PAGE_SIZE, SODB_WRITER_SCHEMA_SQL,
};
pub use types::{
    AppIdentity, AppKey, AppText, BytecodeBlobRef, HashedBytecode, HashedObject, ObjectKind,
    ObjectVersion, Sha256Hash, SodbHeader, SodbSummary, TableId,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
