// SPDX-License-Identifier: MIT

//! Bounds-checked DXBC container parse used only for root signatures.

use crate::Error;

/// Maximum emitted root-signature bytes (raw or RTS0 payload).
pub const MAX_ROOT_SIGNATURE_BYTES: usize = 1024 * 1024;

const DXBC_MAGIC: &[u8; 4] = b"DXBC";
const RTS0: &[u8; 4] = b"RTS0";
const HEADER_LEN: usize = 32;

/// Sanitize a `root_signatures.value` blob.
///
/// DXBC containers must contain **only** an RTS0 part; any other part
/// (including DXIL / SHEX / SHDR / ILDB) is rejected. Non-DXBC blobs are
/// treated as a raw serialized root signature: size-capped and checked for
/// a little-endian version field in `1..=3`.
pub fn sanitize_root_signature(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, Error> {
    if bytes.len() >= 4 && bytes.starts_with(DXBC_MAGIC) {
        return extract_rts0_only(bytes, max_bytes);
    }
    sanitize_raw(bytes, max_bytes)
}

fn sanitize_raw(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, Error> {
    let got = bytes.len() as u64;
    if got > max_bytes as u64 {
        return Err(Error::RootSignatureTooLarge {
            max: max_bytes as u64,
            got,
        });
    }
    if bytes.len() < 4 {
        return Err(Error::RootSignatureBadVersion);
    }
    let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if !(1..=3).contains(&version) {
        return Err(Error::RootSignatureBadVersion);
    }
    Ok(bytes.to_vec())
}

fn extract_rts0_only(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, Error> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::DxbcMalformed);
    }
    let file_size = u32_at(bytes, 24)?;
    if (file_size as usize) < HEADER_LEN || (file_size as usize) > bytes.len() {
        return Err(Error::DxbcMalformed);
    }
    let part_count = u32_at(bytes, 28)? as usize;
    let offsets_end = HEADER_LEN
        .checked_add(part_count.checked_mul(4).ok_or(Error::DxbcMalformed)?)
        .ok_or(Error::DxbcMalformed)?;
    if offsets_end > bytes.len() {
        return Err(Error::DxbcMalformed);
    }
    let mut rts0 = None;
    for i in 0..part_count {
        let off = u32_at(bytes, HEADER_LEN + i * 4)? as usize;
        let fourcc_end = off.checked_add(8).ok_or(Error::DxbcMalformed)?;
        if fourcc_end > bytes.len() {
            return Err(Error::DxbcMalformed);
        }
        let fourcc = &bytes[off..off + 4];
        let size = u32_at(bytes, off + 4)? as usize;
        let data_end = fourcc_end.checked_add(size).ok_or(Error::DxbcMalformed)?;
        if data_end > bytes.len() {
            return Err(Error::DxbcMalformed);
        }
        if fourcc != RTS0 {
            return Err(Error::DxbcForbiddenPart);
        }
        if rts0.is_some() {
            return Err(Error::DxbcMalformed);
        }
        let payload = bytes[fourcc_end..data_end].to_vec();
        if payload.len() > max_bytes {
            return Err(Error::RootSignatureTooLarge {
                max: max_bytes as u64,
                got: payload.len() as u64,
            });
        }
        rts0 = Some(payload);
    }
    rts0.filter(|p| !p.is_empty()).ok_or(Error::DxbcMalformed)
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let end = offset.checked_add(4).ok_or(Error::DxbcMalformed)?;
    let slice = bytes.get(offset..end).ok_or(Error::DxbcMalformed)?;
    let arr: [u8; 4] = slice.try_into().map_err(|_| Error::DxbcMalformed)?;
    Ok(u32::from_le_bytes(arr))
}
