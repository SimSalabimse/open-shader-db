// SPDX-License-Identifier: MIT

//! Unsigned publish-staging writers.
//!
//! The directory this module writes is not a release. It is not signed.
//! Nothing here opens a socket. Reports are counts. The record writer copies
//! records that already passed the strict record loader and whose pipeline
//! content hash reached consensus.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use osdb_coverage::{CoverageIndex, DescriptionHash, IndexHashes, IndexThreshold, Limits};
use osdb_records::{pipeline_content_hash, RecordFile};
use osdb_upload::Sha256Hex;

use crate::consensus::{AgreementUnit, ConsensusResult, HoldReason};
use crate::staging::{build_dir_name, is_single_component};
use crate::threshold::AgreementThreshold;
use crate::{BuildId, Error};

/// Coverage index file inside one build directory.
pub const COVERAGE_FILE_NAME: &str = "coverage.v2.json";

/// Count report beside the index. No per-upload hashes and no paths.
pub const REPORT_FILE_NAME: &str = "consensus-report.json";

/// Filtered record file. Record format version stays 1.
pub const RECORDS_FILE_NAME: &str = "records.json";

/// Printed after a successful local write. This directory is not a release.
pub const UNSIGNED_STAGING_LINE: &str = "unsigned staging, not a release";

/// Printed when no pipeline hash met the threshold. Nothing is written.
pub const WAITING_FOR_DATA_LINE: &str = "waiting for data";

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Where unsigned staging files are written, and which input tree to refuse.
///
/// [`Debug`] is always `<redacted>`.
#[derive(Clone)]
pub struct PublishStagingDir {
    out: PathBuf,
    staging: PathBuf,
    avoid_same_file: Vec<PathBuf>,
}

impl PublishStagingDir {
    /// `out` is the publish root. `staging` is the input tree that must not
    /// contain `out`. Neither path is opened yet.
    #[must_use]
    pub fn new(out: impl Into<PathBuf>, staging: impl Into<PathBuf>) -> Self {
        Self {
            out: out.into(),
            staging: staging.into(),
            avoid_same_file: Vec::new(),
        }
    }

    /// Refuse to write a file that is this path (same path or same file).
    ///
    /// The path is not shown in [`Debug`].
    #[must_use]
    pub fn avoid_same_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.avoid_same_file.push(path.into());
        self
    }

    /// Publish root. Do not write this into a report.
    #[must_use]
    pub fn out(&self) -> &Path {
        &self.out
    }

    /// Staging input. Do not write this into a report.
    #[must_use]
    pub fn staging(&self) -> &Path {
        &self.staging
    }
}

impl std::fmt::Debug for PublishStagingDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Clock and threshold stamped on the index and the count report.
///
/// Pass the same [`AgreementThreshold`] that produced `result`. The writer
/// does not recompute the tally and does not lower the numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishMeta {
    /// UTC `YYYY-MM-DDTHH:MM:SSZ`. The caller supplies it so output is deterministic.
    pub generated_at: osdb_coverage::GeneratedAt,
    /// Threshold that was applied. Stored on the index; not adjusted here.
    pub threshold: AgreementThreshold,
}

/// Whether [`write_index`] created files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    /// `coverage.v2.json` and `consensus-report.json` were written.
    Wrote,
    /// No pipeline hash met the threshold. The directory was not touched.
    Waiting,
}

/// True when at least one pipeline content hash met the threshold.
#[must_use]
pub fn has_consensus_pipeline(result: &ConsensusResult) -> bool {
    result
        .accepted
        .iter()
        .any(|row| row.unit == AgreementUnit::PipelineHash)
}

/// Counts only. No paths and no per-upload or per-hash digests.
#[must_use]
pub fn format_counts(result: &ConsensusResult, meta: &PublishMeta) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "upload_count: {}", result.upload_count);
    let _ = writeln!(text, "counted_uploads: {}", result.counted_uploads);
    for unit in UNITS {
        let _ = writeln!(
            text,
            "accepted_{}: {}",
            unit.code(),
            accepted_count(result, unit)
        );
    }
    for unit in UNITS {
        let _ = writeln!(
            text,
            "rejected_{}: {}",
            unit.code(),
            rejected_count(result, unit)
        );
    }
    for reason in REASONS {
        let _ = writeln!(
            text,
            "held_{}: {}",
            reason.code(),
            held_count(result, reason)
        );
    }
    let _ = writeln!(text, "min_uploads: {}", meta.threshold.min_uploads());
    let _ = writeln!(
        text,
        "min_ratio_per_mille: {}",
        meta.threshold.min_ratio_per_mille()
    );
    let _ = writeln!(text, "generated_at: {}", meta.generated_at.as_str());
    text
}

