// SPDX-License-Identifier: MIT

//! Pipeline-state extraction types. Bytecode bytes are never stored here.

use std::sync::Arc;

use crate::hash::hex_encode;
use crate::{AppKey, ObjectVersion, Sha256Hash};

/// Graphics or compute stage referenced from `pipeline_states`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtractedStage {
    Vs,
    Ps,
    Hs,
    Ds,
    Gs,
    As,
    Ms,
    Cs,
}

impl ExtractedStage {
    /// Spec-order stages (`vs` … `cs`).
    pub const ALL: [Self; 8] = [
        Self::Vs,
        Self::Ps,
        Self::Hs,
        Self::Ds,
        Self::Gs,
        Self::As,
        Self::Ms,
        Self::Cs,
    ];

    /// On-disk JSON name used by record files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vs => "vs",
            Self::Ps => "ps",
            Self::Hs => "hs",
            Self::Ds => "ds",
            Self::Gs => "gs",
            Self::As => "as",
            Self::Ms => "ms",
            Self::Cs => "cs",
        }
    }
}

/// SHA-256 of each graphics/compute stage that referenced `shader_bytecode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedStageHashes {
    pub vs: Option<Sha256Hash>,
    pub ps: Option<Sha256Hash>,
    pub hs: Option<Sha256Hash>,
    pub ds: Option<Sha256Hash>,
    pub gs: Option<Sha256Hash>,
    pub amplification: Option<Sha256Hash>,
    pub ms: Option<Sha256Hash>,
    pub cs: Option<Sha256Hash>,
}

/// Hex-ready descriptor BLOB (sanitized root signature, never DXIL).
#[derive(Clone, PartialEq, Eq)]
pub struct DescriptorBlob(Vec<u8>);

impl DescriptorBlob {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Raw descriptor bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Lowercase hex encoding of the descriptor bytes.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
}

impl std::fmt::Debug for DescriptorBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescriptorBlob")
            .field("len", &self.0.len())
            .finish()
    }
}

/// One `input_element_descs` row (SQL column names from the published schema).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedInputElement {
    pub semantic_name: String,
    pub semantic_index: i64,
    pub format: i64,
    pub input_slot: i64,
    pub aligned_byte_offset: i64,
    pub input_slot_class: i64,
    pub instance_data_step_rate: i64,
}

/// `depth_stencil_op_descs` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDepthStencilOp {
    pub stencil_fail_op: i64,
    pub stencil_depth_fail_op: i64,
    pub stencil_pass_op: i64,
    pub stencil_func: i64,
    pub stencil_read_mask: i64,
    pub stencil_write_mask: i64,
}

/// `depth_stencil_descs` row plus followed face ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDepthStencil {
    pub depth_enable: i64,
    pub depth_write_mask: i64,
    pub depth_func: i64,
    pub stencil_enable: i64,
    pub front_face: Option<ExtractedDepthStencilOp>,
    pub back_face: Option<ExtractedDepthStencilOp>,
    pub depth_bounds_test_enable: i64,
}

/// `render_target_blend_descs` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedRenderTargetBlend {
    pub blend_enable: i64,
    pub logic_op_enable: i64,
    pub src_blend: i64,
    pub dest_blend: i64,
    pub blend_op: i64,
    pub src_blend_alpha: i64,
    pub dest_blend_alpha: i64,
    pub blend_op_alpha: i64,
    pub logic_op: i64,
    pub render_target_write_mask: i64,
}

/// `blend_descs` row plus followed per-RT blend rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedBlend {
    pub alpha_to_coverage_enable: i64,
    pub independent_blend_enable: i64,
    pub render_targets: [Option<ExtractedRenderTargetBlend>; 8],
}

/// `rasterizer_descs` row. `DepthBias*` columns are REAL in the spec.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedRasterizer {
    pub fill_mode: i64,
    pub cull_mode: i64,
    pub front_counter_clockwise: i64,
    pub depth_bias: f64,
    pub depth_bias_clamp: f64,
    pub slope_scaled_depth_bias: f64,
    pub depth_clip_enable: i64,
    pub line_rasterization_mode: i64,
    pub forced_sample_count: i64,
    pub conservative_raster: i64,
}

/// `render_target_formats` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedRenderTargetFormats {
    pub rt_formats: [i64; 8],
    pub num_render_targets: i64,
}

/// `view_instancing_descs` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedViewInstancing {
    pub view_instance_count: i64,
    pub render_flags: i64,
    pub viewport_array_index: [Option<i64>; 4],
    pub render_target_array_index: [Option<i64>; 4],
}

/// `so_declarations` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSoDeclaration {
    pub stream: i64,
    pub semantic_name: String,
    pub semantic_index: i64,
    pub start_component: i64,
    pub component_count: i64,
    pub output_slot: i64,
}

/// `stream_out_descs` row plus followed declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedStreamOut {
    pub buffer_stride: [i64; 4],
    pub num_strides: i64,
    pub rasterized_stream: i64,
    pub declarations: Vec<ExtractedSoDeclaration>,
}

/// One `pipeline_states` row with descriptor values and per-stage hashes.
///
/// Large descriptors are interned (`Arc`) so a shared root signature or
/// input layout is stored once. DXIL / `shader_bytecode.Bytecode` bytes are
/// never stored on this type.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedPipelineState {
    pub key: AppKey,
    pub version: ObjectVersion,
    pub bytecode: ExtractedStageHashes,
    pub unresolved_stages: Vec<ExtractedStage>,
    pub root_signature: Option<Arc<DescriptorBlob>>,
    pub input_layout: Option<Arc<Vec<ExtractedInputElement>>>,
    pub depth_stencil: Option<ExtractedDepthStencil>,
    pub blend: Option<ExtractedBlend>,
    pub rasterizer: Option<ExtractedRasterizer>,
    pub render_target_formats: Option<ExtractedRenderTargetFormats>,
    pub view_instancing: Option<ExtractedViewInstancing>,
    pub stream_out: Option<Arc<ExtractedStreamOut>>,
    pub sample_count: Option<i64>,
    pub sample_quality: Option<i64>,
    pub sample_mask: Option<i64>,
    pub ib_strip_cut_value: Option<i64>,
    pub primitive_topology: Option<i64>,
    pub dsv_format: Option<i64>,
    pub node_mask: Option<i64>,
    pub flags: Option<i64>,
}

/// Interned extract of every `pipeline_states` row.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedSodb {
    pub records: Vec<ExtractedPipelineState>,
}

impl std::ops::Deref for ExtractedSodb {
    type Target = [ExtractedPipelineState];

    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

/// `SemanticName` must be a C-like identifier (no paths).
pub(crate) fn validate_semantic_name(name: &str) -> Result<(), crate::Error> {
    if name.is_empty() || name.len() > 128 {
        return Err(crate::Error::InvalidSemanticName);
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(crate::Error::InvalidSemanticName);
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(crate::Error::InvalidSemanticName);
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(crate::Error::InvalidSemanticName)
    }
}
