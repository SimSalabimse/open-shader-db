// SPDX-License-Identifier: MIT

//! Anonymized upload manifest and local preview. This crate has no network client.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod build;
mod builder;
mod client;
mod error;
mod hash;
mod limits;
mod manifest;
mod ownership;
mod preview;
mod schema;
mod time;

pub use build::{BuildId, ExeHash, GameId, GameVersion};
pub use builder::ManifestBuilder;
pub use client::ClientVersion;
pub use error::Error;
pub use hash::Sha256Hex;
pub use limits::Limits;
pub use manifest::{parse_with_limits, HashSet, UploadManifest, SCHEMA_VERSION};
pub use ownership::{OwnershipProof, UnchallengedReason};
pub use preview::{UploadPreview, NOTHING_SENT_LINE, THESE_EXACT_BYTES_LINE, UNCHALLENGED_LINE};
pub use schema::{
    document as schema_document, BUILD_ID_FIELDS, CLIENT_VERSION_PATTERN, IDENT_PATTERN,
    MANIFEST_FIELDS, SCHEMA_JSON, SHA256_HEX_PATTERN, UTC_HOUR_PATTERN,
};
pub use time::{Clock, FixedClock, SystemClock, UtcHour};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
