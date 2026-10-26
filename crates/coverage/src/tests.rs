// SPDX-License-Identifier: MIT

//! Synthetic fixtures only. Index data is CC0; SODB blobs are fake bytes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::Duration;

use rusqlite::Connection;

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::hash::DescriptionHash;
#[cfg(unix)]
use crate::index::open_regular_nonblocking;
use crate::index::{
    load, load_with_limits, read_capped, reset_read_reached, take_read_reached, CoverageIndex,
    GeneratedAt, IndexHashes, IndexThreshold, RedactedPath, FORMAT_VERSION, FORMAT_VERSION_V2,
    UNSIGNED_INDEX_LINE, UNSIGNED_INDEX_NOTE,
};
use crate::report::{MatchRate, BUILD_IDENTITY_NOT_COMPARED, BUILD_IDENTITY_REASON};
use crate::{compare, compare_hashes, local_description_hashes, Error, Limits};

use sodb::{SODB_APPLICATION_ID, SODB_USER_VERSION};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

const ZERO_EXE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const HASH_Z: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.path);
    }
}

fn temp_path(suffix: &str) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("osdb-coverage-{}-{n}{suffix}", std::process::id()))
}

fn write_temp(suffix: &str, bytes: impl AsRef<[u8]>) -> TempFile {
    let path = temp_path(suffix);
    std::fs::write(&path, bytes).unwrap();
    TempFile { path }
}

fn sample_build() -> BuildId {
    BuildId::new(
        GameId::parse("synthetic").unwrap(),
        GameVersion::parse("1").unwrap(),
        ExeHash::parse_hex(ZERO_EXE).unwrap(),
    )
}

fn index_from_hex(hashes: &[&str]) -> CoverageIndex {
    let parsed = hashes
        .iter()
        .map(|h| DescriptionHash::parse_hex(h).unwrap())
        .collect();
    CoverageIndex::description(
        sample_build(),
        IndexHashes::from_unsorted(parsed, Limits::DEFAULT.max_hashes).unwrap(),
        3,
        2,
        GeneratedAt::parse("2026-10-25T00:00:00Z").unwrap(),
    )
}

fn write_index_json(body: &str) -> TempFile {
    write_temp(".json", body)
}

fn canonical_index_file(hashes: &[&str]) -> TempFile {
    let json = serde_json::to_string(&index_from_hex(hashes)).unwrap();
    write_index_json(&json)
}

fn create_sodb(setup: impl FnOnce(&mut Connection) -> rusqlite::Result<()>) -> TempFile {
    let path = temp_path(".sodb");
    let mut conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)
        .unwrap();
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)
        .unwrap();
    setup(&mut conn).unwrap();
    drop(conn);
    TempFile { path }
}

fn create_core_tables(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE app_id (
            id INTEGER NOT NULL PRIMARY KEY,
            exe TEXT NOT NULL,
            app_name TEXT NOT NULL,
            engine_name TEXT,
            app_version INTEGER NOT NULL,
            engine_version INTEGER NOT NULL
        );
        CREATE TABLE shader_bytecode (
            Key BLOB NOT NULL PRIMARY KEY,
            Type TEXT,
            Bytecode BLOB
        );
        CREATE TABLE root_signatures (
            Key BLOB NOT NULL PRIMARY KEY,
            value BLOB NOT NULL
        );
        CREATE TABLE pipeline_states (
            Key BLOB NOT NULL PRIMARY KEY,
            RootSignature BLOB,
            ByteCode_VS BLOB,
            Flags INTEGER
        );
        CREATE TABLE state_objects (
            Key BLOB NOT NULL PRIMARY KEY,
            Type INTEGER,
            NodeMask INTEGER,
            Flags INTEGER,
            AddToStateObjectParent BLOB
        );
        CREATE TABLE groups (
            Key BLOB NOT NULL PRIMARY KEY,
            Version INTEGER NOT NULL,
            PSOKey BLOB,
            SOKey BLOB
        );
        ",
    )?;
    Ok(())
}

fn insert_app(conn: &Connection, name: &str, version: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_id (id, exe, app_name, app_version, engine_version)
         VALUES (1, 'synthetic.exe', ?1, ?2, 0)",
        rusqlite::params![name, version],
    )?;
    Ok(())
}

