// SPDX-License-Identifier: MIT

//! Public newtypes and summary records. None of these hold bytecode bytes.

use serde::Serialize;
use std::fmt;

use crate::hash::hex_encode;

/// SHA-256 digest. Serialized as lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Hash([u8; 32]);

impl Sha256Hash {
    /// Wrap a finished digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Raw 32-byte digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex encoding.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Sha256Hash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// Application-defined SODB key (not a content hash). Serialized as lowercase hex.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppKey(Vec<u8>);

impl AppKey {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Lowercase hex encoding.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Debug for AppKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for AppKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for AppKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// Bounded TEXT value from `app_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AppText(String);

impl AppText {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    /// Text contents.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Object version from `groups.Version`, or `0` when unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ObjectVersion(u32);

impl ObjectVersion {
    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Integer version.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Kind of hashed SODB object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    PipelineState,
    StateObject,
}

/// Known SODB tables. Display / serde names are the SQL names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TableId {
    AppId,
    PipelineStates,
    StateObjects,
    ShaderBytecode,
    RootSignatures,
    Groups,
    SoToDxilLibAssociations,
    SoToGlobalRsAssociations,
    SoToLocalRsAssociations,
    SoToExistingSoAssociations,
    SoToHitGroupAssociations,
    SoToRtShaderConfigAssociations,
    SoToRtPipelineConfigAssociations,
    SoToDxilSubobjectToExportsAssociations,
    SoToSubobjectToExportsAssociations,
    SoToGenericProgramAssociations,
    SoToWorkGraphAssociations,
    DepthStencilDescs,
    DepthStencilOpDescs,
    BlendDescs,
    RenderTargetBlendDescs,
    RasterizerDescs,
    RenderTargetFormats,
    ViewInstancingDescs,
    StreamOutDescs,
    SoDeclarations,
    InputElementDescs,
    InputLayoutToInputElementAssociations,
    StreamOutputDescToDeclAssociations,
    Exports,
    RtHitGroups,
    RtShaderConfig,
    RtPipelineConfig,
    DxilSubobjectToExportsAssociations,
    SubobjectToExportsAssociations,
    GenericPrograms,
    WorkGraphs,
    NodeIds,
    ShaderNodes,
    NodeOutputOverrides,
    WorkGraphToEntrypointAssociations,
    WorkGraphToNodeAssociations,
    WorkgraphNodeToOutputOverridesAssociations,
    /// Internal catalog (`sqlite_master`), used only for open-time errors.
    SqliteMaster,
}

impl TableId {
    /// SQL name from the Microsoft schema.
    #[must_use]
    pub const fn sql_name(self) -> &'static str {
        match self {
            Self::AppId => "app_id",
            Self::PipelineStates => "pipeline_states",
            Self::StateObjects => "state_objects",
            Self::ShaderBytecode => "shader_bytecode",
            Self::RootSignatures => "root_signatures",
            Self::Groups => "groups",
            Self::SoToDxilLibAssociations => "so_to_dxil_lib_associations",
            Self::SoToGlobalRsAssociations => "so_to_global_rs_associations",
            Self::SoToLocalRsAssociations => "so_to_local_rs_associations",
            Self::SoToExistingSoAssociations => "so_to_existing_so_associations",
            Self::SoToHitGroupAssociations => "so_to_hit_group_associations",
            Self::SoToRtShaderConfigAssociations => "so_to_rt_shader_config_associations",
            Self::SoToRtPipelineConfigAssociations => "so_to_rt_pipeline_config_associations",
            Self::SoToDxilSubobjectToExportsAssociations => {
                "so_to_dxil_subobject_to_exports_associations"
            }
            Self::SoToSubobjectToExportsAssociations => "so_to_subobject_to_exports_associations",
            Self::SoToGenericProgramAssociations => "so_to_generic_program_associations",
            Self::SoToWorkGraphAssociations => "so_to_work_graph_associations",
            Self::DepthStencilDescs => "depth_stencil_descs",
            Self::DepthStencilOpDescs => "depth_stencil_op_descs",
            Self::BlendDescs => "blend_descs",
            Self::RenderTargetBlendDescs => "render_target_blend_descs",
            Self::RasterizerDescs => "rasterizer_descs",
            Self::RenderTargetFormats => "render_target_formats",
            Self::ViewInstancingDescs => "view_instancing_descs",
            Self::StreamOutDescs => "stream_out_descs",
            Self::SoDeclarations => "so_declarations",
            Self::InputElementDescs => "input_element_descs",
            Self::InputLayoutToInputElementAssociations => {
                "input_layout_to_input_element_associations"
            }
            Self::StreamOutputDescToDeclAssociations => {
                "stream_output_desc_to_stream_output_decl_associations"
            }
            Self::Exports => "exports",
            Self::RtHitGroups => "rt_hit_groups",
            Self::RtShaderConfig => "rt_shader_config",
            Self::RtPipelineConfig => "rt_pipeline_config",
            Self::DxilSubobjectToExportsAssociations => "dxil_subobject_to_exports_associations",
            Self::SubobjectToExportsAssociations => "subobject_to_exports_associations",
            Self::GenericPrograms => "generic_programs",
            Self::WorkGraphs => "work_graphs",
            Self::NodeIds => "node_ids",
            Self::ShaderNodes => "shader_nodes",
            Self::NodeOutputOverrides => "node_output_overrides",
            Self::WorkGraphToEntrypointAssociations => {
                "work_graph_to_entrypoint_node_id_associations"
            }
            Self::WorkGraphToNodeAssociations => "work_graph_to_work_graph_node_associations",
            Self::WorkgraphNodeToOutputOverridesAssociations => {
                "workgraph_node_to_node_output_overrides_associations"
            }
            Self::SqliteMaster => "sqlite_master",
        }
    }

