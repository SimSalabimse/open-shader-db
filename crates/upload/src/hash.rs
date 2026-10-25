// SPDX-License-Identifier: MIT

//! SHA-256 hex newtype. Constructors accept 64 lowercase hex characters only.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::Serialize;

use crate::Error;

/// SHA-256 digest. Wire form is exactly 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Hex([u8; 32]);

impl Sha256Hex {
    /// Wrap a finished 32-byte digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parse a 64-character lowercase hex SHA-256.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Ok(Self(parse_hex32(text)?))
    }

    /// Copy a digest already validated by `osdb-records`.
    #[must_use]
    pub fn from_records(hash: osdb_records::Sha256) -> Self {
        Self(*hash.as_bytes())
    }

    /// Raw 32-byte digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex encoding (64 characters).
    #[must_use]
    pub fn to_hex(&self) -> String {
        encode_hex(&self.0)
    }
}

impl fmt::Debug for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Sha256Hex {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for Sha256Hex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(Sha256Visitor)
    }
}

struct Sha256Visitor;

impl Visitor<'_> for Sha256Visitor {
    type Value = Sha256Hex;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a 64-character lowercase hex SHA-256 string")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Sha256Hex::parse(value).map_err(E::custom)
    }
}

pub(crate) fn parse_hex32(text: &str) -> Result<[u8; 32], Error> {
    if text.len() != 64 {
        return Err(Error::WrongHexLength { got: text.len() });
    }
    let bytes = text.as_bytes();
    let mut out = [0_u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = (hex_nibble(bytes[i * 2])? << 4) | hex_nibble(bytes[i * 2 + 1])?;
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

fn encode_hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        let hi = byte >> 4;
        let lo = byte & 0x0f;
        out.push(hex_digit(hi));
        out.push(hex_digit(lo));
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    if nibble < 10 {
        char::from(b'0' + nibble)
    } else {
        char::from(b'a' + (nibble - 10))
    }
}
