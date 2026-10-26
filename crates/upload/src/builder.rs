// SPDX-License-Identifier: MIT

//! Build an [`UploadManifest`] from a record file and explicit build labels.

use std::collections::BTreeSet;
use std::fmt;

use osdb_records::{pipeline_content_hash, RecordFile};
use osdb_steam::GameIdentity;

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::client::ClientVersion;
use crate::hash::Sha256Hex;
use crate::manifest::{HashSet, UploadManifest, SCHEMA_VERSION};
use crate::ownership::OwnershipProof;
use crate::time::{Clock, SystemClock, UtcHour};
use crate::{Error, Limits};

/// Collects allowlisted hashes from a record file.
///
/// Description hashes are the SHA-256 keys already stored on each record
/// for `root_signature`, `input_layout`, and `stream_out`. Bytecode hashes
/// are the per-stage SHA-256 values in `bytecode_hashes`. `pipeline_hashes`
/// is one `osdb_records::pipeline_content_hash` per record (sorted unique). Shader
/// bytes, paths, `app_key`, and `version` are never copied onto the wire.
pub struct ManifestBuilder<'a> {
    records: &'a RecordFile,
    build_id: Option<BuildId>,
    game_id: Option<GameId>,
    version: Option<GameVersion>,
    exe_hash: Option<ExeHash>,
    client_version: Option<ClientVersion>,
    collected_hour: Option<UtcHour>,
    ownership: OwnershipProof,
    limits: Limits,
}

impl fmt::Debug for ManifestBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManifestBuilder")
            .field("record_count", &self.records.records.len())
            .field("build_id", &self.build_id)
            .field("game_id", &self.game_id)
            .field("version", &self.version)
            .field("exe_hash", &self.exe_hash)
            .field("client_version", &self.client_version)
            .field("collected_hour", &self.collected_hour)
            .field("ownership", &self.ownership)
            .field("limits", &self.limits)
            .finish()
    }
}

impl<'a> ManifestBuilder<'a> {
    /// Start from a loaded record file. Ownership defaults to unchallenged.
    #[must_use]
    pub fn new(records: &'a RecordFile) -> Self {
        Self {
            records,
            build_id: None,
            game_id: None,
            version: None,
            exe_hash: None,
            client_version: None,
            collected_hour: None,
            ownership: OwnershipProof::unchallenged(),
            limits: Limits::DEFAULT,
        }
    }

    /// Set the complete build identity (overrides field-wise setters).
    #[must_use]
    pub fn build_id(mut self, build_id: BuildId) -> Self {
        self.build_id = Some(build_id);
        self
    }

    /// Set or override `game_id`.
    #[must_use]
    pub fn game_id(mut self, game_id: GameId) -> Self {
        self.game_id = Some(game_id);
        self
    }

    /// Set or override `version`.
    #[must_use]
    pub fn version(mut self, version: GameVersion) -> Self {
        self.version = Some(version);
        self
    }

    /// Set or override `exe_hash`.
    #[must_use]
    pub fn exe_hash(mut self, exe_hash: ExeHash) -> Self {
        self.exe_hash = Some(exe_hash);
        self
    }

    /// Set the client version (defaults to this crate's version).
    #[must_use]
    pub fn client_version(mut self, client_version: ClientVersion) -> Self {
        self.client_version = Some(client_version);
        self
    }

    /// Set the collected hour (defaults to the system clock, floored).
    #[must_use]
    pub fn collected_hour(mut self, collected_hour: UtcHour) -> Self {
        self.collected_hour = Some(collected_hour);
        self
    }

    /// Floor `clock.now()` to the UTC hour. The original instant is discarded.
    pub fn clock(mut self, clock: &impl Clock) -> Result<Self, Error> {
        self.collected_hour = Some(UtcHour::from_system_time(clock.now())?);
        Ok(self)
    }

    /// Accept Steam upload-safe identity. It is not serialized.
    ///
    /// Steam's depot `BuildId` is a different type from this crate's
    /// [`BuildId`] and is not copied into the manifest.
    #[must_use]
    pub fn steam_identity(self, _identity: &GameIdentity) -> Self {
        self
    }

