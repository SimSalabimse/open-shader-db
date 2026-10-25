// SPDX-License-Identifier: MIT

//! Strong types for shader locations. No paths, no bytecode.

use std::fmt;

/// Opaque index of a scanned source. Absolute paths live only on the scanner.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub(crate) u32);

impl SourceId {
    /// Integer identity of the source (not a path).
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SourceId").field(&self.0).finish()
    }
}

/// What kind of source a [`SourceId`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderSourceKind {
    /// An SODB file the user already has.
    Sodb,
    /// One loose DXBC file found under a scanned directory.
    LooseFile,
}

/// Byte offset of a container inside a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileOffset(pub(crate) u64);

impl FileOffset {
    /// Integer offset (0 for a loose file).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Length of a stored shader container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteLen(pub(crate) u64);

impl ByteLen {
    /// Integer length in bytes.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How to reopen one stored shader. Bytes are not stored.
#[derive(Clone, PartialEq, Eq)]
pub enum ShaderLocator {
    /// Loose DXBC: byte offset and container length.
    Loose { offset: FileOffset, len: ByteLen },
    /// SODB table with an implicit rowid.
    SodbRowid { rowid: i64, len: ByteLen },
    /// SODB `WITHOUT ROWID` table: reopen by application key.
    SodbKey { key: Vec<u8>, len: ByteLen },
}

impl ShaderLocator {
    /// Stored container length.
    #[must_use]
    pub const fn len(&self) -> ByteLen {
        match *self {
            Self::Loose { len, .. } | Self::SodbRowid { len, .. } | Self::SodbKey { len, .. } => {
                len
            }
        }
    }
}

impl fmt::Debug for ShaderLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loose { offset, len } => f
                .debug_struct("Loose")
                .field("offset", offset)
                .field("len", len)
                .finish(),
            Self::SodbRowid { rowid, len } => f
                .debug_struct("SodbRowid")
                .field("rowid", rowid)
                .field("len", len)
                .finish(),
            Self::SodbKey { key, len } => f
                .debug_struct("SodbKey")
                .field("key_len", &key.len())
                .field("len", len)
                .finish(),
        }
    }
}

/// Where a hashed shader can be reopened. Bytes are not stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderLocation {
    pub source: SourceId,
    pub locator: ShaderLocator,
}

impl ShaderLocation {
    /// Loose-file location at offset 0.
    #[must_use]
    pub fn loose(source: SourceId, len: ByteLen) -> Self {
        Self {
            source,
            locator: ShaderLocator::Loose {
                offset: FileOffset(0),
                len,
            },
        }
    }
}
