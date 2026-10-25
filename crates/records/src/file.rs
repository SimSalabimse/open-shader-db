// SPDX-License-Identifier: MIT

//! Record file types and hardened file loading.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Serialize;

use crate::build::{BuildIdentity, ExeHash, GameId, GameVersion};
use crate::types::{
    AppKeyHex, BlendDesc, DepthStencilDesc, HexBytes, InputElementDesc, PipelineStateRecord,
    RasterizerDesc, RenderTargetFormats, SampleDesc, StageHashes, StreamOutDesc,
    ViewInstancingDesc,
};
use crate::{Error, Limits};

/// On-disk schema version implemented by this crate.
pub const FORMAT_VERSION: u32 = 1;

thread_local! {
    static LOAD_LIMITS: Cell<Limits> = const { Cell::new(Limits::DEFAULT) };
    static LAST_TYPED_ERROR: RefCell<Option<Error>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static READ_REACHED: Cell<bool> = const { Cell::new(false) };
}

/// Validated record file. `format_version` is always `1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordFile {
    pub format_version: u32,
    pub build: BuildIdentity,
    pub records: Vec<PipelineStateRecord>,
}

impl RecordFile {
    /// Load `path` with [`Limits::DEFAULT`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        load_with_limits(path, Limits::DEFAULT)
    }

    /// Load `path` with caller-supplied resource caps.
    pub fn load_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<Self, Error> {
        load_with_limits(path, limits)
    }
}

/// Load a record file with default limits.
pub fn load(path: impl AsRef<Path>) -> Result<RecordFile, Error> {
    load_with_limits(path, Limits::DEFAULT)
}

/// Load a record file with caller-supplied resource caps.
pub fn load_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<RecordFile, Error> {
    let path = path.as_ref();
    reject_symlink_or_non_file(path)?;
    let mut file = open_regular_nonblocking(path)?;
    let opened = file.metadata().map_err(|_| Error::Io)?;
    let got = opened.len();
    if got > limits.max_file_bytes {
        return Err(Error::FileTooLarge {
            max: limits.max_file_bytes,
            got,
        });
    }
    let bytes = read_capped(&mut file, limits.max_file_bytes)?;
    parse_record_bytes(bytes, limits)
}

/// Open with `O_NONBLOCK` on Unix and require a regular file on the handle.
pub(crate) fn open_regular_nonblocking(path: &Path) -> Result<File, Error> {
    let file = open_record_file(path)?;
    let opened = file.metadata().map_err(|_| Error::Io)?;
    if !opened.is_file() {
        return Err(Error::NotARegularFile);
    }
    Ok(file)
}

/// Read at most `max_file_bytes + 1` bytes, then reject if the cap is exceeded.
pub(crate) fn read_capped(reader: &mut impl Read, max_file_bytes: u64) -> Result<Vec<u8>, Error> {
    #[cfg(test)]
    READ_REACHED.set(true);
    let mut bytes = Vec::new();
    let limit = max_file_bytes.saturating_add(1);
    reader
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
    let got = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if got > max_file_bytes {
        return Err(Error::FileTooLarge {
            max: max_file_bytes,
            got,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) fn reset_read_reached() {
    READ_REACHED.set(false);
}

#[cfg(test)]
pub(crate) fn take_read_reached() -> bool {
    READ_REACHED.replace(false)
}

fn reject_symlink_or_non_file(path: &Path) -> Result<(), Error> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(Error::NotARegularFile);
    }
    Ok(())
}

fn open_record_file(path: &Path) -> Result<File, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        File::options()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| Error::Io)
    }
    #[cfg(not(unix))]
    {
        File::open(path).map_err(|_| Error::Io)
    }
}

fn parse_record_bytes(bytes: Vec<u8>, limits: Limits) -> Result<RecordFile, Error> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(Error::ByteOrderMark);
    }
    let nesting = json_nesting_depth(&bytes);
    if nesting > limits.max_json_nesting {
        return Err(Error::NestingTooDeep {
            max: limits.max_json_nesting,
        });
    }
    LAST_TYPED_ERROR.with(|slot| slot.borrow_mut().take());
    LOAD_LIMITS.set(limits);
    let raw = {
        let mut de = serde_json::Deserializer::from_slice(&bytes);
        let raw: RawFile = serde::Deserialize::deserialize(&mut de).map_err(take_typed_error)?;
        de.end().map_err(|_| Error::TrailingGarbage)?;
        raw
    };
    drop(bytes);
    raw.into_file()
}

fn take_typed_error(_err: serde_json::Error) -> Error {
    LAST_TYPED_ERROR
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or(Error::InvalidJson)
}

fn store_typed<E: de::Error>(err: Error) -> E {
    LAST_TYPED_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(err);
    });
    E::custom("typed")
}

