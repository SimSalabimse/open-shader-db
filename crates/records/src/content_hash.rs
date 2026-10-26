// SPDX-License-Identifier: MIT

//! `pipeline_content_hash`: SHA-256 of one pipeline-state record.
//!
//! The preimage is compact, key-sorted JSON of that record with `app_key`
//! and `version` removed. Descriptor references stay content hashes.
//! `docs/RECORD_FORMAT.md` is the byte-for-byte spec.

use crate::file::RecordFile;
use crate::hash::{sha256_bytes, Sha256};
use crate::types::{
    BlendDesc, DepthStencilDesc, DepthStencilOpDesc, PipelineStateRecord, RasterizerDesc,
    RenderTargetBlendDesc, RenderTargetFormats, SampleDesc, StageHashes, ViewInstancingDesc,
};

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
    // Non-finite floats have no JSON number. The preimage is the token `null`.
    if rasterizer_is_non_finite(record) {
        return b"null".to_vec();
    }
    let mut buf = Vec::with_capacity(512);
    write_record(&mut buf, record);
    buf
}

/// Explicit canonical JSON. Key order is the order written here (UTF-8
/// lexicographic), not `serde_json` map order, so the `preserve_order`
/// feature cannot change a byte.
fn write_record(buf: &mut Vec<u8>, record: &PipelineStateRecord) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "blend");
    write_opt(buf, record.blend.as_ref(), write_blend);
    push_key(buf, &mut first, "bytecode_hashes");
    write_stages(buf, &record.bytecode_hashes);
    push_key(buf, &mut first, "depth_stencil");
    write_opt(buf, record.depth_stencil.as_ref(), write_depth_stencil);
    push_key(buf, &mut first, "dsv_format");
    write_opt_i64(buf, record.dsv_format);
    push_key(buf, &mut first, "flags");
    write_opt_i64(buf, record.flags);
    push_key(buf, &mut first, "ib_strip_cut_value");
    write_opt_i64(buf, record.ib_strip_cut_value);
    push_key(buf, &mut first, "input_layout");
    write_opt_hash(buf, record.input_layout.as_ref());
    push_key(buf, &mut first, "node_mask");
    write_opt_i64(buf, record.node_mask);
    push_key(buf, &mut first, "primitive_topology");
    write_opt_i64(buf, record.primitive_topology);
    push_key(buf, &mut first, "rasterizer");
    write_opt(buf, record.rasterizer.as_ref(), write_rasterizer);
    push_key(buf, &mut first, "render_target_formats");
    write_opt(buf, record.render_target_formats.as_ref(), write_rt_formats);
    push_key(buf, &mut first, "root_signature");
    write_opt_hash(buf, record.root_signature.as_ref());
    if let Some(reason) = record.root_signature_skip {
        push_key(buf, &mut first, "root_signature_skip");
        push_string(buf, reason.as_str());
    }
    push_key(buf, &mut first, "sample_desc");
    write_opt(buf, record.sample_desc.as_ref(), write_sample_desc);
    push_key(buf, &mut first, "sample_mask");
    write_opt_i64(buf, record.sample_mask);
    push_key(buf, &mut first, "stream_out");
    write_opt_hash(buf, record.stream_out.as_ref());
    if !record.unresolved_stages.is_empty() {
        push_key(buf, &mut first, "unresolved_stages");
        buf.push(b'[');
        for (i, stage) in record.unresolved_stages.iter().enumerate() {
            if i > 0 {
                buf.push(b',');
            }
            push_string(buf, stage.as_str());
        }
        buf.push(b']');
    }
    push_key(buf, &mut first, "view_instancing");
    write_opt(buf, record.view_instancing.as_ref(), write_view_instancing);
    buf.push(b'}');
}

fn write_opt<T>(buf: &mut Vec<u8>, value: Option<&T>, write: fn(&mut Vec<u8>, &T)) {
    match value {
        None => buf.extend_from_slice(b"null"),
        Some(value) => write(buf, value),
    }
}

fn write_opt_i64(buf: &mut Vec<u8>, value: Option<i64>) {
    match value {
        None => buf.extend_from_slice(b"null"),
        Some(value) => push_i64(buf, value),
    }
}

fn write_opt_hash(buf: &mut Vec<u8>, value: Option<&Sha256>) {
    match value {
        None => buf.extend_from_slice(b"null"),
        Some(value) => push_string(buf, &value.to_hex()),
    }
}

