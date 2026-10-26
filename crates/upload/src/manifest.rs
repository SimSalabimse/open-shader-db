// SPDX-License-Identifier: MIT

//! UploadManifest type, canonical JSON, and untrusted-byte parsing.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::client::ClientVersion;
use crate::hash::Sha256Hex;
use crate::limits::MAX_NUMBER_TOKEN_CHARS;
use crate::ownership::{OwnershipProof, UnchallengedReason};
use crate::time::UtcHour;
use crate::{Error, Limits};

/// Schema version written by [`crate::ManifestBuilder`]. Parsers also accept
/// [`SCHEMA_VERSION_V1`].
pub const SCHEMA_VERSION: u32 = 2;

/// Original upload manifest. No `pipeline_hashes` field.
pub const SCHEMA_VERSION_V1: u32 = 1;

/// Sorted unique SHA-256 hex strings.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct HashSet {
    hashes: Vec<Sha256Hex>,
}

impl HashSet {
    /// Sort, deduplicate, and enforce a caller-supplied hash-list cap.
    pub fn from_unsorted(mut hashes: Vec<Sha256Hex>, max_hashes: u64) -> Result<Self, Error> {
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
    pub fn from_sorted_unique(hashes: Vec<Sha256Hex>, max_hashes: u64) -> Result<Self, Error> {
        let got = u64::try_from(hashes.len()).unwrap_or(u64::MAX);
        if got > max_hashes {
            return Err(Error::TooManyHashes {
                max: max_hashes,
                got,
            });
        }
        if !is_strictly_sorted_unique(&hashes) {
            return Err(Error::UnsortedOrDuplicateHashes);
        }
        Ok(Self { hashes })
    }

    /// Build from a set that already enforced the hash cap (builder path).
    pub(crate) fn from_capped_set(hashes: Vec<Sha256Hex>) -> Self {
        Self { hashes }
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
    pub fn as_slice(&self) -> &[Sha256Hex] {
        &self.hashes
    }
}

fn is_strictly_sorted_unique(hashes: &[Sha256Hex]) -> bool {
    hashes.windows(2).all(|pair| pair[0] < pair[1])
}

impl fmt::Debug for HashSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HashSet")
            .field("len", &self.hashes.len())
            .finish()
    }
}

/// Anonymized upload payload. Field set is the allowlist.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
pub struct UploadManifest {
    pub schema_version: u32,
    pub build_id: BuildId,
    pub client_version: ClientVersion,
    pub collected_hour: UtcHour,
    pub description_hashes: HashSet,
    pub bytecode_hashes: HashSet,
    pub ownership: OwnershipProof,
    /// Present on schema version 2. Absent on version 1 (omitted from JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_hashes: Option<HashSet>,
}

impl fmt::Debug for UploadManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadManifest")
            .field("schema_version", &self.schema_version)
            .field("build_id", &self.build_id)
            .field("client_version", &self.client_version)
            .field("collected_hour", &self.collected_hour)
            .field("description_hashes", &self.description_hashes)
            .field("bytecode_hashes", &self.bytecode_hashes)
            .field("ownership", &self.ownership)
            .field("pipeline_hashes", &self.pipeline_hashes)
            .finish()
    }
}

impl UploadManifest {
    /// Compact JSON with object keys sorted lexicographically.
    ///
    /// The same validated input always produces the same bytes.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        self.to_canonical_bytes_with_limits(Limits::DEFAULT)
    }

    /// Canonical encoding with caller-supplied size cap.
    pub fn to_canonical_bytes_with_limits(&self, limits: Limits) -> Result<Vec<u8>, Error> {
        let value = serde_json::to_value(self).map_err(|_| Error::CanonicalJson)?;
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::CanonicalJson)?;
        let got = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if got > limits.max_json_bytes {
            return Err(Error::ManifestTooLarge {
                max: limits.max_json_bytes,
                got,
            });
        }
        Ok(bytes)
    }

    /// Parse untrusted bytes with [`Limits::DEFAULT`].
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        parse_with_limits(bytes, Limits::DEFAULT)
    }

    /// Parse untrusted bytes with caller-supplied caps.
    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
        parse_with_limits(bytes, limits)
    }
}

