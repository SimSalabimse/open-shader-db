// SPDX-License-Identifier: MIT

//! Coverage index types and hardened file loading.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Serialize;

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::hash::DescriptionHash;
use crate::{Error, Limits};

/// On-disk schema version 1: SODB description hashes.
pub const FORMAT_VERSION: u32 = 1;

/// On-disk schema version 2: pipeline content hashes.
pub const FORMAT_VERSION_V2: u32 = 2;

/// Longest string this format stores (a 128-character game id or version).
const MAX_TEXT_CHARS: usize = 128;

/// `\uXXXX` is six raw bytes for one character. A longer token cannot be a
/// legal field, so it is rejected before the bytes are copied.
const MAX_STRING_RAW: usize = MAX_TEXT_CHARS * 6;

/// Longest object key (`min_ratio_per_mille` is 20). Longer keys are unknown.
const MAX_KEY_CHARS: usize = 32;

/// Human-readable notice that the index file was not signature-checked.
pub const UNSIGNED_INDEX_NOTE: &str = "unsigned index, not verified";

/// Final line of human CLI output until M4 signing exists.
pub const UNSIGNED_INDEX_LINE: &str =
    "Index: unsigned (signature verification arrives in a later milestone)";

thread_local! {
    static LOAD_LIMITS: Cell<Limits> = const { Cell::new(Limits::DEFAULT) };
    static LAST_TYPED_ERROR: RefCell<Option<Error>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static READ_REACHED: Cell<bool> = const { Cell::new(false) };
}

/// UTC timestamp from `generated_at` (`YYYY-MM-DDTHH:MM:SSZ`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct GeneratedAt(String);

impl GeneratedAt {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if !is_utc_iso8601(text) {
            return Err(Error::InvalidTimestamp);
        }
        Ok(Self(text.to_string()))
    }

    /// Timestamp text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Sorted, unique description hashes. Duplicates are dropped at construction.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct IndexHashes {
    hashes: Vec<DescriptionHash>,
}

impl IndexHashes {
    /// Sort, deduplicate, and enforce [`Limits::max_hashes`].
    pub fn from_unsorted(mut hashes: Vec<DescriptionHash>, max_hashes: u64) -> Result<Self, Error> {
        let got = u64::try_from(hashes.len()).unwrap_or(u64::MAX);
        if got > max_hashes {
            return Err(Error::TooManyHashes {
                max: max_hashes,
                got,
            });
        }
        hashes.sort_unstable();
        hashes.dedup();
        let got = u64::try_from(hashes.len()).unwrap_or(u64::MAX);
        if got > max_hashes {
            return Err(Error::TooManyHashes {
                max: max_hashes,
                got,
            });
        }
        Ok(Self { hashes })
    }

    /// Accept a list that is already strictly sorted and unique.
    ///
    /// Format version 2 uses this. Version 1 keeps [`Self::from_unsorted`],
    /// which sorts and drops duplicates.
    pub fn from_sorted_unique(
        hashes: Vec<DescriptionHash>,
        max_hashes: u64,
    ) -> Result<Self, Error> {
        let got = u64::try_from(hashes.len()).unwrap_or(u64::MAX);
        if got > max_hashes {
            return Err(Error::TooManyHashes {
                max: max_hashes,
                got,
            });
        }
        for pair in hashes.windows(2) {
            if pair[0] == pair[1] {
                return Err(Error::DuplicateHash);
            }
            if pair[0] > pair[1] {
                return Err(Error::UnsortedHashes);
            }
        }
        Ok(Self { hashes })
    }

    /// Hash count after deduplication.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Sorted unique hashes.
    #[must_use]
    pub fn as_slice(&self) -> &[DescriptionHash] {
        &self.hashes
    }

    /// Binary search for a description hash.
    #[must_use]
    pub fn contains(&self, hash: &DescriptionHash) -> bool {
        self.hashes.binary_search(hash).is_ok()
    }
}

impl fmt::Debug for IndexHashes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexHashes")
            .field("len", &self.hashes.len())
            .finish()
    }
}

