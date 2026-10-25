// SPDX-License-Identifier: MIT

//! Deterministic SODB writer. Shader bytes go only to the rebuilt file.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use osdb_records::{
    BlendDesc, DepthStencilDesc, DepthStencilOpDesc, HexBytes, InputElementDesc, RecordFile,
    RenderTargetBlendDesc, Sha256, SoDeclaration, StreamOutDesc,
};
use osdb_shaders::LocalShaderIndex;
use rusqlite::{Connection, OptionalExtension};
use sodb::{
    hash_shader_container, parse_root_signature, SODB_APPLICATION_ID, SODB_USER_VERSION,
    SODB_WRITER_PAGE_SIZE, SODB_WRITER_SCHEMA_SQL,
};

use crate::{Error, RebuildOptions, RebuildReport};

/// One-line notice: the rebuilt SODB contains bytecode and is local-only.
pub const LOCAL_USE_NOTICE: &str =
    "Rebuilt SODB is for local use only and must not be shared or uploaded.";

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

type DescKey = [u8; 32];
type DepthStencilRow = (DepthStencilDesc, Option<DescKey>, Option<DescKey>);
type BlendRow = ([Option<DescKey>; 8], BlendDesc);

/// Join records to local shaders and write a v2 SODB at `out`.
pub fn rebuild_sodb(
    records: &RecordFile,
    shaders: &LocalShaderIndex,
    out: &Path,
    opts: RebuildOptions,
) -> Result<RebuildReport, Error> {
    refuse_output_path(out, shaders, &opts)?;

    let mut report = RebuildReport::default();
    let mut plan = Plan::default();
    for record in &records.records {
        classify_and_plan(record, records, shaders, &mut plan, &mut report)?;
    }
    report.unique_shaders = plan.shaders.len() as u64;
    report.unique_root_signatures = plan.root_sigs.len() as u64;

    let tmp = temp_sibling(out);
    let mut guard = TempGuard {
        path: tmp.clone(),
        persist: false,
    };
    write_database(&tmp, records, shaders, &plan, opts.max_output_bytes)?;
    let got = fs::metadata(&tmp).map_err(|_| Error::Io)?.len();
    if got > opts.max_output_bytes {
        return Err(Error::OutputTooLarge {
            max: opts.max_output_bytes,
            got,
        });
    }
    let file = File::open(&tmp).map_err(|_| Error::Io)?;
    file.sync_all().map_err(|_| Error::Io)?;
    drop(file);
    fs::rename(&tmp, out).map_err(|_| Error::Io)?;
    guard.persist = true;
    Ok(report)
}

#[derive(Default)]
struct Plan {
    shaders: BTreeMap<Sha256, ()>,
    root_sigs: BTreeMap<Sha256, ()>,
}

enum Skip {
    MissingStage,
    Unresolved,
    RootSignature,
}

fn classify_and_plan(
    record: &osdb_records::PipelineStateRecord,
    file: &RecordFile,
    shaders: &LocalShaderIndex,
    plan: &mut Plan,
    report: &mut RebuildReport,
) -> Result<(), Error> {
    if let Some(reason) = skip_reason(record, shaders) {
        match reason {
            Skip::Unresolved => {
                report.skipped_unresolved_stages =
                    report.skipped_unresolved_stages.saturating_add(1);
            }
            Skip::RootSignature => {
                report.skipped_root_signature = report.skipped_root_signature.saturating_add(1);
            }
            Skip::MissingStage => {
                report.skipped_missing_stage = report.skipped_missing_stage.saturating_add(1);
            }
        }
        return Ok(());
    }
    if let Some(hash) = record.root_signature {
        if !file.root_signatures.contains_key(&hash) {
            return Err(Error::DescriptorHashMismatch);
        }
        plan.root_sigs.insert(hash, ());
    }
    for (_, hash) in record.bytecode_hashes.iter() {
        plan.shaders.insert(hash, ());
    }
    report.rebuilt = report.rebuilt.saturating_add(1);
    Ok(())
}

