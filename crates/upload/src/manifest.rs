// SPDX-License-Identifier: MIT

//! UploadManifest type, canonical JSON, and untrusted-byte parsing.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::Serialize;

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::client::ClientVersion;
use crate::hash::Sha256Hex;
use crate::ownership::{OwnershipProof, UnchallengedReason};
use crate::time::UtcHour;
use crate::{Error, Limits};

/// On-wire schema version implemented by this crate.
pub const SCHEMA_VERSION: u32 = 1;

/// Sorted unique SHA-256 hex strings.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct HashSet {
    hashes: Vec<Sha256Hex>,
}

impl HashSet {
    /// Sort, deduplicate, and enforce [`Limits::max_hashes_per_list`].
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
    LAST_TYPED_ERROR.with(|slot| slot.borrow_mut().take());
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

struct RawManifest {
    schema_version: u64,
    build_id: BuildId,
    client_version: ClientVersion,
    collected_hour: UtcHour,
    description_hashes: Vec<Sha256Hex>,
    bytecode_hashes: Vec<Sha256Hex>,
    ownership: OwnershipProof,
}

impl RawManifest {
    fn into_manifest(self, limits: Limits) -> Result<UploadManifest, Error> {
        if self.schema_version != u64::from(SCHEMA_VERSION) {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(UploadManifest {
            schema_version: SCHEMA_VERSION,
            build_id: self.build_id,
            client_version: self.client_version,
            collected_hour: self.collected_hour,
            description_hashes: HashSet::from_sorted_unique(
                self.description_hashes,
                limits.max_hashes_per_list,
            )?,
            bytecode_hashes: HashSet::from_sorted_unique(
                self.bytecode_hashes,
                limits.max_hashes_per_list,
            )?,
            ownership: self.ownership,
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
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(store_typed(Error::DuplicateKey));
            }
            match key.as_str() {
                "schema_version" => {
                    schema_version = Some(parse_u64(map.next_value()?)?);
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
        })
    }
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
