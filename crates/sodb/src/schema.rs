// SPDX-License-Identifier: MIT

//! SODB SQL identifiers from Microsoft's published schema (`user_version = 2`).
//!
//! Source: <https://microsoft.github.io/DirectX-Specs/d3d/StateObjectDatabase.html#sql-database-schema>
//!
//! # Assumptions
//!
//! Isolated here so a spec mismatch is easy to correct. Items marked
//! **(not in the Microsoft spec)** are project choices:
//!
//! - Table and column names match that document (SQLite case-insensitive).
//! - `groups.Version` is the object version for a PSO or SO key.
//! - **(not in the Microsoft spec)** If `groups` is missing or has no matching
//!   row, the object version is `0`.
//! - `app_id` may contain zero or more rows (the spec does not constrain count).
//! - Keys are application-defined BLOBs, not content hashes.
//! - **(not in the Microsoft spec)** Files whose SQLite header reports WAL
//!   (bytes 18–19 equal to `2`) are opened with the URI
//!   `file:<path>?immutable=1`. Unflushed WAL pages next to the opened path
//!   are ignored, and no `-wal`/`-shm` files are created beside the input.
//!   [`crate::SodbSummary::wal_ignored`] is `true` only when the sibling
//!   `-wal` next to the opened path is larger than the 32-byte WAL header
//!   (so it may contain frames). A header-only `-wal` is not ignored data.
//!   Windows UNC paths use `file:////server/share/...` because bundled
//!   SQLite is not built with `SQLITE_ALLOW_URI_AUTHORITY`.
//! - **(not in the Microsoft spec)** A sibling rollback journal (`<path>-journal`)
//!   is not applied; the parser reads the main database file as stored.
//! - **(not in the Microsoft spec)** Rows from `sqlite_master` are not charged
//!   against [`crate::Limits::max_total_rows`]. A file with exactly
//!   `max_total_rows` data rows is accepted.
//! - **(not in the Microsoft spec)** One cycle in the description-hash follow
//!   graph rejects the entire file (`Error::CycleDetected`).
//! - **(not in the Microsoft spec)** A NULL `app_id.app_version` or
//!   `engine_version` is rejected (`Error::InvalidValue`). A missing column
//!   still defaults to `0`.
//! - `string_associations` is **not** folded into description hashes. Its
//!   `OwningTableKey` may be NULL and is not a clean per-object foreign key.
//!   That is a documented limitation of the hash.
//! - `so_to_dxil_lib_associations.DxilLibKey` contributes the key bytes only.
//!   The `shader_bytecode.Bytecode` blob is never part of a description hash.

use crate::TableId;

/// Required `PRAGMA user_version` for a v2 SODB.
pub const SODB_USER_VERSION: i32 = 2;

/// Required `PRAGMA application_id` (`0xD3D50DB` in the spec).
pub const SODB_APPLICATION_ID: i32 = 0x0D3D_50DB;

pub(crate) const COL_KEY: &str = "Key";
pub(crate) const COL_BYTECODE: &str = "Bytecode";
pub(crate) const COL_VERSION: &str = "Version";
pub(crate) const COL_PSO_KEY: &str = "PSOKey";
pub(crate) const COL_SO_KEY: &str = "SOKey";
pub(crate) const COL_DXIL_LIB_KEY: &str = "DxilLibKey";

pub(crate) const APP_ID_COLUMNS: &[&str] = &[
    "id",
    "exe",
    "app_name",
    "engine_name",
    "app_version",
    "engine_version",
];

/// `pipeline_states` columns used for description hashing (no bytecode FKs).
pub(crate) const PIPELINE_DESC_COLUMNS: &[&str] = &[
    "RootSignature",
    "InputLayout",
    "DepthStencilDesc",
    "RenderTargetFormats",
    "BlendDesc",
    "RasterizerDesc",
    "ViewInstancingDesc",
    "StreamOutDesc",
    "SampleDesc_Count",
    "SampleDesc_Quality",
    "SampleMask",
    "IBStripCutValue",
    "PrimitiveTopology",
    "DSVFormat",
    "NodeMask",
    "Flags",
];

/// Foreign keys from `pipeline_states` into `shader_bytecode`, in spec order.
pub(crate) const PIPELINE_BYTECODE_COLUMNS: &[&str] = &[
    "ByteCode_VS",
    "ByteCode_PS",
    "ByteCode_HS",
    "ByteCode_DS",
    "ByteCode_GS",
    "ByteCode_AS",
    "ByteCode_MS",
    "ByteCode_CS",
];