fn insert_pso(conn: &Connection, key: &[u8], flags: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, 'vs', ?2)",
        rusqlite::params![key, b"FAKE_VS"],
    )?;
    conn.execute(
        "INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (?1, ?2, ?3)",
        rusqlite::params![key, key, flags],
    )?;
    Ok(())
}

fn local_with_psos(name: &str, version: i64, flags: &[i64]) -> (TempFile, sodb::SodbSummary) {
    let file = create_sodb(|conn| {
        create_core_tables(conn)?;
        insert_app(conn, name, version)?;
        for (i, flag) in flags.iter().enumerate() {
            let key = format!("pso-{i}").into_bytes();
            insert_pso(conn, &key, *flag)?;
        }
        Ok(())
    });
    let summary = sodb::open(file.path()).unwrap();
    (file, summary)
}

#[test]
fn full_match() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[0, 1]);
    let hashes = local_description_hashes(&summary);
    assert_eq!(hashes.len(), 2);
    let hex: Vec<String> = hashes.iter().map(DescriptionHash::to_hex).collect();
    let refs: Vec<&str> = hex.iter().map(String::as_str).collect();
    let index_file = canonical_index_file(&refs);
    let index = load(index_file.path()).unwrap();
    let report = compare(&summary, &index, 100).unwrap();
    assert_eq!(report.matched_count, 2);
    assert_eq!(report.unmatched_count, 0);
    assert_eq!(report.match_rate.percent_rounded, 100);
    assert!(report.unmatched_hashes.is_empty());
    assert_eq!(report.build_identity, BUILD_IDENTITY_NOT_COMPARED);
    assert_eq!(report.build_identity_reason, BUILD_IDENTITY_REASON);
    assert!(report.index_unsigned);
    assert!(!report.index_verified);
    assert_eq!(report.index_note, UNSIGNED_INDEX_NOTE);
}

#[test]
fn partial_match() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[0, 1, 2]);
    let hashes = local_description_hashes(&summary);
    let index = index_from_hex(&[&hashes[0].to_hex()]);
    let report = compare(&summary, &index, 100).unwrap();
    assert_eq!(report.matched_count, 1);
    assert_eq!(report.unmatched_count, 2);
    assert_eq!(report.index_only_count, 0);
    assert_eq!(report.unmatched_hashes.len(), 2);
}

#[test]
fn zero_match() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[0]);
    let index = index_from_hex(&[HASH_A]);
    let report = compare(&summary, &index, 100).unwrap();
    assert_eq!(report.matched_count, 0);
    assert_eq!(report.unmatched_count, 1);
    assert_eq!(report.index_only_count, 1);
    assert_eq!(report.match_rate.percent_rounded, 0);
}

#[test]
fn empty_local_sodb() {
    let file = create_sodb(|conn| {
        create_core_tables(conn)?;
        insert_app(conn, "synthetic", 1)?;
        Ok(())
    });
    let summary = sodb::open(file.path()).unwrap();
    let index = index_from_hex(&[HASH_A]);
    let report = compare(&summary, &index, 100).unwrap();
    assert_eq!(report.local_hash_count, 0);
    assert_eq!(report.matched_count, 0);
    assert_eq!(report.index_only_count, 1);
    assert_eq!(report.match_rate.numerator, 0);
    assert_eq!(report.match_rate.denominator, 0);
    assert_eq!(report.match_rate.percent_rounded, 100);
}

#[test]
fn empty_index() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[0]);
    let index = index_from_hex(&[]);
    let report = compare(&summary, &index, 100).unwrap();
    assert_eq!(report.index_hash_count, 0);
    assert_eq!(report.matched_count, 0);
    assert_eq!(report.unmatched_count, 1);
}

#[test]
fn duplicate_hashes_in_index_are_deduped() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{HASH_A}","{HASH_A}"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let index = load(file.path()).unwrap();
    assert_eq!(index.hashes.len(), 1);
    assert_eq!(index.hashes.as_slice()[0].to_hex(), HASH_A);
}

#[test]
fn unsorted_index_is_sorted_at_load() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{HASH_Z}","{HASH_A}"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let index = load(file.path()).unwrap();
    let got: Vec<String> = index.hashes.as_slice().iter().map(|h| h.to_hex()).collect();
    assert_eq!(got, vec![HASH_A.to_string(), HASH_Z.to_string()]);
}