fn skip_reason(
    record: &osdb_records::PipelineStateRecord,
    shaders: &LocalShaderIndex,
) -> Option<Skip> {
    if !record.unresolved_stages.is_empty() {
        return Some(Skip::Unresolved);
    }
    if record.root_signature_skip.is_some() {
        return Some(Skip::RootSignature);
    }
    if record.bytecode_hashes.is_empty() {
        return Some(Skip::MissingStage);
    }
    for (_, hash) in record.bytecode_hashes.iter() {
        if !shaders.contains(&hash) {
            return Some(Skip::MissingStage);
        }
    }
    None
}

fn write_database(
    path: &Path,
    records: &RecordFile,
    shaders: &LocalShaderIndex,
    plan: &Plan,
    max_output: u64,
) -> Result<(), Error> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "page_size", SODB_WRITER_PAGE_SIZE)?;
    conn.pragma_update(None, "encoding", "UTF-8")?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.pragma_update(None, "auto_vacuum", "NONE")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)?;
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)?;
    conn.execute_batch(SODB_WRITER_SCHEMA_SQL)?;

    let mut written = 0_u64;
    let mut shader_type: BTreeMap<Sha256, &'static str> = BTreeMap::new();
    for record in &records.records {
        if skip_reason(record, shaders).is_some() {
            continue;
        }
        for (stage, hash) in record.bytecode_hashes.iter() {
            shader_type.entry(hash).or_insert(stage.as_str());
        }
    }

    for hash in plan.shaders.keys() {
        let bytes = shaders.read_verified(hash)?;
        charge(&mut written, bytes.len() as u64, max_output)?;
        let ty = shader_type.get(hash).copied();
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, ?2, ?3)",
            rusqlite::params![hash.as_bytes().as_slice(), ty, bytes],
        )?;
    }

    for hash in plan.root_sigs.keys() {
        let hex = records
            .root_signatures
            .get(hash)
            .ok_or(Error::DescriptorHashMismatch)?;
        let raw = hex.decoded().map_err(|_| Error::MalformedHex)?;
        let parsed = parse_root_signature(&raw).map_err(|_| Error::RootSignatureMalformed)?;
        let canonical = parsed.serialize();
        if canonical != raw {
            return Err(Error::RootSignatureMalformed);
        }
        charge(&mut written, canonical.len() as u64, max_output)?;
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![hash.as_bytes().as_slice(), canonical],
        )?;
    }

    let mut input_elements: BTreeMap<DescKey, InputElementDesc> = BTreeMap::new();
    let mut layout_assocs: BTreeMap<DescKey, Vec<DescKey>> = BTreeMap::new();
    let mut so_decls: BTreeMap<DescKey, SoDeclaration> = BTreeMap::new();
    let mut so_assocs: BTreeMap<DescKey, Vec<DescKey>> = BTreeMap::new();
    let mut ds_ops: BTreeMap<DescKey, DepthStencilOpDesc> = BTreeMap::new();
    let mut depth_stencils: BTreeMap<DescKey, DepthStencilRow> = BTreeMap::new();
    let mut rt_blends: BTreeMap<DescKey, RenderTargetBlendDesc> = BTreeMap::new();
    let mut blends: BTreeMap<DescKey, BlendRow> = BTreeMap::new();
    let mut rasters: BTreeMap<DescKey, osdb_records::RasterizerDesc> = BTreeMap::new();
    let mut rt_formats: BTreeMap<DescKey, osdb_records::RenderTargetFormats> = BTreeMap::new();
    let mut views: BTreeMap<DescKey, osdb_records::ViewInstancingDesc> = BTreeMap::new();
    let mut stream_outs: BTreeMap<DescKey, StreamOutDesc> = BTreeMap::new();

    for record in &records.records {
        if skip_reason(record, shaders).is_some() {
            continue;
        }
        if let Some(hash) = record.input_layout {
            let elems = records
                .input_layouts
                .get(&hash)
                .ok_or(Error::DescriptorHashMismatch)?;
            let mut ids = Vec::new();
            for elem in elems {
                let id = intern_json(elem)?;
                input_elements.entry(id).or_insert_with(|| elem.clone());
                ids.push(id);
            }
            layout_assocs.insert(*hash.as_bytes(), ids);
        }
        if let Some(hash) = record.stream_out {
            let so = records
                .stream_outputs
                .get(&hash)
                .ok_or(Error::DescriptorHashMismatch)?;
            let mut ids = Vec::new();
            for decl in &so.declarations {
                let id = intern_json(decl)?;
                so_decls.entry(id).or_insert_with(|| decl.clone());
                ids.push(id);
            }
            so_assocs.insert(*hash.as_bytes(), ids);
            stream_outs.insert(*hash.as_bytes(), so.clone());
        }
        if let Some(ds) = &record.depth_stencil {
            let front = ds.front_face.as_ref().map(intern_json).transpose()?;
            let back = ds.back_face.as_ref().map(intern_json).transpose()?;
            if let (Some(id), Some(op)) = (front, ds.front_face.as_ref()) {
                ds_ops.entry(id).or_insert_with(|| op.clone());
            }
            if let (Some(id), Some(op)) = (back, ds.back_face.as_ref()) {
                ds_ops.entry(id).or_insert_with(|| op.clone());
            }
            let id = intern_json(ds)?;
            depth_stencils.insert(id, (ds.clone(), front, back));
        }
        if let Some(blend) = &record.blend {
            let mut rts = [None; 8];
            for (i, rt) in blend.render_targets.iter().enumerate() {
                if let Some(rt) = rt {
                    let id = intern_json(rt)?;
                    rt_blends.entry(id).or_insert_with(|| rt.clone());
                    rts[i] = Some(id);
                }
            }
            let id = intern_json(blend)?;
            blends.insert(id, (rts, blend.clone()));
        }
        if let Some(r) = &record.rasterizer {
            rasters.insert(intern_json(r)?, r.clone());
        }
        if let Some(f) = &record.render_target_formats {
            rt_formats.insert(intern_json(f)?, f.clone());
        }
        if let Some(v) = &record.view_instancing {
            views.insert(intern_json(v)?, v.clone());
        }
    }

    for (key, op) in &ds_ops {
        conn.execute(
            "INSERT INTO depth_stencil_op_descs (
                Key, StencilFailOp, StencilDepthFailOp, StencilPassOp, StencilFunc,
                StencilReadMask, StencilWriteMask
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                key.as_slice(),
                op.stencil_fail_op,
                op.stencil_depth_fail_op,
                op.stencil_pass_op,
                op.stencil_func,
                op.stencil_read_mask,
                op.stencil_write_mask
            ],
        )?;
    }
    for (key, (ds, front, back)) in &depth_stencils {
        conn.execute(
            "INSERT INTO depth_stencil_descs (
                Key, DepthEnable, DepthWriteMask, DepthFunc, StencilEnable,
                FrontFace, BackFace, DepthBoundsTestEnable
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                key.as_slice(),
                ds.depth_enable,
                ds.depth_write_mask,
                ds.depth_func,
                ds.stencil_enable,
                front.as_ref().map(|k| k.as_slice()),
                back.as_ref().map(|k| k.as_slice()),
                ds.depth_bounds_test_enable
            ],
        )?;
    }
    for (key, rt) in &rt_blends {
        conn.execute(
            "INSERT INTO render_target_blend_descs (
                Key, BlendEnable, LogicOpEnable, SrcBlend, DestBlend, BlendOp,
                SrcBlendAlpha, DestBlendAlpha, BlendOpAlpha, LogicOp, RenderTargetWriteMask
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                key.as_slice(),
                rt.blend_enable,
                rt.logic_op_enable,
                rt.src_blend,
                rt.dest_blend,
                rt.blend_op,
                rt.src_blend_alpha,
                rt.dest_blend_alpha,
                rt.blend_op_alpha,
                rt.logic_op,
                rt.render_target_write_mask
            ],
        )?;
    }
    for (key, (rts, blend)) in &blends {
        conn.execute(
            "INSERT INTO blend_descs (
                Key, AlphaToCoverageEnable, IndependentBlendEnable,
                RenderTarget0, RenderTarget1, RenderTarget2, RenderTarget3,
                RenderTarget4, RenderTarget5, RenderTarget6, RenderTarget7
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                key.as_slice(),
                blend.alpha_to_coverage_enable,
                blend.independent_blend_enable,
                rts[0].as_ref().map(|k| k.as_slice()),
                rts[1].as_ref().map(|k| k.as_slice()),
                rts[2].as_ref().map(|k| k.as_slice()),
                rts[3].as_ref().map(|k| k.as_slice()),
                rts[4].as_ref().map(|k| k.as_slice()),
                rts[5].as_ref().map(|k| k.as_slice()),
                rts[6].as_ref().map(|k| k.as_slice()),
                rts[7].as_ref().map(|k| k.as_slice())
            ],
        )?;
    }
    for (key, r) in &rasters {
        conn.execute(
            "INSERT INTO rasterizer_descs (
                Key, FillMode, CullMode, FrontCounterClockwise, DepthBias, DepthBiasClamp,
                SlopeScaledDepthBias, DepthClipEnable, LineRasterizationMode, ForcedSampleCount,
                ConservativeRaster
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                key.as_slice(),
                r.fill_mode,
                r.cull_mode,
                r.front_counter_clockwise,
                r.depth_bias,
                r.depth_bias_clamp,
                r.slope_scaled_depth_bias,
                r.depth_clip_enable,
                r.line_rasterization_mode,
                r.forced_sample_count,
                r.conservative_raster
            ],
        )?;
    }
    for (key, f) in &rt_formats {
        conn.execute(
            "INSERT INTO render_target_formats (
                Key, RTFormat0, RTFormat1, RTFormat2, RTFormat3, RTFormat4, RTFormat5,
                RTFormat6, RTFormat7, NumRenderTargets
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                key.as_slice(),
                f.rt_formats[0],
                f.rt_formats[1],
                f.rt_formats[2],
                f.rt_formats[3],
                f.rt_formats[4],
                f.rt_formats[5],
                f.rt_formats[6],
                f.rt_formats[7],
                f.num_render_targets
            ],
        )?;
    }
    for (key, v) in &views {
        conn.execute(
            "INSERT INTO view_instancing_descs (
                Key, ViewInstanceCount, RenderFlags,
                ViewportArrayIndex0, RenderTargetArrayIndex0,
                ViewportArrayIndex1, RenderTargetArrayIndex1,
                ViewportArrayIndex2, RenderTargetArrayIndex2,
                ViewportArrayIndex3, RenderTargetArrayIndex3
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                key.as_slice(),
                v.view_instance_count,
                v.render_flags,
                v.viewport_array_index[0],
                v.render_target_array_index[0],
                v.viewport_array_index[1],
                v.render_target_array_index[1],
                v.viewport_array_index[2],
                v.render_target_array_index[2],
                v.viewport_array_index[3],
                v.render_target_array_index[3]
            ],
        )?;
    }
    for (key, so) in &stream_outs {
        conn.execute(
            "INSERT INTO stream_out_descs (
                Key, BufferStride0, BufferStride1, BufferStride2, BufferStride3,
                NumStrides, RasterizedStream
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                key.as_slice(),
                so.buffer_stride[0],
                so.buffer_stride[1],
                so.buffer_stride[2],
                so.buffer_stride[3],
                so.num_strides,
                so.rasterized_stream
            ],
        )?;
    }
    for (key, d) in &so_decls {
        conn.execute(
            "INSERT INTO so_declarations (
                Key, Stream, SemanticName, SemanticIndex, StartComponent, ComponentCount, OutputSlot
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                key.as_slice(),
                d.stream,
                &d.semantic_name,
                d.semantic_index,
                d.start_component,
                d.component_count,
                d.output_slot
            ],
        )?;
    }
    for (key, e) in &input_elements {
        conn.execute(
            "INSERT INTO input_element_descs (
                Key, SemanticName, SemanticIndex, Format, InputSlot, AlignedByteOffset,
                InputSlotClass, InstanceDataStepRate
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                key.as_slice(),
                &e.semantic_name,
                e.semantic_index,
                e.format,
                e.input_slot,
                e.aligned_byte_offset,
                e.input_slot_class,
                e.instance_data_step_rate
            ],
        )?;
    }
    for (layout, elems) in &layout_assocs {
        for elem in elems {
            conn.execute(
                "INSERT INTO input_layout_to_input_element_associations
                 (InputLayoutKey, InputElementKey) VALUES (?1, ?2)",
                rusqlite::params![layout.as_slice(), elem.as_slice()],
            )?;
        }
    }
    for (so, decls) in &so_assocs {
        for decl in decls {
            conn.execute(
                "INSERT INTO stream_output_desc_to_stream_output_decl_associations
                 (StreamOutDescKey, StreamOutDeclKey) VALUES (?1, ?2)",
                rusqlite::params![so.as_slice(), decl.as_slice()],
            )?;
        }
    }

    for record in &records.records {
        if skip_reason(record, shaders).is_some() {
            continue;
        }
        let pso_key = decode_hex(record.app_key.as_str())?;
        let group_key = group_key_bytes(&pso_key);
        conn.execute(
            "INSERT INTO groups (Key, Version, PSOKey, SOKey) VALUES (?1, ?2, ?3, NULL)",
            rusqlite::params![group_key.as_slice(), i64::from(record.version), pso_key],
        )?;
        let ds_key = record.depth_stencil.as_ref().map(intern_json).transpose()?;
        let blend_key = record.blend.as_ref().map(intern_json).transpose()?;
        let rast_key = record.rasterizer.as_ref().map(intern_json).transpose()?;
        let rtf_key = record
            .render_target_formats
            .as_ref()
            .map(intern_json)
            .transpose()?;
        let view_key = record
            .view_instancing
            .as_ref()
            .map(intern_json)
            .transpose()?;
        let (sample_count, sample_quality) = match &record.sample_desc {
            Some(s) => (s.count, s.quality),
            None => (None, None),
        };
        conn.execute(
            "INSERT INTO pipeline_states (
                Key, RootSignature, InputLayout,
                ByteCode_VS, ByteCode_PS, ByteCode_HS, ByteCode_DS, ByteCode_GS,
                ByteCode_AS, ByteCode_MS, ByteCode_CS,
                DepthStencilDesc, RenderTargetFormats, BlendDesc, RasterizerDesc,
                ViewInstancingDesc, StreamOutDesc,
                SampleDesc_Count, SampleDesc_Quality, SampleMask, IBStripCutValue,
                PrimitiveTopology, DSVFormat, NodeMask, Flags
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
             )",
            rusqlite::params![
                pso_key,
                record.root_signature.map(|h| h.as_bytes().to_vec()),
                record.input_layout.map(|h| h.as_bytes().to_vec()),
                stage_key(&record.bytecode_hashes.vs),
                stage_key(&record.bytecode_hashes.ps),
                stage_key(&record.bytecode_hashes.hs),
                stage_key(&record.bytecode_hashes.ds),
                stage_key(&record.bytecode_hashes.gs),
                stage_key(&record.bytecode_hashes.as_stage),
                stage_key(&record.bytecode_hashes.ms),
                stage_key(&record.bytecode_hashes.cs),
                ds_key.as_ref().map(|k| k.as_slice()),
                rtf_key.as_ref().map(|k| k.as_slice()),
                blend_key.as_ref().map(|k| k.as_slice()),
                rast_key.as_ref().map(|k| k.as_slice()),
                view_key.as_ref().map(|k| k.as_slice()),
                record.stream_out.map(|h| h.as_bytes().to_vec()),
                sample_count,
                sample_quality,
                record.sample_mask,
                record.ib_strip_cut_value,
                record.primitive_topology,
                record.dsv_format,
                record.node_mask,
                record.flags
            ],
        )?;
    }

    conn.execute_batch("VACUUM;")?;
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)?;
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)?;
    let _ = conn
        .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
        .optional()?;
    drop(conn);
    Ok(())
}

