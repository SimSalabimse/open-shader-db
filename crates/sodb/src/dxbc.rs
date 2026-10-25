// SPDX-License-Identifier: MIT

//! Bounds-checked DXBC container parse used only for root signatures.

use crate::rootsig::parse_root_signature;
use crate::Error;

/// Maximum emitted root-signature bytes (canonical serialized form).
pub const MAX_ROOT_SIGNATURE_BYTES: usize = 1024 * 1024;

const DXBC_MAGIC: &[u8; 4] = b"DXBC";
const RTS0: &[u8; 4] = b"RTS0";
const HEADER_LEN: usize = 32;

/// Sanitize a `root_signatures.value` blob.
///
/// DXBC containers must contain **only** an RTS0 part. The RTS0 payload
/// (or a raw non-DXBC blob) is parsed as a D3D12 root signature version
/// 1.0 or 1.1 and **re-serialized canonically**. Free-form bytes cannot
/// pass through.
pub fn sanitize_root_signature(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, Error> {
    if bytes.len() >= 4 && bytes.starts_with(DXBC_MAGIC) {
        return extract_rts0_only(bytes, max_bytes);
    }
    canonicalize_root_signature(bytes, max_bytes)
}

fn canonicalize_root_signature(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, Error> {
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
    let parsed = parse_root_signature(bytes)?;
    let out = parsed.serialize();
    if out.len() > max_bytes {
        return Err(Error::RootSignatureTooLarge {
            max: max_bytes as u64,
            got: out.len() as u64,
        });
    }
    Ok(out)
}

fn extract_rts0_only(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, Error> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::DxbcMalformed);
    }
    let file_size = u32_at(bytes, 24)? as usize;
    if file_size < HEADER_LEN || file_size > bytes.len() || file_size != bytes.len() {
        return Err(Error::DxbcMalformed);
    }
    let part_count = u32_at(bytes, 28)? as usize;
    let offsets_end = HEADER_LEN
        .checked_add(part_count.checked_mul(4).ok_or(Error::DxbcMalformed)?)
        .ok_or(Error::DxbcMalformed)?;
    if offsets_end > file_size {
        return Err(Error::DxbcMalformed);
    }
    let mut rts0 = None;
    for i in 0..part_count {
        let off = u32_at(bytes, HEADER_LEN + i * 4)? as usize;
        let fourcc_end = off.checked_add(8).ok_or(Error::DxbcMalformed)?;
        if fourcc_end > file_size {
            return Err(Error::DxbcMalformed);
        }
        let fourcc = &bytes[off..off + 4];
        let size = u32_at(bytes, off + 4)? as usize;
        let data_end = fourcc_end.checked_add(size).ok_or(Error::DxbcMalformed)?;
        if data_end > file_size {
            return Err(Error::DxbcMalformed);
        }
        if fourcc != RTS0 {
            return Err(Error::DxbcForbiddenPart);
        }
        if rts0.is_some() {
            return Err(Error::DxbcMalformed);
        }
        let payload = &bytes[fourcc_end..data_end];
        rts0 = Some(canonicalize_root_signature(payload, max_bytes)?);
    }
    rts0.ok_or(Error::DxbcMalformed)
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let end = offset.checked_add(4).ok_or(Error::DxbcMalformed)?;
    let slice = bytes.get(offset..end).ok_or(Error::DxbcMalformed)?;
    let arr: [u8; 4] = slice.try_into().map_err(|_| Error::DxbcMalformed)?;
    Ok(u32::from_le_bytes(arr))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::rootsig::empty_root_signature_v1_0;

    const MARKER: &[u8] = b"DXIL_MARKER_SHOULD_NOT_LEAK_9f3a";

    fn dxbc_with_parts(parts: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let header_len = 32;
        let offsets_len = parts.len() * 4;
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        let mut cursor = header_len + offsets_len;
        for (fcc, data) in parts {
            offsets.push(cursor as u32);
            body.extend_from_slice(*fcc);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            cursor += 8 + data.len();
        }
        let file_size = (header_len + offsets_len + body.len()) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"DXBC");
        out.extend_from_slice(&[0_u8; 16]);
        out.extend_from_slice(&1_u32.to_le_bytes());
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&(parts.len() as u32).to_le_bytes());
        for off in offsets {
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend(body);
        out
    }

    fn reject_marker(err: Error) {
        let text = format!("{err}{err:?}");
        assert!(!text.contains("DXIL_MARKER"));
        assert!(!text.contains("9f3a"));
    }

    #[test]
    fn raw_size_cap_rejects_oversized_valid_signature() {
        let bytes = empty_root_signature_v1_0();
        let err = sanitize_root_signature(&bytes, 10).unwrap_err();
        assert!(
            matches!(err, Error::RootSignatureTooLarge { max: 10, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn file_size_must_equal_buffer_len() {
        let rs = empty_root_signature_v1_0();
        let mut blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
        blob.push(0);
        let err = sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES).unwrap_err();
        assert!(matches!(err, Error::DxbcMalformed), "{err:?}");
    }

    #[test]
    fn parts_are_checked_against_header_file_size() {
        let rs = empty_root_signature_v1_0();
        let mut blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
        let declared = u32::from_le_bytes(blob[24..28].try_into().unwrap());
        blob[24..28].copy_from_slice(&(declared.saturating_sub(8)).to_le_bytes());
        let err = sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES).unwrap_err();
        assert!(matches!(err, Error::DxbcMalformed), "{err:?}");
    }

    #[test]
    fn rts0_payload_must_be_a_root_signature() {
        let mut payload = vec![1, 0, 0, 0];
        payload.extend_from_slice(MARKER);
        let blob = dxbc_with_parts(&[(b"RTS0", &payload)]);
        let err = sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES).unwrap_err();
        reject_marker(err);
    }

    #[test]
    fn rts0_v99_is_rejected() {
        let mut payload = vec![99, 0, 0, 0];
        payload.extend_from_slice(MARKER);
        let blob = dxbc_with_parts(&[(b"RTS0", &payload)]);
        let err = sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES).unwrap_err();
        reject_marker(err);
    }

    #[test]
    fn rts0_only_emits_canonical_payload() {
        let rs = empty_root_signature_v1_0();
        let blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
        let out = sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES).unwrap();
        assert_eq!(out, rs);
    }

    #[test]
    fn spec_samples_sanitize_raw_and_rts0() {
        let samples = [
            crate::empty_root_signature_v1_0(),
            crate::empty_root_signature_v1_1(),
            crate::one_cbv_root_signature_v1_0(),
            crate::one_constants_root_signature_v1_0(),
            crate::one_cbv_root_signature_v1_1(),
            crate::table_three_ranges_v1_0(),
            crate::one_table_root_signature_v1_1(),
            crate::table_two_ranges_v1_1(),
            crate::one_sampler_border4_v1_0(),
            crate::empty_root_signature_v1_0_heap_flag(),
        ];
        for rs in samples {
            assert_eq!(
                sanitize_root_signature(&rs, MAX_ROOT_SIGNATURE_BYTES).unwrap(),
                rs
            );
            let blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
            assert_eq!(
                sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES).unwrap(),
                rs
            );
        }
    }
}