/// Threshold stored on a format version 2 index.
///
/// `min_uploads` is at least 1. `min_ratio_per_mille` is at most 1000.
/// Fields stay private so a struct literal cannot skip [`Self::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct IndexThreshold {
    min_uploads: u32,
    min_ratio_per_mille: u16,
}

impl IndexThreshold {
    /// Validated constructor. A ratio of 0 is valid.
    pub const fn new(min_uploads: u32, min_ratio_per_mille: u16) -> Result<Self, Error> {
        if min_uploads == 0 || min_ratio_per_mille > 1000 {
            return Err(Error::InvalidThreshold);
        }
        Ok(Self {
            min_uploads,
            min_ratio_per_mille,
        })
    }

    /// Minimum counted uploads that must list a hash.
    #[must_use]
    pub const fn min_uploads(self) -> u32 {
        self.min_uploads
    }

    /// Minimum share of the unit denominator, in parts per thousand.
    #[must_use]
    pub const fn min_ratio_per_mille(self) -> u16 {
        self.min_ratio_per_mille
    }
}

/// Validated coverage index. One file describes one [`BuildId`].
///
/// `format_version` 1 stores SODB description hashes. `format_version` 2
/// stores pipeline content hashes (`pipeline_content_hash` values) plus
/// `rejected_count` and `threshold`. Version 1 leaves those two fields absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageIndex {
    pub format_version: u32,
    pub build_id: BuildId,
    pub hashes: IndexHashes,
    pub upload_count: u64,
    pub consensus_count: u64,
    pub generated_at: GeneratedAt,
    /// Present on format version 2. Absent on version 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_count: Option<u64>,
    /// Present on format version 2. Absent on version 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<IndexThreshold>,
}

impl CoverageIndex {
    /// Format version 1 index. `hashes` are SODB description hashes.
    #[must_use]
    pub fn description(
        build_id: BuildId,
        hashes: IndexHashes,
        upload_count: u64,
        consensus_count: u64,
        generated_at: GeneratedAt,
    ) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            build_id,
            hashes,
            upload_count,
            consensus_count,
            generated_at,
            rejected_count: None,
            threshold: None,
        }
    }

    /// Format version 2 index. `hashes` are pipeline content hashes.
    pub fn pipeline(
        build_id: BuildId,
        hashes: IndexHashes,
        upload_count: u64,
        consensus_count: u64,
        rejected_count: u64,
        generated_at: GeneratedAt,
        threshold: IndexThreshold,
    ) -> Result<Self, Error> {
        Ok(Self {
            format_version: FORMAT_VERSION_V2,
            build_id,
            hashes,
            upload_count,
            consensus_count,
            generated_at,
            rejected_count: Some(rejected_count),
            threshold: Some(threshold),
        })
    }

    /// Compact JSON. Object keys are sorted. No trailing newline.
    ///
    /// The same value always produces the same bytes.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        if self.format_version == FORMAT_VERSION {
            if self.rejected_count.is_some() || self.threshold.is_some() {
                return Err(Error::CanonicalJson);
            }
        } else if self.format_version == FORMAT_VERSION_V2 {
            if self.rejected_count.is_none() || self.threshold.is_none() {
                return Err(Error::CanonicalJson);
            }
        } else {
            return Err(Error::UnsupportedFormatVersion {
                found: u64::from(self.format_version),
            });
        }
        let mut build = serde_json::Map::new();
        build.insert(
            "exe_hash".to_string(),
            serde_json::Value::String(self.build_id.exe_hash.to_hex()),
        );
        build.insert(
            "game_id".to_string(),
            serde_json::Value::String(self.build_id.game_id.as_str().to_string()),
        );
        build.insert(
            "version".to_string(),
            serde_json::Value::String(self.build_id.version.as_str().to_string()),
        );
        let hashes: Vec<serde_json::Value> = self
            .hashes
            .as_slice()
            .iter()
            .map(|hash| serde_json::Value::String(hash.to_hex()))
            .collect();
        let mut root = serde_json::Map::new();
        root.insert("build_id".to_string(), serde_json::Value::Object(build));
        root.insert(
            "consensus_count".to_string(),
            serde_json::Value::Number(self.consensus_count.into()),
        );
        root.insert(
            "format_version".to_string(),
            serde_json::Value::Number(self.format_version.into()),
        );
        root.insert(
            "generated_at".to_string(),
            serde_json::Value::String(self.generated_at.as_str().to_string()),
        );
        root.insert("hashes".to_string(), serde_json::Value::Array(hashes));
        if let Some(rejected) = self.rejected_count {
            root.insert(
                "rejected_count".to_string(),
                serde_json::Value::Number(rejected.into()),
            );
        }
        if let Some(threshold) = self.threshold {
            let mut object = serde_json::Map::new();
            object.insert(
                "min_ratio_per_mille".to_string(),
                serde_json::Value::Number(u64::from(threshold.min_ratio_per_mille()).into()),
            );
            object.insert(
                "min_uploads".to_string(),
                serde_json::Value::Number(u64::from(threshold.min_uploads()).into()),
            );
            root.insert("threshold".to_string(), serde_json::Value::Object(object));
        }
        root.insert(
            "upload_count".to_string(),
            serde_json::Value::Number(self.upload_count.into()),
        );
        let bytes = serde_json::to_vec(&serde_json::Value::Object(root))
            .map_err(|_| Error::CanonicalJson)?;
        if bytes.first() == Some(&0xEF)
            || bytes.last() == Some(&b'\n')
            || bytes.last() == Some(&b' ')
        {
            return Err(Error::CanonicalJson);
        }
        Ok(bytes)
    }

    /// Load `path` with [`Limits::DEFAULT`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        load_with_limits(path, Limits::DEFAULT)
    }

    /// Load `path` with caller-supplied resource caps.
    pub fn load_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<Self, Error> {
        load_with_limits(path, limits)
    }
}

