// SPDX-License-Identifier: MIT

//! Coverage index types and hardened file loading.

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
    let mut file = open_index_file(path)?;
    let opened = file.metadata().map_err(|_| Error::Io)?;
    if !opened.is_file() {
        return Err(Error::NotARegularFile);
    }
    let got = opened.len();
    if got > limits.max_file_bytes {
        return Err(Error::FileTooLarge {
            max: limits.max_file_bytes,
            got,
        });
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| Error::Io)?;
    let got = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if got > limits.max_file_bytes {
        return Err(Error::FileTooLarge {
            max: limits.max_file_bytes,
            got,
        });
    }
    parse_index_bytes(&bytes, limits)
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

fn parse_index_bytes(bytes: &[u8], limits: Limits) -> Result<CoverageIndex, Error> {
    let nesting = json_nesting_depth(bytes);
    if nesting > limits.max_json_nesting {
        return Err(Error::NestingTooDeep {
            max: limits.max_json_nesting,
        });
    }
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let raw: RawIndex = serde::Deserialize::deserialize(&mut de).map_err(map_de_error)?;
    de.end().map_err(|_| Error::TrailingGarbage)?;
    raw.into_index(limits)
}

fn map_de_error(err: serde_json::Error) -> Error {
    let msg = err.to_string();
    if msg.contains("duplicate JSON key") {
        Error::DuplicateKey
    } else if msg.contains("unknown field") {
        Error::UnknownField
    } else if msg.contains("malformed hex") {
        Error::MalformedHex
    } else if msg.contains("hex value has length") {
        Error::WrongHexLength {
            got: extract_got_len(&msg).unwrap_or(0),
        }
    } else if msg.contains("recursion") || msg.contains("nested") {
        Error::NestingTooDeep { max: 0 }
    } else {
        Error::InvalidJson
    }
}

fn extract_got_len(msg: &str) -> Option<usize> {
    let after = msg.split("length ").nth(1)?;
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
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
                return Err(de::Error::custom("duplicate JSON key"));
            }
            match key.as_str() {
                "format_version" => format_version = Some(map.next_value()?),
                "build_id" => {
                    let raw: RawBuildId = map.next_value()?;
                    build_id = Some(
                        raw.into_build()
                            .map_err(|e| de::Error::custom(e.to_string()))?,
                    );
                }
                "hashes" => hashes = Some(map.next_value::<RawHashes>()?.0),
                "upload_count" => upload_count = Some(map.next_value()?),
                "consensus_count" => consensus_count = Some(map.next_value()?),
                "generated_at" => {
                    let text: String = map.next_value()?;
                    generated_at = Some(
                        GeneratedAt::parse(&text).map_err(|e| de::Error::custom(e.to_string()))?,
                    );
                }
                _ => return Err(de::Error::custom("unknown field")),
            }
        }
        Ok(RawIndex {
            format_version: format_version
                .ok_or_else(|| de::Error::missing_field("format_version"))?,
            build_id: build_id.ok_or_else(|| de::Error::missing_field("build_id"))?,
            hashes: hashes.ok_or_else(|| de::Error::missing_field("hashes"))?,
            upload_count: upload_count.ok_or_else(|| de::Error::missing_field("upload_count"))?,
            consensus_count: consensus_count
                .ok_or_else(|| de::Error::missing_field("consensus_count"))?,
            generated_at: generated_at.ok_or_else(|| de::Error::missing_field("generated_at"))?,
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
                return Err(de::Error::custom("duplicate JSON key"));
            }
            match key.as_str() {
                "game_id" => game_id = Some(map.next_value()?),
                "version" => version = Some(map.next_value()?),
                "exe_hash" => exe_hash = Some(map.next_value()?),
                _ => return Err(de::Error::custom("unknown field")),
            }
        }
        Ok(RawBuildId {
            game_id: game_id.ok_or_else(|| de::Error::missing_field("game_id"))?,
            version: version.ok_or_else(|| de::Error::missing_field("version"))?,
            exe_hash: exe_hash.ok_or_else(|| de::Error::missing_field("exe_hash"))?,
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
        let mut hashes = Vec::new();
        if let Some(hint) = seq.size_hint() {
            hashes.reserve(hint.min(1_048_576));
        }
        while let Some(text) = seq.next_element::<String>()? {
            let hash = DescriptionHash::parse_hex(&text).map_err(|e| match e {
                Error::WrongHexLength { got } => {
                    de::Error::custom(format!("hex value has length {got}, expected 64"))
                }
                _ => de::Error::custom("malformed hex"),
            })?;
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
    matches!(
        bytes,
        [y0, y1, y2, y3, b'-', m0, m1, b'-', d0, d1, b'T', h0, h1, b':', n0, n1, b':', s0, s1, b'Z']
        if y0.is_ascii_digit()
            && y1.is_ascii_digit()
            && y2.is_ascii_digit()
            && y3.is_ascii_digit()
            && m0.is_ascii_digit()
            && m1.is_ascii_digit()
            && d0.is_ascii_digit()
            && d1.is_ascii_digit()
            && h0.is_ascii_digit()
            && h1.is_ascii_digit()
            && n0.is_ascii_digit()
            && n1.is_ascii_digit()
            && s0.is_ascii_digit()
            && s1.is_ascii_digit()
    )
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