/// Foreign key from a keyed row into another keyed table.
pub(crate) struct KeyedFk {
    pub column: &'static str,
    pub table: TableId,
}

/// Association table loaded once and indexed by an owner key.
pub(crate) struct AssocSpec {
    pub table: TableId,
    pub owner_col: &'static str,
    pub columns: &'static [&'static str],
    pub follow_fks: &'static [KeyedFk],
}

/// Keyed descriptor / subobject table loaded once by `Key`.
pub(crate) struct KeyedSpec {
    pub table: TableId,
    pub columns: &'static [&'static str],
    pub keyed_fks: &'static [KeyedFk],
    pub assocs: &'static [TableId],
}

const PSO_LIKE_DESCS: &[KeyedFk] = &[
    KeyedFk {
        column: "RootSignature",
        table: TableId::RootSignatures,
    },
    KeyedFk {
        column: "DepthStencilDesc",
        table: TableId::DepthStencilDescs,
    },
    KeyedFk {
        column: "RenderTargetFormats",
        table: TableId::RenderTargetFormats,
    },
    KeyedFk {
        column: "BlendDesc",
        table: TableId::BlendDescs,
    },
    KeyedFk {
        column: "RasterizerDesc",
        table: TableId::RasterizerDescs,
    },
    KeyedFk {
        column: "ViewInstancingDesc",
        table: TableId::ViewInstancingDescs,
    },
    KeyedFk {
        column: "StreamOutDesc",
        table: TableId::StreamOutDescs,
    },
];

