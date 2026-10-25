// SPDX-License-Identifier: MIT

//! Structural D3D12 root-signature parse and canonical re-serialization.
//!
//! Wire format matches `D3D12SerializeRootSignature` / DXC
//! `DxilContainerRootSignatureDesc` (`DxilRootSignature.h` and
//! `DxilRootSignatureSerializer.cpp`): 24-byte header, then
//! `NumParameters` × 12-byte `{ParameterType, ShaderVisibility,
//! PayloadOffset}` records, then type-specific payloads (descriptor-table
//! header + ranges, 32-bit constants, or CBV/SRV/UAV), then 52-byte
//! static samplers. [`CanonicalRootSignature::serialize`] emits DXC
//! `SimpleSerializer` order, including unused-array offsets at the
//! current cursor.

use crate::Error;

/// D3D12 root-signature parameter count cap (64 DWORDs on the root).
pub const MAX_PARAMETERS: usize = 64;
/// Maximum descriptor ranges in one table.
pub const MAX_RANGES_PER_TABLE: usize = 1024;
/// Maximum descriptor ranges across the signature.
pub const MAX_RANGES: usize = 4096;
/// Maximum static samplers.
pub const MAX_SAMPLERS: usize = 2048;
/// Maximum `Num32BitValues` on a constants parameter.
pub const MAX_32BIT_VALUES: u32 = 64;

const HEADER_LEN: usize = 24;
const PARAM_LEN: usize = 12;
const TABLE_HDR_LEN: usize = 8;
const CONSTANTS_LEN: usize = 12;
const DESC_1_0: usize = 8;
const DESC_1_1: usize = 12;
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
const BORDER_MAX: u32 = 4;
const ANISO_MAX: u32 = 16;

/// DXC `DxilRootSignatureFlags::ValidFlags` (`0x80000fff`): bits 0–11
/// plus `AllowLowTierReservedHwCbLimit` (`0x80000000`). Includes
/// `CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED` (`0x400`) and
/// `SAMPLER_HEAP_DIRECTLY_INDEXED` (`0x800`).
const FLAGS_KNOWN: u32 = 0x8000_0FFF;
const ROOT_DESC_FLAGS: u32 = 0x000E;
const RANGE_FLAGS: u32 = 0x1_000F;
const OFFSET_APPEND: u32 = 0xFFFF_FFFF;

const MIP_LOD_BIAS_MIN: f32 = -16.0;
const MIP_LOD_BIAS_MAX: f32 = 15.99;

/// `DxilSystemReservedRegisterSpaceValuesStart` — reject in
/// `RootSignatureVerifier::AddRegisterRange`.
const RESERVED_SPACE_START: u32 = 0xFFFF_FFF0;

/// Worst-case loosely constrained numeric bytes: 4 tables × 1024 ranges
/// (16 B) + 60 leftover root parameters (8 B) + 2048 samplers
/// (register/space 8 B + three floats 12 B).
pub const WORST_CASE_LOOSE_NUMERIC_BYTES: usize = 106_976;

const FILTERS: &[u32] = &[
    0x00, 0x01, 0x04, 0x05, 0x10, 0x11, 0x14, 0x15, 0x54, 0x55, 0x80, 0x81, 0x84, 0x85, 0x90, 0x91,
    0x94, 0x95, 0xD4, 0xD5, 0x100, 0x101, 0x104, 0x105, 0x110, 0x111, 0x114, 0x115, 0x154, 0x155,
    0x180, 0x181, 0x184, 0x185, 0x190, 0x191, 0x194, 0x195, 0x1D4, 0x1D5,
];

/// Agility SDK `D3D12_FILTER_COMPARISON_*` including
/// `MIN_MAG_ANISOTROPIC_MIP_POINT` (`0xd4`).
const COMPARISON_FILTERS: &[u32] = &[0x80, 0x81, 0x84, 0x85, 0x90, 0x91, 0x94, 0x95, 0xD4, 0xD5];