/// Load a coverage index file with default limits.
pub fn load(path: impl AsRef<Path>) -> Result<CoverageIndex, Error> {
    load_with_limits(path, Limits::DEFAULT)
}

/// Load a coverage index file with caller-supplied resource caps.
pub fn load_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<CoverageIndex, Error> {
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
    parse_index_bytes(bytes, limits)
}

/// Open with `O_NONBLOCK` on Unix and require a regular file on the handle.
pub(crate) fn open_regular_nonblocking(path: &Path) -> Result<File, Error> {
    let file = open_index_file(path)?;
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

fn open_index_file(path: &Path) -> Result<File, Error> {
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

pub(crate) fn parse_index_bytes(bytes: Vec<u8>, limits: Limits) -> Result<CoverageIndex, Error> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(Error::ByteOrderMark);
    }
    let nesting = json_nesting_depth(&bytes);
    if nesting > limits.max_json_nesting {
        return Err(Error::NestingTooDeep {
            max: limits.max_json_nesting,
        });
    }
    // Issue #29: a hostile key must not be cloned on top of the raw bytes.
    reject_overlong_string(&bytes)?;
    LAST_TYPED_ERROR.with(|slot| slot.borrow_mut().take());
    LOAD_LIMITS.set(limits);
    let raw = {
        let mut de = serde_json::Deserializer::from_slice(&bytes);
        let raw: RawIndex = serde::Deserialize::deserialize(&mut de).map_err(take_typed_error)?;
        de.end().map_err(|_| Error::TrailingGarbage)?;
        raw
    };
    drop(bytes);
    raw.into_index(limits)
}

/// Reject a JSON string token whose raw contents cannot fit in [`MAX_TEXT_CHARS`].
///
/// The scan does not copy the token. Valid fields are at most 128 characters,
/// and the longest escape is six bytes per character.
fn reject_overlong_string(bytes: &[u8]) -> Result<(), Error> {
    let mut index = 0;
    let mut in_str = false;
    let mut escape = false;
    let mut start = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_str {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                if index.saturating_sub(start) > MAX_STRING_RAW {
                    return Err(Error::StringTooLong);
                }
                in_str = false;
            }
        } else if byte == b'"' {
            in_str = true;
            start = index.saturating_add(1);
        }
        index = index.saturating_add(1);
    }
    Ok(())
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

struct RawIndex {
    format_version: u64,
    build_id: BuildId,
    hashes: Vec<DescriptionHash>,
    upload_count: u64,
    consensus_count: u64,
    generated_at: GeneratedAt,
    rejected_count: Option<u64>,
    threshold: Option<IndexThreshold>,
}