/// Keyed tables loaded once for description-hash follows.
pub(crate) const KEYED_SPECS: &[KeyedSpec] = &[
    KeyedSpec {
        table: TableId::RootSignatures,
        columns: &["Key", "value"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::DepthStencilOpDescs,
        columns: &[
            "Key",
            "StencilFailOp",
            "StencilDepthFailOp",
            "StencilPassOp",
            "StencilFunc",
            "StencilReadMask",
            "StencilWriteMask",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::DepthStencilDescs,
        columns: &[
            "Key",
            "DepthEnable",
            "DepthWriteMask",
            "DepthFunc",
            "StencilEnable",
            "FrontFace",
            "BackFace",
            "DepthBoundsTestEnable",
        ],
        keyed_fks: &[
            KeyedFk {
                column: "FrontFace",
                table: TableId::DepthStencilOpDescs,
            },
            KeyedFk {
                column: "BackFace",
                table: TableId::DepthStencilOpDescs,
            },
        ],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::RenderTargetFormats,
        columns: &[
            "Key",
            "RTFormat0",
            "RTFormat1",
            "RTFormat2",
            "RTFormat3",
            "RTFormat4",
            "RTFormat5",
            "RTFormat6",
            "RTFormat7",
            "NumRenderTargets",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::RenderTargetBlendDescs,
        columns: &[
            "Key",
            "BlendEnable",
            "LogicOpEnable",
            "SrcBlend",
            "DestBlend",
            "BlendOp",
            "SrcBlendAlpha",
            "DestBlendAlpha",
            "BlendOpAlpha",
            "LogicOp",
            "RenderTargetWriteMask",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::BlendDescs,
        columns: &[
            "Key",
            "AlphaToCoverageEnable",
            "IndependentBlendEnable",
            "RenderTarget0",
            "RenderTarget1",
            "RenderTarget2",
            "RenderTarget3",
            "RenderTarget4",
            "RenderTarget5",
            "RenderTarget6",
            "RenderTarget7",
        ],
        keyed_fks: &[
            KeyedFk {
                column: "RenderTarget0",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget1",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget2",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget3",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget4",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget5",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget6",
                table: TableId::RenderTargetBlendDescs,
            },
            KeyedFk {
                column: "RenderTarget7",
                table: TableId::RenderTargetBlendDescs,
            },
        ],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::RasterizerDescs,
        columns: &[
            "Key",
            "FillMode",
            "CullMode",
            "FrontCounterClockwise",
            "DepthBias",
            "DepthBiasClamp",
            "SlopeScaledDepthBias",
            "DepthClipEnable",
            "LineRasterizationMode",
            "ForcedSampleCount",
            "ConservativeRaster",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::ViewInstancingDescs,
        columns: &[
            "Key",
            "ViewInstanceCount",
            "RenderFlags",
            "ViewportArrayIndex0",
            "RenderTargetArrayIndex0",
            "ViewportArrayIndex1",
            "RenderTargetArrayIndex1",
            "ViewportArrayIndex2",
            "RenderTargetArrayIndex2",
            "ViewportArrayIndex3",
            "RenderTargetArrayIndex3",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::StreamOutDescs,
        columns: &[
            "Key",
            "BufferStride0",
            "BufferStride1",
            "BufferStride2",
            "BufferStride3",
            "NumStrides",
            "RasterizedStream",
        ],
        keyed_fks: &[],
        assocs: &[TableId::StreamOutputDescToDeclAssociations],
    },
    KeyedSpec {
        table: TableId::SoDeclarations,
        columns: &[
            "Key",
            "Stream",
            "SemanticName",
            "SemanticIndex",
            "StartComponent",
            "ComponentCount",
            "OutputSlot",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::InputElementDescs,
        columns: &[
            "Key",
            "SemanticName",
            "SemanticIndex",
            "Format",
            "InputSlot",
            "AlignedByteOffset",
            "InputSlotClass",
            "InstanceDataStepRate",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::Exports,
        columns: &["Key", "Name", "ExportToRename", "Flags"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::RtHitGroups,
        columns: &[
            "Key",
            "HitGroupExport",
            "Type",
            "AnyHitShaderImport",
            "ClosestHitShaderImport",
            "IntersectionShaderImport",
        ],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::RtShaderConfig,
        columns: &["Key", "MaxPayloadSizeInBytes", "MaxAttributeSizeInBytes"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::RtPipelineConfig,
        columns: &["Key", "MaxTraceRecursionDepth", "Flags"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::DxilSubobjectToExportsAssociations,
        columns: &["Key", "SubobjectToAssociate"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::SubobjectToExportsAssociations,
        columns: &["Key", "SubobjectType", "SubobjectKey"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::GenericPrograms,
        columns: &[
            "Key",
            "ProgramName",
            "InputLayout",
            "DepthStencilDesc",
            "RenderTargetFormats",
            "BlendDesc",
            "RasterizerDesc",
            "ViewInstancingDesc",
            "StreamOutDesc",
            "SampleDesc_Count",
            "SampleDesc_Quality",
            "SampleMask",
            "IBStripCutValue",
            "PrimitiveTopology",
            "DSVFormat",
            "NodeMask",
            "Flags",
        ],
        keyed_fks: PSO_LIKE_DESCS,
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::WorkGraphs,
        columns: &["Key", "ProgramName", "Flags"],
        keyed_fks: &[],
        assocs: &[
            TableId::WorkGraphToEntrypointAssociations,
            TableId::WorkGraphToNodeAssociations,
        ],
    },
    KeyedSpec {
        table: TableId::NodeIds,
        columns: &["Key", "Name", "ArrayIndex"],
        keyed_fks: &[],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::ShaderNodes,
        columns: &[
            "Key",
            "ShaderOrProgram",
            "NodeType",
            "OverridesType",
            "LocalRootArgumentsTableIndex",
            "ProgramEntry",
            "NewName",
            "ShareInputOf",
            "DispatchGridX",
            "DispatchGridY",
            "DispatchGridZ",
            "MaxDispatchGridX",
            "MaxDispatchGridY",
            "MaxDispatchGridZ",
            "MaxInputRecordsPerGraphEntryRecord_RecordCount",
            "MaxInputRecordsPerGraphEntryRecord_bCountSharedAcrossNodeArray",
        ],
        keyed_fks: &[
            KeyedFk {
                column: "NewName",
                table: TableId::NodeIds,
            },
            KeyedFk {
                column: "ShareInputOf",
                table: TableId::NodeIds,
            },
        ],
        assocs: &[TableId::WorkgraphNodeToOutputOverridesAssociations],
    },
    KeyedSpec {
        table: TableId::NodeOutputOverrides,
        columns: &[
            "Key",
            "OutputIndex",
            "NewName",
            "AllowSparseNodes",
            "MaxRecords",
            "MaxRecordsSharedWithOutputIndex",
        ],
        keyed_fks: &[KeyedFk {
            column: "NewName",
            table: TableId::NodeIds,
        }],
        assocs: &[],
    },
    KeyedSpec {
        table: TableId::StateObjects,
        columns: &["Key", "Type", "NodeMask", "Flags", "AddToStateObjectParent"],
        keyed_fks: &[KeyedFk {
            column: "AddToStateObjectParent",
            table: TableId::StateObjects,
        }],
        assocs: &[
            TableId::SoToGlobalRsAssociations,
            TableId::SoToLocalRsAssociations,
            TableId::SoToDxilLibAssociations,
            TableId::SoToExistingSoAssociations,
            TableId::SoToHitGroupAssociations,
            TableId::SoToRtShaderConfigAssociations,
            TableId::SoToRtPipelineConfigAssociations,
            TableId::SoToDxilSubobjectToExportsAssociations,
            TableId::SoToSubobjectToExportsAssociations,
            TableId::SoToGenericProgramAssociations,
            TableId::SoToWorkGraphAssociations,
        ],
    },
];

/// Association tables loaded once and indexed by owner key.
pub(crate) const ASSOC_SPECS: &[AssocSpec] = &[
    AssocSpec {
        table: TableId::InputLayoutToInputElementAssociations,
        owner_col: "InputLayoutKey",
        columns: &["InputLayoutKey", "InputElementKey"],
        follow_fks: &[KeyedFk {
            column: "InputElementKey",
            table: TableId::InputElementDescs,
        }],
    },
    AssocSpec {
        table: TableId::StreamOutputDescToDeclAssociations,
        owner_col: "StreamOutDescKey",
        columns: &["StreamOutDescKey", "StreamOutDeclKey"],
        follow_fks: &[KeyedFk {
            column: "StreamOutDeclKey",
            table: TableId::SoDeclarations,
        }],
    },
    AssocSpec {
        table: TableId::SoToGlobalRsAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "RootSignatureKey"],
        follow_fks: &[KeyedFk {
            column: "RootSignatureKey",
            table: TableId::RootSignatures,
        }],
    },
    AssocSpec {
        table: TableId::SoToLocalRsAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "RootSignatureKey"],
        follow_fks: &[KeyedFk {
            column: "RootSignatureKey",
            table: TableId::RootSignatures,
        }],
    },
    AssocSpec {
        table: TableId::SoToDxilLibAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "DxilLibKey", "ExportKey"],
        follow_fks: &[KeyedFk {
            column: "ExportKey",
            table: TableId::Exports,
        }],
    },
    AssocSpec {
        table: TableId::SoToExistingSoAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "ExistingStateObjectKey", "ExportKey"],
        follow_fks: &[
            KeyedFk {
                column: "ExistingStateObjectKey",
                table: TableId::StateObjects,
            },
            KeyedFk {
                column: "ExportKey",
                table: TableId::Exports,
            },
        ],
    },
    AssocSpec {
        table: TableId::SoToHitGroupAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "HitGroupKey"],
        follow_fks: &[KeyedFk {
            column: "HitGroupKey",
            table: TableId::RtHitGroups,
        }],
    },
    AssocSpec {
        table: TableId::SoToRtShaderConfigAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "ShaderConfigKey"],
        follow_fks: &[KeyedFk {
            column: "ShaderConfigKey",
            table: TableId::RtShaderConfig,
        }],
    },
    AssocSpec {
        table: TableId::SoToRtPipelineConfigAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "PipelineConfigKey"],
        follow_fks: &[KeyedFk {
            column: "PipelineConfigKey",
            table: TableId::RtPipelineConfig,
        }],
    },
    AssocSpec {
        table: TableId::SoToDxilSubobjectToExportsAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "DxilSubobjectToExportsAssociationKey"],
        follow_fks: &[KeyedFk {
            column: "DxilSubobjectToExportsAssociationKey",
            table: TableId::DxilSubobjectToExportsAssociations,
        }],
    },
    AssocSpec {
        table: TableId::SoToSubobjectToExportsAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "SubobjectToExportsAssociationKey"],
        follow_fks: &[KeyedFk {
            column: "SubobjectToExportsAssociationKey",
            table: TableId::SubobjectToExportsAssociations,
        }],
    },
    AssocSpec {
        table: TableId::SoToGenericProgramAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "GenericProgramKey"],
        follow_fks: &[KeyedFk {
            column: "GenericProgramKey",
            table: TableId::GenericPrograms,
        }],
    },
    AssocSpec {
        table: TableId::SoToWorkGraphAssociations,
        owner_col: "StateObjectKey",
        columns: &["StateObjectKey", "WorkGraphKey"],
        follow_fks: &[KeyedFk {
            column: "WorkGraphKey",
            table: TableId::WorkGraphs,
        }],
    },
    AssocSpec {
        table: TableId::WorkGraphToEntrypointAssociations,
        owner_col: "WorkGraphKey",
        columns: &["WorkGraphKey", "NodeIDKey"],
        follow_fks: &[KeyedFk {
            column: "NodeIDKey",
            table: TableId::NodeIds,
        }],
    },
    AssocSpec {
        table: TableId::WorkGraphToNodeAssociations,
        owner_col: "WorkGraphKey",
        columns: &["WorkGraphKey", "WorkGraphNodeKey"],
        follow_fks: &[KeyedFk {
            column: "WorkGraphNodeKey",
            table: TableId::ShaderNodes,
        }],
    },
    AssocSpec {
        table: TableId::WorkgraphNodeToOutputOverridesAssociations,
        owner_col: "WorkGraphNodeKey",
        columns: &["WorkGraphNodeKey", "NodeOutputOverridesKey"],
        follow_fks: &[KeyedFk {
            column: "NodeOutputOverridesKey",
            table: TableId::NodeOutputOverrides,
        }],
    },
];

pub(crate) fn keyed_spec(table: TableId) -> Option<&'static KeyedSpec> {
    KEYED_SPECS.iter().find(|s| s.table == table)
}

/// Keyed tables format-1 records can populate. Used by the writer and the
/// schema-drift test so reader and writer cannot diverge silently.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const WRITER_KEYED_TABLES: &[TableId] = &[
    TableId::RootSignatures,
    TableId::DepthStencilOpDescs,
    TableId::DepthStencilDescs,
    TableId::RenderTargetFormats,
    TableId::RenderTargetBlendDescs,
    TableId::BlendDescs,
    TableId::RasterizerDescs,
    TableId::ViewInstancingDescs,
    TableId::StreamOutDescs,
    TableId::SoDeclarations,
    TableId::InputElementDescs,
];

/// Association tables format-1 records can populate.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const WRITER_ASSOC_TABLES: &[TableId] = &[
    TableId::InputLayoutToInputElementAssociations,
    TableId::StreamOutputDescToDeclAssociations,
];

