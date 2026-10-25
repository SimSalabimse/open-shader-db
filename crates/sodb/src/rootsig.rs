// SPDX-License-Identifier: MIT

//! Structural D3D12 root-signature parse and canonical re-serialization.

use crate::Error;

/// Maximum root parameters in one signature.
pub const MAX_PARAMETERS: usize = 4096;
/// Maximum descriptor ranges across the signature.
pub const MAX_RANGES: usize = 4096;
/// Maximum static samplers.
pub const MAX_SAMPLERS: usize = 2048;

const HEADER_LEN: usize = 24;
const PARAM_LEN: usize = 20;
const RANGE_1_0: usize = 20;
const RANGE_1_1: usize = 24;
const SAMPLER_LEN: usize = 52;

const VERSION_1_0: u32 = 1;
const VERSION_1_1: u32 = 2;

const PARAM_TABLE: u32 = 0;
const PARAM_CONSTANTS: u32 = 1;
const PARAM_CBV: u32 = 2;
const PARAM_SRV: u32 = 3;
const PARAM_UAV: u32 = 4;

const VIS_MAX: u32 = 7;
const RANGE_TYPE_MAX: u32 = 3;
const ADDR_MIN: u32 = 1;
const ADDR_MAX: u32 = 5;
const CMP_MAX: u32 = 8;
const BORDER_MAX: u32 = 3;

const FLAGS_KNOWN: u32 = 0x0FFF;
const ROOT_DESC_FLAGS: u32 = 0x000E;
const RANGE_FLAGS: u32 = 0x1_000F;
const OFFSET_APPEND: u32 = 0xFFFF_FFFF;

const FILTERS: &[u32] = &[
    0x00, 0x01, 0x04, 0x05, 0x10, 0x11, 0x14, 0x15, 0x55, 0x80, 0x81, 0x84, 0x85, 0x90, 0x91, 0x94,
    0x95, 0xD5, 0x100, 0x101, 0x104, 0x105, 0x110, 0x111, 0x114, 0x115, 0x155, 0x180, 0x181, 0x184,
    0x185, 0x190, 0x191, 0x194, 0x195, 0x1D5,
];

