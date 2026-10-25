// SPDX-License-Identifier: MIT

//! Description-hash newtype wrapping [`sodb::Sha256Hash`].

use std::fmt;

use serde::Serialize;

use crate::Error;

/// SHA-256 of a pipeline-state or state-object description, as produced by `sodb`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DescriptionHash(sodb::Sha256Hash);

impl DescriptionHash {
    /// Wrap a parser digest.
    #[must_use]
    pub const fn from_sodb(hash: sodb::Sha256Hash) -> Self {
        Self(hash)
    }

    /// Wrap a finished 32-byte digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(sodb::Sha256Hash::from_bytes(bytes))
    }

    /// Parse a 64-character lowercase hex SHA-256.
    pub fn parse_hex(text: &str) -> Result<Self, Error> {
        Ok(Self::from_bytes(parse_hex32(text)?))
    }

    /// Raw 32-byte digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// The inner `sodb` digest.
    #[must_use]
    pub const fn as_sodb(self) -> sodb::Sha256Hash {
        self.0
    }

    /// Lowercase hex encoding (64 characters).
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }
}

impl fmt::Debug for DescriptionHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for DescriptionHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for DescriptionHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl From<sodb::Sha256Hash> for DescriptionHash {
    fn from(hash: sodb::Sha256Hash) -> Self {
        Self::from_sodb(hash)
    }
}

pub(crate) fn parse_hex32(text: &str) -> Result<[u8; 32], Error> {
    if text.len() != 64 {
        return Err(Error::WrongHexLength { got: text.len() });
    }
    let mut out = [0_u8; 32];
    let (chunks, _) = text.as_bytes().as_chunks::<2>();
    for (slot, chunk) in out.iter_mut().zip(chunks) {
        *slot = (hex_nibble(chunk[0])? << 4) | hex_nibble(chunk[1])?;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, Error> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(Error::MalformedHex),
    }
}
