// SPDX-License-Identifier: MIT

//! SHA-256 newtype. Construct and Deserialize accept lowercase hex only.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::Serialize;

use crate::Error;

/// SHA-256 digest. On-disk form is exactly 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256(sodb::Sha256Hash);

impl Sha256 {
    /// Wrap a finished 32-byte digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(sodb::Sha256Hash::from_bytes(bytes))
    }

    /// Wrap a parser digest.
    #[must_use]
    pub const fn from_sodb(hash: sodb::Sha256Hash) -> Self {
        Self(hash)
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

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Sha256 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(Sha256Visitor)
    }
}

struct Sha256Visitor;

impl Visitor<'_> for Sha256Visitor {
    type Value = Sha256;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a 64-character lowercase hex SHA-256 string")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Sha256::parse_hex(value).map_err(E::custom)
    }
}

impl From<sodb::Sha256Hash> for Sha256 {
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

pub(crate) fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, Error> {
    if !text.len().is_multiple_of(2) {
        return Err(Error::OddHexLength { got: text.len() });
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let (chunks, _) = text.as_bytes().as_chunks::<2>();
    for chunk in chunks {
        out.push((hex_nibble(chunk[0])? << 4) | hex_nibble(chunk[1])?);
    }
    Ok(out)
}

pub(crate) fn hex_nibble(byte: u8) -> Result<u8, Error> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(Error::MalformedHex),
    }
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
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