/// Parsed root signature. [`Self::serialize`] emits the only accepted on-disk form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRootSignature {
    version: u32,
    flags: u32,
    parameters: Vec<Parameter>,
    samplers: Vec<StaticSampler>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Parameter {
    Table {
        visibility: u32,
        ranges: Vec<DescriptorRange>,
    },
    Constants {
        visibility: u32,
        shader_register: u32,
        register_space: u32,
        num_32bit: u32,
    },
    Descriptor {
        kind: u32,
        visibility: u32,
        shader_register: u32,
        register_space: u32,
        flags: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DescriptorRange {
    range_type: u32,
    num_descriptors: u32,
    base_shader_register: u32,
    register_space: u32,
    flags: u32,
    offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaticSampler {
    filter: u32,
    address_u: u32,
    address_v: u32,
    address_w: u32,
    mip_lod_bias: u32,
    max_anisotropy: u32,
    comparison_func: u32,
    border_color: u32,
    min_lod: u32,
    max_lod: u32,
    shader_register: u32,
    register_space: u32,
    visibility: u32,
}

impl CanonicalRootSignature {
    /// Canonical little-endian serialization (tight header / params / ranges / samplers).
    pub fn serialize(&self) -> Vec<u8> {
        let v11 = self.version == VERSION_1_1;
        let range_len = if v11 { RANGE_1_1 } else { RANGE_1_0 };
        let params_size = self.parameters.len().saturating_mul(PARAM_LEN);
        let mut ranges_size: usize = 0;
        for p in &self.parameters {
            if let Parameter::Table { ranges, .. } = p {
                ranges_size = ranges_size.saturating_add(ranges.len().saturating_mul(range_len));
            }
        }
        let samplers_size = self.samplers.len().saturating_mul(SAMPLER_LEN);
        let mut out = Vec::with_capacity(HEADER_LEN + params_size + ranges_size + samplers_size);
        let params_off = if self.parameters.is_empty() {
            0
        } else {
            HEADER_LEN as u32
        };
        let mut cursor = HEADER_LEN + params_size;
        let mut range_offs = Vec::with_capacity(self.parameters.len());
        for p in &self.parameters {
            if let Parameter::Table { ranges, .. } = p {
                if ranges.is_empty() {
                    range_offs.push(0_u32);
                } else {
                    range_offs.push(cursor as u32);
                    cursor += ranges.len() * range_len;
                }
            } else {
                range_offs.push(0);
            }
        }
        let samplers_off = if self.samplers.is_empty() {
            0
        } else {
            cursor as u32
        };
        push_u32(&mut out, self.version);
        push_u32(&mut out, self.parameters.len() as u32);
        push_u32(&mut out, params_off);
        push_u32(&mut out, self.samplers.len() as u32);
        push_u32(&mut out, samplers_off);
        push_u32(&mut out, self.flags);
        for (p, range_off) in self.parameters.iter().zip(range_offs.iter().copied()) {
            match p {
                Parameter::Table { visibility, ranges } => {
                    push_u32(&mut out, PARAM_TABLE);
                    push_u32(&mut out, ranges.len() as u32);
                    push_u32(&mut out, range_off);
                    push_u32(&mut out, 0);
                    push_u32(&mut out, *visibility);
                }
                Parameter::Constants {
                    visibility,
                    shader_register,
                    register_space,
                    num_32bit,
                } => {
                    push_u32(&mut out, PARAM_CONSTANTS);
                    push_u32(&mut out, *shader_register);
                    push_u32(&mut out, *register_space);
                    push_u32(&mut out, *num_32bit);
                    push_u32(&mut out, *visibility);
                }
                Parameter::Descriptor {
                    kind,
                    visibility,
                    shader_register,
                    register_space,
                    flags,
                } => {
                    push_u32(&mut out, *kind);
                    push_u32(&mut out, *shader_register);
                    push_u32(&mut out, *register_space);
                    push_u32(&mut out, *flags);
                    push_u32(&mut out, *visibility);
                }
            }
        }
        for p in &self.parameters {
            if let Parameter::Table { ranges, .. } = p {
                for r in ranges {
                    push_u32(&mut out, r.range_type);
                    push_u32(&mut out, r.num_descriptors);
                    push_u32(&mut out, r.base_shader_register);
                    push_u32(&mut out, r.register_space);
                    if v11 {
                        push_u32(&mut out, r.flags);
                    }
                    push_u32(&mut out, r.offset);
                }
            }
        }
        for s in &self.samplers {
            push_u32(&mut out, s.filter);
            push_u32(&mut out, s.address_u);
            push_u32(&mut out, s.address_v);
            push_u32(&mut out, s.address_w);
            push_u32(&mut out, s.mip_lod_bias);
            push_u32(&mut out, s.max_anisotropy);
            push_u32(&mut out, s.comparison_func);
            push_u32(&mut out, s.border_color);
            push_u32(&mut out, s.min_lod);
            push_u32(&mut out, s.max_lod);
            push_u32(&mut out, s.shader_register);
            push_u32(&mut out, s.register_space);
            push_u32(&mut out, s.visibility);
        }
        out
    }
}

/// Parse a serialized D3D12 root signature (version 1.0 or 1.1) and rebuild it.
pub fn parse_root_signature(bytes: &[u8]) -> Result<CanonicalRootSignature, Error> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::RootSignatureMalformed);
    }
    let mut cover = Coverage::new();
    cover.mark(0, HEADER_LEN)?;
    let version = u32_at(bytes, 0)?;
    if version != VERSION_1_0 && version != VERSION_1_1 {
        return Err(Error::RootSignatureBadVersion);
    }
    let v11 = version == VERSION_1_1;
    let num_params = u32_at(bytes, 4)? as usize;
    let params_off = u32_at(bytes, 8)? as usize;
    let num_samplers = u32_at(bytes, 12)? as usize;
    let samplers_off = u32_at(bytes, 16)? as usize;
    let flags = u32_at(bytes, 20)?;
    if flags & !FLAGS_KNOWN != 0 {
        return Err(Error::RootSignatureMalformed);
    }
    if num_params > MAX_PARAMETERS || num_samplers > MAX_SAMPLERS {
        return Err(Error::RootSignatureMalformed);
    }
    if num_params == 0 {
        if params_off != 0 {
            return Err(Error::RootSignatureMalformed);
        }
    } else {
        let end = params_off
            .checked_add(
                num_params
                    .checked_mul(PARAM_LEN)
                    .ok_or(Error::RootSignatureMalformed)?,
            )
            .ok_or(Error::RootSignatureMalformed)?;
        cover.mark(params_off, end)?;
    }
    if num_samplers == 0 {
        if samplers_off != 0 {
            return Err(Error::RootSignatureMalformed);
        }
    } else {
        let end = samplers_off
            .checked_add(
                num_samplers
                    .checked_mul(SAMPLER_LEN)
                    .ok_or(Error::RootSignatureMalformed)?,
            )
            .ok_or(Error::RootSignatureMalformed)?;
        cover.mark(samplers_off, end)?;
    }

    let mut parameters = Vec::with_capacity(num_params);
    let mut total_ranges = 0_usize;
    let range_len = if v11 { RANGE_1_1 } else { RANGE_1_0 };
    for i in 0..num_params {
        let base = params_off + i * PARAM_LEN;
        let kind = u32_at(bytes, base)?;
        let a = u32_at(bytes, base + 4)?;
        let b = u32_at(bytes, base + 8)?;
        let c = u32_at(bytes, base + 12)?;
        let visibility = u32_at(bytes, base + 16)?;
        if visibility > VIS_MAX {
            return Err(Error::RootSignatureMalformed);
        }
        let param = match kind {
            PARAM_TABLE => {
                let num_ranges = a as usize;
                let ranges_off = b as usize;
                if c != 0 {
                    return Err(Error::RootSignatureMalformed);
                }
                if num_ranges > MAX_RANGES {
                    return Err(Error::RootSignatureMalformed);
                }
                total_ranges = total_ranges
                    .checked_add(num_ranges)
                    .ok_or(Error::RootSignatureMalformed)?;
                if total_ranges > MAX_RANGES {
                    return Err(Error::RootSignatureMalformed);
                }
                let ranges = if num_ranges == 0 {
                    if ranges_off != 0 {
                        return Err(Error::RootSignatureMalformed);
                    }
                    Vec::new()
                } else {
                    let end = ranges_off
                        .checked_add(
                            num_ranges
                                .checked_mul(range_len)
                                .ok_or(Error::RootSignatureMalformed)?,
                        )
                        .ok_or(Error::RootSignatureMalformed)?;
                    cover.mark(ranges_off, end)?;
                    let mut ranges = Vec::with_capacity(num_ranges);
                    for j in 0..num_ranges {
                        let ro = ranges_off + j * range_len;
                        ranges.push(parse_range(bytes, ro, v11)?);
                    }
                    ranges
                };
                Parameter::Table { visibility, ranges }
            }
            PARAM_CONSTANTS => Parameter::Constants {
                visibility,
                shader_register: a,
                register_space: b,
                num_32bit: c,
            },
            PARAM_CBV | PARAM_SRV | PARAM_UAV => {
                if !v11 && c != 0 {
                    return Err(Error::RootSignatureMalformed);
                }
                if v11 && c & !ROOT_DESC_FLAGS != 0 {
                    return Err(Error::RootSignatureMalformed);
                }
                Parameter::Descriptor {
                    kind,
                    visibility,
                    shader_register: a,
                    register_space: b,
                    flags: c,
                }
            }
            _ => return Err(Error::RootSignatureMalformed),
        };
        parameters.push(param);
    }

    let mut samplers = Vec::with_capacity(num_samplers);
    for i in 0..num_samplers {
        samplers.push(parse_sampler(bytes, samplers_off + i * SAMPLER_LEN)?);
    }
    cover.finish(bytes.len())?;
    Ok(CanonicalRootSignature {
        version,
        flags,
        parameters,
        samplers,
    })
}

