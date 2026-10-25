// SPDX-License-Identifier: MIT

//! Game build identification. Field text is identifiers, never install paths.

use std::fmt;

use serde::Serialize;

use crate::hash::parse_hex32;
use crate::Error;

/// SHA-256 of a game executable, encoded as 64-character lowercase hex on disk.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExeHash([u8; 32]);

impl ExeHash {
    /// Parse a 64-character lowercase hex SHA-256.
    pub fn parse_hex(text: &str) -> Result<Self, Error> {
        Ok(Self(parse_hex32(text)?))
    }

    /// Lowercase hex encoding.
    #[must_use]
    pub fn to_hex(&self) -> String {
        DescriptionHex(&self.0).to_string()
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

/// Uniquely identifies a game build (`game_id` + `version` + `exe_hash`).
///
/// [`GameBuild`] is the same type (`type GameBuild = BuildId`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct BuildId {
    pub game_id: GameId,
    pub version: GameVersion,
    pub exe_hash: ExeHash,
}

/// Alias used in some architecture sketches.
pub type GameBuild = BuildId;

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

fn validate_ident(text: &str) -> Result<(), Error> {
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

struct DescriptionHex<'a>(&'a [u8; 32]);

impl fmt::Display for DescriptionHex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