/// Parsed root signature. [`Self::serialize`] emits the DXC order
/// (header, parameter array, per-parameter payloads, static samplers).
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
    /// Canonical little-endian serialization matching DXC `SimpleSerializer`
    /// order: header, parameter array, payloads (table header then ranges),
    /// static samplers.
    pub fn serialize(&self) -> Vec<u8> {
        let v11 = self.version == VERSION_1_1;
        let range_len = if v11 { RANGE_1_1 } else { RANGE_1_0 };
        let desc_len = if v11 { DESC_1_1 } else { DESC_1_0 };
        let params_size = self.parameters.len().saturating_mul(PARAM_LEN);
        let mut payload_size = 0_usize;
        for p in &self.parameters {
            payload_size = payload_size.saturating_add(payload_len(p, range_len, desc_len));
        }
        let samplers_size = self.samplers.len().saturating_mul(SAMPLER_LEN);
        let mut out = Vec::with_capacity(HEADER_LEN + params_size + payload_size + samplers_size);
        // DXC `SimpleSerializer` always reserves the parameter array and the
        // sampler array (size 0 when empty) and stores those cursor offsets.
        let params_off = HEADER_LEN as u32;
        let mut cursor = HEADER_LEN + params_size;
        let mut payload_offs = Vec::with_capacity(self.parameters.len());
        for p in &self.parameters {
            payload_offs.push(cursor as u32);
            cursor += payload_len(p, range_len, desc_len);
        }
        let samplers_off = cursor as u32;
        push_u32(&mut out, self.version);
        push_u32(&mut out, self.parameters.len() as u32);
        push_u32(&mut out, params_off);
        push_u32(&mut out, self.samplers.len() as u32);
        push_u32(&mut out, samplers_off);
        push_u32(&mut out, self.flags);
        for (p, payload_off) in self.parameters.iter().zip(payload_offs.iter().copied()) {
            let kind = match p {
                Parameter::Table { .. } => PARAM_TABLE,
                Parameter::Constants { .. } => PARAM_CONSTANTS,
                Parameter::Descriptor { kind, .. } => *kind,
            };
            let visibility = match p {
                Parameter::Table { visibility, .. }
                | Parameter::Constants { visibility, .. }
                | Parameter::Descriptor { visibility, .. } => *visibility,
            };
            push_u32(&mut out, kind);
            push_u32(&mut out, visibility);
            push_u32(&mut out, payload_off);
        }
        for p in &self.parameters {
            match p {
                Parameter::Table { ranges, .. } => {
                    push_u32(&mut out, ranges.len() as u32);
                    // DXC `ReserveBlock` of the range array (size 0 when empty).
                    let ranges_off = (out.len() + 4) as u32;
                    push_u32(&mut out, ranges_off);
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
                Parameter::Constants {
                    shader_register,
                    register_space,
                    num_32bit,
                    ..
                } => {
                    push_u32(&mut out, *shader_register);
                    push_u32(&mut out, *register_space);
                    push_u32(&mut out, *num_32bit);
                }
                Parameter::Descriptor {
                    shader_register,
                    register_space,
                    flags,
                    ..
                } => {
                    push_u32(&mut out, *shader_register);
                    push_u32(&mut out, *register_space);
                    if v11 {
                        push_u32(&mut out, *flags);
                    }
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

fn payload_len(param: &Parameter, range_len: usize, desc_len: usize) -> usize {
    match param {
        Parameter::Table { ranges, .. } => {
            TABLE_HDR_LEN.saturating_add(ranges.len().saturating_mul(range_len))
        }
        Parameter::Constants { .. } => CONSTANTS_LEN,
        Parameter::Descriptor { .. } => desc_len,
    }
}

/// Parse a serialized D3D12 root signature (version 1.0 or 1.1) and rebuild it.
pub fn parse_root_signature(bytes: &[u8]) -> Result<CanonicalRootSignature, Error> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::RootSignatureMalformed);
    }
    if bytes.len() > crate::dxbc::MAX_ROOT_SIGNATURE_BYTES {
        return Err(Error::RootSignatureTooLarge {
            max: crate::dxbc::MAX_ROOT_SIGNATURE_BYTES as u64,
            got: bytes.len() as u64,
        });
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
    if num_params > 0 {
        let end = params_off
            .checked_add(
                num_params
                    .checked_mul(PARAM_LEN)
                    .ok_or(Error::RootSignatureMalformed)?,
            )
            .ok_or(Error::RootSignatureMalformed)?;
        cover.mark(params_off, end)?;
    }
    if num_samplers > 0 {
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
    let desc_len = if v11 { DESC_1_1 } else { DESC_1_0 };
    for i in 0..num_params {
        let base = params_off + i * PARAM_LEN;
        let kind = u32_at(bytes, base)?;
        let visibility = u32_at(bytes, base + 4)?;
        let payload_off = u32_at(bytes, base + 8)? as usize;
        if visibility > VIS_MAX {
            return Err(Error::RootSignatureMalformed);
        }
        if payload_off < HEADER_LEN {
            return Err(Error::RootSignatureMalformed);
        }
        let param = match kind {
            PARAM_TABLE => {
                let table_end = payload_off
                    .checked_add(TABLE_HDR_LEN)
                    .ok_or(Error::RootSignatureMalformed)?;
                cover.mark(payload_off, table_end)?;
                let num_ranges = u32_at(bytes, payload_off)? as usize;
                let ranges_off = u32_at(bytes, payload_off + 4)? as usize;
                if num_ranges > MAX_RANGES_PER_TABLE {
                    return Err(Error::RootSignatureMalformed);
                }
                total_ranges = total_ranges
                    .checked_add(num_ranges)
                    .ok_or(Error::RootSignatureMalformed)?;
                if total_ranges > MAX_RANGES {
                    return Err(Error::RootSignatureMalformed);
                }
                let ranges = if num_ranges == 0 {
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
                        ranges.push(parse_range(bytes, ranges_off + j * range_len, v11)?);
                    }
                    ranges
                };
                Parameter::Table { visibility, ranges }
            }
            PARAM_CONSTANTS => {
                let end = payload_off
                    .checked_add(CONSTANTS_LEN)
                    .ok_or(Error::RootSignatureMalformed)?;
                cover.mark(payload_off, end)?;
                let num_32bit = u32_at(bytes, payload_off + 8)?;
                if num_32bit > MAX_32BIT_VALUES {
                    return Err(Error::RootSignatureMalformed);
                }
                Parameter::Constants {
                    visibility,
                    shader_register: u32_at(bytes, payload_off)?,
                    register_space: u32_at(bytes, payload_off + 4)?,
                    num_32bit,
                }
            }
            PARAM_CBV | PARAM_SRV | PARAM_UAV => {
                let end = payload_off
                    .checked_add(desc_len)
                    .ok_or(Error::RootSignatureMalformed)?;
                cover.mark(payload_off, end)?;
                let flags = if v11 {
                    let flags = u32_at(bytes, payload_off + 8)?;
                    if flags & !ROOT_DESC_FLAGS != 0 {
                        return Err(Error::RootSignatureMalformed);
                    }
                    flags
                } else {
                    0
                };
                Parameter::Descriptor {
                    kind,
                    visibility,
                    shader_register: u32_at(bytes, payload_off)?,
                    register_space: u32_at(bytes, payload_off + 4)?,
                    flags,
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
    let parsed = CanonicalRootSignature {
        version,
        flags,
        parameters,
        samplers,
    };
    parsed.verify_dxc_bindings()?;
    Ok(parsed)
}

fn parse_range(bytes: &[u8], off: usize, v11: bool) -> Result<DescriptorRange, Error> {
    let range_type = u32_at(bytes, off)?;
    if range_type > RANGE_TYPE_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    let num_descriptors = u32_at(bytes, off + 4)?;
    // `DescriptorTableVerifier::Verify`: NumDescriptors cannot be 0.
    // `UINT_MAX` is the unbounded-range exception.
    if num_descriptors == 0 {
        return Err(Error::RootSignatureMalformed);
    }
    let base_shader_register = u32_at(bytes, off + 8)?;
    let register_space = u32_at(bytes, off + 12)?;
    if is_reserved_space(register_space) {
        return Err(Error::RootSignatureMalformed);
    }
    if num_descriptors != u32::MAX {
        let last = u64::from(base_shader_register)
            .checked_add(u64::from(num_descriptors))
            .and_then(|v| v.checked_sub(1))
            .ok_or(Error::RootSignatureMalformed)?;
        if last > u64::from(u32::MAX) {
            return Err(Error::RootSignatureMalformed);
        }
    }
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
    let mip_lod_bias = sampler_mip_lod_bias(u32_at(bytes, off + 16)?)?;
    let max_anisotropy = u32_at(bytes, off + 20)?;
    if max_anisotropy > ANISO_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    let comparison_func = u32_at(bytes, off + 24)?;
    // `StaticSamplerVerifier::Verify`: ComparisonFunc is checked only for
    // COMPARISON_* filters (`IsDxilComparisonFunc`).
    if COMPARISON_FILTERS.contains(&filter) && !(1..=CMP_MAX).contains(&comparison_func) {
        return Err(Error::RootSignatureMalformed);
    }
    let border_color = u32_at(bytes, off + 28)?;
    if border_color > BORDER_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    let min_lod = sampler_lod_bits(u32_at(bytes, off + 32)?)?;
    let max_lod = sampler_lod_bits(u32_at(bytes, off + 36)?)?;
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

/// `StaticSamplerVerifier::Verify` + DXC `isNaN`: MinLOD/MaxLOD accept every
/// non-NaN value including ±INF and `-FLT_MAX`. `-0.0` is canonicalised to `+0`.
fn sampler_lod_bits(bits: u32) -> Result<u32, Error> {
    let value = f32::from_bits(bits);
    if value.is_nan() {
        return Err(Error::RootSignatureMalformed);
    }
    if value == 0.0 {
        return Ok(0);
    }
    Ok(bits)
}

/// `StaticSamplerVerifier::Verify`: MipLODBias rejects NaN and values outside
/// `[DxilMipLodBiaxMin, DxilMipLodBiaxMax]`. `-0.0` is canonicalised to `+0`.
fn sampler_mip_lod_bias(bits: u32) -> Result<u32, Error> {
    let value = f32::from_bits(bits);
    if value.is_nan() || value < MIP_LOD_BIAS_MIN || value > MIP_LOD_BIAS_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    if value == 0.0 {
        return Ok(0);
    }
    Ok(bits)
}

fn is_reserved_space(space: u32) -> bool {
    space >= RESERVED_SPACE_START
}

impl CanonicalRootSignature {
    /// `RootSignatureVerifier::VerifyRootSignature` +
    /// `DescriptorTableVerifier::Verify` + `AddRegisterRange`.
    fn verify_dxc_bindings(&self) -> Result<(), Error> {
        let mut kinds: [[Vec<Interval>; 4]; 8] = Default::default();
        for (i, param) in self.parameters.iter().enumerate() {
            match param {
                Parameter::Table { visibility, ranges } => {
                    verify_descriptor_table(ranges, i)?;
                    for r in ranges {
                        verify_range_flag_combo(r)?;
                        add_register_range(
                            &mut kinds,
                            *visibility,
                            r.range_type,
                            r.num_descriptors,
                            r.base_shader_register,
                            r.register_space,
                        )?;
                    }
                }
                Parameter::Constants {
                    visibility,
                    shader_register,
                    register_space,
                    ..
                } => {
                    add_register_range(
                        &mut kinds,
                        *visibility,
                        2,
                        1,
                        *shader_register,
                        *register_space,
                    )?;
                }
                Parameter::Descriptor {
                    kind,
                    visibility,
                    shader_register,
                    register_space,
                    flags,
                } => {
                    if self.version == VERSION_1_1 {
                        verify_root_desc_flags(*flags)?;
                    }
                    let ty = match *kind {
                        PARAM_CBV => 2,
                        PARAM_SRV => 0,
                        PARAM_UAV => 1,
                        _ => return Err(Error::RootSignatureMalformed),
                    };
                    add_register_range(
                        &mut kinds,
                        *visibility,
                        ty,
                        1,
                        *shader_register,
                        *register_space,
                    )?;
                }
            }
        }
        for s in &self.samplers {
            add_register_range(
                &mut kinds,
                s.visibility,
                3,
                1,
                s.shader_register,
                s.register_space,
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Interval {
    space: u32,
    lb: u32,
    ub: u32,
}

impl Interval {
    fn overlaps(self, other: Self) -> bool {
        self.space == other.space && self.lb <= other.ub && other.lb <= self.ub
    }
}

/// `DescriptorTableVerifier::Verify`
fn verify_descriptor_table(ranges: &[DescriptorRange], _i_rp: usize) -> Result<(), Error> {
    let mut has_samplers = false;
    let mut has_resources = false;
    let mut append_start: u64 = 0;
    for r in ranges {
        match r.range_type {
            0..=2 => has_resources = true,
            3 => has_samplers = true,
            _ => return Err(Error::RootSignatureMalformed),
        }
        if has_resources && has_samplers {
            return Err(Error::RootSignatureMalformed);
        }
        let mut start = append_start;
        if r.offset != OFFSET_APPEND {
            start = u64::from(r.offset);
        }
        if start > u64::from(u32::MAX) {
            return Err(Error::RootSignatureMalformed);
        }
        if r.num_descriptors != u32::MAX {
            let ub = start
                .checked_add(u64::from(r.num_descriptors))
                .and_then(|v| v.checked_sub(1))
                .ok_or(Error::RootSignatureMalformed)?;
            if ub > u64::from(u32::MAX) {
                return Err(Error::RootSignatureMalformed);
            }
            append_start = start.saturating_add(u64::from(r.num_descriptors));
        } else {
            append_start = 1u64 + u64::from(u32::MAX);
        }
    }
    Ok(())
}

/// `RootSignatureVerifier::VerifyRootSignature` descriptor-range flag combos.
fn verify_range_flag_combo(r: &DescriptorRange) -> Result<(), Error> {
    let flags = r.flags;
    if r.range_type == 3 {
        let data = 0x2 | 0x4 | 0x8;
        if flags & data != 0 {
            return Err(Error::RootSignatureMalformed);
        }
    } else {
        let mut data_flags = 0;
        if flags & 0x2 != 0 {
            data_flags += 1;
        }
        if flags & 0x4 != 0 {
            data_flags += 1;
        }
        if flags & 0x8 != 0 {
            data_flags += 1;
        }
        if data_flags > 1 {
            return Err(Error::RootSignatureMalformed);
        }
        if flags & 0x8 != 0 && flags & 0x1 != 0 {
            return Err(Error::RootSignatureMalformed);
        }
    }
    Ok(())
}

fn verify_root_desc_flags(flags: u32) -> Result<(), Error> {
    let mut data_flags = 0;
    if flags & 0x2 != 0 {
        data_flags += 1;
    }
    if flags & 0x4 != 0 {
        data_flags += 1;
    }
    if flags & 0x8 != 0 {
        data_flags += 1;
    }
    if data_flags > 1 {
        return Err(Error::RootSignatureMalformed);
    }
    Ok(())
}

/// `RootSignatureVerifier::AddRegisterRange`
fn add_register_range(
    kinds: &mut [[Vec<Interval>; 4]; 8],
    visibility: u32,
    desc_type: u32,
    num: u32,
    base: u32,
    space: u32,
) -> Result<(), Error> {
    if visibility > VIS_MAX || desc_type > RANGE_TYPE_MAX {
        return Err(Error::RootSignatureMalformed);
    }
    if is_reserved_space(space) {
        return Err(Error::RootSignatureMalformed);
    }
    let ub = if num == u32::MAX {
        u32::MAX
    } else {
        base.saturating_add(num.saturating_sub(1))
    };
    let interval = Interval {
        space,
        lb: base,
        ub,
    };
    let ty = desc_type as usize;
    if visibility == 0 {
        for slot in kinds.iter() {
            if slot[ty].iter().any(|ex| ex.overlaps(interval)) {
                return Err(Error::RootSignatureMalformed);
            }
        }
    } else {
        let vis = visibility as usize;
        if kinds[vis][ty].iter().any(|ex| ex.overlaps(interval))
            || kinds[0][ty].iter().any(|ex| ex.overlaps(interval))
        {
            return Err(Error::RootSignatureMalformed);
        }
    }
    kinds[visibility as usize][ty].push(interval);
    Ok(())
}

/// Documented worst-case loosely constrained numeric capacity.
#[must_use]
pub fn worst_case_loose_numeric_bytes() -> usize {
    let range_bytes = MAX_RANGES.saturating_mul(16);
    let leftover_params = MAX_PARAMETERS.saturating_sub(MAX_RANGES / MAX_RANGES_PER_TABLE);
    let param_bytes = leftover_params.saturating_mul(8);
    let sampler_bytes = MAX_SAMPLERS.saturating_mul(20);
    range_bytes
        .saturating_add(param_bytes)
        .saturating_add(sampler_bytes)
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

fn empty(version: u32, flags: u32) -> Vec<u8> {
    CanonicalRootSignature {
        version,
        flags,
        parameters: Vec::new(),
        samplers: Vec::new(),
    }
    .serialize()
}

/// Empty version-1.0 root signature (24-byte canonical form).
#[must_use]
pub fn empty_root_signature_v1_0() -> Vec<u8> {
    empty(VERSION_1_0, 0)
}

/// Empty version-1.1 root signature (24-byte canonical form).
#[must_use]
pub fn empty_root_signature_v1_1() -> Vec<u8> {
    empty(VERSION_1_1, 0)
}

/// Empty v1.0 with `AllowLowTierReservedHwCbLimit` (`0x80000000`).
#[must_use]
pub fn empty_root_signature_v1_0_heap_flag() -> Vec<u8> {
    empty(VERSION_1_0, 0x8000_0000)
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

/// One 32-bit-constants root parameter, version 1.0.
#[must_use]
pub fn one_constants_root_signature_v1_0() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_0,
        flags: 0,
        parameters: vec![Parameter::Constants {
            visibility: 1,
            shader_register: 2,
            register_space: 0,
            num_32bit: 4,
        }],
        samplers: Vec::new(),
    }
    .serialize()
}

/// One CBV root parameter with 1.1 flags.
#[must_use]
pub fn one_cbv_root_signature_v1_1() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_1,
        flags: 0,
        parameters: vec![Parameter::Descriptor {
            kind: PARAM_CBV,
            visibility: 0,
            shader_register: 1,
            register_space: 2,
            flags: 0x8,
        }],
        samplers: Vec::new(),
    }
    .serialize()
}

fn table(version: u32, ranges: Vec<DescriptorRange>) -> Vec<u8> {
    CanonicalRootSignature {
        version,
        flags: 0,
        parameters: vec![Parameter::Table {
            visibility: 5,
            ranges,
        }],
        samplers: Vec::new(),
    }
    .serialize()
}

/// Descriptor table with three 1.0 ranges (SRV, CBV, sampler).
#[must_use]
pub fn table_three_ranges_v1_0() -> Vec<u8> {
    table(
        VERSION_1_0,
        vec![
            DescriptorRange {
                range_type: 0,
                num_descriptors: 2,
                base_shader_register: 0,
                register_space: 0,
                flags: 0,
                offset: OFFSET_APPEND,
            },
            DescriptorRange {
                range_type: 2,
                num_descriptors: 1,
                base_shader_register: 0,
                register_space: 0,
                flags: 0,
                offset: 0,
            },
            DescriptorRange {
                range_type: 1,
                num_descriptors: 1,
                base_shader_register: 0,
                register_space: 0,
                flags: 0,
                offset: OFFSET_APPEND,
            },
        ],
    )
}

/// One descriptor table with one CBV range, version 1.1.
#[must_use]
pub fn one_table_root_signature_v1_1() -> Vec<u8> {
    table(
        VERSION_1_1,
        vec![DescriptorRange {
            range_type: 2,
            num_descriptors: 1,
            base_shader_register: 0,
            register_space: 0,
            flags: 0,
            offset: OFFSET_APPEND,
        }],
    )
}

/// Descriptor table with two 1.1 ranges (flags set).
#[must_use]
pub fn table_two_ranges_v1_1() -> Vec<u8> {
    table(
        VERSION_1_1,
        vec![
            DescriptorRange {
                range_type: 0,
                num_descriptors: 4,
                base_shader_register: 1,
                register_space: 0,
                flags: 0x8,
                offset: 0,
            },
            DescriptorRange {
                range_type: 1,
                num_descriptors: 1,
                base_shader_register: 0,
                register_space: 1,
                flags: 0x2,
                offset: OFFSET_APPEND,
            },
        ],
    )
}

/// One static sampler, border color `OpaqueWhiteUint` (4).
#[must_use]
pub fn one_sampler_border4_v1_0() -> Vec<u8> {
    CanonicalRootSignature {
        version: VERSION_1_0,
        flags: 0,
        parameters: Vec::new(),
        samplers: vec![StaticSampler {
            filter: 0,
            address_u: 1,
            address_v: 1,
            address_w: 1,
            mip_lod_bias: 0,
            max_anisotropy: 0,
            comparison_func: 0,
            border_color: 4,
            min_lod: 0,
            max_lod: 0,
            shader_register: 0,
            register_space: 0,
            visibility: 0,
        }],
    }
    .serialize()
}

/// Valid v1.0 CBV whose payload is packed before the parameter array
/// (not the DXC order). Parse succeeds; [`CanonicalRootSignature::serialize`]
/// emits a different byte string.
#[must_use]
pub fn one_cbv_v1_0_payload_before_params() -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, VERSION_1_0);
    push_u32(&mut out, 1);
    push_u32(&mut out, 32);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, PARAM_CBV);
    push_u32(&mut out, 0);
    push_u32(&mut out, 24);
    out
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
                Error::RootSignatureMalformed
                    | Error::RootSignatureBadVersion
                    | Error::RootSignatureTooLarge { .. }
            ),
            "{err:?}"
        );
        assert!(!text.contains("DXIL_MARKER"));
        assert!(!text.contains("9f3a"));
        assert!(!text.as_bytes().windows(MARKER.len()).any(|w| w == MARKER));
    }

    fn round_trip(bytes: &[u8]) {
        let parsed = parse_root_signature(bytes).unwrap();
        assert_eq!(parsed.serialize(), bytes);
    }

    #[test]
    fn empty_v1_0_round_trips_byte_identical() {
        let bytes = empty_root_signature_v1_0();
        round_trip(&bytes);
        assert_eq!(bytes.len(), 24);
        assert_eq!(&bytes[..4], &[1, 0, 0, 0]);
        // DXC `ReserveBlock` of the empty parameter and sampler arrays.
        assert_eq!(&bytes[8..12], &24_u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &24_u32.to_le_bytes());
    }

    #[test]
    fn empty_v1_1_round_trips_byte_identical() {
        let bytes = empty_root_signature_v1_1();
        round_trip(&bytes);
        assert_eq!(&bytes[..4], &[2, 0, 0, 0]);
        assert_eq!(&bytes[8..12], &24_u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &24_u32.to_le_bytes());
    }

    #[test]
    fn empty_zero_offsets_parse_but_are_not_canonical() {
        let bytes = vec![
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let parsed = parse_root_signature(&bytes).unwrap();
        assert_ne!(parsed.serialize(), bytes);
        assert_eq!(parsed.serialize(), empty_root_signature_v1_0());
    }

    #[test]
    fn one_cbv_v1_0_round_trips() {
        let bytes = one_cbv_root_signature_v1_0();
        assert_eq!(bytes.len(), 44);
        assert_eq!(&bytes[24..28], &PARAM_CBV.to_le_bytes());
        assert_eq!(&bytes[32..36], &36_u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &44_u32.to_le_bytes());
        round_trip(&bytes);
    }

    #[test]
    fn one_constants_v1_0_round_trips() {
        let bytes = one_constants_root_signature_v1_0();
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[24..28], &PARAM_CONSTANTS.to_le_bytes());
        round_trip(&bytes);
    }

    #[test]
    fn one_cbv_v1_1_with_flags_round_trips() {
        let bytes = one_cbv_root_signature_v1_1();
        assert_eq!(bytes.len(), 48);
        round_trip(&bytes);
    }

    #[test]
    fn table_three_ranges_v1_0_round_trips() {
        let bytes = table_three_ranges_v1_0();
        assert_eq!(bytes.len(), 24 + 12 + 8 + 60);
        round_trip(&bytes);
    }

    #[test]
    fn table_two_ranges_v1_1_round_trips() {
        let bytes = table_two_ranges_v1_1();
        assert_eq!(bytes.len(), 24 + 12 + 8 + 48);
        round_trip(&bytes);
    }

    #[test]
    fn one_table_v1_1_round_trips() {
        let bytes = one_table_root_signature_v1_1();
        round_trip(&bytes);
    }

    #[test]
    fn sampler_border4_round_trips() {
        let bytes = one_sampler_border4_v1_0();
        assert_eq!(bytes.len(), 24 + 52);
        assert_eq!(&bytes[8..12], &24_u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &24_u32.to_le_bytes());
        assert_eq!(&bytes[24 + 28..24 + 32], &4_u32.to_le_bytes());
        round_trip(&bytes);
    }

    #[test]
    fn official_heap_index_flags_round_trip() {
        let bytes = empty(VERSION_1_0, 0x400 | 0x800);
        assert_eq!(&bytes[20..24], &0xC00_u32.to_le_bytes());
        round_trip(&bytes);
    }

    #[test]
    fn flag_0x80000000_round_trips() {
        let bytes = empty_root_signature_v1_0_heap_flag();
        assert_eq!(&bytes[20..24], &0x8000_0000_u32.to_le_bytes());
        round_trip(&bytes);
    }

    #[test]
    fn payload_before_params_parses_but_is_not_canonical() {
        let bytes = one_cbv_v1_0_payload_before_params();
        let parsed = parse_root_signature(&bytes).unwrap();
        let canonical = parsed.serialize();
        assert_ne!(canonical, bytes);
        assert_eq!(canonical, one_cbv_root_signature_v1_0());
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
        let mut bytes = one_cbv_root_signature_v1_0();
        bytes[32..36].copy_from_slice(&40_u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        reject(&bytes);
    }

    #[test]
    fn overlapping_payload_offsets_are_rejected() {
        let mut bytes = table_two_ranges_v1_1();
        bytes[16..20].copy_from_slice(&36_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&[0_u8; SAMPLER_LEN]);
        reject(&bytes);
    }

    #[test]
    fn payload_offset_into_header_is_rejected() {
        let mut bytes = one_cbv_root_signature_v1_0();
        bytes[32..36].copy_from_slice(&4_u32.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn payload_offset_into_other_payload_is_rejected() {
        let mut bytes = CanonicalRootSignature {
            version: VERSION_1_0,
            flags: 0,
            parameters: vec![
                Parameter::Descriptor {
                    kind: PARAM_CBV,
                    visibility: 0,
                    shader_register: 0,
                    register_space: 0,
                    flags: 0,
                },
                Parameter::Descriptor {
                    kind: PARAM_SRV,
                    visibility: 0,
                    shader_register: 1,
                    register_space: 0,
                    flags: 0,
                },
            ],
            samplers: Vec::new(),
        }
        .serialize();
        let first_payload = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
        bytes[44..48].copy_from_slice(&first_payload.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn bad_enums_are_rejected() {
        let mut vis = one_cbv_root_signature_v1_0();
        vis[28..32].copy_from_slice(&8_u32.to_le_bytes());
        reject(&vis);

        let mut kind = one_cbv_root_signature_v1_0();
        kind[24..28].copy_from_slice(&99_u32.to_le_bytes());
        reject(&kind);

        let mut range = one_table_root_signature_v1_1();
        range[44..48].copy_from_slice(&4_u32.to_le_bytes());
        reject(&range);

        let mut border = one_sampler_border4_v1_0();
        border[24 + 28..24 + 32].copy_from_slice(&5_u32.to_le_bytes());
        reject(&border);

        let mut filter = one_sampler_border4_v1_0();
        filter[24..28].copy_from_slice(&0x2_u32.to_le_bytes());
        reject(&filter);

        let mut addr = one_sampler_border4_v1_0();
        addr[24 + 4..24 + 8].copy_from_slice(&0_u32.to_le_bytes());
        reject(&addr);

        let mut cmp = one_sampler_border4_v1_0();
        cmp[24..28].copy_from_slice(&0x80_u32.to_le_bytes());
        cmp[24 + 24..24 + 28].copy_from_slice(&9_u32.to_le_bytes());
        reject(&cmp);
    }

    #[test]
    fn comparison_func_is_ignored_for_non_comparison_filter() {
        let mut bytes = one_sampler_border4_v1_0();
        bytes[24 + 24..24 + 28].copy_from_slice(&9_u32.to_le_bytes());
        round_trip(&bytes);
    }

    #[test]
    fn counts_over_cap_are_rejected() {
        let mut params = empty_root_signature_v1_0();
        params[4..8].copy_from_slice(&((MAX_PARAMETERS as u32) + 1).to_le_bytes());
        params[8..12].copy_from_slice(&24_u32.to_le_bytes());
        reject(&params);

        let many = {
            let ranges = (0..=MAX_RANGES_PER_TABLE as u32)
                .map(|space| DescriptorRange {
                    range_type: 2,
                    num_descriptors: 1,
                    base_shader_register: 0,
                    register_space: space,
                    flags: 0,
                    offset: 0,
                })
                .collect();
            table(VERSION_1_0, ranges)
        };
        reject(&many);

        let mut samplers = empty_root_signature_v1_0();
        samplers[12..16].copy_from_slice(&((MAX_SAMPLERS as u32) + 1).to_le_bytes());
        samplers[16..20].copy_from_slice(&24_u32.to_le_bytes());
        reject(&samplers);

        let many_samplers = CanonicalRootSignature {
            version: VERSION_1_0,
            flags: 0,
            parameters: Vec::new(),
            samplers: (0..=MAX_SAMPLERS as u32)
                .map(|i| StaticSampler {
                    filter: 0,
                    address_u: 1,
                    address_v: 1,
                    address_w: 1,
                    mip_lod_bias: 0,
                    max_anisotropy: 0,
                    comparison_func: 0,
                    border_color: 0,
                    min_lod: 0,
                    max_lod: 0,
                    shader_register: i,
                    register_space: 0,
                    visibility: 0,
                })
                .collect(),
        }
        .serialize();
        reject(&many_samplers);
    }

    #[test]
    fn sixty_five_cbv_parameters_are_rejected() {
        let parameters = (0..65_u32)
            .map(|i| Parameter::Descriptor {
                kind: PARAM_CBV,
                visibility: 0,
                shader_register: i,
                register_space: 0,
                flags: 0,
            })
            .collect();
        let bytes = CanonicalRootSignature {
            version: VERSION_1_0,
            flags: 0,
            parameters,
            samplers: Vec::new(),
        }
        .serialize();
        reject(&bytes);
    }

    #[test]
    fn nan_sampler_floats_are_rejected() {
        let mut nan_bias = one_sampler_border4_v1_0();
        nan_bias[24 + 16..24 + 20].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        reject(&nan_bias);
        let mut nan_min = one_sampler_border4_v1_0();
        nan_min[24 + 32..24 + 36].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        reject(&nan_min);
        let mut nan_max = one_sampler_border4_v1_0();
        nan_max[24 + 36..24 + 40].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        reject(&nan_max);
    }

    #[test]
    fn sampler_lod_accepts_inf_and_neg_float_max() {
        let mut bytes = one_sampler_border4_v1_0();
        bytes[24 + 32..24 + 36].copy_from_slice(&f32::NEG_INFINITY.to_bits().to_le_bytes());
        bytes[24 + 36..24 + 40].copy_from_slice(&f32::INFINITY.to_bits().to_le_bytes());
        round_trip(&bytes);
        let mut min_max = one_sampler_border4_v1_0();
        min_max[24 + 32..24 + 36].copy_from_slice(&(-f32::MAX).to_bits().to_le_bytes());
        round_trip(&min_max);
    }

    #[test]
    fn out_of_range_mip_lod_bias_is_rejected() {
        let mut bias = one_sampler_border4_v1_0();
        bias[24 + 16..24 + 20].copy_from_slice(&20.0_f32.to_bits().to_le_bytes());
        reject(&bias);
        let mut inf_bias = one_sampler_border4_v1_0();
        inf_bias[24 + 16..24 + 20].copy_from_slice(&f32::INFINITY.to_bits().to_le_bytes());
        reject(&inf_bias);
    }

    #[test]
    fn negative_zero_sampler_float_is_canonicalised() {
        let mut bytes = one_sampler_border4_v1_0();
        bytes[24 + 16..24 + 20].copy_from_slice(&(-0.0_f32).to_bits().to_le_bytes());
        let parsed = parse_root_signature(&bytes).unwrap();
        assert_eq!(parsed.serialize(), one_sampler_border4_v1_0());
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
        let mut reserved = empty_root_signature_v1_0();
        reserved[20..24].copy_from_slice(&0x4000_0000_u32.to_le_bytes());
        reject(&reserved);
    }

    #[test]
    fn numeric_field_caps_are_rejected() {
        let mut constants = one_constants_root_signature_v1_0();
        constants[44..48].copy_from_slice(&65_u32.to_le_bytes());
        reject(&constants);

        let mut aniso = one_sampler_border4_v1_0();
        aniso[24 + 20..24 + 24].copy_from_slice(&17_u32.to_le_bytes());
        reject(&aniso);

        let mut offset = table_three_ranges_v1_0();
        offset[60..64].copy_from_slice(&(i32::MAX as u32 + 1).to_le_bytes());
        reject(&offset);
    }

    #[test]
    fn total_ranges_over_cap_are_rejected() {
        let tables = 5;
        let per = MAX_RANGES_PER_TABLE;
        let mut parameters = Vec::new();
        let mut space = 0_u32;
        for _ in 0..tables {
            let mut ranges = Vec::with_capacity(per);
            for _ in 0..per {
                ranges.push(DescriptorRange {
                    range_type: 2,
                    num_descriptors: 1,
                    base_shader_register: 0,
                    register_space: space,
                    flags: 0,
                    offset: 0,
                });
                space += 1;
            }
            parameters.push(Parameter::Table {
                visibility: 0,
                ranges,
            });
        }
        let bytes = CanonicalRootSignature {
            version: VERSION_1_0,
            flags: 0,
            parameters,
            samplers: Vec::new(),
        }
        .serialize();
        reject(&bytes);
    }

    #[test]
    fn root_descriptor_flags_mask_is_enforced() {
        let mut bytes = one_cbv_root_signature_v1_1();
        bytes[44..48].copy_from_slice(&0x1_u32.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn range_flags_mask_is_enforced() {
        let mut bytes = table_two_ranges_v1_1();
        bytes[60..64].copy_from_slice(&0x20_u32.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn sampler_visibility_over_max_is_rejected() {
        let mut bytes = one_sampler_border4_v1_0();
        bytes[24 + 48..24 + 52].copy_from_slice(&8_u32.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn reserved_register_space_is_rejected() {
        let mut bytes = one_cbv_root_signature_v1_0();
        bytes[40..44].copy_from_slice(&RESERVED_SPACE_START.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn zero_num_descriptors_is_rejected() {
        let mut bytes = one_table_root_signature_v1_1();
        bytes[48..52].copy_from_slice(&0_u32.to_le_bytes());
        reject(&bytes);
    }

    #[test]
    fn overlapping_register_bindings_are_rejected() {
        let bytes = CanonicalRootSignature {
            version: VERSION_1_0,
            flags: 0,
            parameters: vec![
                Parameter::Descriptor {
                    kind: PARAM_CBV,
                    visibility: 0,
                    shader_register: 0,
                    register_space: 0,
                    flags: 0,
                },
                Parameter::Descriptor {
                    kind: PARAM_CBV,
                    visibility: 1,
                    shader_register: 0,
                    register_space: 0,
                    flags: 0,
                },
            ],
            samplers: Vec::new(),
        }
        .serialize();
        reject(&bytes);
    }

    #[test]
    fn base_plus_num_overflow_is_rejected() {
        let bytes = table(
            VERSION_1_0,
            vec![DescriptorRange {
                range_type: 2,
                num_descriptors: 2,
                base_shader_register: u32::MAX,
                register_space: 0,
                flags: 0,
                offset: 0,
            }],
        );
        reject(&bytes);
    }

    #[test]
    fn worst_case_loose_numeric_capacity_is_documented() {
        assert_eq!(
            worst_case_loose_numeric_bytes(),
            WORST_CASE_LOOSE_NUMERIC_BYTES
        );
        assert_eq!(WORST_CASE_LOOSE_NUMERIC_BYTES, 106_976);
    }

    #[test]
    fn agility_aniso_mip_point_filters_round_trip() {
        for filter in [0x54_u32, 0xD4, 0x154, 0x1D4] {
            let bytes = CanonicalRootSignature {
                version: VERSION_1_0,
                flags: 0,
                parameters: Vec::new(),
                samplers: vec![StaticSampler {
                    filter,
                    address_u: 1,
                    address_v: 1,
                    address_w: 1,
                    mip_lod_bias: 0,
                    max_anisotropy: 0,
                    comparison_func: if filter == 0xD4 { 1 } else { 0 },
                    border_color: 0,
                    min_lod: 0,
                    max_lod: 0,
                    shader_register: 0,
                    register_space: 0,
                    visibility: 0,
                }],
            }
            .serialize();
            round_trip(&bytes);
        }
    }
}