/// Write `coverage.v2.json` and `consensus-report.json` for `result.build`.
///
/// When no pipeline hash was accepted, returns [`PublishOutcome::Waiting`]
/// and creates nothing. The files are counts plus the accepted pipeline
/// hashes. They are not a signed release.
pub fn write_index(
    dir: &PublishStagingDir,
    result: &ConsensusResult,
    meta: &PublishMeta,
) -> Result<PublishOutcome, Error> {
    if !has_consensus_pipeline(result) {
        return Ok(PublishOutcome::Waiting);
    }
    let index_bytes = coverage_bytes(result, meta)?;
    let report_bytes = report_bytes(result, meta)?;
    let coverage = output_file(dir, &result.build, COVERAGE_FILE_NAME)?;
    let report = output_file(dir, &result.build, REPORT_FILE_NAME)?;
    refuse_same_file(&coverage, &dir.avoid_same_file)?;
    refuse_same_file(&report, &dir.avoid_same_file)?;
    let build_dir = prepare_build_dir(dir, &result.build)?;
    let coverage = build_dir.join(COVERAGE_FILE_NAME);
    let report = build_dir.join(REPORT_FILE_NAME);
    write_new_file(&coverage, &index_bytes, &dir.avoid_same_file)?;
    if let Err(err) = write_new_file(&report, &report_bytes, &dir.avoid_same_file) {
        let _ = fs::remove_file(&coverage);
        return Err(err);
    }
    Ok(PublishOutcome::Wrote)
}

/// Bytes of the filtered record file, after a second pass through the strict loader.
///
/// Records whose pipeline content hash is not in the accepted set are dropped,
/// and descriptor tables that nothing references are dropped with them.
/// Zero qualifying records is [`Error::NoConsensusRecords`]. A record that
/// fails the loader (including a root signature the loader rejects) is
/// [`Error::RecordRejected`] and the bytes are not returned.
pub fn consensus_record_bytes(
    result: &ConsensusResult,
    candidate: &RecordFile,
) -> Result<Vec<u8>, Error> {
    if !record_build_matches(candidate, &result.build) {
        return Err(Error::RecordBuildMismatch);
    }
    let accepted = accepted_pipelines(result);
    let filtered = filter_records(candidate, &accepted)?;
    let encoded = osdb_records::encode_record_file(&filtered).map_err(|_| Error::RecordRejected)?;
    let checked = osdb_records::load_slice(&encoded).map_err(|_| Error::RecordRejected)?;
    if checked.records.len() != filtered.records.len() {
        return Err(Error::RecordRejected);
    }
    Ok(encoded)
}

/// Write [`RECORDS_FILE_NAME`] for records that reached pipeline consensus.
///
/// Refuses when none qualify. Does not write an index. The output is still
/// unsigned staging, not a release.
pub fn write_record_set(
    dir: &PublishStagingDir,
    result: &ConsensusResult,
    candidate: &RecordFile,
) -> Result<(), Error> {
    let bytes = consensus_record_bytes(result, candidate)?;
    let dest = output_file(dir, &result.build, RECORDS_FILE_NAME)?;
    refuse_same_file(&dest, &dir.avoid_same_file)?;
    let build_dir = prepare_build_dir(dir, &result.build)?;
    let dest = build_dir.join(RECORDS_FILE_NAME);
    write_new_file(&dest, &bytes, &dir.avoid_same_file)
}

fn output_file(dir: &PublishStagingDir, build: &BuildId, name: &str) -> Result<PathBuf, Error> {
    let game = build.game_id.as_str();
    let build_name = build_dir_name(build)?;
    if !is_single_component(game) || !is_single_component(&build_name) || !is_single_component(name)
    {
        return Err(Error::InvalidBuildId);
    }
    Ok(dir.out.join(game).join(build_name).join(name))
}

fn refuse_same_file(dest: &Path, avoid: &[PathBuf]) -> Result<(), Error> {
    for input in avoid {
        if paths_are_same(dest, input)? {
            return Err(Error::SameAsInput);
        }
    }
    Ok(())
}

