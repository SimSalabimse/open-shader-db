// SPDX-License-Identifier: MIT

//! Extract pipeline-state records from a local SODB (hashes only).

use std::path::Path;

use sodb::{
    extract_pipeline_states_with_limits, ExtractedBlend, ExtractedDepthStencil,
    ExtractedDepthStencilOp, ExtractedInputElement, ExtractedPipelineState, ExtractedRasterizer,
    ExtractedRenderTargetBlend, ExtractedRenderTargetFormats, ExtractedSoDeclaration,
    ExtractedStreamOut, ExtractedViewInstancing,
};

use crate::build::BuildIdentity;
use crate::hash::Sha256;
use crate::types::{
    AppKeyHex, BlendDesc, DepthStencilDesc, DepthStencilOpDesc, HexBytes, InputElementDesc,
    PipelineStateRecord, RasterizerDesc, RenderTargetBlendDesc, RenderTargetFormats, SampleDesc,
    SoDeclaration, StageHashes, StreamOutDesc, ViewInstancingDesc,
};
use crate::{Error, RecordFile, FORMAT_VERSION};

/// Extract records from `path` using SODB default limits.
pub fn extract(path: impl AsRef<Path>, build: BuildIdentity) -> Result<RecordFile, Error> {
    extract_with_sodb_limits(path, build, sodb::Limits::DEFAULT)
}

/// Extract records using caller-supplied SODB resource caps.
pub fn extract_with_sodb_limits(
    path: impl AsRef<Path>,
    build: BuildIdentity,
    limits: sodb::Limits,
) -> Result<RecordFile, Error> {
    let extracted = extract_pipeline_states_with_limits(path, limits)?;
    Ok(RecordFile {
        format_version: FORMAT_VERSION,
        build,
        records: extracted.into_iter().map(from_extracted).collect(),
    })
}

/// Deterministic pretty JSON (same SODB in → byte-identical bytes out).
pub fn encode_record_file(file: &RecordFile) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(file).map_err(|_| Error::InvalidJson)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn from_extracted(row: ExtractedPipelineState) -> PipelineStateRecord {
    let sample_desc = match (row.sample_count, row.sample_quality) {
        (None, None) => None,
        (count, quality) => Some(SampleDesc { count, quality }),
    };
    PipelineStateRecord {
        app_key: AppKeyHex::from_bytes(row.key.as_bytes()),
        version: row.version.get(),
        bytecode_hashes: StageHashes {
            vs: row.bytecode.vs.map(Sha256::from_sodb),
            ps: row.bytecode.ps.map(Sha256::from_sodb),
            hs: row.bytecode.hs.map(Sha256::from_sodb),
            ds: row.bytecode.ds.map(Sha256::from_sodb),
            gs: row.bytecode.gs.map(Sha256::from_sodb),
            as_stage: row.bytecode.amplification.map(Sha256::from_sodb),
            ms: row.bytecode.ms.map(Sha256::from_sodb),
            cs: row.bytecode.cs.map(Sha256::from_sodb),
        },
        root_signature: row
            .root_signature
            .map(|b| HexBytes::from_bytes(b.as_bytes())),
        input_layout: row
            .input_layout
            .map(|elems| elems.into_iter().map(map_input).collect()),
        depth_stencil: row.depth_stencil.map(map_ds),
        blend: row.blend.map(map_blend),
        rasterizer: row.rasterizer.map(map_raster),
        render_target_formats: row.render_target_formats.map(map_rtf),
        view_instancing: row.view_instancing.map(map_view),
        stream_out: row.stream_out.map(map_so),
        sample_desc,
        sample_mask: row.sample_mask,
        ib_strip_cut_value: row.ib_strip_cut_value,
        primitive_topology: row.primitive_topology,
        dsv_format: row.dsv_format,
        node_mask: row.node_mask,
        flags: row.flags,
    }
}