fn parse_range(bytes: &[u8], off: usize, v11: bool) -> Result<DescriptorRange, Error> {
    let range_type = u32_at(bytes, off)?;
    if range_type > RANGE_TYPE_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    let num_descriptors = u32_at(bytes, off + 4)?;
    let base_shader_register = u32_at(bytes, off + 8)?;
    let register_space = u32_at(bytes, off + 12)?;
    let (flags, offset) = if v11 {
        let flags = u32_at(bytes, off + 16)?;
        if flags & !RANGE_FLAGS != 0 {
            return Err(Error::RootSignatureMalformed);
        }
        (flags, u32_at(bytes, off + 20)?)
    } else {
        (0, u32_at(bytes, off + 16)?)
    };
    if offset != OFFSET_APPEND && offset > i32::MAX as u32 {
        return Err(Error::RootSignatureMalformed);
    }
    let _ = num_descriptors;
    let _ = base_shader_register;
    let _ = register_space;
    Ok(DescriptorRange {
        range_type,
        num_descriptors,
        base_shader_register,
        register_space,
        flags,
        offset,
    })
}

fn parse_sampler(bytes: &[u8], off: usize) -> Result<StaticSampler, Error> {
    let filter = u32_at(bytes, off)?;
    if !FILTERS.contains(&filter) {
        return Err(Error::RootSignatureMalformed);
    }
    let address_u = u32_at(bytes, off + 4)?;
    let address_v = u32_at(bytes, off + 8)?;
    let address_w = u32_at(bytes, off + 12)?;
    for addr in [address_u, address_v, address_w] {
        if !(ADDR_MIN..=ADDR_MAX).contains(&addr) {
            return Err(Error::RootSignatureMalformed);
        }
    }
    let mip_lod_bias = u32_at(bytes, off + 16)?;
    let max_anisotropy = u32_at(bytes, off + 20)?;
    if max_anisotropy > 16 {
        return Err(Error::RootSignatureMalformed);
    }
    let comparison_func = u32_at(bytes, off + 24)?;
    if comparison_func > CMP_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    let border_color = u32_at(bytes, off + 28)?;
    if border_color > BORDER_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    let min_lod = u32_at(bytes, off + 32)?;
    let max_lod = u32_at(bytes, off + 36)?;
    let shader_register = u32_at(bytes, off + 40)?;
    let register_space = u32_at(bytes, off + 44)?;
    let visibility = u32_at(bytes, off + 48)?;
    if visibility > VIS_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    Ok(StaticSampler {
        filter,
        address_u,
        address_v,
        address_w,
        mip_lod_bias,
        max_anisotropy,
        comparison_func,
        border_color,
        min_lod,
        max_lod,
        shader_register,
        register_space,
        visibility,
    })
}