#[test]
fn huge_file_is_rejected() {
    let limits = Limits {
        max_file_bytes: 64,
        ..Limits::DEFAULT
    };
    let file = write_temp(".json", vec![b'x'; 80]);
    let err = load_with_limits(file.path(), limits).unwrap_err();
    assert!(
        matches!(err, Error::FileTooLarge { max: 64, got: 80 }),
        "got {err:?}"
    );
}

#[test]
fn huge_hash_count_is_rejected() {
    let limits = Limits {
        max_hashes: 2,
        ..Limits::DEFAULT
    };
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{HASH_A}","{HASH_B}","{HASH_C}"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load_with_limits(file.path(), limits).unwrap_err();
    assert!(
        matches!(err, Error::TooManyHashes { max: 2, got: 3 }),
        "got {err:?}"
    );
}

#[test]
fn bad_hex_is_rejected() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::MalformedHex), "got {err:?}");
}

#[test]
fn uppercase_hex_is_rejected() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::MalformedHex), "got {err:?}");
}

#[test]
fn wrong_hex_length_is_rejected() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["aaa"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(
        matches!(err, Error::WrongHexLength { got: 3 }),
        "got {err:?}"
    );
}

#[test]
fn unknown_format_version_is_rejected() {
    let json = format!(
        r#"{{"format_version":3,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedFormatVersion { found: 3 }),
        "got {err:?}"
    );
}

#[test]
fn duplicate_keys_are_rejected() {
    let json = format!(
        r#"{{"format_version":1,"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "got {err:?}");
}

#[test]
fn unknown_top_level_field_is_rejected() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z","extra":true}}"#
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::UnknownField), "got {err:?}");
}

#[test]
fn truncated_json_is_rejected() {
    let file = write_index_json(r#"{"format_version":1"#);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::InvalidJson), "got {err:?}");
}

#[test]
fn deep_nesting_is_rejected() {
    let nested = "[".repeat(16) + &"]".repeat(16);
    let file = write_index_json(&nested);
    let err = load(file.path()).unwrap_err();
    assert!(
        matches!(err, Error::NestingTooDeep { max: 8 }),
        "got {err:?}"
    );
}

#[test]
fn trailing_garbage_is_rejected() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}{{}}"#,
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::TrailingGarbage), "got {err:?}");
}

#[test]
fn directory_is_not_a_regular_file() {
    let dir = temp_path("-dir");
    std::fs::create_dir(&dir).unwrap();
    let err = load(&dir).unwrap_err();
    let _ = std::fs::remove_dir(&dir);
    assert!(matches!(err, Error::NotARegularFile), "got {err:?}");
}

#[cfg(unix)]
#[test]
fn symlink_is_not_a_regular_file() {
    let target = canonical_index_file(&[]);
    let link = temp_path(".json");
    std::os::unix::fs::symlink(target.path(), &link).unwrap();
    let err = load(&link).unwrap_err();
    let _ = std::fs::remove_file(&link);
    assert!(matches!(err, Error::NotARegularFile), "got {err:?}");
}

#[cfg(unix)]
#[test]
fn fifo_is_not_a_regular_file_without_hanging() {
    let path = temp_path(".json");
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());

    let path_for_thread = path.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let result = load(&path_for_thread);
        let _ = tx.send(result);
    });
    let received = rx.recv_timeout(Duration::from_secs(2));
    let _ = std::fs::remove_file(&path);
    let result = received.expect("FIFO open hung; regular-file / O_NONBLOCK check regressed");
    assert!(
        matches!(result, Err(Error::NotARegularFile) | Err(Error::Io)),
        "got {result:?}"
    );
    handle.join().unwrap();
}

#[test]
fn build_identity_is_not_compared() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[0]);
    let mut index = index_from_hex(&[]);
    index.build_id = BuildId::new(
        GameId::parse("othergame").unwrap(),
        GameVersion::parse("9").unwrap(),
        ExeHash::parse_hex(ZERO_EXE).unwrap(),
    );
    let report = compare(&summary, &index, 100).unwrap();
    assert_eq!(report.build_identity, BUILD_IDENTITY_NOT_COMPARED);
    let human = report.format_human();
    assert!(human.contains("Build identity: not_compared"));
    assert!(human.contains(BUILD_IDENTITY_REASON));
    assert!(!human.contains("does not match"));
}