thread_local! {
    static LAST_TYPED_ERROR: RefCell<Option<Error>> = const { RefCell::new(None) };
    /// Cap for a `pipeline_hashes` array read before `schema_version` is known.
    static PIPELINE_STRING_CAP: Cell<u64> = const { Cell::new(0) };
}

/// Parse an upload manifest from memory. No filesystem I/O.
pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<UploadManifest, Error> {
    let got = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if got > limits.max_json_bytes {
        return Err(Error::ManifestTooLarge {
            max: limits.max_json_bytes,
            got,
        });
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(Error::ByteOrderMark);
    }
    let nesting = json_nesting_depth(bytes);
    if nesting > limits.max_json_nesting {
        return Err(Error::NestingTooDeep {
            max: limits.max_json_nesting,
        });
    }
    if number_token_exceeds(bytes, MAX_NUMBER_TOKEN_CHARS) {
        return Err(Error::InvalidInteger);
    }
    LAST_TYPED_ERROR.with(|slot| slot.borrow_mut().take());
    PIPELINE_STRING_CAP.with(|cap| cap.set(limits.max_pipeline_hashes));
    let raw = {
        let mut de = serde_json::Deserializer::from_slice(bytes);
        let raw: RawManifest =
            serde::Deserialize::deserialize(&mut de).map_err(take_typed_error)?;
        de.end().map_err(|_| Error::TrailingGarbage)?;
        raw
    };
    raw.into_manifest(limits)
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

fn number_token_exceeds(bytes: &[u8], max_chars: usize) -> bool {
    let mut in_str = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if byte == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
                if i.saturating_sub(start) > max_chars {
                    return true;
                }
            }
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                    if i.saturating_sub(start) > max_chars {
                        return true;
                    }
                }
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                    if i.saturating_sub(start) > max_chars {
                        return true;
                    }
                }
            }
            if i.saturating_sub(start) > max_chars {
                return true;
            }
            continue;
        }
        i += 1;
    }
    false
}

struct RawManifest {
    schema_version: u64,
    build_id: BuildId,
    client_version: ClientVersion,
    collected_hour: UtcHour,
    description_hashes: Vec<Sha256Hex>,
    bytecode_hashes: Vec<Sha256Hex>,
    ownership: OwnershipProof,
    /// `Some` only when the key was present and the version allows it.
    pipeline_hashes: Option<Vec<Sha256Hex>>,
}

impl RawManifest {
    fn into_manifest(self, limits: Limits) -> Result<UploadManifest, Error> {
        if self.schema_version != u64::from(SCHEMA_VERSION_V1)
            && self.schema_version != u64::from(SCHEMA_VERSION)
        {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        let description_hashes =
            HashSet::from_sorted_unique(self.description_hashes, limits.max_description_hashes)?;
        let bytecode_hashes =
            HashSet::from_sorted_unique(self.bytecode_hashes, limits.max_bytecode_hashes)?;
        let (schema_version, pipeline_hashes) =
            if self.schema_version == u64::from(SCHEMA_VERSION_V1) {
                if self.pipeline_hashes.is_some() {
                    return Err(Error::UnknownField);
                }
                (SCHEMA_VERSION_V1, None)
            } else {
                let raw = self.pipeline_hashes.ok_or(Error::MissingField)?;
                let set = HashSet::from_sorted_unique(raw, limits.max_pipeline_hashes)?;
                (SCHEMA_VERSION, Some(set))
            };
        Ok(UploadManifest {
            schema_version,
            build_id: self.build_id,
            client_version: self.client_version,
            collected_hour: self.collected_hour,
            description_hashes,
            bytecode_hashes,
            ownership: self.ownership,
            pipeline_hashes,
        })
    }
}

impl<'de> serde::Deserialize<'de> for RawManifest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawManifestVisitor)
    }
}

struct RawManifestVisitor;