fn stage_key(hash: &Option<Sha256>) -> Option<Vec<u8>> {
    hash.map(|h| h.as_bytes().to_vec())
}

fn intern_json<T: serde::Serialize>(value: &T) -> Result<[u8; 32], Error> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::DescriptorHashMismatch)?;
    Ok(*hash_shader_container(&bytes).as_bytes())
}

fn group_key_bytes(pso_key: &[u8]) -> [u8; 32] {
    let mut tagged = Vec::with_capacity(6 + pso_key.len());
    tagged.extend_from_slice(b"group\0");
    tagged.extend_from_slice(pso_key);
    *hash_shader_container(&tagged).as_bytes()
}

fn decode_hex(text: &str) -> Result<Vec<u8>, Error> {
    HexBytes::parse_hex(text)
        .and_then(|h| h.decoded())
        .map_err(|_| Error::MalformedHex)
}

fn charge(written: &mut u64, add: u64, max: u64) -> Result<(), Error> {
    let next = written.saturating_add(add);
    if next > max {
        return Err(Error::OutputTooLarge { max, got: next });
    }
    *written = next;
    Ok(())
}

fn refuse_output_path(
    out: &Path,
    shaders: &LocalShaderIndex,
    opts: &RebuildOptions,
) -> Result<(), Error> {
    if let Some(rec) = opts.record_input.as_deref() {
        if paths_are_same(out, rec)? {
            return Err(Error::SameAsInput);
        }
    }
    for src in shaders.source_paths() {
        if paths_are_same(out, src)? {
            return Err(Error::SameAsInput);
        }
    }
    for root in shaders.directory_roots() {
        if path_is_under(out, root)? {
            return Err(Error::InsideScanRoot);
        }
    }
    match fs::symlink_metadata(out) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.file_type().is_file() {
                return Err(Error::NotRegular);
            }
            if !opts.force {
                return Err(Error::AlreadyExists);
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::Io),
    }
    Ok(())
}