fn map_input(elem: ExtractedInputElement) -> InputElementDesc {
    InputElementDesc {
        semantic_name: elem.semantic_name,
        semantic_index: elem.semantic_index,
        format: elem.format,
        input_slot: elem.input_slot,
        aligned_byte_offset: elem.aligned_byte_offset,
        input_slot_class: elem.input_slot_class,
        instance_data_step_rate: elem.instance_data_step_rate,
    }
}

fn map_op(op: ExtractedDepthStencilOp) -> DepthStencilOpDesc {
    DepthStencilOpDesc {
        stencil_fail_op: op.stencil_fail_op,
        stencil_depth_fail_op: op.stencil_depth_fail_op,
        stencil_pass_op: op.stencil_pass_op,
        stencil_func: op.stencil_func,
        stencil_read_mask: op.stencil_read_mask,
        stencil_write_mask: op.stencil_write_mask,
    }
}

fn map_ds(ds: ExtractedDepthStencil) -> DepthStencilDesc {
    DepthStencilDesc {
        depth_enable: ds.depth_enable,
        depth_write_mask: ds.depth_write_mask,
        depth_func: ds.depth_func,
        stencil_enable: ds.stencil_enable,
        front_face: ds.front_face.map(map_op),
        back_face: ds.back_face.map(map_op),
        depth_bounds_test_enable: ds.depth_bounds_test_enable,
    }
}

fn map_rt_blend(rt: ExtractedRenderTargetBlend) -> RenderTargetBlendDesc {
    RenderTargetBlendDesc {
        blend_enable: rt.blend_enable,
        logic_op_enable: rt.logic_op_enable,
        src_blend: rt.src_blend,
        dest_blend: rt.dest_blend,
        blend_op: rt.blend_op,
        src_blend_alpha: rt.src_blend_alpha,
        dest_blend_alpha: rt.dest_blend_alpha,
        blend_op_alpha: rt.blend_op_alpha,
        logic_op: rt.logic_op,
        render_target_write_mask: rt.render_target_write_mask,
    }
}

fn map_blend(blend: ExtractedBlend) -> BlendDesc {
    BlendDesc {
        alpha_to_coverage_enable: blend.alpha_to_coverage_enable,
        independent_blend_enable: blend.independent_blend_enable,
        render_targets: blend.render_targets.map(|rt| rt.map(map_rt_blend)),
    }
}

fn map_raster(r: ExtractedRasterizer) -> RasterizerDesc {
    RasterizerDesc {
        fill_mode: r.fill_mode,
        cull_mode: r.cull_mode,
        front_counter_clockwise: r.front_counter_clockwise,
        depth_bias: r.depth_bias,
        depth_bias_clamp: r.depth_bias_clamp,
        slope_scaled_depth_bias: r.slope_scaled_depth_bias,
        depth_clip_enable: r.depth_clip_enable,
        line_rasterization_mode: r.line_rasterization_mode,
        forced_sample_count: r.forced_sample_count,
        conservative_raster: r.conservative_raster,
    }
}

fn map_rtf(f: ExtractedRenderTargetFormats) -> RenderTargetFormats {
    RenderTargetFormats {
        rt_formats: f.rt_formats,
        num_render_targets: f.num_render_targets,
    }
}

fn map_view(v: ExtractedViewInstancing) -> ViewInstancingDesc {
    ViewInstancingDesc {
        view_instance_count: v.view_instance_count,
        render_flags: v.render_flags,
        viewport_array_index: v.viewport_array_index,
        render_target_array_index: v.render_target_array_index,
    }
}

fn map_so_decl(d: ExtractedSoDeclaration) -> SoDeclaration {
    SoDeclaration {
        stream: d.stream,
        semantic_name: d.semantic_name,
        semantic_index: d.semantic_index,
        start_component: d.start_component,
        component_count: d.component_count,
        output_slot: d.output_slot,
    }
}

fn map_so(so: ExtractedStreamOut) -> StreamOutDesc {
    StreamOutDesc {
        buffer_stride: so.buffer_stride,
        num_strides: so.num_strides,
        rasterized_stream: so.rasterized_stream,
        declarations: so.declarations.into_iter().map(map_so_decl).collect(),
    }
}
