// SPDX-License-Identifier: MIT

//! Bounds-checked DXBC container parse used only for root signatures.

use serde::{Deserialize, Serialize};

use crate::rootsig::parse_root_signature;
use crate::Error;

/// Maximum emitted root-signature bytes (canonical serialized form).
pub const MAX_ROOT_SIGNATURE_BYTES: usize = 1024 * 1024;

const DXBC_MAGIC: &[u8; 4] = b"DXBC";
const RTS0: &[u8; 4] = b"RTS0";
const HEADER_LEN: usize = 32;

/// Why a referenced root signature was not copied into the record file.
///
/// On-disk codes are `unsupported_version`, `invalid`, and
/// `unsupported_container`. They never include bytes or content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootSignatureSkipReason {
    /// Version field is not D3D12 1.0 or 1.1 (includes 1.2 / `Version = 3`).
    UnsupportedVersion,
    /// Structurally invalid, over cap, or a container with no usable RTS0.
    Invalid,
    /// Starts with `DXBC` but the container header cannot be walked.
    UnsupportedContainer,
}

impl RootSignatureSkipReason {
    /// Stable on-disk reason code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedVersion => "unsupported_version",
            Self::Invalid => "invalid",
            Self::UnsupportedContainer => "unsupported_container",
        }
    }

    /// Parse a documented reason code. Unknown strings are `None`.
    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "unsupported_version" => Some(Self::UnsupportedVersion),
            "invalid" => Some(Self::Invalid),
            "unsupported_container" => Some(Self::UnsupportedContainer),
            _ => None,
        }
    }
}

/// Result of sanitizing a `root_signatures.value` blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SanitizedRootSignature {
    /// Canonical DXC-order serialization.
    Canonical(Vec<u8>),
    /// Do not copy the blob; the referencing record stores a reason code.
    Skipped(RootSignatureSkipReason),
}

/// Sanitize a `root_signatures.value` blob.
///
/// A DXBC container may contain other parts (DXIL, …). Only the RTS0
/// payload is parsed; other parts are ignored and never copied. Two RTS0
/// parts is invalid. Version 1.2 (`Version = 3`) is
/// `unsupported_version` — DXC does not serialize that layout.
pub fn sanitize_root_signature(bytes: &[u8], max_bytes: usize) -> SanitizedRootSignature {
    if bytes.len() >= 4 && bytes.starts_with(DXBC_MAGIC) {
        return extract_rts0_only(bytes, max_bytes);
    }
    canonicalize_root_signature(bytes, max_bytes)
}

fn canonicalize_root_signature(bytes: &[u8], max_bytes: usize) -> SanitizedRootSignature {
    if bytes.len() as u64 > max_bytes as u64 {
        return SanitizedRootSignature::Skipped(RootSignatureSkipReason::Invalid);
    }
    if bytes.len() < 4 {
        return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedVersion);
    }
    match parse_root_signature(bytes) {
        Ok(parsed) => {
            let out = parsed.serialize();
            if out.len() > max_bytes {
                SanitizedRootSignature::Skipped(RootSignatureSkipReason::Invalid)
            } else {
                SanitizedRootSignature::Canonical(out)
            }
        }
        Err(Error::RootSignatureBadVersion) => {
            SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedVersion)
        }
        Err(_) => SanitizedRootSignature::Skipped(RootSignatureSkipReason::Invalid),
    }
}