const UNITS: [AgreementUnit; 3] = [
    AgreementUnit::BytecodeHash,
    AgreementUnit::DescriptorKey,
    AgreementUnit::PipelineHash,
];

const REASONS: [HoldReason; 3] = [
    HoldReason::BuildMismatch,
    HoldReason::WorkerExeBindingUnverified,
    HoldReason::WorkerUnchallenged,
];

fn accepted_count(result: &ConsensusResult, unit: AgreementUnit) -> u64 {
    u64::try_from(
        result
            .accepted
            .iter()
            .filter(|row| row.unit == unit)
            .count(),
    )
    .unwrap_or(u64::MAX)
}

fn rejected_count(result: &ConsensusResult, unit: AgreementUnit) -> u64 {
    result.rejected_counts.get(&unit).copied().unwrap_or(0)
}

fn held_count(result: &ConsensusResult, reason: HoldReason) -> u64 {
    u64::try_from(
        result
            .held
            .iter()
            .filter(|row| row.reason == reason)
            .count(),
    )
    .unwrap_or(u64::MAX)
}

fn accepted_pipelines(result: &ConsensusResult) -> BTreeSet<Sha256Hex> {
    result
        .accepted
        .iter()
        .filter(|row| row.unit == AgreementUnit::PipelineHash)
        .map(|row| row.hash)
        .collect()
}

fn coverage_bytes(result: &ConsensusResult, meta: &PublishMeta) -> Result<Vec<u8>, Error> {
    let mut hashes = Vec::new();
    for row in &result.accepted {
        if row.unit == AgreementUnit::PipelineHash {
            hashes.push(DescriptionHash::from_bytes(*row.hash.as_bytes()));
        }
    }
    let threshold = IndexThreshold::new(
        meta.threshold.min_uploads(),
        meta.threshold.min_ratio_per_mille(),
    )
    .map_err(|_| Error::InvalidThreshold)?;
    let rejected = rejected_count(result, AgreementUnit::PipelineHash);
    let index = CoverageIndex::pipeline(
        coverage_build(&result.build)?,
        IndexHashes::from_sorted_unique(hashes, Limits::DEFAULT.max_hashes)
            .map_err(|_| Error::CoverageRejected)?,
        u64::from(result.upload_count),
        accepted_count(result, AgreementUnit::PipelineHash),
        rejected,
        meta.generated_at.clone(),
        threshold,
    )
    .map_err(|_| Error::CoverageRejected)?;
    index
        .to_canonical_bytes()
        .map_err(|_| Error::CoverageRejected)
}

fn coverage_build(build: &BuildId) -> Result<osdb_coverage::BuildId, Error> {
    let game =
        osdb_coverage::GameId::parse(build.game_id.as_str()).map_err(|_| Error::InvalidBuildId)?;
    let version = osdb_coverage::GameVersion::parse(build.version.as_str())
        .map_err(|_| Error::InvalidBuildId)?;
    let exe = osdb_coverage::ExeHash::parse_hex(&build.exe_hash.to_hex())
        .map_err(|_| Error::InvalidBuildId)?;
    Ok(osdb_coverage::BuildId::new(game, version, exe))
}

fn report_bytes(result: &ConsensusResult, meta: &PublishMeta) -> Result<Vec<u8>, Error> {
    let mut accepted = serde_json::Map::new();
    let mut rejected = serde_json::Map::new();
    for unit in UNITS {
        accepted.insert(
            unit.code().to_string(),
            json_u64(accepted_count(result, unit)),
        );
        rejected.insert(
            unit.code().to_string(),
            json_u64(rejected_count(result, unit)),
        );
    }
    let mut held = serde_json::Map::new();
    for reason in REASONS {
        held.insert(
            reason.code().to_string(),
            json_u64(held_count(result, reason)),
        );
    }
    let mut threshold = serde_json::Map::new();
    threshold.insert(
        "min_ratio_per_mille".to_string(),
        json_u64(u64::from(meta.threshold.min_ratio_per_mille())),
    );
    threshold.insert(
        "min_uploads".to_string(),
        json_u64(u64::from(meta.threshold.min_uploads())),
    );
    let mut root = serde_json::Map::new();
    root.insert(
        "accepted_counts".to_string(),
        serde_json::Value::Object(accepted),
    );
    root.insert(
        "counted_uploads".to_string(),
        json_u64(u64::from(result.counted_uploads)),
    );
    root.insert(
        "generated_at".to_string(),
        serde_json::Value::String(meta.generated_at.as_str().to_string()),
    );
    root.insert("held_counts".to_string(), serde_json::Value::Object(held));
    root.insert(
        "rejected_counts".to_string(),
        serde_json::Value::Object(rejected),
    );
    root.insert(
        "threshold".to_string(),
        serde_json::Value::Object(threshold),
    );
    root.insert(
        "upload_count".to_string(),
        json_u64(u64::from(result.upload_count)),
    );
    serde_json::to_vec(&serde_json::Value::Object(root)).map_err(|_| Error::CoverageRejected)
}