struct Coverage {
    spans: Vec<(usize, usize)>,
}

impl Coverage {
    fn new() -> Self {
        Self { spans: Vec::new() }
    }

    fn mark(&mut self, start: usize, end: usize) -> Result<(), Error> {
        if start > end {
            return Err(Error::RootSignatureMalformed);
        }
        if start == end {
            return Ok(());
        }
        for &(s, e) in &self.spans {
            if start < e && s < end {
                return Err(Error::RootSignatureMalformed);
            }
        }
        self.spans.push((start, end));
        Ok(())
    }

    fn finish(mut self, len: usize) -> Result<(), Error> {
        self.spans.sort_unstable();
        let mut pos = 0;
        for (s, e) in self.spans {
            if s != pos {
                return Err(Error::RootSignatureMalformed);
            }
            pos = e;
        }
        if pos != len {
            return Err(Error::RootSignatureMalformed);
        }
        Ok(())
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let end = offset.checked_add(4).ok_or(Error::RootSignatureMalformed)?;
    let slice = bytes
        .get(offset..end)
        .ok_or(Error::RootSignatureMalformed)?;
    let arr: [u8; 4] = slice
        .try_into()
        .map_err(|_| Error::RootSignatureMalformed)?;
    Ok(u32::from_le_bytes(arr))
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Empty version-1.0 root signature (24-byte canonical form).
#[must_use]
pub fn empty_root_signature_v1_0() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_0,
        flags: 0,
        parameters: Vec::new(),
        samplers: Vec::new(),
    }
    .serialize()
}

/// Empty version-1.1 root signature (24-byte canonical form).
#[must_use]
pub fn empty_root_signature_v1_1() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_1,
        flags: 0,
        parameters: Vec::new(),
        samplers: Vec::new(),
    }
    .serialize()
}

/// One CBV root parameter, version 1.0.
#[must_use]
pub fn one_cbv_root_signature_v1_0() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_0,
        flags: 0,
        parameters: vec![Parameter::Descriptor {
            kind: PARAM_CBV,
            visibility: 0,
            shader_register: 0,
            register_space: 0,
            flags: 0,
        }],
        samplers: Vec::new(),
    }
    .serialize()
}

