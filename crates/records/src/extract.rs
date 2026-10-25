// SPDX-License-Identifier: MIT

//! Extract pipeline-state records from a local SODB (hashes only).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use sodb::{
    extract_pipeline_states_with_limits, ExtractedBlend, ExtractedDepthStencil,
    ExtractedDepthStencilOp, ExtractedInputElement, ExtractedPipelineState, ExtractedRasterizer,
    ExtractedRenderTargetBlend, ExtractedRenderTargetFormats, ExtractedSoDeclaration,
    ExtractedStage, ExtractedStreamOut, ExtractedViewInstancing,
};

use crate::build::BuildIdentity;
use crate::hash::{sha256_bytes, sha256_json, Sha256};
use crate::types::{
    AppKeyHex, BlendDesc, DepthStencilDesc, DepthStencilOpDesc, HexBytes, InputElementDesc,
    PipelineStateRecord, RasterizerDesc, RenderTargetBlendDesc, RenderTargetFormats, SampleDesc,
    ShaderStage, SoDeclaration, StageHashes, StreamOutDesc, ViewInstancingDesc,
};
use crate::{Error, Limits, RecordFile, FORMAT_VERSION};

/// Extract records from `path` using SODB default limits.
pub fn extract(path: impl AsRef<Path>, build: BuildIdentity) -> Result<RecordFile, Error> {
    extract_with_options(path, build, sodb::Limits::DEFAULT, false)
}

/// Extract records using caller-supplied SODB resource caps.
pub fn extract_with_sodb_limits(
    path: impl AsRef<Path>,
    build: BuildIdentity,
    limits: sodb::Limits,
) -> Result<RecordFile, Error> {
    extract_with_options(path, build, limits, false)
}

/// Extract records, optionally replacing each `app_key` with SHA-256 of the key.
pub fn extract_with_options(
    path: impl AsRef<Path>,
    build: BuildIdentity,
    limits: sodb::Limits,
    hash_app_keys: bool,
) -> Result<RecordFile, Error> {
    let extracted = extract_pipeline_states_with_limits(path, limits)?;
    let mut root_signatures = BTreeMap::new();
    let mut input_layouts = BTreeMap::new();
    let mut stream_outputs = BTreeMap::new();
    let mut records = Vec::with_capacity(extracted.records.len());
    for row in extracted.records {
        records.push(from_extracted(
            row,
            &mut root_signatures,
            &mut input_layouts,
            &mut stream_outputs,
        )?);
    }
    if hash_app_keys {
        for record in &mut records {
            let raw = crate::hash::parse_hex_bytes(record.app_key.as_str())?;
            record.app_key = AppKeyHex::from_bytes(sha256_bytes(&raw).as_bytes());
        }
        records.sort_by(|a, b| a.app_key.cmp(&b.app_key));
    }
    Ok(RecordFile {
        format_version: FORMAT_VERSION,
        build,
        root_signatures,
        input_layouts,
        stream_outputs,
        records,
    })
}

/// Deterministic pretty JSON streamed into `writer`, capped at 64 MiB.
pub fn encode_record_file_to_writer<W: Write>(file: &RecordFile, writer: W) -> Result<(), Error> {
    let mut capped = CappedWriter {
        inner: writer,
        written: 0,
        max: Limits::DEFAULT.max_file_bytes,
        exceeded: false,
    };
    let pretty = serde_json::to_writer_pretty(&mut capped, file);
    if capped.exceeded {
        return Err(Error::OutputTooLarge { max: capped.max });
    }
    pretty.map_err(|_| Error::InvalidJson)?;
    if let Err(err) = capped.write_all(b"\n") {
        if capped.exceeded {
            return Err(Error::OutputTooLarge { max: capped.max });
        }
        return Err(map_write_err(err));
    }
    Ok(())
}

/// Deterministic pretty JSON (same SODB in → byte-identical bytes out).
pub fn encode_record_file(file: &RecordFile) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    encode_record_file_to_writer(file, &mut bytes)?;
    Ok(bytes)
}