/// Expected writer table → columns, derived from the parser specs.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn writer_expected_columns() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut tables = Vec::new();
    tables.push(("shader_bytecode", vec!["Key", "Type", "Bytecode"]));
    for spec in KEYED_SPECS {
        if WRITER_KEYED_TABLES.contains(&spec.table) {
            tables.push((spec.table.sql_name(), spec.columns.to_vec()));
        }
    }
    for spec in ASSOC_SPECS {
        if WRITER_ASSOC_TABLES.contains(&spec.table) {
            tables.push((spec.table.sql_name(), spec.columns.to_vec()));
        }
    }
    tables.push(("groups", vec!["Key", "Version", "PSOKey", "SOKey"]));
    let mut pso =
        Vec::with_capacity(1 + PIPELINE_BYTECODE_COLUMNS.len() + PIPELINE_DESC_COLUMNS.len());
    pso.push("Key");
    pso.extend_from_slice(PIPELINE_BYTECODE_COLUMNS);
    pso.extend_from_slice(PIPELINE_DESC_COLUMNS);
    tables.push(("pipeline_states", pso));
    tables
}

/// Expected writer table → (column, declared SQLite type).
///
/// Names come from [`writer_expected_columns`] (parser specs). Types match
/// [`SODB_WRITER_SCHEMA_SQL`]. A drift in either list fails the schema test.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn writer_expected_schema() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    writer_expected_columns()
        .into_iter()
        .map(|(table, cols)| {
            let typed = cols
                .into_iter()
                .map(|col| (col, writer_declared_type(table, col)))
                .collect();
            (table, typed)
        })
        .collect()
}

