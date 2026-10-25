// SPDX-License-Identifier: MIT

//! Optional user-supplied build labels. Never install paths.

use std::fmt;

use serde::Serialize;

use crate::hash::{parse_hex32, Sha256};
use crate::Error;

/// Stable game identifier (for example `cyberpunk2077`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GameId(String);

impl GameId {
    /// Validate a game id: non-empty, no path separators or ASCII whitespace.
    pub fn parse(text: &str) -> Result<Self, Error> {
        validate_ident(text)?;
        Ok(Self(text.to_string()))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Publisher or storefront version string (for example `2.13.0`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GameVersion(String);

impl GameVersion {
    /// Validate a version string: non-empty, no path separators or ASCII whitespace.
    pub fn parse(text: &str) -> Result<Self, Error> {
        validate_ident(text)?;
        Ok(Self(text.to_string()))
    }

    /// Version text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GameVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// SHA-256 of a game executable, encoded as 64-character lowercase hex on disk.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExeHash(Sha256);

impl ExeHash {
    /// Parse a 64-character lowercase hex SHA-256.
    pub fn parse_hex(text: &str) -> Result<Self, Error> {
        let _ = parse_hex32(text)?;
        Ok(Self(Sha256::parse_hex(text)?))
    }

    /// Lowercase hex encoding.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// The digest.
    #[must_use]
    pub const fn as_sha256(self) -> Sha256 {
        self.0
    }
}

impl fmt::Debug for ExeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for ExeHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// Optional labels supplied by the user. Missing fields serialize as JSON null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildIdentity {
    pub game_id: Option<GameId>,
    pub version: Option<GameVersion>,
    pub exe_hash: Option<ExeHash>,
}

impl BuildIdentity {
    /// Empty labels (all fields omitted by the user).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            game_id: None,
            version: None,
            exe_hash: None,
        }
    }

    /// Construct from already-validated fields.
    #[must_use]
    pub fn new(
        game_id: Option<GameId>,
        version: Option<GameVersion>,
        exe_hash: Option<ExeHash>,
    ) -> Self {
        Self {
            game_id,
            version,
            exe_hash,
        }
    }
}

pub(crate) fn validate_ident(text: &str) -> Result<(), Error> {
    if text.is_empty()
        || text.len() > 128
        || text.bytes().any(|b| {
            b == b'/'
                || b == b'\\'
                || b == b':'
                || b.is_ascii_whitespace()
                || b == 0
                || !(b.is_ascii_graphic())
        })
    {
        return Err(Error::InvalidBuildId);
    }
    Ok(())
}