fn json_nesting_depth(bytes: &[u8]) -> u32 {
    let mut depth: u32 = 0;
    let mut max: u32 = 0;
    let mut in_str = false;
    let mut escape = false;
    for &byte in bytes {
        if in_str {
            if escape {
                escape = false;
                continue;
            }
            if byte == b'\\' {
                escape = true;
                continue;
            }
            if byte == b'"' {
                in_str = false;
            }
            continue;
        }
        match byte {
            b'"' => in_str = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

struct RawFile {
    format_version: u64,
    build: BuildIdentity,
    records: Vec<PipelineStateRecord>,
}

impl RawFile {
    fn into_file(self) -> Result<RecordFile, Error> {
        if self.format_version != u64::from(FORMAT_VERSION) {
            return Err(Error::UnsupportedFormatVersion {
                found: self.format_version,
                expected: FORMAT_VERSION,
            });
        }
        Ok(RecordFile {
            format_version: FORMAT_VERSION,
            build: self.build,
            records: self.records,
        })
    }
}

impl<'de> serde::Deserialize<'de> for RawFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawFileVisitor)
    }
}

struct RawFileVisitor;

impl<'de> Visitor<'de> for RawFileVisitor {
    type Value = RawFile;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a pipeline-state record file object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut format_version = None;
        let mut build = None;
        let mut records = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "format_version" => format_version = Some(map.next_value()?),
                "build" => {
                    let raw: RawBuild = map.next_value()?;
                    build = Some(raw.into_build().map_err(store_typed)?);
                }
                "records" => records = Some(map.next_value::<RawRecords>()?.0),
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        Ok(RawFile {
            format_version: format_version.ok_or_else(|| store_typed(Error::InvalidJson))?,
            build: build.ok_or_else(|| store_typed(Error::InvalidJson))?,
            records: records.ok_or_else(|| store_typed(Error::InvalidJson))?,
        })
    }
}

struct RawBuild {
    game_id: Option<String>,
    version: Option<String>,
    exe_hash: Option<String>,
}

impl RawBuild {
    fn into_build(self) -> Result<BuildIdentity, Error> {
        let game_id = match self.game_id {
            Some(text) => Some(GameId::parse(&text)?),
            None => None,
        };
        let version = match self.version {
            Some(text) => Some(GameVersion::parse(&text)?),
            None => None,
        };
        let exe_hash = match self.exe_hash {
            Some(text) => Some(ExeHash::parse_hex(&text)?),
            None => None,
        };
        Ok(BuildIdentity::new(game_id, version, exe_hash))
    }
}

impl<'de> serde::Deserialize<'de> for RawBuild {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawBuildVisitor)
    }
}

struct RawBuildVisitor;

impl<'de> Visitor<'de> for RawBuildVisitor {
    type Value = RawBuild;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a build identity object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut game_id = None;
        let mut version = None;
        let mut exe_hash = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "game_id" => game_id = map.next_value()?,
                "version" => version = map.next_value()?,
                "exe_hash" => exe_hash = map.next_value()?,
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        Ok(RawBuild {
            game_id,
            version,
            exe_hash,
        })
    }
}

struct RawRecords(Vec<PipelineStateRecord>);

impl<'de> serde::Deserialize<'de> for RawRecords {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(RawRecordsVisitor)
    }
}

struct RawRecordsVisitor;

impl<'de> Visitor<'de> for RawRecordsVisitor {
    type Value = RawRecords;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of pipeline-state records")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let max_records = LOAD_LIMITS.get().max_records;
        let mut records = Vec::new();
        if let Some(hint) = seq.size_hint() {
            let cap = usize::try_from(max_records).unwrap_or(usize::MAX);
            records.reserve(hint.min(cap));
        }
        while let Some(record) = seq.next_element::<RawRecord>()? {
            let got = u64::try_from(records.len().saturating_add(1)).unwrap_or(u64::MAX);
            if got > max_records {
                return Err(store_typed(Error::TooManyRecords {
                    max: max_records,
                    got,
                }));
            }
            records.push(record.into_record().map_err(store_typed)?);
        }
        Ok(RawRecords(records))
    }
}

struct RawRecord {
    app_key: String,
    version: u32,
    bytecode_hashes: StageHashes,
    root_signature: Option<String>,
    input_layout: Option<Vec<InputElementDesc>>,
    depth_stencil: Option<DepthStencilDesc>,
    blend: Option<BlendDesc>,
    rasterizer: Option<RasterizerDesc>,
    render_target_formats: Option<RenderTargetFormats>,
    view_instancing: Option<ViewInstancingDesc>,
    stream_out: Option<StreamOutDesc>,
    sample_desc: Option<SampleDesc>,
    sample_mask: Option<i64>,
    ib_strip_cut_value: Option<i64>,
    primitive_topology: Option<i64>,
    dsv_format: Option<i64>,
    node_mask: Option<i64>,
    flags: Option<i64>,
}