fn json_u64(value: u64) -> serde_json::Value {
    serde_json::Value::Number(value.into())
}

fn record_build_matches(file: &RecordFile, build: &BuildId) -> bool {
    let Some(game) = file.build.game_id.as_ref() else {
        return false;
    };
    let Some(version) = file.build.version.as_ref() else {
        return false;
    };
    let Some(exe) = file.build.exe_hash else {
        return false;
    };
    game.as_str() == build.game_id.as_str()
        && version.as_str() == build.version.as_str()
        && exe.to_hex() == build.exe_hash.to_hex()
}

fn filter_records(
    candidate: &RecordFile,
    accepted: &BTreeSet<Sha256Hex>,
) -> Result<RecordFile, Error> {
    let mut records = Vec::new();
    for record in &candidate.records {
        let hash = Sha256Hex::from_records(pipeline_content_hash(candidate, record));
        if accepted.contains(&hash) {
            records.push(record.clone());
        }
    }
    if records.is_empty() {
        return Err(Error::NoConsensusRecords);
    }
    let mut root_signatures = BTreeMap::new();
    let mut input_layouts = BTreeMap::new();
    let mut stream_outputs = BTreeMap::new();
    for record in &records {
        if let Some(hash) = record.root_signature {
            let body = candidate
                .root_signatures
                .get(&hash)
                .ok_or(Error::RecordRejected)?;
            root_signatures.insert(hash, body.clone());
        }
        if let Some(hash) = record.input_layout {
            let body = candidate
                .input_layouts
                .get(&hash)
                .ok_or(Error::RecordRejected)?;
            input_layouts.insert(hash, body.clone());
        }
        if let Some(hash) = record.stream_out {
            let body = candidate
                .stream_outputs
                .get(&hash)
                .ok_or(Error::RecordRejected)?;
            stream_outputs.insert(hash, body.clone());
        }
    }
    Ok(RecordFile {
        format_version: candidate.format_version,
        build: candidate.build.clone(),
        root_signatures,
        input_layouts,
        stream_outputs,
        records,
    })
}

fn prepare_build_dir(dir: &PublishStagingDir, build: &BuildId) -> Result<PathBuf, Error> {
    refuse_output_dir(&dir.out, &dir.staging)?;
    let game = build.game_id.as_str();
    let build_name = build_dir_name(build)?;
    if !is_single_component(game) || !is_single_component(&build_name) {
        return Err(Error::InvalidBuildId);
    }
    ensure_real_dir(&dir.out)?;
    let game_dir = dir.out.join(game);
    if !is_single_component(game) {
        return Err(Error::InvalidBuildId);
    }
    ensure_real_dir(&game_dir)?;
    let build_dir = game_dir.join(&build_name);
    ensure_real_dir(&build_dir)?;
    refuse_output_dir(&build_dir, &dir.staging)?;
    Ok(build_dir)
}

fn refuse_output_dir(out: &Path, staging: &Path) -> Result<(), Error> {
    if let Ok(meta) = fs::symlink_metadata(out) {
        if meta.file_type().is_symlink() {
            return Err(Error::Symlink);
        }
    }
    let staging = fs::canonicalize(staging).map_err(|_| Error::Io)?;
    let probe = canonical_probe(out)?;
    if probe == staging || probe.starts_with(&staging) {
        return Err(Error::InsideStaging);
    }
    Ok(())
}

fn canonical_probe(path: &Path) -> Result<PathBuf, Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::Symlink),
        Ok(_) => fs::canonicalize(path).map_err(|_| Error::Io),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .ok_or(Error::NotADirectory)?;
            let name = path.file_name().ok_or(Error::NotADirectory)?;
            Ok(canonical_probe(parent)?.join(name))
        }
        Err(_) => Err(Error::Io),
    }
}

