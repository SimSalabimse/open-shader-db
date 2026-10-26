// SPDX-License-Identifier: MIT

//! Offline `consensus build` and `consensus report`.
//!
//! Counts only. No network. The threshold is the one the caller passed.
//! This module does not lower it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, ValueEnum};
use osdb_consensus::{
    build_consensus, consensus_record_bytes, format_counts, has_consensus_pipeline, load_build,
    write_index, write_record_set, AgreementThreshold, BuildId, CountingPolicy, IngestPath,
    LoadLimits, PublishMeta, PublishOutcome, PublishStagingDir, StagingRoot, COVERAGE_FILE_NAME,
    REPORT_FILE_NAME, UNSIGNED_STAGING_LINE, WAITING_FOR_DATA_LINE,
};
use osdb_coverage::GeneratedAt;
use osdb_upload::{ExeHash, GameId, GameVersion};

/// Where the caller says the staging bytes came from.
///
/// The loader does not infer this from the directory name.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum IngestArg {
    /// Reviewed pull-request tree. Those uploads are counted.
    #[value(name = "reviewed-pr")]
    ReviewedPr,
    /// Worker staging layout. Worker uploads are held unless a count flag is set.
    #[value(name = "worker")]
    Worker,
}

impl IngestArg {
    fn to_path(self) -> IngestPath {
        match self {
            Self::ReviewedPr => IngestPath::ReviewedPullRequest,
            Self::Worker => IngestPath::WorkerStaging,
        }
    }
}

/// Shared inputs for `consensus build` and `consensus report`.
///
/// Defaults are the documented bar (3 counted uploads, 500 per mille).
/// Passing a lower number is an explicit policy choice. The tool does not
/// lower the bar when few uploads were counted.
#[derive(Args)]
pub(crate) struct SharedArgs {
    /// Staging tree the caller already has. Read only. Nothing is uploaded.
    #[arg(long)]
    pub(crate) staging: PathBuf,
    /// Game id. Same rules as an upload manifest `build_id.game_id`.
    #[arg(long)]
    pub(crate) game_id: String,
    /// Version string. Same rules as an upload manifest `build_id.version`.
    #[arg(long)]
    pub(crate) version: String,
    /// Executable SHA-256, 64 lowercase hex characters.
    #[arg(long)]
    pub(crate) exe_hash: String,
    /// Minimum counted uploads that must list a hash. Default 3.
    ///
    /// Not lowered when fewer uploads were counted.
    #[arg(long, default_value_t = AgreementThreshold::DEFAULT_MIN_UPLOADS)]
    pub(crate) min_uploads: u32,
    /// Minimum share, in parts per thousand, of uploads that carry the unit.
    /// Default 500. Not lowered automatically.
    #[arg(long, default_value_t = AgreementThreshold::DEFAULT_MIN_RATIO_PER_MILLE)]
    pub(crate) min_ratio_per_mille: u16,
    /// Count Worker uploads whose ownership kind is `exe_binding_v1`.
    ///
    /// Policy only. This flag does not verify a binding and does not show
    /// that uploaders are different people or machines. Default is off, so
    /// those uploads are held. Unchallenged Worker uploads stay held. There
    /// is no flag that counts them.
    #[arg(long, default_value_t = false)]
    pub(crate) count_worker_exe_binding: bool,
    /// Where the caller says these bytes came from. Default `reviewed-pr`.
    ///
    /// `reviewed-pr` counts the uploads. `worker` holds them unless
    /// `--count-worker-exe-binding` is also set. The choice does not lower
    /// the threshold.
    #[arg(long, value_enum, default_value = "reviewed-pr")]
    pub(crate) ingest_path: IngestArg,
}

impl std::fmt::Debug for SharedArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedArgs")
            .field("staging", &"<redacted>")
            .field("game_id", &self.game_id)
            .field("version", &self.version)
            .field("exe_hash", &self.exe_hash)
            .field("min_uploads", &self.min_uploads)
            .field("min_ratio_per_mille", &self.min_ratio_per_mille)
            .field("count_worker_exe_binding", &self.count_worker_exe_binding)
            .field("ingest_path", &self.ingest_path)
            .finish()
    }
}