#[test]
fn compare_output_is_deterministic() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[4, 1, 9]);
    let index = index_from_hex(&[HASH_A, HASH_B]);
    let a = serde_json::to_string(&compare(&summary, &index, 100).unwrap()).unwrap();
    let b = serde_json::to_string(&compare(&summary, &index, 100).unwrap()).unwrap();
    assert_eq!(a, b);
    let report = compare(&summary, &index, 100).unwrap();
    let hashes: Vec<String> = report
        .unmatched_hashes
        .iter()
        .map(DescriptionHash::to_hex)
        .collect();
    let mut sorted = hashes.clone();
    sorted.sort();
    assert_eq!(hashes, sorted);
}

#[test]
fn serialized_report_contains_no_personal_data() {
    let marker_user = "os-user-not-personal";
    let marker_tmp = "TotallySecretTempDir";
    let dir = std::env::temp_dir().join(format!(
        "{marker_tmp}-{}-{}",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sodb_path = dir.join(format!("{marker_user}-game.sodb"));
    let index_path = dir.join(format!("{marker_user}-index.json"));

    let mut conn = Connection::open(&sodb_path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)
        .unwrap();
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)
        .unwrap();
    create_core_tables(&mut conn).unwrap();
    insert_app(&conn, "synthetic", 1).unwrap();
    insert_pso(&conn, b"pso-0", 0).unwrap();
    drop(conn);

    let summary = sodb::open(&sodb_path).unwrap();
    let json = serde_json::to_string(&index_from_hex(&[HASH_A])).unwrap();
    std::fs::write(&index_path, json).unwrap();
    let index = load(&index_path).unwrap();
    let report = compare(&summary, &index, 100).unwrap();
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains(marker_user));
    assert!(!serialized.contains(marker_tmp));
    assert!(!serialized.contains(sodb_path.to_string_lossy().as_ref()));
    assert!(!serialized.contains("/home/"));
    assert!(!serialized.contains("Users"));
    assert!(!report.format_human().contains(marker_user));

    let _ = std::fs::remove_file(&sodb_path);
    let _ = std::fs::remove_file(&index_path);
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn redacted_path_debug_hides_location() {
    let path = RedactedPath::new("/home/os-user/secret/index.json");
    let debug = format!("{path:?}");
    assert_eq!(debug, "<redacted>");
    assert!(!debug.contains("os-user"));
    assert!(!debug.contains("secret"));
}

#[test]
fn match_rate_uses_integer_rounding() {
    assert_eq!(MatchRate::from_counts(1, 3).percent_rounded, 33);
    assert_eq!(MatchRate::from_counts(2, 3).percent_rounded, 67);
    assert_eq!(MatchRate::from_counts(1, 2).percent_rounded, 50);
    assert_eq!(MatchRate::from_counts(0, 0).percent_rounded, 100);
    assert_eq!(MatchRate::from_counts(0, 5).percent_rounded, 0);
    assert_eq!(MatchRate::from_counts(5, 5).percent_rounded, 100);
    assert_eq!(MatchRate::from_counts(199, 200).percent_rounded, 99);
    assert_eq!(MatchRate::from_counts(199, 200).numerator, 199);
    let one_third = MatchRate::from_counts(1, 3);
    assert_eq!(one_third.numerator, 1);
    assert_eq!(one_third.denominator, 3);
}

#[test]
fn max_unmatched_cut_is_reported() {
    let hashes: Vec<DescriptionHash> = [HASH_A, HASH_B, HASH_C]
        .into_iter()
        .map(|h| DescriptionHash::parse_hex(h).unwrap())
        .collect();
    let index = index_from_hex(&[]);
    let report = compare_hashes(&hashes, &index, 1).unwrap();
    assert_eq!(report.unmatched_hashes.len(), 1);
    assert_eq!(report.unmatched_omitted, 2);
    assert_eq!(report.unmatched_count, 3);
    let human = report.format_human();
    assert!(human.contains("2 unmatched hashes omitted"));
    let one = compare_hashes(&hashes[..2], &index, 1).unwrap();
    assert!(one.format_human().contains("1 unmatched hash omitted"));
    assert!(human.contains(UNSIGNED_INDEX_LINE));
    assert!(human.contains(UNSIGNED_INDEX_NOTE));
}

#[test]
fn error_display_never_includes_path() {
    let path = temp_path("-missing.json");
    let err = load(&path).unwrap_err();
    let text = err.to_string();
    assert!(!text.contains(path.to_string_lossy().as_ref()));
    assert!(!text.contains("osdb-coverage"));
}

#[test]
fn default_hash_cap_rejects_over_one_million_in_visitor() {
    let limits = Limits {
        max_hashes: 2,
        ..Limits::DEFAULT
    };
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{HASH_A}","{HASH_B}","{HASH_C}"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let err = load_with_limits(file.path(), limits).unwrap_err();
    assert!(
        matches!(err, Error::TooManyHashes { max: 2, got: 3 }),
        "got {err:?}"
    );
}

#[test]
fn load_rejects_invalid_build_ids_as_typed_errors() {
    for game_id in [
        "C:\\\\Games\\\\Title",
        "has space",
        "\\\\u0000nul",
        "ä",
        &"x".repeat(129),
    ] {
        let json = format!(
            r#"{{"format_version":1,"build_id":{{"game_id":"{game_id}","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
        );
        let file = write_index_json(&json);
        let err = load(file.path()).unwrap_err();
        assert!(
            matches!(err, Error::InvalidBuildId),
            "game_id {game_id:?} -> {err:?}"
        );
        assert_eq!(err.to_string(), "invalid build identifier");
    }
}

#[test]
fn load_rejects_invalid_timestamps_as_typed_errors() {
    for stamp in [
        "2026-02-29T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-10-25T24:00:00Z",
        "2026-10-25T00:00:00+00:00",
        "2026-10-25T00:00:60Z",
        "not-a-date",
    ] {
        let json = format!(
            r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"{stamp}"}}"#
        );
        let file = write_index_json(&json);
        let err = load(file.path()).unwrap_err();
        assert!(
            matches!(err, Error::InvalidTimestamp),
            "stamp {stamp} -> {err:?}"
        );
    }
    let leap = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2024-02-29T00:00:00Z"}}"#
    );
    let file = write_index_json(&leap);
    load(file.path()).unwrap();
}