#[cfg_attr(not(test), allow(dead_code))]
fn writer_declared_type(table: &str, col: &str) -> &'static str {
    match (table, col) {
        ("shader_bytecode", "Type") | (_, "SemanticName") => "TEXT",
        ("rasterizer_descs", "DepthBias" | "DepthBiasClamp" | "SlopeScaledDepthBias") => "REAL",
        (
            _,
            "Key"
            | "Bytecode"
            | "value"
            | "FrontFace"
            | "BackFace"
            | "RenderTarget0"
            | "RenderTarget1"
            | "RenderTarget2"
            | "RenderTarget3"
            | "RenderTarget4"
            | "RenderTarget5"
            | "RenderTarget6"
            | "RenderTarget7"
            | "InputLayoutKey"
            | "InputElementKey"
            | "StreamOutDescKey"
            | "StreamOutDeclKey"
            | "PSOKey"
            | "SOKey"
            | "RootSignature"
            | "InputLayout"
            | "ByteCode_VS"
            | "ByteCode_PS"
            | "ByteCode_HS"
            | "ByteCode_DS"
            | "ByteCode_GS"
            | "ByteCode_AS"
            | "ByteCode_MS"
            | "ByteCode_CS"
            | "DepthStencilDesc"
            | "RenderTargetFormats"
            | "BlendDesc"
            | "RasterizerDesc"
            | "ViewInstancingDesc"
            | "StreamOutDesc",
        ) => "BLOB",
        _ => "INTEGER",
    }
}

