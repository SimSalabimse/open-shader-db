// SPDX-License-Identifier: MIT

//! On-disk pipeline-state record types. No bytecode bytes.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};

use crate::hash::{encode_hex, parse_hex_bytes, Sha256};
use crate::Error;

/// Graphics or compute stage stored on a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderStage {
    Vs,
    Ps,
    Hs,
    Ds,
    Gs,
    #[serde(rename = "as")]
    As,
    Ms,
    Cs,
}

impl ShaderStage {
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

    /// On-disk JSON name.
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

    fn from_str(name: &str) -> Option<Self> {
        match name {
            "vs" => Some(Self::Vs),
            "ps" => Some(Self::Ps),
            "hs" => Some(Self::Hs),
            "ds" => Some(Self::Ds),
            "gs" => Some(Self::Gs),
            "as" => Some(Self::As),
            "ms" => Some(Self::Ms),
            "cs" => Some(Self::Cs),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for ShaderStage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StageVisitor;
        impl Visitor<'_> for StageVisitor {
            type Value = ShaderStage;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a shader stage name")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                ShaderStage::from_str(value).ok_or_else(|| E::custom("unknown shader stage"))
            }
        }
        deserializer.deserialize_str(StageVisitor)
    }
}

/// Per-stage SHA-256 of DXIL blobs. Absent stages are omitted on write.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct StageHashes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vs: Option<Sha256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ps: Option<Sha256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hs: Option<Sha256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ds: Option<Sha256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gs: Option<Sha256>,
    #[serde(rename = "as", skip_serializing_if = "Option::is_none")]
    pub as_stage: Option<Sha256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<Sha256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cs: Option<Sha256>,
}

impl StageHashes {
    /// Iterate present stages in spec order.
    pub fn iter(&self) -> impl Iterator<Item = (ShaderStage, Sha256)> + '_ {
        [
            (ShaderStage::Vs, self.vs),
            (ShaderStage::Ps, self.ps),
            (ShaderStage::Hs, self.hs),
            (ShaderStage::Ds, self.ds),
            (ShaderStage::Gs, self.gs),
            (ShaderStage::As, self.as_stage),
            (ShaderStage::Ms, self.ms),
            (ShaderStage::Cs, self.cs),
        ]
        .into_iter()
        .filter_map(|(stage, hash)| hash.map(|h| (stage, h)))
    }

    /// Number of present stages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Whether no stage hashes are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Application-defined SODB key, lowercase hex, no `0x` prefix.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AppKeyHex(String);

impl AppKeyHex {
    /// Encode raw key bytes as lowercase hex.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(encode_hex(bytes))
    }

    /// Validate even-length lowercase hex.
    pub fn parse_hex(text: &str) -> Result<Self, Error> {
        let _ = parse_hex_bytes(text)?;
        Ok(Self(text.to_string()))
    }

    /// Hex text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AppKeyHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Lowercase hex of a descriptor BLOB (root signature `value`).
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HexBytes(String);

impl HexBytes {
    /// Encode raw descriptor bytes as lowercase hex.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(encode_hex(bytes))
    }

    /// Validate even-length lowercase hex.
    pub fn parse_hex(text: &str) -> Result<Self, Error> {
        let _ = parse_hex_bytes(text)?;
        Ok(Self(text.to_string()))
    }

    /// Hex text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Decode the hex payload.
    pub fn decoded(&self) -> Result<Vec<u8>, Error> {
        parse_hex_bytes(&self.0)
    }
}

impl fmt::Debug for HexBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HexBytes")
            .field("len", &self.0.len())
            .finish()
    }
}

/// `input_element_descs` columns (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputElementDesc {
    pub semantic_name: String,
    pub semantic_index: i64,
    pub format: i64,
    pub input_slot: i64,
    pub aligned_byte_offset: i64,
    pub input_slot_class: i64,
    pub instance_data_step_rate: i64,
}

/// `depth_stencil_op_descs` columns (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepthStencilOpDesc {
    pub stencil_fail_op: i64,
    pub stencil_depth_fail_op: i64,
    pub stencil_pass_op: i64,
    pub stencil_func: i64,
    pub stencil_read_mask: i64,
    pub stencil_write_mask: i64,
}

/// `depth_stencil_descs` plus followed face ops (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepthStencilDesc {
    pub depth_enable: i64,
    pub depth_write_mask: i64,
    pub depth_func: i64,
    pub stencil_enable: i64,
    pub front_face: Option<DepthStencilOpDesc>,
    pub back_face: Option<DepthStencilOpDesc>,
    pub depth_bounds_test_enable: i64,
}

/// `render_target_blend_descs` columns (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetBlendDesc {
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

/// `blend_descs` plus followed per-RT rows (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendDesc {
    pub alpha_to_coverage_enable: i64,
    pub independent_blend_enable: i64,
    pub render_targets: [Option<RenderTargetBlendDesc>; 8],
}

/// `rasterizer_descs` columns. Bias fields are REAL in the spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RasterizerDesc {
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

/// `render_target_formats` columns (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetFormats {
    pub rt_formats: [i64; 8],
    pub num_render_targets: i64,
}

/// `view_instancing_descs` columns (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewInstancingDesc {
    pub view_instance_count: i64,
    pub render_flags: i64,
    pub viewport_array_index: [Option<i64>; 4],
    pub render_target_array_index: [Option<i64>; 4],
}

/// `so_declarations` columns (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoDeclaration {
    pub stream: i64,
    pub semantic_name: String,
    pub semantic_index: i64,
    pub start_component: i64,
    pub component_count: i64,
    pub output_slot: i64,
}

/// `stream_out_descs` plus followed declarations (verified SQL schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamOutDesc {
    pub buffer_stride: [i64; 4],
    pub num_strides: i64,
    pub rasterized_stream: i64,
    pub declarations: Vec<SoDeclaration>,
}

/// Sample count / quality from `pipeline_states` INTEGER columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleDesc {
    pub count: Option<i64>,
    pub quality: Option<i64>,
}

/// One pipeline-state record: descriptor hashes plus per-stage bytecode hashes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipelineStateRecord {
    pub app_key: AppKeyHex,
    pub version: u32,
    pub bytecode_hashes: StageHashes,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved_stages: Vec<ShaderStage>,
    pub root_signature: Option<Sha256>,
    pub input_layout: Option<Sha256>,
    pub depth_stencil: Option<DepthStencilDesc>,
    pub blend: Option<BlendDesc>,
    pub rasterizer: Option<RasterizerDesc>,
    pub render_target_formats: Option<RenderTargetFormats>,
    pub view_instancing: Option<ViewInstancingDesc>,
    pub stream_out: Option<Sha256>,
    pub sample_desc: Option<SampleDesc>,
    pub sample_mask: Option<i64>,
    pub ib_strip_cut_value: Option<i64>,
    pub primitive_topology: Option<i64>,
    pub dsv_format: Option<i64>,
    pub node_mask: Option<i64>,
    pub flags: Option<i64>,
}
