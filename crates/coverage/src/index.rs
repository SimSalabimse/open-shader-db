// SPDX-License-Identifier: MIT

//! Coverage index types and hardened file loading.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Serialize;

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::hash::DescriptionHash;
use crate::{Error, Limits};

/// On-disk schema version implemented by this crate.
pub const FORMAT_VERSION: u32 = 1;

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
    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
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

/// Validated coverage index. One file describes one [`BuildId`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageIndex {
    pub format_version: u32,
    pub build_id: BuildId,
    pub hashes: IndexHashes,
    pub upload_count: u64,
    pub consensus_count: u64,
    pub generated_at: GeneratedAt,
}

impl CoverageIndex {
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

fn parse_index_bytes(bytes: Vec<u8>, limits: Limits) -> Result<CoverageIndex, Error> {
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
        let raw: RawIndex = serde::Deserialize::deserialize(&mut de).map_err(take_typed_error)?;
        de.end().map_err(|_| Error::TrailingGarbage)?;
        raw
    };
    drop(bytes);
    raw.into_index(limits)
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
}

impl RawIndex {
    fn into_index(self, limits: Limits) -> Result<CoverageIndex, Error> {
        if self.format_version != u64::from(FORMAT_VERSION) {
            return Err(Error::UnsupportedFormatVersion {
                found: self.format_version,
                expected: FORMAT_VERSION,
            });
        }
        Ok(CoverageIndex {
            format_version: FORMAT_VERSION,
            build_id: self.build_id,
            hashes: IndexHashes::from_unsorted(self.hashes, limits.max_hashes)?,
            upload_count: self.upload_count,
            consensus_count: self.consensus_count,
            generated_at: self.generated_at,
        })
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
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "format_version" => format_version = Some(map.next_value()?),
                "build_id" => {
                    let raw: RawBuildId = map.next_value()?;
                    build_id = Some(raw.into_build().map_err(store_typed)?);
                }
                "hashes" => hashes = Some(map.next_value::<RawHashes>()?.0),
                "upload_count" => upload_count = Some(map.next_value()?),
                "consensus_count" => consensus_count = Some(map.next_value()?),
                "generated_at" => {
                    let text: String = map.next_value()?;
                    generated_at = Some(GeneratedAt::parse(&text).map_err(store_typed)?);
                }
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        Ok(RawIndex {
            format_version: format_version.ok_or_else(|| store_typed(Error::InvalidJson))?,
            build_id: build_id.ok_or_else(|| store_typed(Error::InvalidJson))?,
            hashes: hashes.ok_or_else(|| store_typed(Error::InvalidJson))?,
            upload_count: upload_count.ok_or_else(|| store_typed(Error::InvalidJson))?,
            consensus_count: consensus_count.ok_or_else(|| store_typed(Error::InvalidJson))?,
            generated_at: generated_at.ok_or_else(|| store_typed(Error::InvalidJson))?,
        })
    }
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
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "game_id" => game_id = Some(map.next_value()?),
                "version" => version = Some(map.next_value()?),
                "exe_hash" => exe_hash = Some(map.next_value()?),
                _ => return Err(store_typed(Error::UnknownField)),
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
        while let Some(text) = seq.next_element::<String>()? {
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