fn ensure_real_dir(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(Error::Symlink),
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(_) => return Err(Error::NotADirectory),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::Io),
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(Error::Io),
    }
    let meta = fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if meta.file_type().is_symlink() {
        return Err(Error::Symlink);
    }
    if !meta.is_dir() {
        return Err(Error::NotADirectory);
    }
    Ok(())
}

/// Same-directory temp file, fsync, rename. Refuses symlinks, non-files,
/// existing files, and a destination that is the same file as an input.
fn write_new_file(dest: &Path, bytes: &[u8], avoid: &[PathBuf]) -> Result<(), Error> {
    refuse_same_file(dest, avoid)?;
    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(Error::Symlink),
        Ok(meta) if !meta.file_type().is_file() => return Err(Error::NotRegular),
        Ok(_) => return Err(Error::AlreadyExists),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::Io),
    }
    let tmp = temp_sibling(dest);
    let mut guard = TempGuard {
        path: tmp.clone(),
        persist: false,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|_| Error::Io)?;
    file.write_all(bytes).map_err(|_| Error::Io)?;
    file.flush().map_err(|_| Error::Io)?;
    file.sync_all().map_err(|_| Error::Io)?;
    drop(file);
    fs::rename(&tmp, dest).map_err(|_| Error::Io)?;
    guard.persist = true;
    Ok(())
}

fn paths_are_same(dest: &Path, input: &Path) -> Result<bool, Error> {
    if dest == input {
        return Ok(true);
    }
    match (fs::symlink_metadata(dest), fs::symlink_metadata(input)) {
        (Ok(left), Ok(right)) => {
            if left.file_type().is_symlink() || right.file_type().is_symlink() {
                return Ok(false);
            }
            if left.file_type().is_file() && right.file_type().is_file() {
                return same_file::is_same_file(dest, input).map_err(|_| Error::Io);
            }
            Ok(false)
        }
        (Err(err), _) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        (_, Err(err)) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        _ => Err(Error::Io),
    }
}

fn temp_sibling(dest: &Path) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("out"));
    dest.with_file_name(format!(".{name}.osdb-{}-{n}.tmp", std::process::id()))
}