#[test]
fn unicode_escapes_in_hashes_are_accepted() {
    let escaped: String = HASH_A.bytes().map(|b| format!("\\u00{b:02x}")).collect();
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{escaped}"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let file = write_index_json(&json);
    let index = load(file.path()).unwrap();
    assert_eq!(index.hashes.as_slice()[0].to_hex(), HASH_A);
}

#[test]
fn bom_is_rejected() {
    let json = format!(
        "\u{feff}{{\"format_version\":1,\"build_id\":{{\"game_id\":\"synthetic\",\"version\":\"1\",\"exe_hash\":\"{ZERO_EXE}\"}},\"hashes\":[],\"upload_count\":1,\"consensus_count\":1,\"generated_at\":\"2026-10-25T00:00:00Z\"}}"
    );
    let file = write_index_json(&json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::ByteOrderMark), "got {err:?}");
}

#[test]
fn trailing_whitespace_is_allowed() {
    let json = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}
"#
    );
    let file = write_index_json(&json);
    load(file.path()).unwrap();
}

#[test]
fn read_capped_stops_after_max_plus_one() {
    let data = vec![b'x'; 100];
    let mut cursor = std::io::Cursor::new(data);
    let err = read_capped(&mut cursor, 10).unwrap_err();
    assert!(
        matches!(err, Error::FileTooLarge { max: 10, got: 11 }),
        "got {err:?}"
    );
}

#[test]
fn metadata_size_check_rejects_sparse_file_before_read() {
    reset_read_reached();
    let path = temp_path(".sparse");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(10 * 1024 * 1024).unwrap();
    drop(file);
    let limits = Limits {
        max_file_bytes: 64,
        ..Limits::DEFAULT
    };
    let err = load_with_limits(&path, limits).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(
        matches!(
            err,
            Error::FileTooLarge {
                max: 64,
                got: 10_485_760
            }
        ),
        "got {err:?}"
    );
    assert!(
        !take_read_reached(),
        "metadata size check missed; read_capped ran"
    );
}

#[test]
fn library_rejects_65_character_hash() {
    let mut long = HASH_A.to_string();
    long.push('a');
    let err = DescriptionHash::parse_hex(&long).unwrap_err();
    assert!(
        matches!(err, Error::WrongHexLength { got: 65 }),
        "got {err:?}"
    );
}

#[test]
fn local_duplicate_hashes_are_deduped() {
    let same = DescriptionHash::from_bytes([0x11; 32]);
    let deduped = crate::compare::unique_sorted(vec![same, same]);
    assert_eq!(deduped.len(), 1, "hashes.dedup() missing");
}