impl RawIndex {
    fn into_index(self, limits: Limits) -> Result<CoverageIndex, Error> {
        if self.format_version == u64::from(FORMAT_VERSION) {
            if self.rejected_count.is_some() || self.threshold.is_some() {
                return Err(Error::UnknownField);
            }
            return Ok(CoverageIndex::description(
                self.build_id,
                IndexHashes::from_unsorted(self.hashes, limits.max_hashes)?,
                self.upload_count,
                self.consensus_count,
                self.generated_at,
            ));
        }
        if self.format_version != u64::from(FORMAT_VERSION_V2) {
            return Err(Error::UnsupportedFormatVersion {
                found: self.format_version,
            });
        }
        let rejected_count = self.rejected_count.ok_or(Error::InvalidJson)?;
        let threshold = self.threshold.ok_or(Error::InvalidJson)?;
        CoverageIndex::pipeline(
            self.build_id,
            IndexHashes::from_sorted_unique(self.hashes, limits.max_hashes)?,
            self.upload_count,
            self.consensus_count,
            rejected_count,
            self.generated_at,
            threshold,
        )
    }
}

impl<'de> serde::Deserialize<'de> for RawIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawIndexVisitor)
    }
}

struct RawIndexVisitor;

impl<'de> Visitor<'de> for RawIndexVisitor {
    type Value = RawIndex;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a coverage index object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut format_version = None;
        let mut build_id = None;
        let mut hashes = None;
        let mut upload_count = None;
        let mut consensus_count = None;
        let mut generated_at = None;
        let mut rejected_count = None;
        let mut threshold = None;
        while let Some(key) = map.next_key_seed(KeySeed(classify_top))? {
            if !seen.insert(key) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key {
                TopKey::FormatVersion => {
                    let found = next_u64(&mut map)?;
                    if found == u64::from(FORMAT_VERSION)
                        && (rejected_count.is_some() || threshold.is_some())
                    {
                        return Err(store_typed(Error::UnknownField));
                    }
                    format_version = Some(found);
                }
                TopKey::BuildId => {
                    let raw: RawBuildId = map.next_value()?;
                    build_id = Some(raw.into_build().map_err(store_typed)?);
                }
                TopKey::Hashes => hashes = Some(map.next_value::<RawHashes>()?.0),
                TopKey::UploadCount => upload_count = Some(next_u64(&mut map)?),
                TopKey::ConsensusCount => consensus_count = Some(next_u64(&mut map)?),
                TopKey::GeneratedAt => {
                    let text = map.next_value_seed(CappedString(20))?;
                    generated_at = Some(GeneratedAt::parse(&text).map_err(store_typed)?);
                }
                TopKey::RejectedCount => {
                    if format_version == Some(u64::from(FORMAT_VERSION)) {
                        return Err(store_typed(Error::UnknownField));
                    }
                    rejected_count = Some(next_u64(&mut map)?);
                }
                TopKey::Threshold => {
                    if format_version == Some(u64::from(FORMAT_VERSION)) {
                        return Err(store_typed(Error::UnknownField));
                    }
                    threshold = Some(map.next_value::<RawThreshold>()?.0);
                }
            }
        }
        Ok(RawIndex {
            format_version: format_version.ok_or_else(|| store_typed(Error::InvalidJson))?,
            build_id: build_id.ok_or_else(|| store_typed(Error::InvalidJson))?,
            hashes: hashes.ok_or_else(|| store_typed(Error::InvalidJson))?,
            upload_count: upload_count.ok_or_else(|| store_typed(Error::InvalidJson))?,
            consensus_count: consensus_count.ok_or_else(|| store_typed(Error::InvalidJson))?,
            generated_at: generated_at.ok_or_else(|| store_typed(Error::InvalidJson))?,
            rejected_count,
            threshold,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TopKey {
    FormatVersion,
    BuildId,
    Hashes,
    UploadCount,
    ConsensusCount,
    GeneratedAt,
    RejectedCount,
    Threshold,
}

fn classify_top(text: &str) -> Result<TopKey, Error> {
    if text.len() > MAX_KEY_CHARS {
        return Err(Error::UnknownField);
    }
    match text {
        "format_version" => Ok(TopKey::FormatVersion),
        "build_id" => Ok(TopKey::BuildId),
        "hashes" => Ok(TopKey::Hashes),
        "upload_count" => Ok(TopKey::UploadCount),
        "consensus_count" => Ok(TopKey::ConsensusCount),
        "generated_at" => Ok(TopKey::GeneratedAt),
        "rejected_count" => Ok(TopKey::RejectedCount),
        "threshold" => Ok(TopKey::Threshold),
        _ => Err(Error::UnknownField),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BuildKey {
    GameId,
    Version,
    ExeHash,
}

fn classify_build(text: &str) -> Result<BuildKey, Error> {
    if text.len() > MAX_KEY_CHARS {
        return Err(Error::UnknownField);
    }
    match text {
        "game_id" => Ok(BuildKey::GameId),
        "version" => Ok(BuildKey::Version),
        "exe_hash" => Ok(BuildKey::ExeHash),
        _ => Err(Error::UnknownField),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ThresholdKey {
    MinUploads,
    MinRatio,
}

fn classify_threshold(text: &str) -> Result<ThresholdKey, Error> {
    if text.len() > MAX_KEY_CHARS {
        return Err(Error::UnknownField);
    }
    match text {
        "min_uploads" => Ok(ThresholdKey::MinUploads),
        "min_ratio_per_mille" => Ok(ThresholdKey::MinRatio),
        _ => Err(Error::UnknownField),
    }
}

struct KeySeed<T>(fn(&str) -> Result<T, Error>);

impl<'de, T> DeserializeSeed<'de> for KeySeed<T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        deserializer.deserialize_str(KeyVisitor(self.0))
    }
}

struct KeyVisitor<T>(fn(&str) -> Result<T, Error>);

impl<'de, T> Visitor<'de> for KeyVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object key")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<T, E> {
        (self.0)(value).map_err(store_typed)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<T, E> {
        (self.0)(value).map_err(store_typed)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<T, E> {
        let parsed = (self.0)(&value).map_err(store_typed);
        drop(value);
        parsed
    }
}

struct CappedString(usize);

impl<'de> DeserializeSeed<'de> for CappedString {
    type Value = String;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(CappedStringVisitor(self.0))
    }
}

struct CappedStringVisitor(usize);

impl<'de> Visitor<'de> for CappedStringVisitor {
    type Value = String;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a short string")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<String, E> {
        copy_capped(value, self.0)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        copy_capped(value, self.0)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
        if value.len() > self.0 {
            let err = cap_error(value.len(), self.0);
            drop(value);
            return Err(store_typed(err));
        }
        Ok(value)
    }
}

fn copy_capped<E: de::Error>(value: &str, max: usize) -> Result<String, E> {
    if value.len() > max {
        return Err(store_typed(cap_error(value.len(), max)));
    }
    Ok(value.to_string())
}

fn cap_error(got: usize, max: usize) -> Error {
    if max == 64 {
        Error::WrongHexLength { got }
    } else if max == 20 {
        Error::InvalidTimestamp
    } else {
        Error::InvalidBuildId
    }
}

fn next_u64<'de, A: MapAccess<'de>>(map: &mut A) -> Result<u64, A::Error> {
    map.next_value::<u64>()
        .map_err(|_| store_typed(Error::InvalidInteger))
}

struct RawBuildId {
    game_id: String,
    version: String,
    exe_hash: String,
}

impl RawBuildId {
    fn into_build(self) -> Result<BuildId, Error> {
        let game_id = GameId::parse(&self.game_id)?;
        let version = GameVersion::parse(&self.version)?;
        let exe_hash = ExeHash::parse_hex(&self.exe_hash)?;
        Ok(BuildId::new(game_id, version, exe_hash))
    }
}

impl<'de> serde::Deserialize<'de> for RawBuildId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawBuildVisitor)
    }
}