fn write_stages(buf: &mut Vec<u8>, stages: &StageHashes) {
    buf.push(b'{');
    let mut first = true;
    // Stage names in UTF-8 order, not spec order (`as` before `cs` … `vs`).
    if let Some(hash) = &stages.as_stage {
        push_hash_field(buf, &mut first, "as", hash);
    }
    if let Some(hash) = &stages.cs {
        push_hash_field(buf, &mut first, "cs", hash);
    }
    if let Some(hash) = &stages.ds {
        push_hash_field(buf, &mut first, "ds", hash);
    }
    if let Some(hash) = &stages.gs {
        push_hash_field(buf, &mut first, "gs", hash);
    }
    if let Some(hash) = &stages.hs {
        push_hash_field(buf, &mut first, "hs", hash);
    }
    if let Some(hash) = &stages.ms {
        push_hash_field(buf, &mut first, "ms", hash);
    }
    if let Some(hash) = &stages.ps {
        push_hash_field(buf, &mut first, "ps", hash);
    }
    if let Some(hash) = &stages.vs {
        push_hash_field(buf, &mut first, "vs", hash);
    }
    buf.push(b'}');
}

fn push_hash_field(buf: &mut Vec<u8>, first: &mut bool, key: &str, hash: &Sha256) {
    push_key(buf, first, key);
    push_string(buf, &hash.to_hex());
}

fn write_blend(buf: &mut Vec<u8>, blend: &BlendDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "alpha_to_coverage_enable");
    push_i64(buf, blend.alpha_to_coverage_enable);
    push_key(buf, &mut first, "independent_blend_enable");
    push_i64(buf, blend.independent_blend_enable);
    push_key(buf, &mut first, "render_targets");
    buf.push(b'[');
    for (i, target) in blend.render_targets.iter().enumerate() {
        if i > 0 {
            buf.push(b',');
        }
        write_opt(buf, target.as_ref(), write_rt_blend);
    }
    buf.push(b']');
    buf.push(b'}');
}

fn write_rt_blend(buf: &mut Vec<u8>, rt: &RenderTargetBlendDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "blend_enable");
    push_i64(buf, rt.blend_enable);
    push_key(buf, &mut first, "blend_op");
    push_i64(buf, rt.blend_op);
    push_key(buf, &mut first, "blend_op_alpha");
    push_i64(buf, rt.blend_op_alpha);
    push_key(buf, &mut first, "dest_blend");
    push_i64(buf, rt.dest_blend);
    push_key(buf, &mut first, "dest_blend_alpha");
    push_i64(buf, rt.dest_blend_alpha);
    push_key(buf, &mut first, "logic_op");
    push_i64(buf, rt.logic_op);
    push_key(buf, &mut first, "logic_op_enable");
    push_i64(buf, rt.logic_op_enable);
    push_key(buf, &mut first, "render_target_write_mask");
    push_i64(buf, rt.render_target_write_mask);
    push_key(buf, &mut first, "src_blend");
    push_i64(buf, rt.src_blend);
    push_key(buf, &mut first, "src_blend_alpha");
    push_i64(buf, rt.src_blend_alpha);
    buf.push(b'}');
}

fn write_depth_stencil(buf: &mut Vec<u8>, desc: &DepthStencilDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "back_face");
    write_opt(buf, desc.back_face.as_ref(), write_stencil_op);
    push_key(buf, &mut first, "depth_bounds_test_enable");
    push_i64(buf, desc.depth_bounds_test_enable);
    push_key(buf, &mut first, "depth_enable");
    push_i64(buf, desc.depth_enable);
    push_key(buf, &mut first, "depth_func");
    push_i64(buf, desc.depth_func);
    push_key(buf, &mut first, "depth_write_mask");
    push_i64(buf, desc.depth_write_mask);
    push_key(buf, &mut first, "front_face");
    write_opt(buf, desc.front_face.as_ref(), write_stencil_op);
    push_key(buf, &mut first, "stencil_enable");
    push_i64(buf, desc.stencil_enable);
    buf.push(b'}');
}

