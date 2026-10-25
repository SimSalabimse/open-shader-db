// SPDX-License-Identifier: MIT

//! Human preview rendered from the same canonical bytes that would be sent.

use std::fmt;
use std::fmt::Write as _;

use crate::manifest::UploadManifest;
use crate::{Error, Limits};

/// Notice printed by the CLI and included in every preview.
pub const NOTHING_SENT_LINE: &str = "Nothing is sent by this command. There is no network upload.";

/// Statement that the following JSON is the wire payload.
pub const THESE_EXACT_BYTES_LINE: &str = "These exact bytes will be sent.";

/// Plain-language ownership warning for the unchallenged state.
pub const UNCHALLENGED_LINE: &str =
    "Ownership is unchallenged (no_server_challenge). This field proves nothing. UNVERIFIED until a server challenge exists.";

/// Preview of an upload manifest, produced from canonical JSON bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadPreview {
    canonical: Vec<u8>,
    text: String,
}

impl fmt::Debug for UploadPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadPreview")
            .field("byte_len", &self.canonical.len())
            .field("text_len", &self.text.len())
            .finish()
    }
}

impl UploadPreview {
    /// Encode `manifest` and render the preview from those bytes.
    pub fn from_manifest(manifest: &UploadManifest) -> Result<Self, Error> {
        Self::from_manifest_with_limits(manifest, Limits::DEFAULT)
    }

    /// Encode with caller-supplied caps.
    pub fn from_manifest_with_limits(
        manifest: &UploadManifest,
        limits: Limits,
    ) -> Result<Self, Error> {
        let canonical = manifest.to_canonical_bytes_with_limits(limits)?;
        Self::from_canonical_bytes_with_limits(canonical, limits)
    }

    /// Render a preview from bytes that are already the wire payload.
    pub fn from_canonical_bytes(canonical: Vec<u8>) -> Result<Self, Error> {
        Self::from_canonical_bytes_with_limits(canonical, Limits::DEFAULT)
    }

    /// Render from canonical bytes with caller-supplied parse caps.
    pub fn from_canonical_bytes_with_limits(
        canonical: Vec<u8>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let parsed = UploadManifest::parse_with_limits(&canonical, limits)?;
        let text = render_preview(&canonical, &parsed)?;
        Ok(Self { canonical, text })
    }

    /// The exact bytes that would be sent (and written by `--out`).
    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// Human-readable preview, including the full canonical JSON.
    #[must_use]
    pub fn as_text(&self) -> &str {
        &self.text
    }
}

fn render_preview(canonical: &[u8], parsed: &UploadManifest) -> Result<String, Error> {
    let value: serde_json::Value =
        serde_json::from_slice(canonical).map_err(|_| Error::InvalidJson)?;
    let mut text = String::new();
    writeln!(text, "Upload preview ({} bytes)", canonical.len())
        .map_err(|_| Error::CanonicalJson)?;
    writeln!(text, "{THESE_EXACT_BYTES_LINE}").map_err(|_| Error::CanonicalJson)?;
    writeln!(text, "{NOTHING_SENT_LINE}").map_err(|_| Error::CanonicalJson)?;
    writeln!(text).map_err(|_| Error::CanonicalJson)?;
    writeln!(text, "Fields that will be sent:").map_err(|_| Error::CanonicalJson)?;
    write_fields(&mut text, "", &value)?;
    writeln!(text).map_err(|_| Error::CanonicalJson)?;
    match &parsed.ownership {
        crate::ownership::OwnershipProof::Unchallenged { .. } => {
            writeln!(text, "{UNCHALLENGED_LINE}").map_err(|_| Error::CanonicalJson)?;
        }
        crate::ownership::OwnershipProof::ExeBindingV1 { .. } => {
            writeln!(
                text,
                "Ownership is exe_binding_v1. Server verification is UNVERIFIED."
            )
            .map_err(|_| Error::CanonicalJson)?;
        }
    }
    writeln!(text).map_err(|_| Error::CanonicalJson)?;
    writeln!(text, "Canonical JSON:").map_err(|_| Error::CanonicalJson)?;
    let json = std::str::from_utf8(canonical).map_err(|_| Error::InvalidJson)?;
    text.push_str(json);
    text.push('\n');
    Ok(text)
}

fn write_fields(out: &mut String, prefix: &str, value: &serde_json::Value) -> Result<(), Error> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match child {
                    serde_json::Value::Object(_) => write_fields(out, &path, child)?,
                    serde_json::Value::Array(items) => {
                        writeln!(out, "  {path} ({})", items.len())
                            .map_err(|_| Error::CanonicalJson)?;
                    }
                    other => {
                        writeln!(out, "  {path}: {}", format_scalar(other))
                            .map_err(|_| Error::CanonicalJson)?;
                    }
                }
            }
            Ok(())
        }
        _ => Err(Error::InvalidJson),
    }
}

fn format_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::from("null"),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => String::from("<nested>"),
    }
}