    pub(crate) fn from_sql_name(name: &str) -> Option<Self> {
        Self::all_named()
            .iter()
            .copied()
            .find(|id| id.sql_name().eq_ignore_ascii_case(name))
    }

    const fn all_named() -> &'static [TableId] {
        &[
            TableId::AppId,
            TableId::PipelineStates,
            TableId::StateObjects,
            TableId::ShaderBytecode,
            TableId::RootSignatures,
            TableId::Groups,
            TableId::SoToDxilLibAssociations,
            TableId::SoToGlobalRsAssociations,
            TableId::SoToLocalRsAssociations,
            TableId::SoToExistingSoAssociations,
            TableId::SoToHitGroupAssociations,
            TableId::SoToRtShaderConfigAssociations,
            TableId::SoToRtPipelineConfigAssociations,
            TableId::SoToDxilSubobjectToExportsAssociations,
            TableId::SoToSubobjectToExportsAssociations,
            TableId::SoToGenericProgramAssociations,
            TableId::SoToWorkGraphAssociations,
            TableId::DepthStencilDescs,
            TableId::DepthStencilOpDescs,
            TableId::BlendDescs,
            TableId::RenderTargetBlendDescs,
            TableId::RasterizerDescs,
            TableId::RenderTargetFormats,
            TableId::ViewInstancingDescs,
            TableId::StreamOutDescs,
            TableId::SoDeclarations,
            TableId::InputElementDescs,
            TableId::InputLayoutToInputElementAssociations,
            TableId::StreamOutputDescToDeclAssociations,
            TableId::Exports,
            TableId::RtHitGroups,
            TableId::RtShaderConfig,
            TableId::RtPipelineConfig,
            TableId::DxilSubobjectToExportsAssociations,
            TableId::SubobjectToExportsAssociations,
            TableId::GenericPrograms,
            TableId::WorkGraphs,
            TableId::NodeIds,
            TableId::ShaderNodes,
            TableId::NodeOutputOverrides,
            TableId::WorkGraphToEntrypointAssociations,
            TableId::WorkGraphToNodeAssociations,
            TableId::WorkgraphNodeToOutputOverridesAssociations,
        ]
    }
}

impl fmt::Display for TableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.sql_name())
    }
}

/// One row from `app_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppIdentity {
    pub id: i64,
    pub exe: AppText,
    pub app_name: AppText,
    pub engine_name: Option<AppText>,
    pub app_version: u64,
    pub engine_version: u64,
}

/// Pipeline state or state object: description hash and referenced bytecode hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HashedObject {
    pub kind: ObjectKind,
    pub key: AppKey,
    pub version: ObjectVersion,
    pub desc_hash: Sha256Hash,
    pub bytecode_hashes: Vec<Sha256Hash>,
}

/// One `shader_bytecode` key and the SHA-256 of its blob. The blob is not retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HashedBytecode {
    pub key: AppKey,
    pub bytecode_hash: Sha256Hash,
}

/// SQLite header fields used to recognize an SODB without parsing tables.
///
/// `user_version` and `application_id` are stored in the first 100 bytes of a
/// SQLite database (big-endian). Reading them does not load bytecode or rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SodbHeader {
    pub user_version: i32,
    pub application_id: i32,
}

/// Parsed SODB summary. Contains hashes and keys only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SodbSummary {
    pub user_version: i32,
    pub application_id: i32,
    /// True when the file is WAL-mode and a `-wal` sibling with frames was ignored.
    pub wal_ignored: bool,
    pub app_id: Vec<AppIdentity>,
    pub missing_tables: Vec<TableId>,
    pub pipeline_states: Vec<HashedObject>,
    pub state_objects: Vec<HashedObject>,
    pub shader_bytecode: Vec<HashedBytecode>,
}