struct RawBuildVisitor;

impl<'de> Visitor<'de> for RawBuildVisitor {
    type Value = RawBuildId;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a build_id object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut game_id = None;
        let mut version = None;
        let mut exe_hash = None;
        while let Some(key) = map.next_key_seed(KeySeed(classify_build))? {
            if !seen.insert(key) {
                return Err(store_typed(Error::DuplicateKey));
            }
            let text = map.next_value_seed(CappedString(MAX_TEXT_CHARS))?;
            match key {
                BuildKey::GameId => game_id = Some(text),
                BuildKey::Version => version = Some(text),
                BuildKey::ExeHash => exe_hash = Some(text),
            }
        }
        Ok(RawBuildId {
            game_id: game_id.ok_or_else(|| store_typed(Error::InvalidJson))?,
            version: version.ok_or_else(|| store_typed(Error::InvalidJson))?,
            exe_hash: exe_hash.ok_or_else(|| store_typed(Error::InvalidJson))?,
        })
    }
}

struct RawHashes(Vec<DescriptionHash>);

impl<'de> serde::Deserialize<'de> for RawHashes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(RawHashesVisitor)
    }
}

struct RawHashesVisitor;

impl<'de> Visitor<'de> for RawHashesVisitor {
    type Value = RawHashes;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of 64-character lowercase hex SHA-256 strings")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let max_hashes = LOAD_LIMITS.get().max_hashes;
        let mut hashes = Vec::new();
        if let Some(hint) = seq.size_hint() {
            let cap = usize::try_from(max_hashes).unwrap_or(usize::MAX);
            hashes.reserve(hint.min(cap));
        }
        while let Some(text) = seq.next_element_seed(CappedString(64))? {
            let got = u64::try_from(hashes.len().saturating_add(1)).unwrap_or(u64::MAX);
            if got > max_hashes {
                return Err(store_typed(Error::TooManyHashes {
                    max: max_hashes,
                    got,
                }));
            }
            let hash = DescriptionHash::parse_hex(&text).map_err(store_typed)?;
            hashes.push(hash);
        }
        Ok(RawHashes(hashes))
    }
}