fn extract_rts0_only(bytes: &[u8], max_bytes: usize) -> SanitizedRootSignature {
    if bytes.len() < HEADER_LEN {
        return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
    }
    let file_size = match u32_at(bytes, 24) {
        Ok(v) => v as usize,
        Err(()) => {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer)
        }
    };
    if file_size < HEADER_LEN || file_size > bytes.len() || file_size != bytes.len() {
        return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
    }
    let part_count = match u32_at(bytes, 28) {
        Ok(v) => v as usize,
        Err(()) => {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer)
        }
    };
    let Some(offsets_end) = part_count
        .checked_mul(4)
        .and_then(|n| HEADER_LEN.checked_add(n))
    else {
        return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
    };
    if offsets_end > file_size {
        return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
    }
    let mut rts0 = None;
    for i in 0..part_count {
        let off = match u32_at(bytes, HEADER_LEN + i * 4) {
            Ok(v) => v as usize,
            Err(()) => {
                return SanitizedRootSignature::Skipped(
                    RootSignatureSkipReason::UnsupportedContainer,
                )
            }
        };
        let Some(fourcc_end) = off.checked_add(8) else {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
        };
        if fourcc_end > file_size {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
        }
        let fourcc = &bytes[off..off + 4];
        let size = match u32_at(bytes, off + 4) {
            Ok(v) => v as usize,
            Err(()) => {
                return SanitizedRootSignature::Skipped(
                    RootSignatureSkipReason::UnsupportedContainer,
                )
            }
        };
        let Some(data_end) = fourcc_end.checked_add(size) else {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
        };
        if data_end > file_size {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::UnsupportedContainer);
        }
        if fourcc != RTS0 {
            continue;
        }
        if rts0.is_some() {
            return SanitizedRootSignature::Skipped(RootSignatureSkipReason::Invalid);
        }
        let payload = &bytes[fourcc_end..data_end];
        rts0 = Some(canonicalize_root_signature(payload, max_bytes));
    }
    match rts0 {
        Some(outcome) => outcome,
        None => SanitizedRootSignature::Skipped(RootSignatureSkipReason::Invalid),
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, ()> {
    let end = offset.checked_add(4).ok_or(())?;
    let slice = bytes.get(offset..end).ok_or(())?;
    let arr: [u8; 4] = slice.try_into().map_err(|_| ())?;
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

    fn skip_reason(out: SanitizedRootSignature) -> RootSignatureSkipReason {
        match out {
            SanitizedRootSignature::Skipped(reason) => reason,
            SanitizedRootSignature::Canonical(_) => panic!("expected skip"),
        }
    }

    fn assert_no_marker(reason: RootSignatureSkipReason) {
        let text = format!("{reason:?}{}", reason.as_str());
        assert!(!text.contains("DXIL_MARKER"));
        assert!(!text.contains("9f3a"));
    }

    #[test]
    fn raw_size_cap_skips_oversized_valid_signature() {
        let bytes = empty_root_signature_v1_0();
        assert_eq!(
            skip_reason(sanitize_root_signature(&bytes, 10)),
            RootSignatureSkipReason::Invalid
        );
    }

    #[test]
    fn file_size_must_equal_buffer_len() {
        let rs = empty_root_signature_v1_0();
        let mut blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
        blob.push(0);
        assert_eq!(
            skip_reason(sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES)),
            RootSignatureSkipReason::UnsupportedContainer
        );
    }

    #[test]
    fn parts_are_checked_against_header_file_size() {
        let rs = empty_root_signature_v1_0();
        let mut blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
        let declared = u32::from_le_bytes(blob[24..28].try_into().unwrap());
        blob[24..28].copy_from_slice(&(declared.saturating_sub(8)).to_le_bytes());
        assert_eq!(
            skip_reason(sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES)),
            RootSignatureSkipReason::UnsupportedContainer
        );
    }

    #[test]
    fn rts0_payload_must_be_a_root_signature() {
        let mut payload = vec![1, 0, 0, 0];
        payload.extend_from_slice(MARKER);
        let blob = dxbc_with_parts(&[(b"RTS0", &payload)]);
        let reason = skip_reason(sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES));
        assert_eq!(reason, RootSignatureSkipReason::Invalid);
        assert_no_marker(reason);
    }

    #[test]
    fn rts0_v99_is_unsupported_version() {
        let mut payload = vec![99, 0, 0, 0];
        payload.extend_from_slice(MARKER);
        let blob = dxbc_with_parts(&[(b"RTS0", &payload)]);
        let reason = skip_reason(sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES));
        assert_eq!(reason, RootSignatureSkipReason::UnsupportedVersion);
        assert_no_marker(reason);
    }

    #[test]
    fn version_1_2_is_unsupported_version() {
        let mut bytes = empty_root_signature_v1_0();
        bytes[0..4].copy_from_slice(&3_u32.to_le_bytes());
        assert_eq!(
            skip_reason(sanitize_root_signature(&bytes, MAX_ROOT_SIGNATURE_BYTES)),
            RootSignatureSkipReason::UnsupportedVersion
        );
    }

    #[test]
    fn rts0_only_emits_canonical_payload() {
        let rs = empty_root_signature_v1_0();
        let blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
        match sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES) {
            SanitizedRootSignature::Canonical(out) => assert_eq!(out, rs),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rts0_plus_dxil_extracts_rts0_and_drops_marker() {
        let rs = empty_root_signature_v1_0();
        let blob = dxbc_with_parts(&[(b"RTS0", &rs), (b"DXIL", MARKER)]);
        match sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES) {
            SanitizedRootSignature::Canonical(out) => {
                assert_eq!(out, rs);
                let text = format!("{out:?}");
                assert!(!text.contains("DXIL_MARKER"));
                assert!(!out.windows(MARKER.len()).any(|w| w == MARKER));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn two_rts0_parts_are_invalid() {
        let rs = empty_root_signature_v1_0();
        let blob = dxbc_with_parts(&[(b"RTS0", &rs), (b"RTS0", &rs)]);
        assert_eq!(
            skip_reason(sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES)),
            RootSignatureSkipReason::Invalid
        );
    }

    #[test]
    fn dxbc_without_rts0_is_invalid() {
        let blob = dxbc_with_parts(&[(b"DXIL", MARKER)]);
        let reason = skip_reason(sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES));
        assert_eq!(reason, RootSignatureSkipReason::Invalid);
        assert_no_marker(reason);
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
            match sanitize_root_signature(&rs, MAX_ROOT_SIGNATURE_BYTES) {
                SanitizedRootSignature::Canonical(out) => assert_eq!(out, rs),
                other => panic!("{other:?}"),
            }
            let blob = dxbc_with_parts(&[(b"RTS0", &rs)]);
            match sanitize_root_signature(&blob, MAX_ROOT_SIGNATURE_BYTES) {
                SanitizedRootSignature::Canonical(out) => assert_eq!(out, rs),
                other => panic!("{other:?}"),
            }
        }
    }
}