#[cfg(unix)]
#[test]
fn opened_handle_and_nonblock_reject_fifo_without_hanging() {
    let path = temp_path(".fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let path_for_thread = path.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let result = open_regular_nonblocking(&path_for_thread);
        let _ = tx.send(result.map(|_| ()));
    });
    let received = rx.recv_timeout(Duration::from_secs(2));
    let _ = std::fs::remove_file(&path);
    let result = received.expect("FIFO open hung; O_NONBLOCK or is_file check regressed");
    assert!(
        matches!(result, Err(Error::NotARegularFile) | Err(Error::Io)),
        "got {result:?}"
    );
    handle.join().unwrap();
}

#[cfg(unix)]
#[test]
fn growing_file_read_is_bounded() {
    use std::io::Write;
    let path = temp_path(".grow");
    std::fs::write(&path, b"start").unwrap();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&writer_path) {
            let buf = [0_u8; 65_536];
            for _ in 0..2_000 {
                if file.write_all(&buf).is_err() {
                    break;
                }
            }
        }
    });
    std::thread::sleep(Duration::from_millis(20));
    let (tx, rx) = std::sync::mpsc::channel();
    let reader_path = path.clone();
    let handle = std::thread::spawn(move || {
        let mut file = std::fs::File::open(&reader_path).unwrap();
        let result = read_capped(&mut file, 4_096);
        let _ = tx.send(result);
    });
    let received = rx.recv_timeout(Duration::from_secs(2));
    let result = received.expect("growing-file read hung");
    match result {
        Err(Error::FileTooLarge { max: 4_096, got }) => assert_eq!(got, 4_097),
        Ok(bytes) => assert!(bytes.len() <= 4_096),
        other => panic!("unexpected {other:?}"),
    }
    let _ = writer.join();
    handle.join().unwrap();
    let _ = std::fs::remove_file(&path);
}

fn pipeline_index(hashes: &[&str]) -> CoverageIndex {
    let parsed = hashes
        .iter()
        .map(|hash| DescriptionHash::parse_hex(hash).unwrap())
        .collect();
    CoverageIndex::pipeline(
        sample_build(),
        IndexHashes::from_sorted_unique(parsed, Limits::DEFAULT.max_hashes).unwrap(),
        3,
        u64::try_from(hashes.len()).unwrap_or(u64::MAX),
        1,
        GeneratedAt::parse("2026-10-26T00:00:00Z").unwrap(),
        IndexThreshold::new(3, 500).unwrap(),
    )
    .unwrap()
}

fn v2_json(hashes: &str) -> String {
    format!(
        r#"{{"upload_count":3,"threshold":{{"min_uploads":3,"min_ratio_per_mille":500}},"rejected_count":1,"hashes":[{hashes}],"generated_at":"2026-10-26T00:00:00Z","format_version":2,"consensus_count":1,"build_id":{{"version":"1","game_id":"synthetic","exe_hash":"{ZERO_EXE}"}}}}"#
    )
}

#[test]
fn format_v2_round_trips_canonical_bytes() {
    let index = pipeline_index(&[HASH_A, HASH_B]);
    let bytes = index.to_canonical_bytes().unwrap();
    assert!(!bytes.ends_with(b"\n"));
    assert!(!bytes.ends_with(b" "));
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.find("\"build_id\"").unwrap() < text.find("\"consensus_count\"").unwrap());
    assert!(text.find("\"min_ratio_per_mille\"").unwrap() < text.find("\"min_uploads\"").unwrap());
    assert!(text.contains(&format!("\"{HASH_A}\",\"{HASH_B}\"")));
    let file = write_index_json(text);
    let loaded = load(file.path()).unwrap();
    assert_eq!(loaded, index);
    assert_eq!(loaded.format_version, FORMAT_VERSION_V2);
    assert_eq!(loaded.to_canonical_bytes().unwrap(), bytes);
    let again = loaded.to_canonical_bytes().unwrap();
    assert_eq!(again, bytes);
}

#[test]
fn format_v1_canonical_bytes_omit_v2_fields_and_still_load() {
    let index = index_from_hex(&[HASH_B, HASH_A]);
    assert_eq!(index.format_version, FORMAT_VERSION);
    let bytes = index.to_canonical_bytes().unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("rejected_count"));
    assert!(!text.contains("threshold"));
    assert!(text.contains(&format!("\"{HASH_A}\",\"{HASH_B}\"")));
    let file = write_temp(".json", &bytes);
    let loaded = load(file.path()).unwrap();
    assert_eq!(loaded.hashes.as_slice(), index.hashes.as_slice());
    assert_eq!(loaded.to_canonical_bytes().unwrap(), bytes);
}

