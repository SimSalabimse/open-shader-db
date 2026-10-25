// SPDX-License-Identifier: MIT

//! In-memory shader index. Bytes are reopened and re-hashed on demand.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use osdb_records::Sha256;
use sodb::{hash_shader_container, parse_dxbc_header, starts_with_dxbc};

use crate::scan::open_regular_nonblocking;
use crate::types::{ShaderLocation, ShaderSourceKind, SourceId};
use crate::{Error, ScanReport};

pub(crate) struct SourceSlot {
    pub id: SourceId,
    pub kind: ShaderSourceKind,
    pub path: PathBuf,
}

/// Map from SHA-256 to one or more reopenable locations. No bytecode stored.
pub struct LocalShaderIndex {
    pub(crate) sources: Vec<SourceSlot>,
    pub(crate) directory_roots: Vec<PathBuf>,
    pub(crate) by_hash: BTreeMap<Sha256, Vec<ShaderLocation>>,
}

impl std::fmt::Debug for LocalShaderIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalShaderIndex")
            .field("sources", &self.sources.len())
            .field("unique_hashes", &self.by_hash.len())
            .finish()
    }
}

impl LocalShaderIndex {
    pub(crate) fn new() -> Self {
        Self {
            sources: Vec::new(),
            directory_roots: Vec::new(),
            by_hash: BTreeMap::new(),
        }
    }

    /// Number of distinct hashes.
    #[must_use]
    pub fn unique_len(&self) -> usize {
        self.by_hash.len()
    }

    /// Whether this hash was indexed.
    #[must_use]
    pub fn contains(&self, hash: &Sha256) -> bool {
        self.by_hash.contains_key(hash)
    }

    /// Locations stored for `hash` (empty slice when unknown).
    #[must_use]
    pub fn locations(&self, hash: &Sha256) -> &[ShaderLocation] {
        self.by_hash.get(hash).map_or(&[], Vec::as_slice)
    }

    /// Iterate indexed hashes in sorted order.
    pub fn hashes(&self) -> impl Iterator<Item = Sha256> + '_ {
        self.by_hash.keys().copied()
    }

    /// Kind of a source id, if it belongs to this index.
    #[must_use]
    pub fn source_kind(&self, id: SourceId) -> Option<ShaderSourceKind> {
        self.sources.iter().find(|s| s.id == id).map(|s| s.kind)
    }

    /// Absolute source paths. For same-file checks only; never print these.
    pub fn source_paths(&self) -> impl Iterator<Item = &Path> {
        self.sources.iter().map(|s| s.path.as_path())
    }

    /// Directory roots that were scanned. For same-file checks only.
    pub fn directory_roots(&self) -> impl Iterator<Item = &Path> {
        self.directory_roots.iter().map(PathBuf::as_path)
    }

    /// Reopen the first location of `hash`, re-verify hash and DXBC header.
    pub fn read_verified(&self, hash: &Sha256) -> Result<Vec<u8>, Error> {
        let loc = self
            .locations(hash)
            .first()
            .copied()
            .ok_or(Error::ShaderChanged)?;
        self.read_location(&loc, hash)
    }

    /// Reopen one stored location and refuse silently-changed bytes.
    pub fn read_location(&self, loc: &ShaderLocation, expected: &Sha256) -> Result<Vec<u8>, Error> {
        let source = self
            .sources
            .iter()
            .find(|s| s.id == loc.source)
            .ok_or(Error::ShaderChanged)?;
        match source.kind {
            ShaderSourceKind::Sodb => read_sodb_blob(&source.path, loc, expected),
            ShaderSourceKind::LooseFile => read_loose_file(&source.path, loc, expected),
        }
    }

    pub(crate) fn add_source(&mut self, kind: ShaderSourceKind, path: PathBuf) -> SourceId {
        let id = SourceId(self.sources.len() as u32);
        self.sources.push(SourceSlot { id, kind, path });
        id
    }

    pub(crate) fn insert(
        &mut self,
        hash: Sha256,
        loc: ShaderLocation,
        report: &mut ScanReport,
        max_entries: u64,
    ) -> Result<(), Error> {
        let entries = self.by_hash.values().map(Vec::len).sum::<usize>() as u64;
        if entries >= max_entries {
            return Err(Error::CapExceeded {
                cap: crate::ScanCap::IndexEntries,
                max: max_entries,
                got: entries.saturating_add(1),
            });
        }
        let slots = self.by_hash.entry(hash).or_default();
        if !slots.is_empty() {
            report.duplicates = report.duplicates.saturating_add(1);
        }
        slots.push(loc);
        report.shaders_indexed = report.shaders_indexed.saturating_add(1);
        report.unique_hashes = self.by_hash.len() as u64;
        Ok(())
    }
}

fn read_sodb_blob(path: &Path, loc: &ShaderLocation, expected: &Sha256) -> Result<Vec<u8>, Error> {
    let expected_hash = expected.as_sodb();
    match sodb::read_bytecode_blob(
        path,
        loc.offset.get() as i64,
        expected_hash,
        loc.len.get(),
        sodb::Limits::DEFAULT,
    ) {
        Ok(bytes) => Ok(bytes),
        Err(_) => {
            let refs = sodb::list_bytecode_blobs(path)?;
            let found = refs
                .iter()
                .find(|r| r.hash == expected_hash && r.len == loc.len.get())
                .ok_or(Error::ShaderChanged)?;
            match found.rowid {
                Some(rowid) => Ok(sodb::read_bytecode_blob(
                    path,
                    rowid,
                    expected_hash,
                    loc.len.get(),
                    sodb::Limits::DEFAULT,
                )?),
                None => Ok(sodb::read_bytecode_blob_by_key(
                    path,
                    &found.key,
                    expected_hash,
                    loc.len.get(),
                    sodb::Limits::DEFAULT,
                )?),
            }
        }
    }
}

fn read_loose_file(path: &Path, loc: &ShaderLocation, expected: &Sha256) -> Result<Vec<u8>, Error> {
    let mut file = open_regular_nonblocking(path)?;
    let meta = file.metadata().map_err(|_| Error::Io)?;
    if !meta.is_file() {
        return Err(Error::NotARegularFile);
    }
    if meta.len() < loc.offset.get().saturating_add(loc.len.get()) {
        return Err(Error::ShaderChanged);
    }
    file.seek(SeekFrom::Start(loc.offset.get()))
        .map_err(|_| Error::Io)?;
    let mut limited = file.take(loc.len.get());
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).map_err(|_| Error::Io)?;
    if bytes.len() as u64 != loc.len.get() {
        return Err(Error::ShaderChanged);
    }
    if !starts_with_dxbc(&bytes) {
        return Err(Error::ShaderNotDxbc);
    }
    if parse_dxbc_header(&bytes).is_err() {
        return Err(Error::ShaderNotDxbc);
    }
    let hash = Sha256::from_sodb(hash_shader_container(&bytes));
    if hash != *expected {
        return Err(Error::ShaderHashMismatch);
    }
    Ok(bytes)
}