pub(crate) fn run(command: super::ConsensusCommand) -> Result<(), Box<dyn std::error::Error>> {
    let (shared, write) = match command {
        super::ConsensusCommand::Build {
            shared,
            out,
            records,
        } => (shared, Some((out, records))),
        super::ConsensusCommand::Report { shared } => (shared, None),
    };
    let build = parse_build(&shared)?;
    let threshold = AgreementThreshold::new(shared.min_uploads, shared.min_ratio_per_mille)?;
    let policy = CountingPolicy {
        count_worker_unchallenged: false,
        count_worker_exe_binding: shared.count_worker_exe_binding,
    };
    let root = StagingRoot::new(&shared.staging);
    let loaded = load_build(
        &root,
        &build,
        shared.ingest_path.to_path(),
        LoadLimits::default(),
    )?;
    let result = build_consensus(&build, &loaded.loaded, threshold, policy);
    let generated_at = GeneratedAt::parse(&utc_now()?)?;
    let meta = PublishMeta {
        generated_at,
        threshold,
    };
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "{}", format_counts(&result, &meta))?;
    if !has_consensus_pipeline(&result) {
        writeln!(stdout, "{WAITING_FOR_DATA_LINE}")?;
        return Ok(());
    }
    let Some((out, records)) = write else {
        return Ok(());
    };
    let filtered = match &records {
        Some(path) => {
            let candidate =
                osdb_records::load(path).map_err(|_| osdb_consensus::Error::RecordRejected)?;
            Some(consensus_record_bytes(&result, &candidate)?)
        }
        None => None,
    };
    let mut dir = PublishStagingDir::new(&out, &shared.staging);
    if let Some(path) = &records {
        dir = dir.avoid_same_file(path);
    }
    let outcome = write_index(&dir, &result, &meta)?;
    if outcome != PublishOutcome::Wrote {
        writeln!(stdout, "{WAITING_FOR_DATA_LINE}")?;
        return Ok(());
    }
    if let Some(bytes) = filtered {
        if bytes.is_empty() {
            return Err(osdb_consensus::Error::NoConsensusRecords.into());
        }
        if let Some(path) = &records {
            let candidate =
                osdb_records::load(path).map_err(|_| osdb_consensus::Error::RecordRejected)?;
            if let Err(err) = write_record_set(&dir, &result, &candidate) {
                remove_written(&out, &build);
                return Err(err.into());
            }
        }
    }
    writeln!(stdout, "{UNSIGNED_STAGING_LINE}")?;
    Ok(())
}

fn parse_build(shared: &SharedArgs) -> Result<BuildId, osdb_upload::Error> {
    Ok(BuildId::new(
        GameId::parse(&shared.game_id)?,
        GameVersion::parse(&shared.version)?,
        ExeHash::parse(&shared.exe_hash)?,
    ))
}

fn remove_written(out: &Path, build: &BuildId) {
    let dir = out.join(build.game_id.as_str()).join(format!(
        "{}--{}",
        build.version.as_str(),
        build.exe_hash.to_hex()
    ));
    let _ = std::fs::remove_file(dir.join(COVERAGE_FILE_NAME));
    let _ = std::fs::remove_file(dir.join(REPORT_FILE_NAME));
}

fn utc_now() -> Result<String, osdb_coverage::Error> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| osdb_coverage::Error::InvalidTimestamp)?;
    format_utc(secs.as_secs())
}

/// `secs` is seconds since 1970-01-01T00:00:00Z. Leap seconds are ignored.
pub(crate) fn format_utc(secs: u64) -> Result<String, osdb_coverage::Error> {
    let days = i64::try_from(secs / 86_400).map_err(|_| osdb_coverage::Error::InvalidTimestamp)?;
    let sod = secs % 86_400;
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;
    let (year, month, day) = civil_from_days(days);
    if !(0..=9_999).contains(&year) {
        return Err(osdb_coverage::Error::InvalidTimestamp);
    }
    let text = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    let _ = GeneratedAt::parse(&text)?;
    Ok(text)
}

/// Howard Hinnant's `civil_from_days`. `days` is days since 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    let year = i32::try_from(year).unwrap_or(i32::MAX);
    (
        year,
        u32::try_from(month).unwrap_or(0),
        u32::try_from(day).unwrap_or(0),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::format_utc;

    #[test]
    fn utc_epochs_match_known_instants() {
        assert_eq!(format_utc(0).unwrap(), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000).unwrap(), "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(1_761_436_800).unwrap(), "2025-10-26T00:00:00Z");
    }
}