/// One descriptor table with one CBV range, version 1.1.
#[must_use]
pub fn one_table_root_signature_v1_1() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_1,
        flags: 0,
        parameters: vec![Parameter::Table {
            visibility: 5,
            ranges: vec![DescriptorRange {
                range_type: 2,
                num_descriptors: 1,
                base_shader_register: 0,
                register_space: 0,
                flags: 0,
                offset: OFFSET_APPEND,
            }],
        }],
        samplers: Vec::new(),
    }
    .serialize()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const MARKER: &[u8] = b"DXIL_MARKER_SHOULD_NOT_LEAK_9f3a";

    fn reject(bytes: &[u8]) {
        let err = parse_root_signature(bytes).unwrap_err();
        let text = format!("{err}{err:?}");
        assert!(
            matches!(
                err,
                Error::RootSignatureMalformed | Error::RootSignatureBadVersion
            ),
            "{err:?}"
        );
        assert!(!text.contains("DXIL_MARKER"));
        assert!(!text.contains("9f3a"));
        assert!(!text.as_bytes().windows(MARKER.len()).any(|w| w == MARKER));
    }

    #[test]
    fn empty_v1_0_round_trips_byte_identical() {
        let bytes = empty_root_signature_v1_0();
        let parsed = parse_root_signature(&bytes).unwrap();
        assert_eq!(parsed.serialize(), bytes);
        assert_eq!(bytes.len(), 24);
        assert_eq!(&bytes[..4], &[1, 0, 0, 0]);
    }

    #[test]
    fn empty_v1_1_round_trips_byte_identical() {
        let bytes = empty_root_signature_v1_1();
        let parsed = parse_root_signature(&bytes).unwrap();
        assert_eq!(parsed.serialize(), bytes);
        assert_eq!(&bytes[..4], &[2, 0, 0, 0]);
    }

    #[test]
    fn one_cbv_v1_0_round_trips() {
        let bytes = one_cbv_root_signature_v1_0();
        assert_eq!(parse_root_signature(&bytes).unwrap().serialize(), bytes);
    }

    #[test]
    fn one_table_v1_1_round_trips() {
        let bytes = one_table_root_signature_v1_1();
        assert_eq!(parse_root_signature(&bytes).unwrap().serialize(), bytes);
    }

    #[test]
    fn raw_smuggle_shapes_are_rejected() {
        let mut raw_smuggle = vec![1, 0, 0, 0];
        raw_smuggle.extend_from_slice(MARKER);
        reject(&raw_smuggle);

        let mut raw_smuggle2 = vec![2, 0, 0, 0];
        raw_smuggle2.extend_from_slice(MARKER);
        reject(&raw_smuggle2);

        let mut v99 = vec![99, 0, 0, 0];
        v99.extend_from_slice(MARKER);
        reject(&v99);
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = empty_root_signature_v1_0();
        bytes.push(0);
        reject(&bytes);
        bytes.pop();
        bytes.extend_from_slice(MARKER);
        reject(&bytes);
    }

    #[test]
    fn gap_bytes_are_rejected() {
        // Header + 20-byte CBV param starting at offset 28 leaves a 4-byte gap at 24.
        let mut bytes = vec![0_u8; 48];
        bytes[0..4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&28_u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&PARAM_CBV.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn overlapping_offsets_are_rejected() {
        let mut bytes = one_table_root_signature_v1_1();
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&24_u32.to_le_bytes());
        bytes.extend_from_slice(&[0_u8; SAMPLER_LEN]);
        reject(&bytes);
    }

    #[test]
    fn bad_enums_are_rejected() {
        let mut vis = one_cbv_root_signature_v1_0();
        vis[40..44].copy_from_slice(&8_u32.to_le_bytes());
        reject(&vis);

        let mut kind = one_cbv_root_signature_v1_0();
        kind[24..28].copy_from_slice(&99_u32.to_le_bytes());
        reject(&kind);

        let mut range = one_table_root_signature_v1_1();
        range[44..48].copy_from_slice(&4_u32.to_le_bytes());
        reject(&range);
    }

    #[test]
    fn counts_over_cap_are_rejected() {
        let mut params = empty_root_signature_v1_0();
        params[4..8].copy_from_slice(&((MAX_PARAMETERS as u32) + 1).to_le_bytes());
        params[8..12].copy_from_slice(&24_u32.to_le_bytes());
        reject(&params);

        let mut samplers = empty_root_signature_v1_0();
        samplers[12..16].copy_from_slice(&((MAX_SAMPLERS as u32) + 1).to_le_bytes());
        samplers[16..20].copy_from_slice(&24_u32.to_le_bytes());
        reject(&samplers);
    }

    #[test]
    fn version_3_is_rejected() {
        let mut bytes = empty_root_signature_v1_0();
        bytes[0..4].copy_from_slice(&3_u32.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn unknown_flags_are_rejected() {
        let mut bytes = empty_root_signature_v1_0();
        bytes[20..24].copy_from_slice(&0x1000_u32.to_le_bytes());
        reject(&bytes);
    }
}