fn paths_are_same(a: &Path, b: &Path) -> Result<bool, Error> {
    if a == b {
        return Ok(true);
    }
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(da), Ok(db)) => {
            if da.file_type().is_symlink() || db.file_type().is_symlink() {
                return Ok(false);
            }
            if da.file_type().is_file() && db.file_type().is_file() {
                return same_file::is_same_file(a, b).map_err(|_| Error::Io);
            }
            Ok(false)
        }
        (Err(err), _) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        (_, Err(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        _ => Err(Error::Io),
    }
}

fn path_is_under(dest: &Path, root: &Path) -> Result<bool, Error> {
    let root = fs::canonicalize(root).map_err(|_| Error::Io)?;
    let dest_abs = if dest.exists() {
        fs::canonicalize(dest).map_err(|_| Error::Io)?
    } else {
        match dest.parent() {
            Some(parent) if parent.as_os_str().is_empty() => {
                std::env::current_dir().map_err(|_| Error::Io)?.join(dest)
            }
            Some(parent) if parent.exists() => {
                let parent = fs::canonicalize(parent).map_err(|_| Error::Io)?;
                match dest.file_name() {
                    Some(name) => parent.join(name),
                    None => parent,
                }
            }
            _ => dest.to_path_buf(),
        }
    };
    Ok(dest_abs.starts_with(&root))
}

fn temp_sibling(dest: &Path) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("out"));
    dest.with_file_name(format!(
        ".{name}.osdb-rebuild-{}-{n}.tmp",
        std::process::id()
    ))
}

struct TempGuard {
    path: PathBuf,
    persist: bool,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if !self.persist {
            let _ = fs::remove_file(&self.path);
        }
    }
}
