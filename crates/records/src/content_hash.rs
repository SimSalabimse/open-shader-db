// SPDX-License-Identifier: MIT

//! `pipeline_content_hash`: SHA-256 of one pipeline-state record.
//!
//! The preimage is compact, key-sorted JSON of that record with `app_key`
//! and `version` removed. Descriptor references stay content hashes.
//! `docs/RECORD_FORMAT.md` is the byte-for-byte spec.

use crate::file::RecordFile;
use crate::hash::{sha256_bytes, Sha256};
use crate::types::PipelineStateRecord;

/// SHA-256 of the canonical preimage of `record`.
///
/// `file` is the record file the caller loaded. Its descriptor tables,
/// `build`, and `format_version` are not inputs. `root_signature`,
/// `input_layout`, and `stream_out` stay the content hashes already stored
/// on `record`; the table bodies are not read.
///
/// The preimage rules are in `docs/RECORD_FORMAT.md`. A record that cannot
/// be encoded as JSON (a non-finite float) has no object preimage; the
/// hash is then SHA-256 of the four UTF-8 bytes `null`.
#[must_use]
pub fn pipeline_content_hash(file: &RecordFile, record: &PipelineStateRecord) -> Sha256 {
    // `file` is part of the signature so a later consensus step passes the
    // loaded file. Descriptor bodies are not hashed.
    let _ = file.format_version;
    sha256_bytes(&pipeline_content_preimage(record))
}

pub(crate) fn pipeline_content_preimage(record: &PipelineStateRecord) -> Vec<u8> {
    // serde_json turns NaN and infinity into JSON null instead of failing,
    // which would hash a different object. Rule 11 requires the token `null`.
    if rasterizer_is_non_finite(record) {
        return b"null".to_vec();
    }
    let Ok(mut value) = serde_json::to_value(record) else {
        return b"null".to_vec();
    };
    if let Some(object) = value.as_object_mut() {
        object.remove("app_key");
        object.remove("version");
    } else {
        return b"null".to_vec();
    }
    match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(_) => b"null".to_vec(),
    }
}