fn write_stencil_op(buf: &mut Vec<u8>, op: &DepthStencilOpDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "stencil_depth_fail_op");
    push_i64(buf, op.stencil_depth_fail_op);
    push_key(buf, &mut first, "stencil_fail_op");
    push_i64(buf, op.stencil_fail_op);
    push_key(buf, &mut first, "stencil_func");
    push_i64(buf, op.stencil_func);
    push_key(buf, &mut first, "stencil_pass_op");
    push_i64(buf, op.stencil_pass_op);
    push_key(buf, &mut first, "stencil_read_mask");
    push_i64(buf, op.stencil_read_mask);
    push_key(buf, &mut first, "stencil_write_mask");
    push_i64(buf, op.stencil_write_mask);
    buf.push(b'}');
}

fn write_rasterizer(buf: &mut Vec<u8>, desc: &RasterizerDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "conservative_raster");
    push_i64(buf, desc.conservative_raster);
    push_key(buf, &mut first, "cull_mode");
    push_i64(buf, desc.cull_mode);
    push_key(buf, &mut first, "depth_bias");
    push_f64(buf, desc.depth_bias);
    push_key(buf, &mut first, "depth_bias_clamp");
    push_f64(buf, desc.depth_bias_clamp);
    push_key(buf, &mut first, "depth_clip_enable");
    push_i64(buf, desc.depth_clip_enable);
    push_key(buf, &mut first, "fill_mode");
    push_i64(buf, desc.fill_mode);
    push_key(buf, &mut first, "forced_sample_count");
    push_i64(buf, desc.forced_sample_count);
    push_key(buf, &mut first, "front_counter_clockwise");
    push_i64(buf, desc.front_counter_clockwise);
    push_key(buf, &mut first, "line_rasterization_mode");
    push_i64(buf, desc.line_rasterization_mode);
    push_key(buf, &mut first, "slope_scaled_depth_bias");
    push_f64(buf, desc.slope_scaled_depth_bias);
    buf.push(b'}');
}

fn write_rt_formats(buf: &mut Vec<u8>, formats: &RenderTargetFormats) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "num_render_targets");
    push_i64(buf, formats.num_render_targets);
    push_key(buf, &mut first, "rt_formats");
    buf.push(b'[');
    for (i, format) in formats.rt_formats.iter().enumerate() {
        if i > 0 {
            buf.push(b',');
        }
        push_i64(buf, *format);
    }
    buf.push(b']');
    buf.push(b'}');
}

fn write_sample_desc(buf: &mut Vec<u8>, desc: &SampleDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "count");
    write_opt_i64(buf, desc.count);
    push_key(buf, &mut first, "quality");
    write_opt_i64(buf, desc.quality);
    buf.push(b'}');
}

fn write_view_instancing(buf: &mut Vec<u8>, desc: &ViewInstancingDesc) {
    buf.push(b'{');
    let mut first = true;
    push_key(buf, &mut first, "render_flags");
    push_i64(buf, desc.render_flags);
    push_key(buf, &mut first, "render_target_array_index");
    write_opt_i64_array(buf, &desc.render_target_array_index);
    push_key(buf, &mut first, "view_instance_count");
    push_i64(buf, desc.view_instance_count);
    push_key(buf, &mut first, "viewport_array_index");
    write_opt_i64_array(buf, &desc.viewport_array_index);
    buf.push(b'}');
}

fn write_opt_i64_array(buf: &mut Vec<u8>, values: &[Option<i64>]) {
    buf.push(b'[');
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            buf.push(b',');
        }
        write_opt_i64(buf, *value);
    }
    buf.push(b']');
}