    /// Override ownership. The prepare command leaves the default (unchallenged).
    #[must_use]
    pub fn ownership(mut self, ownership: OwnershipProof) -> Self {
        self.ownership = ownership;
        self
    }

    /// Override resource caps.
    #[must_use]
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Derive hashes and assemble a validated manifest.
    pub fn build(self) -> Result<UploadManifest, Error> {
        let build_id = self.resolve_build_id()?;
        let client_version = match self.client_version {
            Some(v) => v,
            None => ClientVersion::current()?,
        };
        let collected_hour = match self.collected_hour {
            Some(hour) => hour,
            None => UtcHour::from_system_time(SystemClock.now())?,
        };
        let (description_hashes, bytecode_hashes) = collect_hashes(
            self.records,
            self.limits.max_description_hashes,
            self.limits.max_bytecode_hashes,
        )?;
        let pipeline_hashes =
            collect_pipeline_hashes(self.records, self.limits.max_pipeline_hashes)?;
        Ok(UploadManifest {
            schema_version: SCHEMA_VERSION,
            build_id,
            client_version,
            collected_hour,
            description_hashes,
            bytecode_hashes,
            ownership: self.ownership,
            pipeline_hashes: Some(pipeline_hashes),
        })
    }

    fn resolve_build_id(&self) -> Result<BuildId, Error> {
        if let Some(build_id) = self.build_id.clone() {
            return Ok(build_id);
        }
        let game_id = match &self.game_id {
            Some(id) => id.clone(),
            None => match &self.records.build.game_id {
                Some(id) => GameId::from_records(id.clone())?,
                None => return Err(Error::IncompleteBuildId),
            },
        };
        let version = match &self.version {
            Some(v) => v.clone(),
            None => match &self.records.build.version {
                Some(v) => GameVersion::from_records(v.clone())?,
                None => return Err(Error::IncompleteBuildId),
            },
        };
        let exe_hash = match self.exe_hash {
            Some(h) => h,
            None => match self.records.build.exe_hash {
                Some(h) => ExeHash::from_records(h),
                None => return Err(Error::IncompleteBuildId),
            },
        };
        Ok(BuildId::new(game_id, version, exe_hash))
    }
}

fn collect_hashes(
    file: &RecordFile,
    max_description: u64,
    max_bytecode: u64,
) -> Result<(HashSet, HashSet), Error> {
    let mut description = BTreeSet::new();
    let mut bytecode = BTreeSet::new();
    for record in &file.records {
        if let Some(hash) = record.root_signature {
            insert_hash(
                &mut description,
                Sha256Hex::from_records(hash),
                max_description,
            )?;
        }
        if let Some(hash) = record.input_layout {
            insert_hash(
                &mut description,
                Sha256Hex::from_records(hash),
                max_description,
            )?;
        }
        if let Some(hash) = record.stream_out {
            insert_hash(
                &mut description,
                Sha256Hex::from_records(hash),
                max_description,
            )?;
        }
        for (_, hash) in record.bytecode_hashes.iter() {
            insert_hash(&mut bytecode, Sha256Hex::from_records(hash), max_bytecode)?;
        }
    }
    Ok((
        HashSet::from_capped_set(description.into_iter().collect()),
        HashSet::from_capped_set(bytecode.into_iter().collect()),
    ))
}

fn collect_pipeline_hashes(file: &RecordFile, max_hashes: u64) -> Result<HashSet, Error> {
    let mut hashes = BTreeSet::new();
    for record in &file.records {
        let digest = pipeline_content_hash(file, record);
        insert_hash(&mut hashes, Sha256Hex::from_records(digest), max_hashes)?;
    }
    Ok(HashSet::from_capped_set(hashes.into_iter().collect()))
}

fn insert_hash(
    set: &mut BTreeSet<Sha256Hex>,
    hash: Sha256Hex,
    max_hashes: u64,
) -> Result<(), Error> {
    set.insert(hash);
    let got = u64::try_from(set.len()).unwrap_or(u64::MAX);
    if got > max_hashes {
        return Err(Error::TooManyHashes {
            max: max_hashes,
            got,
        });
    }
    Ok(())
}