fn map_write_err(_err: std::io::Error) -> Error {
    Error::Io
}

struct CappedWriter<W> {
    inner: W,
    written: u64,
    max: u64,
    exceeded: bool,
}

impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let add = buf.len() as u64;
        if self.written.saturating_add(add) > self.max {
            self.exceeded = true;
            return Err(std::io::Error::other("record file output cap"));
        }
        let n = self.inner.write(buf)?;
        self.written = self.written.saturating_add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn from_extracted(
    row: ExtractedPipelineState,
    root_signatures: &mut BTreeMap<Sha256, HexBytes>,
    input_layouts: &mut BTreeMap<Sha256, Vec<InputElementDesc>>,
    stream_outputs: &mut BTreeMap<Sha256, StreamOutDesc>,
) -> Result<PipelineStateRecord, Error> {
    let sample_desc = match (row.sample_count, row.sample_quality) {
        (None, None) => None,
        (count, quality) => Some(SampleDesc { count, quality }),
    };
    let root_signature = row
        .root_signature
        .as_ref()
        .map(|blob| intern_hex(root_signatures, blob.as_bytes()));
    let input_layout = match row.input_layout.as_ref() {
        Some(elems) => {
            let mapped: Vec<InputElementDesc> = elems.iter().cloned().map(map_input).collect();
            Some(intern_json(input_layouts, mapped)?)
        }
        None => None,
    };
    let stream_out = match row.stream_out.as_ref() {
        Some(so) => {
            let mapped = map_so(so);
            Some(intern_json(stream_outputs, mapped)?)
        }
        None => None,
    };
    Ok(PipelineStateRecord {
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
        unresolved_stages: row.unresolved_stages.into_iter().map(map_stage).collect(),
        root_signature,
        input_layout,
        depth_stencil: row.depth_stencil.map(map_ds),
        blend: row.blend.map(map_blend),
        rasterizer: row.rasterizer.map(map_raster),
        render_target_formats: row.render_target_formats.map(map_rtf),
        view_instancing: row.view_instancing.map(map_view),
        stream_out,
        sample_desc,
        sample_mask: row.sample_mask,
        ib_strip_cut_value: row.ib_strip_cut_value,
        primitive_topology: row.primitive_topology,
        dsv_format: row.dsv_format,
        node_mask: row.node_mask,
        flags: row.flags,
    })
}

fn intern_hex(table: &mut BTreeMap<Sha256, HexBytes>, bytes: &[u8]) -> Sha256 {
    let hash = sha256_bytes(bytes);
    table
        .entry(hash)
        .or_insert_with(|| HexBytes::from_bytes(bytes));
    hash
}

fn intern_json<T: serde::Serialize>(
    table: &mut BTreeMap<Sha256, T>,
    value: T,
) -> Result<Sha256, Error> {
    let hash = sha256_json(&value)?;
    table.entry(hash).or_insert(value);
    Ok(hash)
}

fn map_stage(stage: ExtractedStage) -> ShaderStage {
    match stage {
        ExtractedStage::Vs => ShaderStage::Vs,
        ExtractedStage::Ps => ShaderStage::Ps,
        ExtractedStage::Hs => ShaderStage::Hs,
        ExtractedStage::Ds => ShaderStage::Ds,
        ExtractedStage::Gs => ShaderStage::Gs,
        ExtractedStage::As => ShaderStage::As,
        ExtractedStage::Ms => ShaderStage::Ms,
        ExtractedStage::Cs => ShaderStage::Cs,
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

fn map_so(so: &ExtractedStreamOut) -> StreamOutDesc {
    StreamOutDesc {
        buffer_stride: so.buffer_stride,
        num_strides: so.num_strides,
        rasterized_stream: so.rasterized_stream,
        declarations: so.declarations.iter().cloned().map(map_so_decl).collect(),
    }
}