/// The only `f64` fields on a pipeline record are the three rasterizer biases.
fn rasterizer_is_non_finite(record: &PipelineStateRecord) -> bool {
    let Some(rasterizer) = &record.rasterizer else {
        return false;
    };
    !rasterizer.depth_bias.is_finite()
        || !rasterizer.depth_bias_clamp.is_finite()
        || !rasterizer.slope_scaled_depth_bias.is_finite()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{pipeline_content_hash, pipeline_content_preimage};
    use crate::file::RecordFile;
    use crate::hash::{encode_hex, Sha256};
    use crate::types::{
        AppKeyHex, BlendDesc, DepthStencilDesc, DepthStencilOpDesc, HexBytes, PipelineStateRecord,
        RasterizerDesc, RenderTargetBlendDesc, SampleDesc, ShaderStage, StageHashes,
    };
    use crate::BuildIdentity;

    const MINIMAL_PREIMAGE: &str = r#"{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":null,"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}"#;
    const MINIMAL_HASH: &str = "d2bf3066e02ce21bd8e7af9d601299f921793901a6df3aa775fade455884e615";
    const STAGES_PREIMAGE: &str = r#"{"blend":null,"bytecode_hashes":{"as":"3333333333333333333333333333333333333333333333333333333333333333","ps":"2222222222222222222222222222222222222222222222222222222222222222","vs":"1111111111111111111111111111111111111111111111111111111111111111"},"depth_stencil":null,"dsv_format":null,"flags":0,"ib_strip_cut_value":null,"input_layout":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","node_mask":null,"primitive_topology":null,"rasterizer":null,"render_target_formats":null,"root_signature":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sample_desc":{"count":1,"quality":null},"sample_mask":4294967295,"stream_out":null,"view_instancing":null}"#;
    const STAGES_HASH: &str = "26809b705494bb1915b52f1e9fe3a36ff9a21ea1eb23cef1137280d8ed79518a";
    const NESTED_PREIMAGE: &str = r#"{"blend":{"alpha_to_coverage_enable":0,"independent_blend_enable":1,"render_targets":[{"blend_enable":0,"blend_op":3,"blend_op_alpha":6,"dest_blend":2,"dest_blend_alpha":5,"logic_op":7,"logic_op_enable":0,"render_target_write_mask":15,"src_blend":1,"src_blend_alpha":4},null,null,null,null,null,null,null]},"bytecode_hashes":{},"depth_stencil":{"back_face":null,"depth_bounds_test_enable":0,"depth_enable":1,"depth_func":2,"depth_write_mask":1,"front_face":{"stencil_depth_fail_op":2,"stencil_fail_op":1,"stencil_func":4,"stencil_pass_op":3,"stencil_read_mask":255,"stencil_write_mask":255},"stencil_enable":0},"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":null,"render_target_formats":null,"root_signature":null,"root_signature_skip":"invalid","sample_desc":null,"sample_mask":null,"stream_out":null,"unresolved_stages":["vs","ps"],"view_instancing":null}"#;
    const NESTED_HASH: &str = "54ec9d9e5ea052ddb266be20aa9e27840374cba3b2bcffcd8444e6a6cd9baf7d";
    const FLOATS_PREIMAGE: &str = r#"{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":{"conservative_raster":0,"cull_mode":1,"depth_bias":1.5,"depth_bias_clamp":-0.0,"depth_clip_enable":1,"fill_mode":3,"forced_sample_count":0,"front_counter_clockwise":0,"line_rasterization_mode":0,"slope_scaled_depth_bias":0.0},"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}"#;
    const FLOATS_HASH: &str = "e1f347fd625cc1c8755fa8f10cb5dc74da72208bbf501aee594b32586742fae0";
    const NULL_HASH: &str = "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b";

    fn blank(app_key: &str, version: u32) -> PipelineStateRecord {
        PipelineStateRecord {
            app_key: AppKeyHex::parse_hex(app_key).unwrap(),
            version,
            bytecode_hashes: StageHashes::default(),
            unresolved_stages: Vec::new(),
            root_signature: None,
            root_signature_skip: None,
            input_layout: None,
            depth_stencil: None,
            blend: None,
            rasterizer: None,
            render_target_formats: None,
            view_instancing: None,
            stream_out: None,
            sample_desc: None,
            sample_mask: None,
            ib_strip_cut_value: None,
            primitive_topology: None,
            dsv_format: None,
            node_mask: None,
            flags: None,
        }
    }

    fn empty_file(records: Vec<PipelineStateRecord>) -> RecordFile {
        RecordFile {
            format_version: 1,
            build: BuildIdentity::empty(),
            root_signatures: std::collections::BTreeMap::new(),
            input_layouts: std::collections::BTreeMap::new(),
            stream_outputs: std::collections::BTreeMap::new(),
            records,
        }
    }

    fn assert_vector(name: &str, record: &PipelineStateRecord, preimage: &str, hash_hex: &str) {
        let bytes = pipeline_content_preimage(record);
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            preimage,
            "{name} preimage"
        );
        assert!(!preimage.contains("app_key"), "{name} still has app_key");
        assert!(
            !preimage.contains("\"version\""),
            "{name} still has version"
        );
        let file = empty_file(vec![record.clone()]);
        let hash = pipeline_content_hash(&file, record);
        assert_eq!(hash.to_hex(), hash_hex, "{name} hash");
        let doc = include_str!("../../../docs/RECORD_FORMAT.md");
        assert!(
            doc.contains(preimage),
            "{name} preimage missing from RECORD_FORMAT.md"
        );
        assert!(
            doc.contains(hash_hex),
            "{name} hash missing from RECORD_FORMAT.md"
        );
        assert!(
            doc.contains(&encode_hex(preimage.as_bytes())),
            "{name} preimage hex missing from RECORD_FORMAT.md"
        );
    }

    fn stages_record() -> PipelineStateRecord {
        let mut stages = blank("ab", 3);
        stages.bytecode_hashes.vs = Some(Sha256::parse_hex(&"11".repeat(32)).unwrap());
        stages.bytecode_hashes.ps = Some(Sha256::parse_hex(&"22".repeat(32)).unwrap());
        stages.bytecode_hashes.as_stage = Some(Sha256::parse_hex(&"33".repeat(32)).unwrap());
        stages.root_signature = Some(Sha256::parse_hex(&"aa".repeat(32)).unwrap());
        stages.input_layout = Some(Sha256::parse_hex(&"bb".repeat(32)).unwrap());
        stages.sample_desc = Some(SampleDesc {
            count: Some(1),
            quality: None,
        });
        stages.sample_mask = Some(4_294_967_295);
        stages.flags = Some(0);
        stages
    }

    fn nested_record() -> PipelineStateRecord {
        let mut nested = blank("01", 1);
        nested.unresolved_stages = vec![ShaderStage::Vs, ShaderStage::Ps];
        nested.root_signature_skip = Some(sodb::RootSignatureSkipReason::Invalid);
        nested.depth_stencil = Some(DepthStencilDesc {
            depth_enable: 1,
            depth_write_mask: 1,
            depth_func: 2,
            stencil_enable: 0,
            front_face: Some(DepthStencilOpDesc {
                stencil_fail_op: 1,
                stencil_depth_fail_op: 2,
                stencil_pass_op: 3,
                stencil_func: 4,
                stencil_read_mask: 255,
                stencil_write_mask: 255,
            }),
            back_face: None,
            depth_bounds_test_enable: 0,
        });
        let rt = RenderTargetBlendDesc {
            blend_enable: 0,
            logic_op_enable: 0,
            src_blend: 1,
            dest_blend: 2,
            blend_op: 3,
            src_blend_alpha: 4,
            dest_blend_alpha: 5,
            blend_op_alpha: 6,
            logic_op: 7,
            render_target_write_mask: 15,
        };
        let mut targets = [None, None, None, None, None, None, None, None];
        targets[0] = Some(rt);
        nested.blend = Some(BlendDesc {
            alpha_to_coverage_enable: 0,
            independent_blend_enable: 1,
            render_targets: targets,
        });
        nested
    }

    fn floats_record() -> PipelineStateRecord {
        let mut floats = blank("01", 0);
        floats.rasterizer = Some(RasterizerDesc {
            fill_mode: 3,
            cull_mode: 1,
            front_counter_clockwise: 0,
            depth_bias: 1.5,
            depth_bias_clamp: -0.0,
            slope_scaled_depth_bias: 0.0,
            depth_clip_enable: 1,
            line_rasterization_mode: 0,
            forced_sample_count: 0,
            conservative_raster: 0,
        });
        floats
    }

    #[test]
    fn golden_vectors_match_the_record_format_doc() {
        assert_vector("minimal", &blank("01", 0), MINIMAL_PREIMAGE, MINIMAL_HASH);
        assert_vector("stages", &stages_record(), STAGES_PREIMAGE, STAGES_HASH);
        assert_vector("nested", &nested_record(), NESTED_PREIMAGE, NESTED_HASH);
        assert_vector("floats", &floats_record(), FLOATS_PREIMAGE, FLOATS_HASH);
        let doc = include_str!("../../../docs/RECORD_FORMAT.md");
        assert!(doc.contains(NULL_HASH));
    }

    #[test]
    fn app_key_version_and_descriptor_bodies_do_not_change_the_hash() {
        let left = blank("01", 0);
        let right = blank("ff", 7);
        let file_left = empty_file(vec![left.clone()]);
        let mut file_right = empty_file(vec![right.clone()]);
        let key = Sha256::parse_hex(&"aa".repeat(32)).unwrap();
        file_right
            .root_signatures
            .insert(key, HexBytes::from_bytes(&[0x01, 0x02, 0x03]));
        assert_eq!(
            pipeline_content_hash(&file_left, &left),
            pipeline_content_hash(&file_right, &right)
        );
        assert_eq!(
            pipeline_content_preimage(&left),
            pipeline_content_preimage(&right)
        );
    }

    #[test]
    fn non_finite_float_hashes_the_null_token() {
        let mut record = blank("01", 0);
        record.rasterizer = Some(RasterizerDesc {
            fill_mode: 0,
            cull_mode: 0,
            front_counter_clockwise: 0,
            depth_bias: f64::NAN,
            depth_bias_clamp: 0.0,
            slope_scaled_depth_bias: 0.0,
            depth_clip_enable: 0,
            line_rasterization_mode: 0,
            forced_sample_count: 0,
            conservative_raster: 0,
        });
        let file = empty_file(vec![record.clone()]);
        assert_eq!(pipeline_content_preimage(&record), b"null");
        assert_eq!(pipeline_content_hash(&file, &record).to_hex(), NULL_HASH);
        for bad in [f64::INFINITY, f64::NEG_INFINITY] {
            record.rasterizer.as_mut().unwrap().depth_bias_clamp = bad;
            assert_eq!(pipeline_content_preimage(&record), b"null");
        }
    }
}