impl<'de> Visitor<'de> for RawManifestVisitor {
    type Value = RawManifest;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an upload manifest object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut schema_version = None;
        let mut build_id = None;
        let mut client_version = None;
        let mut collected_hour = None;
        let mut description_hashes = None;
        let mut bytecode_hashes = None;
        let mut ownership = None;
        let mut pipeline_hashes: Option<Vec<Sha256Hex>> = None;
        let mut pipeline_pending: Option<Vec<String>> = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "schema_version" => {
                    let found = parse_u64(map.next_value()?)?;
                    if found == u64::from(SCHEMA_VERSION_V1)
                        && (pipeline_hashes.is_some() || pipeline_pending.is_some())
                    {
                        return Err(store_typed(Error::UnknownField));
                    }
                    if found == u64::from(SCHEMA_VERSION) {
                        if let Some(strings) = pipeline_pending.take() {
                            pipeline_hashes =
                                Some(parse_pending_hashes(strings).map_err(store_typed)?);
                        }
                    }
                    schema_version = Some(found);
                }
                "build_id" => {
                    let raw: RawBuildId = map.next_value()?;
                    build_id = Some(raw.into_build().map_err(store_typed)?);
                }
                "client_version" => {
                    let text: String = map.next_value()?;
                    client_version = Some(ClientVersion::parse(&text).map_err(store_typed)?);
                }
                "collected_hour" => {
                    let text: String = map.next_value()?;
                    collected_hour = Some(UtcHour::parse(&text).map_err(store_typed)?);
                }
                "description_hashes" => {
                    description_hashes = Some(map.next_value::<RawHashes>()?.0);
                }
                "bytecode_hashes" => {
                    bytecode_hashes = Some(map.next_value::<RawHashes>()?.0);
                }
                "ownership" => {
                    ownership = Some(map.next_value::<RawOwnership>()?.0);
                }
                "pipeline_hashes" => match schema_version {
                    Some(found) if found == u64::from(SCHEMA_VERSION_V1) => {
                        return Err(store_typed(Error::UnknownField));
                    }
                    Some(found) if found == u64::from(SCHEMA_VERSION) => {
                        pipeline_hashes = Some(map.next_value::<RawHashes>()?.0);
                    }
                    Some(_) => {
                        // Not version 1 or 2. Read typed strings (same cap) and
                        // drop them so the unsupported-version error wins.
                        let _ignored = map.next_value::<PendingHashStrings>()?;
                    }
                    None => {
                        pipeline_pending = Some(map.next_value::<PendingHashStrings>()?.0);
                    }
                },
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        Ok(RawManifest {
            schema_version: schema_version.ok_or_else(|| store_typed(Error::MissingField))?,
            build_id: build_id.ok_or_else(|| store_typed(Error::MissingField))?,
            client_version: client_version.ok_or_else(|| store_typed(Error::MissingField))?,
            collected_hour: collected_hour.ok_or_else(|| store_typed(Error::MissingField))?,
            description_hashes: description_hashes
                .ok_or_else(|| store_typed(Error::MissingField))?,
            bytecode_hashes: bytecode_hashes.ok_or_else(|| store_typed(Error::MissingField))?,
            ownership: ownership.ok_or_else(|| store_typed(Error::MissingField))?,
            pipeline_hashes,
        })
    }
}

fn parse_pending_hashes(strings: Vec<String>) -> Result<Vec<Sha256Hex>, Error> {
    let mut hashes = Vec::with_capacity(strings.len());
    for text in strings {
        hashes.push(Sha256Hex::parse(&text)?);
    }
    Ok(hashes)
}

/// SHA-256 hex is 64 characters. Longer strings are rejected before they are copied.
const MAX_PENDING_HASH_CHARS: usize = 64;

/// `pipeline_hashes` read before the version is known: strings only, with the
/// caller's count cap. Non-strings fail as `INVALID_JSON` on the first element.
/// A `serde_json::Value` would retain every element (numbers, objects, one
/// huge string) and a junk document near the 64 MiB local cap can reach
/// hundreds of megabytes of RSS.
struct PendingHashStrings(Vec<String>);

impl<'de> Deserialize<'de> for PendingHashStrings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(PendingHashStringsVisitor)
    }
}

struct PendingHashStringsVisitor;