/// Fixed page size used by the SODB writer so output is byte-identical.
pub const SODB_WRITER_PAGE_SIZE: i64 = 4096;

/// SQLite statements that create the tables the writer emits.
///
/// Column names match [`KEYED_SPECS`], [`PIPELINE_DESC_COLUMNS`],
/// [`PIPELINE_BYTECODE_COLUMNS`], and the association specs the parser
/// reads. Tables that format-1 records cannot populate (`state_objects`,
/// `app_id`, raytracing, work graphs, …) are omitted on purpose.
pub const SODB_WRITER_SCHEMA_SQL: &str = "
CREATE TABLE shader_bytecode (
    Key BLOB NOT NULL PRIMARY KEY,
    Type TEXT,
    Bytecode BLOB
);
CREATE TABLE root_signatures (
    Key BLOB NOT NULL PRIMARY KEY,
    value BLOB NOT NULL
);
CREATE TABLE depth_stencil_op_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    StencilFailOp INTEGER NOT NULL,
    StencilDepthFailOp INTEGER NOT NULL,
    StencilPassOp INTEGER NOT NULL,
    StencilFunc INTEGER NOT NULL,
    StencilReadMask INTEGER NOT NULL,
    StencilWriteMask INTEGER NOT NULL
);
CREATE TABLE depth_stencil_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    DepthEnable INTEGER NOT NULL,
    DepthWriteMask INTEGER NOT NULL,
    DepthFunc INTEGER NOT NULL,
    StencilEnable INTEGER NOT NULL,
    FrontFace BLOB,
    BackFace BLOB,
    DepthBoundsTestEnable INTEGER NOT NULL
);
CREATE TABLE render_target_formats (
    Key BLOB NOT NULL PRIMARY KEY,
    RTFormat0 INTEGER NOT NULL,
    RTFormat1 INTEGER NOT NULL,
    RTFormat2 INTEGER NOT NULL,
    RTFormat3 INTEGER NOT NULL,
    RTFormat4 INTEGER NOT NULL,
    RTFormat5 INTEGER NOT NULL,
    RTFormat6 INTEGER NOT NULL,
    RTFormat7 INTEGER NOT NULL,
    NumRenderTargets INTEGER NOT NULL
);
CREATE TABLE render_target_blend_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    BlendEnable INTEGER NOT NULL,
    LogicOpEnable INTEGER NOT NULL,
    SrcBlend INTEGER NOT NULL,
    DestBlend INTEGER NOT NULL,
    BlendOp INTEGER NOT NULL,
    SrcBlendAlpha INTEGER NOT NULL,
    DestBlendAlpha INTEGER NOT NULL,
    BlendOpAlpha INTEGER NOT NULL,
    LogicOp INTEGER NOT NULL,
    RenderTargetWriteMask INTEGER NOT NULL
);
CREATE TABLE blend_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    AlphaToCoverageEnable INTEGER NOT NULL,
    IndependentBlendEnable INTEGER NOT NULL,
    RenderTarget0 BLOB,
    RenderTarget1 BLOB,
    RenderTarget2 BLOB,
    RenderTarget3 BLOB,
    RenderTarget4 BLOB,
    RenderTarget5 BLOB,
    RenderTarget6 BLOB,
    RenderTarget7 BLOB
);
CREATE TABLE rasterizer_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    FillMode INTEGER NOT NULL,
    CullMode INTEGER NOT NULL,
    FrontCounterClockwise INTEGER NOT NULL,
    DepthBias REAL NOT NULL,
    DepthBiasClamp REAL NOT NULL,
    SlopeScaledDepthBias REAL NOT NULL,
    DepthClipEnable INTEGER NOT NULL,
    LineRasterizationMode INTEGER NOT NULL,
    ForcedSampleCount INTEGER NOT NULL,
    ConservativeRaster INTEGER NOT NULL
);
CREATE TABLE view_instancing_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    ViewInstanceCount INTEGER NOT NULL,
    RenderFlags INTEGER NOT NULL,
    ViewportArrayIndex0 INTEGER,
    RenderTargetArrayIndex0 INTEGER,
    ViewportArrayIndex1 INTEGER,
    RenderTargetArrayIndex1 INTEGER,
    ViewportArrayIndex2 INTEGER,
    RenderTargetArrayIndex2 INTEGER,
    ViewportArrayIndex3 INTEGER,
    RenderTargetArrayIndex3 INTEGER
);
CREATE TABLE stream_out_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    BufferStride0 INTEGER NOT NULL,
    BufferStride1 INTEGER NOT NULL,
    BufferStride2 INTEGER NOT NULL,
    BufferStride3 INTEGER NOT NULL,
    NumStrides INTEGER NOT NULL,
    RasterizedStream INTEGER NOT NULL
);
CREATE TABLE so_declarations (
    Key BLOB NOT NULL PRIMARY KEY,
    Stream INTEGER NOT NULL,
    SemanticName TEXT NOT NULL,
    SemanticIndex INTEGER NOT NULL,
    StartComponent INTEGER NOT NULL,
    ComponentCount INTEGER NOT NULL,
    OutputSlot INTEGER NOT NULL
);
CREATE TABLE input_element_descs (
    Key BLOB NOT NULL PRIMARY KEY,
    SemanticName TEXT NOT NULL,
    SemanticIndex INTEGER NOT NULL,
    Format INTEGER NOT NULL,
    InputSlot INTEGER NOT NULL,
    AlignedByteOffset INTEGER NOT NULL,
    InputSlotClass INTEGER NOT NULL,
    InstanceDataStepRate INTEGER NOT NULL
);
CREATE TABLE input_layout_to_input_element_associations (
    InputLayoutKey BLOB NOT NULL,
    InputElementKey BLOB NOT NULL
);
CREATE TABLE stream_output_desc_to_stream_output_decl_associations (
    StreamOutDescKey BLOB NOT NULL,
    StreamOutDeclKey BLOB NOT NULL
);
CREATE TABLE groups (
    Key BLOB NOT NULL PRIMARY KEY,
    Version INTEGER NOT NULL,
    PSOKey BLOB,
    SOKey BLOB
);
CREATE TABLE pipeline_states (
    Key BLOB NOT NULL PRIMARY KEY,
    RootSignature BLOB,
    InputLayout BLOB,
    ByteCode_VS BLOB,
    ByteCode_PS BLOB,
    ByteCode_HS BLOB,
    ByteCode_DS BLOB,
    ByteCode_GS BLOB,
    ByteCode_AS BLOB,
    ByteCode_MS BLOB,
    ByteCode_CS BLOB,
    DepthStencilDesc BLOB,
    RenderTargetFormats BLOB,
    BlendDesc BLOB,
    RasterizerDesc BLOB,
    ViewInstancingDesc BLOB,
    StreamOutDesc BLOB,
    SampleDesc_Count INTEGER,
    SampleDesc_Quality INTEGER,
    SampleMask INTEGER,
    IBStripCutValue INTEGER,
    PrimitiveTopology INTEGER,
    DSVFormat INTEGER,
    NodeMask INTEGER,
    Flags INTEGER
);
";

/// PSO-level keyed follows (same set as [`PSO_LIKE_DESCS`]).
pub(crate) const PIPELINE_KEYED_FOLLOWS: &[KeyedFk] = PSO_LIKE_DESCS;

/// Tables listed in [`crate::SodbSummary::missing_tables`] when absent.
pub(crate) const REPORTED_IF_MISSING: &[TableId] = &[
    TableId::AppId,
    TableId::PipelineStates,
    TableId::StateObjects,
    TableId::ShaderBytecode,
    TableId::RootSignatures,
    TableId::Groups,
    TableId::SoToDxilLibAssociations,
];