impl RawRecord {
    fn into_record(self) -> Result<PipelineStateRecord, Error> {
        Ok(PipelineStateRecord {
            app_key: AppKeyHex::parse_hex(&self.app_key)?,
            version: self.version,
            bytecode_hashes: self.bytecode_hashes,
            root_signature: match self.root_signature {
                Some(text) => Some(HexBytes::parse_hex(&text)?),
                None => None,
            },
            input_layout: self.input_layout,
            depth_stencil: self.depth_stencil,
            blend: self.blend,
            rasterizer: self.rasterizer,
            render_target_formats: self.render_target_formats,
            view_instancing: self.view_instancing,
            stream_out: self.stream_out,
            sample_desc: self.sample_desc,
            sample_mask: self.sample_mask,
            ib_strip_cut_value: self.ib_strip_cut_value,
            primitive_topology: self.primitive_topology,
            dsv_format: self.dsv_format,
            node_mask: self.node_mask,
            flags: self.flags,
        })
    }
}

impl<'de> serde::Deserialize<'de> for RawRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawRecordVisitor)
    }
}

struct RawRecordVisitor;

impl<'de> Visitor<'de> for RawRecordVisitor {
    type Value = RawRecord;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a pipeline-state record object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut app_key = None;
        let mut version = None;
        let mut bytecode_hashes = None;
        let mut root_signature = None;
        let mut input_layout = None;
        let mut depth_stencil = None;
        let mut blend = None;
        let mut rasterizer = None;
        let mut render_target_formats = None;
        let mut view_instancing = None;
        let mut stream_out = None;
        let mut sample_desc = None;
        let mut sample_mask = None;
        let mut ib_strip_cut_value = None;
        let mut primitive_topology = None;
        let mut dsv_format = None;
        let mut node_mask = None;
        let mut flags = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "app_key" => app_key = Some(map.next_value()?),
                "version" => version = Some(map.next_value()?),
                "bytecode_hashes" => {
                    bytecode_hashes = Some(map.next_value::<RawStages>()?.0);
                }
                "root_signature" => root_signature = map.next_value()?,
                "input_layout" => input_layout = map.next_value()?,
                "depth_stencil" => depth_stencil = map.next_value()?,
                "blend" => blend = map.next_value()?,
                "rasterizer" => rasterizer = map.next_value()?,
                "render_target_formats" => render_target_formats = map.next_value()?,
                "view_instancing" => view_instancing = map.next_value()?,
                "stream_out" => stream_out = map.next_value()?,
                "sample_desc" => sample_desc = map.next_value()?,
                "sample_mask" => sample_mask = map.next_value()?,
                "ib_strip_cut_value" => ib_strip_cut_value = map.next_value()?,
                "primitive_topology" => primitive_topology = map.next_value()?,
                "dsv_format" => dsv_format = map.next_value()?,
                "node_mask" => node_mask = map.next_value()?,
                "flags" => flags = map.next_value()?,
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        Ok(RawRecord {
            app_key: app_key.ok_or_else(|| store_typed(Error::InvalidJson))?,
            version: version.ok_or_else(|| store_typed(Error::InvalidJson))?,
            bytecode_hashes: bytecode_hashes.ok_or_else(|| store_typed(Error::InvalidJson))?,
            root_signature,
            input_layout,
            depth_stencil,
            blend,
            rasterizer,
            render_target_formats,
            view_instancing,
            stream_out,
            sample_desc,
            sample_mask,
            ib_strip_cut_value,
            primitive_topology,
            dsv_format,
            node_mask,
            flags,
        })
    }
}

struct RawStages(StageHashes);

impl<'de> serde::Deserialize<'de> for RawStages {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawStagesVisitor)
    }
}

struct RawStagesVisitor;

impl<'de> Visitor<'de> for RawStagesVisitor {
    type Value = RawStages;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a map of shader stages to SHA-256 hex")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut hashes = StageHashes::default();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            let text: String = map.next_value()?;
            let hash = crate::hash::Sha256::parse_hex(&text).map_err(store_typed)?;
            match key.as_str() {
                "vs" => hashes.vs = Some(hash),
                "ps" => hashes.ps = Some(hash),
                "hs" => hashes.hs = Some(hash),
                "ds" => hashes.ds = Some(hash),
                "gs" => hashes.gs = Some(hash),
                "as" => hashes.as_stage = Some(hash),
                "ms" => hashes.ms = Some(hash),
                "cs" => hashes.cs = Some(hash),
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        Ok(RawStages(hashes))
    }
}

/// Path wrapper whose [`Debug`] output is always `<redacted>`.
pub struct RedactedPath(std::path::PathBuf);

impl RedactedPath {
    /// Wrap a filesystem path.
    #[must_use]
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self(path.into())
    }

    /// Borrow the path for opening. Callers must not write it into a record file.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl fmt::Debug for RedactedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl AsRef<Path> for RedactedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}
