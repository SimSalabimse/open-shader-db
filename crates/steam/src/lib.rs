// SPDX-License-Identifier: MIT

//! Steam library discovery with install-path isolation.
//!
//! # Path isolation
//!
//! [`LocalPath`] wraps a game install or SODB path and does **not** implement
//! [`serde::Serialize`] or [`std::fmt::Display`]. [`Debug`] prints a redacted
//! placeholder. Public upload-safe data is [`GameIdentity`] and
//! [`UploadSafeDiscovery`]. [`InstalledGame`] and [`Discovery`] are not
//! serializable. See the `serialized_payload_never_contains_paths` test.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod discover;
mod error;
mod roots;
mod scan;
mod types;
mod vdf;

pub use discover::{discover, discover_with_scan};
pub use error::Error;
pub use roots::{default_steam_roots, discover_default};
pub use scan::{
    ScanLimits, DEFAULT_MAX_DURATION, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_FILES, DEFAULT_SODB_DEPTH,
};
pub use types::{
    AppId, BuildId, Discovery, GameIdentity, GameListing, GameName, InstalledGame, LocalPath,
    UploadSafeDiscovery, Warning, WarningKind,
};
pub use vdf::{
    parse_vdf, parse_vdf_file, VdfDocument, VdfValue, MAX_VDF_BYTES, MAX_VDF_DEPTH, MAX_VDF_TOKENS,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
