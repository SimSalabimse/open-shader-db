// SPDX-License-Identifier: MIT

//! Game build identification. Field text is identifiers, never install paths.

use std::fmt;

use serde::Serialize;

use crate::hash::Sha256Hex;
use crate::Error;

/// SHA-256 of a game executable, encoded as 64-character lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExeHash(Sha256Hex);

impl ExeHash {
    /// Parse a 64-character lowercase hex SHA-256.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Ok(Self(Sha256Hex::parse(text)?))
    }

    /// Wrap a digest already validated as a SHA-256.
    #[must_use]
    pub const fn from_sha256(hash: Sha256Hex) -> Self {
        Self(hash)
    }

    /// Copy a digest already validated by `osdb-records`.
    pub fn from_records(hash: osdb_records::ExeHash) -> Self {
        Self(Sha256Hex::from_records(hash.as_sha256()))
    }

    /// Lowercase hex encoding.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// The digest.
    #[must_use]
    pub const fn as_sha256(self) -> Sha256Hex {
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

    /// Copy an identifier already validated by `osdb-records`.
    pub fn from_records(id: osdb_records::GameId) -> Result<Self, Error> {
        Self::parse(id.as_str())
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

    /// Copy a version already validated by `osdb-records`.
    pub fn from_records(version: osdb_records::GameVersion) -> Result<Self, Error> {
        Self::parse(version.as_str())
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

/// Uniquely identifies a game build (`game_id` + `version` + `exe_hash`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct BuildId {
    pub game_id: GameId,
    pub version: GameVersion,
    pub exe_hash: ExeHash,
}

impl BuildId {
    /// Construct from already-validated fields.
    #[must_use]
    pub fn new(game_id: GameId, version: GameVersion, exe_hash: ExeHash) -> Self {
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
        || text == "."
        || text == ".."
        || text.bytes().any(|b| {
            b == b'/'
                || b == b'\\'
                || b == b':'
                || b.is_ascii_whitespace()
                || b == 0
                || !b.is_ascii_graphic()
        })
    {
        return Err(Error::InvalidBuildId);
    }
    Ok(())
}