impl<'de> Visitor<'de> for PendingHashStringsVisitor {
    type Value = PendingHashStrings;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of hash strings")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let cap = PIPELINE_STRING_CAP.with(Cell::get);
        let mut out = Vec::new();
        while let Some(text) = seq.next_element_seed(CappedHashString)? {
            let len = u64::try_from(out.len()).unwrap_or(u64::MAX);
            if len >= cap {
                return Err(store_typed(Error::TooManyHashes {
                    max: cap,
                    got: cap.saturating_add(1),
                }));
            }
            out.push(text);
        }
        Ok(PendingHashStrings(out))
    }
}

struct CappedHashString;

impl<'de> DeserializeSeed<'de> for CappedHashString {
    type Value = String;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(CappedHashStringVisitor)
    }
}

struct CappedHashStringVisitor;

impl<'de> Visitor<'de> for CappedHashStringVisitor {
    type Value = String;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a hash string")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<String, E> {
        capped_hash_string(value)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        capped_hash_string(value)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
        if value.len() > MAX_PENDING_HASH_CHARS {
            return Err(store_typed(Error::WrongHexLength { got: value.len() }));
        }
        Ok(value)
    }
}

fn capped_hash_string<E: de::Error>(value: &str) -> Result<String, E> {
    if value.len() > MAX_PENDING_HASH_CHARS {
        return Err(store_typed(Error::WrongHexLength { got: value.len() }));
    }
    Ok(value.to_string())
}

fn parse_u64<E: de::Error>(value: serde_json::Value) -> Result<u64, E> {
    match value {
        serde_json::Value::Number(n) => {
            n.as_u64().ok_or_else(|| store_typed(Error::InvalidInteger))
        }
        _ => Err(store_typed(Error::InvalidInteger)),
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
        let exe_hash = ExeHash::parse(&self.exe_hash)?;
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
            game_id: game_id.ok_or_else(|| store_typed(Error::MissingField))?,
            version: version.ok_or_else(|| store_typed(Error::MissingField))?,
            exe_hash: exe_hash.ok_or_else(|| store_typed(Error::MissingField))?,
        })
    }
}

struct RawHashes(Vec<Sha256Hex>);

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

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut hashes = Vec::new();
        while let Some(text) = seq.next_element::<String>()? {
            let hash = Sha256Hex::parse(&text).map_err(store_typed)?;
            hashes.push(hash);
        }
        Ok(RawHashes(hashes))
    }
}

struct RawOwnership(OwnershipProof);

impl<'de> serde::Deserialize<'de> for RawOwnership {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RawOwnershipVisitor)
    }
}

struct RawOwnershipVisitor;

impl<'de> Visitor<'de> for RawOwnershipVisitor {
    type Value = RawOwnership;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an ownership object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = BTreeSet::new();
        let mut kind = None;
        let mut reason = None;
        let mut challenge = None;
        let mut response = None;
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "kind" => kind = Some(map.next_value::<String>()?),
                "reason" => reason = Some(map.next_value::<String>()?),
                "challenge" => {
                    let text: String = map.next_value()?;
                    challenge = Some(Sha256Hex::parse(&text).map_err(store_typed)?);
                }
                "response" => {
                    let text: String = map.next_value()?;
                    response = Some(Sha256Hex::parse(&text).map_err(store_typed)?);
                }
                _ => return Err(store_typed(Error::UnknownField)),
            }
        }
        let kind = kind.ok_or_else(|| store_typed(Error::MissingField))?;
        let proof = match kind.as_str() {
            "unchallenged" => {
                if challenge.is_some() || response.is_some() {
                    return Err(store_typed(Error::UnknownField));
                }
                let reason = reason.ok_or_else(|| store_typed(Error::MissingField))?;
                if reason != UnchallengedReason::NoServerChallenge.as_str() {
                    return Err(store_typed(Error::UnknownOwnershipReason));
                }
                OwnershipProof::unchallenged()
            }
            "exe_binding_v1" => {
                if reason.is_some() {
                    return Err(store_typed(Error::UnknownField));
                }
                OwnershipProof::exe_binding_v1(
                    challenge.ok_or_else(|| store_typed(Error::MissingField))?,
                    response.ok_or_else(|| store_typed(Error::MissingField))?,
                )
            }
            _ => return Err(store_typed(Error::UnknownOwnershipKind)),
        };
        Ok(RawOwnership(proof))
    }
}