#[test]
fn format_v2_rejects_unknown_unsorted_duplicate_and_bad_hex() {
    let unknown = format!(
        r#"{{"format_version":2,"note":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{HASH_A}"],"upload_count":1,"consensus_count":1,"rejected_count":0,"generated_at":"2026-10-26T00:00:00Z","threshold":{{"min_uploads":3,"min_ratio_per_mille":500}}}}"#
    );
    assert!(matches!(
        load(write_index_json(&unknown).path()).unwrap_err(),
        Error::UnknownField
    ));
    let unsorted = v2_json(&format!("\"{HASH_B}\",\"{HASH_A}\""));
    assert!(matches!(
        load(write_index_json(&unsorted).path()).unwrap_err(),
        Error::UnsortedHashes
    ));
    let duplicate = v2_json(&format!("\"{HASH_A}\",\"{HASH_A}\""));
    assert!(matches!(
        load(write_index_json(&duplicate).path()).unwrap_err(),
        Error::DuplicateHash
    ));
    let upper = v2_json(&format!("\"{}\"", "A".repeat(64)));
    let err = load(write_index_json(&upper).path()).unwrap_err();
    assert!(matches!(err, Error::MalformedHex), "{err:?}");
    assert!(!err.to_string().contains('A'));
    let short = v2_json("\"abcd\"");
    assert!(matches!(
        load(write_index_json(&short).path()).unwrap_err(),
        Error::WrongHexLength { got: 4 }
    ));
}

#[test]
fn format_v1_still_sorts_and_rejects_v2_fields() {
    let unsorted = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":["{HASH_B}","{HASH_A}"],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    let loaded = load(write_index_json(&unsorted).path()).unwrap();
    assert_eq!(loaded.hashes.as_slice()[0].to_hex(), HASH_A);
    let with_rejected = format!(
        r#"{{"format_version":1,"rejected_count":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    assert!(matches!(
        load(write_index_json(&with_rejected).path()).unwrap_err(),
        Error::UnknownField
    ));
}

#[test]
fn format_v2_requires_threshold_and_rejects_a_zero_minimum() {
    let missing = format!(
        r#"{{"format_version":2,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":0,"rejected_count":0,"generated_at":"2026-10-26T00:00:00Z"}}"#
    );
    assert!(matches!(
        load(write_index_json(&missing).path()).unwrap_err(),
        Error::InvalidJson
    ));
    let zero = format!(
        r#"{{"format_version":2,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"{ZERO_EXE}"}},"hashes":[],"upload_count":1,"consensus_count":0,"rejected_count":0,"generated_at":"2026-10-26T00:00:00Z","threshold":{{"min_uploads":0,"min_ratio_per_mille":500}}}}"#
    );
    assert!(matches!(
        load(write_index_json(&zero).path()).unwrap_err(),
        Error::InvalidThreshold
    ));
    assert!(IndexThreshold::new(0, 500).is_err());
    assert!(IndexThreshold::new(1, 1001).is_err());
}

#[test]
fn compare_refuses_pipeline_index() {
    let (_sodb, summary) = local_with_psos("synthetic", 1, &[0]);
    let index = pipeline_index(&[HASH_A]);
    let err = compare(&summary, &index, 10).unwrap_err();
    assert!(matches!(err, Error::NotADescriptionIndex), "{err:?}");
    assert!(!err.to_string().contains(HASH_A));
}

#[cfg(target_os = "linux")]
fn rss_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn run_rss_isolated(test_name: &str) -> bool {
    if std::env::var_os("OSDB_RSS_CHILD").is_some() {
        return false;
    }
    let exe = std::env::current_exe().expect("test exe");
    let qualified = format!("tests::{test_name}");
    let output = std::process::Command::new(exe)
        .args([&qualified, "--exact", "--nocapture"])
        .env("OSDB_RSS_CHILD", "1")
        .env("RUST_TEST_THREADS", "1")
        .output()
        .expect("spawn rss child");
    assert!(
        output.status.success(),
        "isolated RSS child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "RSS child did not run exactly one test:\n{stdout}"
    );
    true
}