struct TempGuard {
    path: PathBuf,
    persist: bool,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if !self.persist {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use osdb_records::{
        pipeline_content_hash, AppKeyHex, BuildIdentity, ExeHash as RecordExe,
        GameId as RecordGame, GameVersion as RecordVersion, HexBytes, PipelineStateRecord,
        RecordFile, Sha256, StageHashes,
    };
    use osdb_upload::{
        ClientVersion, ExeHash, GameId, GameVersion, HashSet, OwnershipProof, Sha256Hex,
        UploadManifest, UtcHour,
    };

    use crate::staging::{sha256_of, IngestPath, LoadLimits, StagingRoot};
    use crate::threshold::{AgreementThreshold, CountingPolicy};
    use crate::{build_consensus, load_build, BuildId};

    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "osdb-publish-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn build_id() -> BuildId {
        BuildId::new(
            GameId::parse("synthetic").unwrap(),
            GameVersion::parse("1.0.0").unwrap(),
            ExeHash::parse("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap(),
        )
    }

    fn record_build() -> BuildIdentity {
        BuildIdentity::new(
            Some(RecordGame::parse("synthetic").unwrap()),
            Some(RecordVersion::parse("1.0.0").unwrap()),
            Some(
                RecordExe::parse_hex(
                    "1111111111111111111111111111111111111111111111111111111111111111",
                )
                .unwrap(),
            ),
        )
    }

    fn bare_record(app: &str, mask: Option<i64>) -> PipelineStateRecord {
        PipelineStateRecord {
            app_key: AppKeyHex::parse_hex(app).unwrap(),
            version: 0,
            bytecode_hashes: StageHashes::default(),
            unresolved_stages: Vec::new(),
            root_signature: None,
            root_signature_skip: None,
            input_layout: None,
            depth_stencil: None,
            blend: None,
            rasterizer: None,
            render_target_formats: None,
            view_instancing: None,
            stream_out: None,
            sample_desc: None,
            sample_mask: mask,
            ib_strip_cut_value: None,
            primitive_topology: None,
            dsv_format: None,
            node_mask: None,
            flags: None,
        }
    }

    fn file_with(records: Vec<PipelineStateRecord>) -> RecordFile {
        RecordFile {
            format_version: 1,
            build: record_build(),
            root_signatures: BTreeMap::new(),
            input_layouts: BTreeMap::new(),
            stream_outputs: BTreeMap::new(),
            records,
        }
    }

    fn hash_set(items: &[Sha256Hex]) -> HashSet {
        let mut owned = items.to_vec();
        owned.sort();
        owned.dedup();
        HashSet::from_sorted_unique(owned, 100).unwrap()
    }

    fn manifest(build: &BuildId, hour: u8, pipelines: &[Sha256Hex]) -> UploadManifest {
        UploadManifest {
            schema_version: 2,
            build_id: build.clone(),
            client_version: ClientVersion::parse("0.1.0").unwrap(),
            collected_hour: UtcHour::from_ymd_h(2026, 10, 26, hour).unwrap(),
            description_hashes: hash_set(&[Sha256Hex::from_bytes([0x22; 32])]),
            bytecode_hashes: hash_set(&[Sha256Hex::from_bytes([0x33; 32])]),
            ownership: OwnershipProof::unchallenged(),
            pipeline_hashes: Some(hash_set(pipelines)),
        }
    }

    fn meta() -> PublishMeta {
        PublishMeta {
            generated_at: osdb_coverage::GeneratedAt::parse("2026-10-26T00:00:00Z").unwrap(),
            threshold: AgreementThreshold::default(),
        }
    }

    fn write_tree(root: &Path, manifests: &[UploadManifest]) {
        let build = build_id();
        let dir = root.join(build.game_id.as_str()).join(format!(
            "{}--{}",
            build.version.as_str(),
            build.exe_hash.to_hex()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for manifest in manifests {
            let bytes = manifest.to_canonical_bytes().unwrap();
            let name = sha256_of(&bytes).to_hex();
            std::fs::write(dir.join(format!("{name}.json")), bytes).unwrap();
        }
    }

    fn result_from_tree(root: &Path) -> crate::ConsensusResult {
        let staging = StagingRoot::new(root);
        let build = build_id();
        let report = load_build(
            &staging,
            &build,
            IngestPath::ReviewedPullRequest,
            LoadLimits::default(),
        )
        .unwrap();
        build_consensus(
            &build,
            &report.loaded,
            AgreementThreshold::default(),
            CountingPolicy::default(),
        )
    }

    fn publish_at(staging: &Path, out: &Path, manifests: &[UploadManifest]) -> PathBuf {
        write_tree(staging, manifests);
        let result = result_from_tree(staging);
        let dir = PublishStagingDir::new(out, staging);
        let outcome = write_index(&dir, &result, &meta()).unwrap();
        assert_eq!(outcome, PublishOutcome::Wrote);
        let build = build_id();
        out.join(build.game_id.as_str()).join(format!(
            "{}--{}",
            build.version.as_str(),
            build.exe_hash.to_hex()
        ))
    }

    #[test]
    fn publish_bytes_ignore_directory_order() {
        let file = file_with(vec![bare_record("01", None), bare_record("02", Some(1))]);
        let agreed = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[0]));
        let other = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[1]));
        let build = build_id();
        let manifests = [
            manifest(&build, 1, &[agreed, other]),
            manifest(&build, 2, &[agreed]),
            manifest(&build, 3, &[agreed]),
        ];
        let staging_a = Temp(temp_dir("stage-a"));
        let out_a = Temp(temp_dir("out-a"));
        let dir_a = publish_at(&staging_a.0, &out_a.0, &manifests);
        let reversed: Vec<_> = manifests.iter().rev().cloned().collect();
        let staging_b = Temp(temp_dir("stage-b"));
        let out_b = Temp(temp_dir("out-b"));
        let dir_b = publish_at(&staging_b.0, &out_b.0, &reversed);
        let coverage_a = std::fs::read(dir_a.join(COVERAGE_FILE_NAME)).unwrap();
        let coverage_b = std::fs::read(dir_b.join(COVERAGE_FILE_NAME)).unwrap();
        let report_a = std::fs::read(dir_a.join(REPORT_FILE_NAME)).unwrap();
        let report_b = std::fs::read(dir_b.join(REPORT_FILE_NAME)).unwrap();
        assert_eq!(coverage_a, coverage_b);
        assert_eq!(report_a, report_b);
        let expected = format!(
            r#"{{"build_id":{{"exe_hash":"{}","game_id":"synthetic","version":"1.0.0"}},"consensus_count":1,"format_version":2,"generated_at":"2026-10-26T00:00:00Z","hashes":["{}"],"rejected_count":1,"threshold":{{"min_ratio_per_mille":500,"min_uploads":3}},"upload_count":3}}"#,
            build.exe_hash.to_hex(),
            agreed.to_hex()
        );
        assert_eq!(std::str::from_utf8(&coverage_a).unwrap(), expected);
        let report = std::str::from_utf8(&report_a).unwrap();
        assert!(report.contains(
            r#""accepted_counts":{"bytecode_hash":1,"descriptor_key":1,"pipeline_hash":1}"#
        ));
        assert!(report.contains(
            r#""rejected_counts":{"bytecode_hash":0,"descriptor_key":0,"pipeline_hash":1}"#
        ));
        assert!(report.contains(r#""upload_count":3"#));
        assert!(!report.contains(&agreed.to_hex()));
        assert!(!report.contains(&other.to_hex()));
        let object = sha256_of(&manifests[0].to_canonical_bytes().unwrap()).to_hex();
        assert!(!report.contains(&object));
        assert!(!std::str::from_utf8(&coverage_a).unwrap().contains(&object));
        let counts = format_counts(&result_from_tree(&staging_a.0), &meta());
        assert!(counts.contains("accepted_pipeline_hash: 1"));
        assert!(counts.contains("rejected_pipeline_hash: 1"));
        assert!(!counts.contains(&agreed.to_hex()));
        assert!(!counts.contains(&object));
        assert!(!counts.contains(&staging_a.0.to_string_lossy().to_string()));
    }

    #[test]
    fn below_bar_writes_nothing() {
        let file = file_with(vec![bare_record("01", None)]);
        let agreed = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[0]));
        let build = build_id();
        let manifests = [
            manifest(&build, 1, &[agreed]),
            manifest(&build, 2, &[agreed]),
        ];
        let staging = Temp(temp_dir("stage-wait"));
        let out = Temp(temp_dir("out-wait"));
        write_tree(&staging.0, &manifests);
        let result = result_from_tree(&staging.0);
        let outcome = write_index(
            &PublishStagingDir::new(&out.0, &staging.0),
            &result,
            &meta(),
        )
        .unwrap();
        assert_eq!(outcome, PublishOutcome::Waiting);
        assert!(std::fs::read_dir(&out.0).unwrap().next().is_none());
        let text = format_counts(&result, &meta());
        assert!(text.contains("counted_uploads: 2"));
        assert!(!has_consensus_pipeline(&result));
    }

    #[test]
    fn record_set_keeps_only_consensus_records() {
        let file = file_with(vec![bare_record("01", None), bare_record("02", Some(7))]);
        let agreed = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[0]));
        let other = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[1]));
        let build = build_id();
        let manifests = [
            manifest(&build, 1, &[agreed]),
            manifest(&build, 2, &[agreed]),
            manifest(&build, 3, &[agreed, other]),
        ];
        let staging = Temp(temp_dir("stage-rec"));
        let out = Temp(temp_dir("out-rec"));
        let dir = publish_at(&staging.0, &out.0, &manifests);
        let result = result_from_tree(&staging.0);
        write_record_set(&PublishStagingDir::new(&out.0, &staging.0), &result, &file).unwrap();
        let bytes = std::fs::read(dir.join(RECORDS_FILE_NAME)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"app_key\": \"01\""));
        assert!(!text.contains("\"app_key\": \"02\""));
        assert!(!text.contains(&other.to_hex()));
        let loaded = osdb_records::load_slice(text.as_bytes()).unwrap();
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(
            Sha256Hex::from_records(pipeline_content_hash(&loaded, &loaded.records[0])),
            agreed
        );
    }

    #[test]
    fn malformed_record_is_refused_and_writes_nothing() {
        const CANARY: &[u8] = b"PUBLISH_DXIL_CANARY_9f3a";
        let mut hostile = vec![1, 0, 0, 0];
        hostile.extend_from_slice(CANARY);
        let hash = Sha256::from_bytes(*sha256_of(&hostile).as_bytes());
        let mut record = bare_record("01", None);
        record.root_signature = Some(hash);
        let mut file = file_with(vec![record]);
        file.root_signatures
            .insert(hash, HexBytes::from_bytes(&hostile));
        let agreed = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[0]));
        let build = build_id();
        let manifests = [
            manifest(&build, 1, &[agreed]),
            manifest(&build, 2, &[agreed]),
            manifest(&build, 3, &[agreed]),
        ];
        let staging = Temp(temp_dir("stage-bad"));
        write_tree(&staging.0, &manifests);
        let out = Temp(temp_dir("out-bad"));
        let result = result_from_tree(&staging.0);
        let err = write_record_set(&PublishStagingDir::new(&out.0, &staging.0), &result, &file)
            .unwrap_err();
        assert!(matches!(err, Error::RecordRejected), "{err:?}");
        assert!(!err.to_string().contains("PUBLISH_DXIL_CANARY"));
        assert!(std::fs::read_dir(&out.0).unwrap().next().is_none());
    }

    #[test]
    fn empty_result_writes_nothing() {
        let result = build_consensus(
            &build_id(),
            &[],
            AgreementThreshold::default(),
            CountingPolicy::default(),
        );
        let staging = Temp(temp_dir("stage-empty"));
        let out = Temp(temp_dir("out-empty"));
        let dir = PublishStagingDir::new(&out.0, &staging.0);
        assert_eq!(
            write_index(&dir, &result, &meta()).unwrap(),
            PublishOutcome::Waiting
        );
        let err =
            write_record_set(&dir, &result, &file_with(vec![bare_record("01", None)])).unwrap_err();
        assert!(matches!(err, Error::NoConsensusRecords), "{err:?}");
        assert!(std::fs::read_dir(&out.0).unwrap().next().is_none());
    }

    #[test]
    fn refuses_output_inside_staging_symlink_and_existing_file() {
        let file = file_with(vec![bare_record("01", None)]);
        let agreed = Sha256Hex::from_records(pipeline_content_hash(&file, &file.records[0]));
        let build = build_id();
        let manifests = [
            manifest(&build, 1, &[agreed]),
            manifest(&build, 2, &[agreed]),
            manifest(&build, 3, &[agreed]),
        ];
        let staging = Temp(temp_dir("stage-refuse"));
        write_tree(&staging.0, &manifests);
        let result = result_from_tree(&staging.0);
        let inside = staging.0.join("nested-out");
        let err = write_index(
            &PublishStagingDir::new(&inside, &staging.0),
            &result,
            &meta(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::InsideStaging), "{err:?}");
        assert!(!inside.exists());

        let out = Temp(temp_dir("out-exist"));
        let dir = publish_at(&staging.0, &out.0, &manifests);
        let coverage = dir.join(COVERAGE_FILE_NAME);
        let original = std::fs::read(&coverage).unwrap();
        let err = write_index(
            &PublishStagingDir::new(&out.0, &staging.0),
            &result,
            &meta(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::AlreadyExists), "{err:?}");
        assert_eq!(std::fs::read(&coverage).unwrap(), original);

        let out_same = Temp(temp_dir("out-same-real"));
        let predicted = output_file(
            &PublishStagingDir::new(&out_same.0, &staging.0),
            &result.build,
            RECORDS_FILE_NAME,
        )
        .unwrap();
        let err = write_record_set(
            &PublishStagingDir::new(&out_same.0, &staging.0).avoid_same_file(&predicted),
            &result,
            &file,
        )
        .unwrap_err();
        assert!(matches!(err, Error::SameAsInput), "{err:?}");
        assert!(!predicted.exists());

        #[cfg(unix)]
        {
            let target = Temp(temp_dir("link-target"));
            let link = target.0.with_file_name(format!(
                "osdb-publish-link-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            std::os::unix::fs::symlink(&target.0, &link).unwrap();
            let err = write_index(&PublishStagingDir::new(&link, &staging.0), &result, &meta())
                .unwrap_err();
            assert!(matches!(err, Error::Symlink), "{err:?}");
            assert!(std::fs::read_dir(&target.0).unwrap().next().is_none());
            let _ = std::fs::remove_file(&link);
        }
    }

    #[test]
    fn debug_redacts_publish_dir() {
        let staging = Temp(temp_dir("redact-stage"));
        let out = Temp(temp_dir("redact-out"));
        let dir = PublishStagingDir::new(&out.0, &staging.0).avoid_same_file(&out.0);
        let text = format!("{dir:?}");
        assert_eq!(text, "<redacted>");
        assert!(!text.contains(&out.0.to_string_lossy().to_string()));
    }
}