struct RawThreshold(IndexThreshold);

impl<'de> serde::Deserialize<'de> for RawThreshold {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawThresholdVisitor)
    }
}

struct RawThresholdVisitor;

impl<'de> Visitor<'de> for RawThresholdVisitor {
    type Value = RawThreshold;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a threshold object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut min_uploads = None;
        let mut min_ratio = None;
        while let Some(key) = map.next_key_seed(KeySeed(classify_threshold))? {
            if !seen.insert(key) {
                return Err(store_typed(Error::DuplicateKey));
            }
            let value = next_u64(&mut map)?;
            match key {
                ThresholdKey::MinUploads => min_uploads = Some(value),
                ThresholdKey::MinRatio => min_ratio = Some(value),
            }
        }
        let min_uploads = min_uploads.ok_or_else(|| store_typed(Error::InvalidJson))?;
        let min_ratio = min_ratio.ok_or_else(|| store_typed(Error::InvalidJson))?;
        let uploads = u32::try_from(min_uploads).map_err(|_| store_typed(Error::InvalidInteger))?;
        let ratio = u16::try_from(min_ratio).map_err(|_| store_typed(Error::InvalidThreshold))?;
        let threshold = IndexThreshold::new(uploads, ratio).map_err(store_typed)?;
        Ok(RawThreshold(threshold))
    }
}

fn is_utc_iso8601(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    let Some(year) = parse_digits(&bytes[0..4]) else {
        return false;
    };
    let Some(month) = parse_digits(&bytes[5..7]) else {
        return false;
    };
    let Some(day) = parse_digits(&bytes[8..10]) else {
        return false;
    };
    let Some(hour) = parse_digits(&bytes[11..13]) else {
        return false;
    };
    let Some(minute) = parse_digits(&bytes[14..16]) else {
        return false;
    };
    let Some(second) = parse_digits(&bytes[17..19]) else {
        return false;
    };
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    day >= 1 && day <= days_in_month(year, month)
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    let mut value = 0_u32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(u32::from(byte - b'0'));
    }
    Some(value)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// Path wrapper whose [`Debug`] output is always `<redacted>`.
pub struct RedactedPath(std::path::PathBuf);

impl RedactedPath {
    /// Wrap a filesystem path.
    #[must_use]
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self(path.into())
    }

    /// Borrow the path for opening. Callers must not write it into a report.
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
