// SPDX-License-Identifier: MIT

//! SHA-256 helpers. Bytecode is hashed and then dropped.

use std::io::Read;

use sha2::{Digest, Sha256};

use crate::Sha256Hash;

/// Fixed buffer used by the streaming shader-container hasher.
pub const SHADER_HASH_BUFFER: usize = 8192;

/// SHA-256 of shader container bytes (the whole blob, as stored).
///
/// This is the same definition `records extract` uses for
/// `shader_bytecode.Bytecode`. Callers that hash a file or a SQLite blob
/// must stream through [`hash_shader_reader`] so the definition cannot drift.
#[must_use]
pub fn hash_shader_container(bytes: &[u8]) -> Sha256Hash {
    let mut hasher = StreamingShaderHasher::new();
    hasher.update(bytes);
    hasher.finish()
}

/// SHA-256 of bytes read from `reader` using a fixed buffer.
///
/// Used for SODB blob I/O and for loose DXBC files so both paths share one
/// hasher. Does not retain the bytes.
pub fn hash_shader_reader<R: Read>(reader: &mut R) -> std::io::Result<Sha256Hash> {
    let mut hasher = StreamingShaderHasher::new();
    let mut buf = [0_u8; SHADER_HASH_BUFFER];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish())
}

/// Incremental SHA-256 of shader container bytes.
pub struct StreamingShaderHasher {
    inner: Sha256,
}

impl StreamingShaderHasher {
    /// Start a new container hash.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    /// Absorb the next chunk.
    pub fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    /// Finish the digest.
    #[must_use]
    pub fn finish(self) -> Sha256Hash {
        Sha256Hash::from_bytes(self.inner.finalize().into())
    }
}

impl Default for StreamingShaderHasher {
    fn default() -> Self {
        Self::new()
    }
}

fn hex_digit(nibble: u8) -> char {
    let nibble = nibble & 0x0f;
    if nibble < 10 {
        char::from(b'0' + nibble)
    } else {
        char::from(b'a' + (nibble - 10))
    }
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

pub(crate) fn sha256(bytes: &[u8]) -> Sha256Hash {
    hash_shader_container(bytes)
}

/// Length-prefixed canonical hasher for description hashes.
pub(crate) struct CanonicalHasher {
    inner: Sha256,
}

impl CanonicalHasher {
    pub(crate) fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    /// Domain-separated tag. Length-prefixed like [`Self::bytes`].
    pub(crate) fn tag(&mut self, tag: &[u8]) {
        self.inner.update([0x1f]);
        self.bytes(tag);
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.inner.update((bytes.len() as u64).to_le_bytes());
        self.inner.update(bytes);
    }

    pub(crate) fn feed_null(&mut self) {
        self.inner.update([0]);
    }

    pub(crate) fn feed_text(&mut self, bytes: &[u8]) {
        self.inner.update([4]);
        self.bytes(bytes);
    }

    pub(crate) fn feed_blob(&mut self, bytes: &[u8]) {
        self.inner.update([5]);
        self.bytes(bytes);
    }

    pub(crate) fn opt_i64(&mut self, value: Option<i64>) {
        match value {
            Some(v) => {
                self.inner.update([2]);
                self.inner.update(v.to_le_bytes());
            }
            None => self.inner.update([0]),
        }
    }

    pub(crate) fn opt_f64(&mut self, value: Option<f64>) {
        match value {
            Some(v) => {
                self.inner.update([3]);
                self.inner.update(v.to_le_bytes());
            }
            None => self.inner.update([0]),
        }
    }

    pub(crate) fn finish(self) -> Sha256Hash {
        Sha256Hash::from_bytes(self.inner.finalize().into())
    }
}