#[test]
fn hostile_unknown_key_is_rejected_without_copying_it() {
    #[cfg(target_os = "linux")]
    if run_rss_isolated("hostile_unknown_key_is_rejected_without_copying_it") {
        return;
    }
    let mut body = Vec::with_capacity(60 * 1024 * 1024 + 16);
    body.extend_from_slice(b"{\"");
    body.extend(std::iter::repeat_n(b'a', 60 * 1024 * 1024));
    body.extend_from_slice(br#"":1}"#);
    assert!(body.len() < 64 * 1024 * 1024);
    let started = std::time::Instant::now();
    #[cfg(target_os = "linux")]
    let before = rss_bytes().expect("rss");
    let err = crate::index::parse_index_bytes(body, Limits::DEFAULT).unwrap_err();
    #[cfg(target_os = "linux")]
    let after = rss_bytes().expect("rss");
    assert!(matches!(err, Error::StringTooLong), "{err:?}");
    assert!(!err.to_string().contains("aaa"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "hostile parse took {:?}",
        started.elapsed()
    );
    #[cfg(target_os = "linux")]
    {
        let growth = after.saturating_sub(before);
        assert!(
            growth < 16 * 1024 * 1024,
            "RSS grew by {growth} bytes (before {before}, after {after})"
        );
    }
}

fn coverage_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/coverage_index")
}

#[test]
fn schema_goldens_round_trip_v2_and_keep_v1() {
    let root = coverage_fixture_root();
    let v1 = std::fs::read(root.join("valid/v1-canonical.json")).unwrap();
    let loaded_v1 = crate::index::parse_index_bytes(v1.clone(), Limits::DEFAULT).unwrap();
    assert_eq!(loaded_v1.format_version, FORMAT_VERSION);
    assert_eq!(loaded_v1.to_canonical_bytes().unwrap(), v1);
    assert!(loaded_v1.rejected_count.is_none());
    assert!(loaded_v1.threshold.is_none());

    let v2 = std::fs::read(root.join("valid/v2-canonical.json")).unwrap();
    let loaded_v2 = crate::index::parse_index_bytes(v2.clone(), Limits::DEFAULT).unwrap();
    assert_eq!(loaded_v2.format_version, FORMAT_VERSION_V2);
    assert_eq!(loaded_v2.to_canonical_bytes().unwrap(), v2);
    assert_eq!(loaded_v2.rejected_count, Some(1));
    assert_eq!(loaded_v2.threshold.unwrap().min_uploads(), 3);
    assert_eq!(loaded_v2.threshold.unwrap().min_ratio_per_mille(), 500);
    assert_eq!(loaded_v2.hashes.len(), 2);

    let unsorted = std::fs::read(root.join("loadable/v1-unsorted.json")).unwrap();
    let sorted = crate::index::parse_index_bytes(unsorted.clone(), Limits::DEFAULT).unwrap();
    assert_eq!(sorted.hashes.as_slice()[0].to_hex(), HASH_A);
    assert_ne!(sorted.to_canonical_bytes().unwrap(), unsorted);

    let cases = [
        ("invalid/v2-unknown-key.json", "unknown_field"),
        ("invalid/v2-unsorted.json", "unsorted"),
        ("invalid/v2-duplicate.json", "duplicate"),
        ("invalid/v2-short-hash.json", "short"),
        ("invalid/v2-uppercase.json", "upper"),
        ("invalid/v2-overlong-string.json", "overlong"),
        ("invalid/v1-rejected-count.json", "unknown_field"),
    ];
    for (rel, kind) in cases {
        let bytes = std::fs::read(root.join(rel)).unwrap();
        let err = crate::index::parse_index_bytes(bytes, Limits::DEFAULT).unwrap_err();
        let matched = match kind {
            "unknown_field" => matches!(err, Error::UnknownField),
            "unsorted" => matches!(err, Error::UnsortedHashes),
            "duplicate" => matches!(err, Error::DuplicateHash),
            "short" => matches!(err, Error::WrongHexLength { got: 4 }),
            "upper" => matches!(err, Error::MalformedHex),
            "overlong" => matches!(err, Error::StringTooLong),
            _ => false,
        };
        assert!(matched, "{rel}: {err:?}");
        assert!(!err.to_string().contains(HASH_A));
        assert!(!err.to_string().contains('A'));
    }
}
