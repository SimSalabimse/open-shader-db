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

/// Byte offset of a container inside a source. For an SODB this is the rowid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileOffset(pub(crate) u64);

impl FileOffset {
    /// Integer offset (SODB rowid, or 0 for a loose file).
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

/// Where a hashed shader can be reopened. Bytes are not stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderLocation {
    pub source: SourceId,
    pub offset: FileOffset,
    pub len: ByteLen,
}