fn push_key(buf: &mut Vec<u8>, first: &mut bool, key: &str) {
    if *first {
        *first = false;
    } else {
        buf.push(b',');
    }
    push_string(buf, key);
    buf.push(b':');
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

fn push_string(buf: &mut Vec<u8>, text: &str) {
    buf.push(b'"');
    for &byte in text.as_bytes() {
        match byte {
            b'"' => buf.extend_from_slice(br#"\""#),
            b'\\' => buf.extend_from_slice(br#"\\"#),
            0x00..=0x1f => {
                buf.extend_from_slice(b"\\u00");
                buf.push(HEX_DIGITS[usize::from(byte >> 4)]);
                buf.push(HEX_DIGITS[usize::from(byte & 0x0f)]);
            }
            _ => buf.push(byte),
        }
    }
    buf.push(b'"');
}

fn push_i64(buf: &mut Vec<u8>, value: i64) {
    if value == i64::MIN {
        buf.extend_from_slice(b"-9223372036854775808");
        return;
    }
    let neg = value < 0;
    let mut n = value.unsigned_abs();
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + u8::try_from(n % 10).unwrap_or(0);
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if neg {
        buf.push(b'-');
    }
    buf.extend_from_slice(&tmp[i..]);
}

fn push_f64(buf: &mut Vec<u8>, value: f64) {
    let mut tmp = [0u8; 48];
    let n = format_finite(value, &mut tmp);
    buf.extend_from_slice(&tmp[..n]);
}

/// Shortest correctly-rounded spelling of a finite `f64`.
///
/// See `docs/RECORD_FORMAT.md`. `out` must hold at least 48 bytes.
/// Returns the length written.
fn format_finite(value: f64, out: &mut [u8]) -> usize {
    debug_assert!(value.is_finite());
    if value == 0.0 {
        let token: &[u8] = if value.is_sign_negative() {
            b"-0.0"
        } else {
            b"0.0"
        };
        out[..token.len()].copy_from_slice(token);
        return token.len();
    }
    let negative = value.is_sign_negative();
    let (significand, power) = significand_power(value.abs().to_bits());
    let (digits, k) = decimal_digits(significand, power);
    let max_len = digits.len().min(17);
    let mut rounded = [0u8; 17];
    for len in 1..=max_len {
        let (n, rk) = round_digits(&digits, k, len, &mut rounded);
        let written = spell_digits(negative, &rounded[..n], rk, out);
        let token = std::str::from_utf8(&out[..written]).unwrap_or("");
        if let Ok(parsed) = token.parse::<f64>() {
            if parsed.to_bits() == value.to_bits() {
                return written;
            }
        }
    }
    let (n, rk) = round_digits(&digits, k, max_len, &mut rounded);
    spell_digits(negative, &rounded[..n], rk, out)
}

fn significand_power(bits: u64) -> (u64, i32) {
    let exp = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let frac = bits & ((1_u64 << 52) - 1);
    if exp == 0 {
        (frac, -1074)
    } else {
        (frac | (1_u64 << 52), exp - 1023 - 52)
    }
}

/// Exact significant digits (most significant first, no trailing zero)
/// and the scientific exponent `k` in `d0.d1… × 10^k`.
fn decimal_digits(significand: u64, power: i32) -> (Vec<u8>, i32) {
    let mut magnitude = Big::from_u64(significand);
    if power >= 0 {
        magnitude.shl_bits(u32::try_from(power).unwrap_or(0));
    } else {
        let times = u32::try_from(-power).unwrap_or(0);
        for _ in 0..times {
            magnitude.mul_u32(5);
        }
    }
    let mut least_first = Vec::new();
    while !magnitude.is_zero() {
        least_first.push(magnitude.divmod_10());
    }
    if least_first.is_empty() {
        least_first.push(0);
    }
    let full_len = i32::try_from(least_first.len()).unwrap_or(i32::MAX);
    let mut trailing = 0;
    while trailing + 1 < least_first.len() && least_first[trailing] == 0 {
        trailing += 1;
    }
    least_first.drain(0..trailing);
    least_first.reverse();
    let k = if power >= 0 {
        full_len - 1
    } else {
        full_len - 1 + power
    };
    (least_first, k)
}

fn round_digits(digits: &[u8], k: i32, len: usize, out: &mut [u8]) -> (usize, i32) {
    let len = len.min(digits.len());
    out[..len].copy_from_slice(&digits[..len]);
    if len == digits.len() || !should_round_up(digits, len) {
        return (len, k);
    }
    let mut i = len;
    while i > 0 {
        i -= 1;
        if out[i] < 9 {
            out[i] += 1;
            return (len, k);
        }
        out[i] = 0;
    }
    out[0] = 1;
    for slot in &mut out[1..len] {
        *slot = 0;
    }
    (len, k.saturating_add(1))
}

fn should_round_up(digits: &[u8], len: usize) -> bool {
    let Some(next) = digits.get(len) else {
        return false;
    };
    if *next < 5 {
        return false;
    }
    if *next > 5 {
        return true;
    }
    if digits[len + 1..].iter().any(|digit| *digit != 0) {
        return true;
    }
    digits[len - 1] % 2 == 1
}

/// Plain decimal when `-4 <= k <= 15`, otherwise `d[.d]e±k` with an
/// explicit exponent sign and no leading zeros. Integer magnitudes in
/// plain form always keep a `.0`.
fn spell_digits(negative: bool, digits: &[u8], k: i32, out: &mut [u8]) -> usize {
    let mut n = 0;
    let push = |out: &mut [u8], n: &mut usize, byte: u8| {
        out[*n] = byte;
        *n += 1;
    };
    if negative {
        push(out, &mut n, b'-');
    }
    let len = digits.len();
    if (-4..=15).contains(&k) {
        if k >= 0 {
            let k_us = usize::try_from(k).unwrap_or(0);
            if k_us >= len - 1 {
                for &digit in digits {
                    push(out, &mut n, b'0' + digit);
                }
                for _ in 0..(k_us - (len - 1)) {
                    push(out, &mut n, b'0');
                }
                push(out, &mut n, b'.');
                push(out, &mut n, b'0');
            } else {
                for &digit in &digits[..=k_us] {
                    push(out, &mut n, b'0' + digit);
                }
                push(out, &mut n, b'.');
                for &digit in &digits[k_us + 1..] {
                    push(out, &mut n, b'0' + digit);
                }
            }
        } else {
            push(out, &mut n, b'0');
            push(out, &mut n, b'.');
            let zeros = usize::try_from(-k - 1).unwrap_or(0);
            for _ in 0..zeros {
                push(out, &mut n, b'0');
            }
            for &digit in digits {
                push(out, &mut n, b'0' + digit);
            }
        }
    } else {
        push(out, &mut n, b'0' + digits[0]);
        if len > 1 {
            push(out, &mut n, b'.');
            for &digit in &digits[1..] {
                push(out, &mut n, b'0' + digit);
            }
        }
        push(out, &mut n, b'e');
        if k >= 0 {
            push(out, &mut n, b'+');
        } else {
            push(out, &mut n, b'-');
        }
        push_u32_digits(out, &mut n, k.unsigned_abs());
    }
    n
}

fn push_u32_digits(out: &mut [u8], n: &mut usize, mut value: u32) {
    let mut tmp = [0u8; 10];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for &byte in &tmp[i..] {
        out[*n] = byte;
        *n += 1;
    }
}

/// Little-endian base-2^32 integer.
struct Big {
    limbs: Vec<u32>,
}

impl Big {
    fn from_u64(value: u64) -> Self {
        let lo = u32::try_from(value & 0xffff_ffff).unwrap_or(0);
        let hi = u32::try_from(value >> 32).unwrap_or(0);
        if hi == 0 {
            if lo == 0 {
                Self { limbs: Vec::new() }
            } else {
                Self { limbs: vec![lo] }
            }
        } else {
            Self {
                limbs: vec![lo, hi],
            }
        }
    }

    fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    fn shl_bits(&mut self, bits: u32) {
        if self.limbs.is_empty() || bits == 0 {
            return;
        }
        let limb_shift = usize::try_from(bits / 32).unwrap_or(0);
        let bit_shift = bits % 32;
        let mut next = vec![0_u32; self.limbs.len() + limb_shift + 1];
        if bit_shift == 0 {
            next[limb_shift..limb_shift + self.limbs.len()].copy_from_slice(&self.limbs);
        } else {
            let mut carry = 0_u32;
            for (i, &limb) in self.limbs.iter().enumerate() {
                let wide = (u64::from(limb) << bit_shift) | u64::from(carry);
                next[i + limb_shift] = u32::try_from(wide & 0xffff_ffff).unwrap_or(0);
                carry = u32::try_from(wide >> 32).unwrap_or(0);
            }
            if carry > 0 {
                next[self.limbs.len() + limb_shift] = carry;
            }
        }
        while next.last() == Some(&0) {
            next.pop();
        }
        self.limbs = next;
    }

    fn mul_u32(&mut self, factor: u32) {
        if factor == 0 || self.limbs.is_empty() {
            self.limbs.clear();
            return;
        }
        let mut carry = 0_u64;
        for limb in &mut self.limbs {
            let prod = u64::from(*limb) * u64::from(factor) + carry;
            *limb = u32::try_from(prod & 0xffff_ffff).unwrap_or(0);
            carry = prod >> 32;
        }
        if carry > 0 {
            self.limbs.push(u32::try_from(carry).unwrap_or(0));
        }
    }

    fn divmod_10(&mut self) -> u8 {
        let mut rem = 0_u64;
        for limb in self.limbs.iter_mut().rev() {
            let cur = (rem << 32) | u64::from(*limb);
            *limb = u32::try_from(cur / 10).unwrap_or(0);
            rem = cur % 10;
        }
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
        u8::try_from(rem).unwrap_or(0)
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
    const EXPONENTS_PREIMAGE: &str = r#"{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":{"conservative_raster":0,"cull_mode":0,"depth_bias":1e+16,"depth_bias_clamp":1e-7,"depth_clip_enable":0,"fill_mode":0,"forced_sample_count":0,"front_counter_clockwise":0,"line_rasterization_mode":0,"slope_scaled_depth_bias":-0.0},"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}"#;
    const EXPONENTS_HASH: &str = "1cf3ea8044bef955acd52a0f9ed19eabf6a4a1f9742ad3b9e023be859e29e16d";
    const WIDENED_PREIMAGE: &str = r#"{"blend":null,"bytecode_hashes":{},"depth_stencil":null,"dsv_format":null,"flags":null,"ib_strip_cut_value":null,"input_layout":null,"node_mask":null,"primitive_topology":null,"rasterizer":{"conservative_raster":0,"cull_mode":0,"depth_bias":4.869141201346849e-10,"depth_bias_clamp":4.869141201346848e-10,"depth_clip_enable":0,"fill_mode":0,"forced_sample_count":0,"front_counter_clockwise":0,"line_rasterization_mode":0,"slope_scaled_depth_bias":0.0},"render_target_formats":null,"root_signature":null,"sample_desc":null,"sample_mask":null,"stream_out":null,"view_instancing":null}"#;
    const WIDENED_HASH: &str = "762d76dbfc66de83ea604a198a728bdd8f1b5c711abd98190c55c739cd486332";
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
        assert_vector(
            "exponents",
            &zero_raster(1e16, 1e-7, -0.0),
            EXPONENTS_PREIMAGE,
            EXPONENTS_HASH,
        );
        let exact = f64::from(f32::from_bits(0x3005_d789));
        let writer = serde_json::to_string(&exact).unwrap();
        assert_eq!(writer, "4.869141201346849e-10");
        let loaded: f64 = serde_json::from_str(&writer).unwrap();
        assert_eq!(exact.to_bits(), 0x3e00_baf1_2000_0000);
        assert_eq!(loaded.to_bits(), 0x3e00_baf1_1fff_ffff);
        assert_vector(
            "widened",
            &zero_raster(exact, loaded, 0.0),
            WIDENED_PREIMAGE,
            WIDENED_HASH,
        );
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

    fn spell(value: f64) -> String {
        let mut buf = [0u8; 48];
        let n = super::format_finite(value, &mut buf);
        String::from_utf8(buf[..n].to_vec()).unwrap()
    }

    #[test]
    fn finite_float_spelling_is_explicit() {
        assert_eq!(spell(0.0), "0.0");
        assert_eq!(spell(-0.0), "-0.0");
        assert_eq!(spell(1.5), "1.5");
        assert_eq!(spell(1.0), "1.0");
        assert_eq!(spell(10.0), "10.0");
        assert_eq!(spell(1e15), "1000000000000000.0");
        assert_eq!(spell(1e16), "1e+16");
        assert_eq!(spell(1e-4), "0.0001");
        assert_eq!(spell(1e-5), "1e-5");
        assert_eq!(spell(1e-7), "1e-7");
        assert_eq!(spell(-1.5), "-1.5");
        for value in [f64::MIN_POSITIVE, f64::MAX, f64::from_bits(1), 0.1, -0.1] {
            let text = spell(value);
            let parsed: f64 = text.parse().unwrap();
            assert_eq!(parsed.to_bits(), value.to_bits(), "{text}");
        }
        let mut state = 0x1234_5678_9abc_def0_u64;
        for _ in 0..2_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let value = f64::from_bits(state);
            if !value.is_finite() {
                continue;
            }
            let text = spell(value);
            let parsed: f64 = text.parse().unwrap();
            assert_eq!(parsed.to_bits(), value.to_bits(), "{value:e} -> {text}");
        }
    }

    #[test]
    fn preimage_keys_are_lexicographic_without_map_order() {
        let mut record = stages_record();
        record.bytecode_hashes.cs = Some(Sha256::parse_hex(&"44".repeat(32)).unwrap());
        record.bytecode_hashes.hs = Some(Sha256::parse_hex(&"55".repeat(32)).unwrap());
        let bytes = pipeline_content_preimage(&record);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.starts_with("{\"blend\":"),
            "struct field order would not start with blend: {text}"
        );
        assert_sorted_object_keys(&bytes);
        assert!(text.contains("\"as\":"));
        let as_at = text.find("\"as\":").unwrap();
        let cs_at = text.find("\"cs\":").unwrap();
        let hs_at = text.find("\"hs\":").unwrap();
        let ps_at = text.find("\"ps\":").unwrap();
        let vs_at = text.find("\"vs\":").unwrap();
        assert!(as_at < cs_at && cs_at < hs_at && hs_at < ps_at && ps_at < vs_at);
    }

    fn assert_sorted_object_keys(bytes: &[u8]) {
        fn read_string(bytes: &[u8], i: &mut usize) -> Vec<u8> {
            assert_eq!(bytes[*i], b'"');
            *i += 1;
            let start = *i;
            while bytes[*i] != b'"' {
                if bytes[*i] == b'\\' {
                    *i += 2;
                } else {
                    *i += 1;
                }
            }
            let body = bytes[start..*i].to_vec();
            *i += 1;
            body
        }
        fn scan_value(bytes: &[u8], i: &mut usize) {
            match bytes[*i] {
                b'{' => {
                    *i += 1;
                    let mut prev: Option<Vec<u8>> = None;
                    loop {
                        if bytes.get(*i) == Some(&b'}') {
                            *i += 1;
                            break;
                        }
                        let key = read_string(bytes, i);
                        if let Some(prev) = &prev {
                            assert!(prev.as_slice() < key.as_slice(), "unsorted key");
                        }
                        prev = Some(key);
                        assert_eq!(bytes[*i], b':');
                        *i += 1;
                        scan_value(bytes, i);
                        if bytes.get(*i) == Some(&b',') {
                            *i += 1;
                            continue;
                        }
                        assert_eq!(bytes[*i], b'}');
                    }
                }
                b'[' => {
                    *i += 1;
                    loop {
                        if bytes.get(*i) == Some(&b']') {
                            *i += 1;
                            break;
                        }
                        scan_value(bytes, i);
                        if bytes.get(*i) == Some(&b',') {
                            *i += 1;
                            continue;
                        }
                        assert_eq!(bytes[*i], b']');
                    }
                }
                b'"' => {
                    let _ = read_string(bytes, i);
                }
                b't' | b'f' | b'n' => {
                    while *i < bytes.len() && bytes[*i].is_ascii_alphabetic() {
                        *i += 1;
                    }
                }
                b'-' | b'0'..=b'9' => {
                    while *i < bytes.len()
                        && (bytes[*i].is_ascii_digit()
                            || matches!(bytes[*i], b'-' | b'+' | b'.' | b'e' | b'E'))
                    {
                        *i += 1;
                    }
                }
                other => panic!("bad json byte {other}"),
            }
        }
        let mut i = 0;
        scan_value(bytes, &mut i);
        assert_eq!(i, bytes.len());
    }

    #[test]
    fn string_escaping_matches_the_documented_rule() {
        let mut buf = Vec::new();
        super::push_string(&mut buf, "a\"b\\c/d\n\u{0001}");
        assert_eq!(
            std::str::from_utf8(&buf).unwrap(),
            "\"a\\\"b\\\\c/d\\u000a\\u0001\""
        );
    }

    fn zero_raster(bias: f64, clamp: f64, slope: f64) -> PipelineStateRecord {
        let mut record = blank("01", 0);
        record.rasterizer = Some(RasterizerDesc {
            fill_mode: 0,
            cull_mode: 0,
            front_counter_clockwise: 0,
            depth_bias: bias,
            depth_bias_clamp: clamp,
            slope_scaled_depth_bias: slope,
            depth_clip_enable: 0,
            line_rasterization_mode: 0,
            forced_sample_count: 0,
            conservative_raster: 0,
        });
        record
    }
}
