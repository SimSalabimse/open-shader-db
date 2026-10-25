// SPDX-License-Identifier: MIT

//! Synthetic SODB and record-file fixtures. Blobs are fake bytes, never real DXIL.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::sync::Mutex;
#[cfg(unix)]
use std::time::Duration;

use rusqlite::Connection;
use sha2::{Digest, Sha256 as Sha256Digest};

use crate::build::{BuildIdentity, ExeHash, GameId, GameVersion};
#[cfg(target_os = "linux")]
use crate::extract::extract_with_sodb_limits;
use crate::extract::{
    encode_record_file, encode_record_file_to_writer, extract, extract_with_options,
};
#[cfg(unix)]
use crate::file::open_regular_nonblocking;
use crate::file::{
    load, load_with_limits, read_capped, reset_read_reached, take_read_reached, RecordFile,
    RedactedPath, FORMAT_VERSION,
};
use crate::hash::{sha256_json, Sha256};
use crate::matching::{match_local_hashes, CoverageKind};
use crate::types::{HexBytes, InputElementDesc, ShaderStage, SoDeclaration, StreamOutDesc};
use crate::{Error, Limits};

use sodb::{RootSignatureSkipReason, SODB_APPLICATION_ID, SODB_USER_VERSION};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static RSS_MEASURE_LOCK: Mutex<()> = Mutex::new(());

const ZERO_EXE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FAKE_VS: &[u8] = b"FAKE_VS_BLOB_BYTES";
const FAKE_PS: &[u8] = b"FAKE_PS_BLOB_BYTES";
/// Canonical empty D3D12 root signature 1.0 (DXC unused-offset emit).
const FAKE_ROOT: &[u8] = &[
    1, 0, 0, 0, 0, 0, 0, 0, 24, 0, 0, 0, 0, 0, 0, 0, 24, 0, 0, 0, 0, 0, 0, 0,
];

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
    std::env::temp_dir().join(format!("osdb-records-{}-{n}{suffix}", std::process::id()))
}

fn write_temp(suffix: &str, bytes: impl AsRef<[u8]>) -> TempFile {
    let path = temp_path(suffix);
    std::fs::write(&path, bytes).unwrap();
    TempFile { path }
}

fn sha256_hex(bytes: &[u8]) -> String {
    bytes_to_hex(Sha256Digest::digest(bytes))
}

fn bytes_to_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
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
        CREATE TABLE shader_bytecode (
            Key BLOB NOT NULL PRIMARY KEY,
            Type TEXT,
            Bytecode BLOB
        );
        CREATE TABLE root_signatures (
            Key BLOB NOT NULL PRIMARY KEY,
            value BLOB NOT NULL
        );
        CREATE TABLE rasterizer_descs (
            Key BLOB NOT NULL PRIMARY KEY,
            FillMode INTEGER NOT NULL,
            CullMode INTEGER NOT NULL,
            FrontCounterClockwise INTEGER NOT NULL,
            DepthBias REAL NOT NULL,
            DepthBiasClamp REAL NOT NULL,
            SlopeScaledDepthBias REAL NOT NULL,
            DepthClipEnable INTEGER NOT NULL,
            LineRasterizationMode INTEGER NOT NULL,
            ForcedSampleCount INTEGER NOT NULL,
            ConservativeRaster INTEGER NOT NULL
        );
        CREATE TABLE pipeline_states (
            Key BLOB NOT NULL PRIMARY KEY,
            RootSignature BLOB,
            ByteCode_VS BLOB,
            ByteCode_PS BLOB,
            RasterizerDesc BLOB,
            SampleDesc_Count INTEGER,
            SampleMask INTEGER,
            Flags INTEGER
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

fn insert_pso(
    conn: &Connection,
    key: &[u8],
    vs_key: &[u8],
    vs: &[u8],
    flags: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, 'vs', ?2)",
        rusqlite::params![vs_key, vs],
    )?;
    conn.execute(
        "INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (?1, ?2, ?3)",
        rusqlite::params![key, vs_key, flags],
    )?;
    Ok(())
}

fn sample_build() -> BuildIdentity {
    BuildIdentity::new(
        Some(GameId::parse("synthetic").unwrap()),
        Some(GameVersion::parse("1.0.0").unwrap()),
        Some(ExeHash::parse_hex(ZERO_EXE).unwrap()),
    )
}

fn empty_record_json() -> String {
    format!(
        r#"{{"format_version":1,"build":{{"game_id":"synthetic","version":"1.0.0","exe_hash":"{ZERO_EXE}"}},"records":[]}}"#
    )
}

fn record_shell(inner: &str) -> String {
    format!(
        r#"{{"format_version":1,"build":{{"game_id":"synthetic","version":"1.0.0","exe_hash":"{ZERO_EXE}"}},"records":[{inner}]}}"#
    )
}

#[test]
fn extract_round_trip_and_no_bytecode() {
    let sodb = create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, 'vs', ?2)",
            rusqlite::params![b"bc-vs", FAKE_VS],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, 'ps', ?2)",
            rusqlite::params![b"bc-ps", FAKE_PS],
        )?;
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![b"rs-1", FAKE_ROOT],
        )?;
        conn.execute(
            "INSERT INTO rasterizer_descs VALUES (?1, 3, 1, 0, 0.0, 0.0, 0.0, 1, 0, 0, 0)",
            rusqlite::params![b"rast-1"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, RootSignature, ByteCode_VS, ByteCode_PS, RasterizerDesc, SampleDesc_Count, SampleMask, Flags)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, 4294967295, 7)",
            rusqlite::params![b"pso-1", b"rs-1", b"bc-vs", b"bc-ps", b"rast-1"],
        )?;
        conn.execute(
            "INSERT INTO groups (Key, Version, PSOKey) VALUES (?1, 4, ?2)",
            rusqlite::params![b"g1", b"pso-1"],
        )?;
        Ok(())
    });

    let file = extract(sodb.path(), sample_build()).unwrap();
    assert_eq!(file.format_version, FORMAT_VERSION);
    assert_eq!(file.records.len(), 1);
    let rec = &file.records[0];
    assert_eq!(rec.app_key.as_str(), "70736f2d31");
    assert_eq!(rec.version, 4);
    assert_eq!(
        rec.bytecode_hashes.vs.unwrap().to_hex(),
        sha256_hex(FAKE_VS)
    );
    assert_eq!(
        rec.bytecode_hashes.ps.unwrap().to_hex(),
        sha256_hex(FAKE_PS)
    );
    let rs = rec.root_signature.expect("root signature hash");
    let stored = file
        .root_signatures
        .get(&rs)
        .expect("interned root signature");
    assert_eq!(stored.as_str(), bytes_to_hex(FAKE_ROOT));
    assert_eq!(rs.to_hex(), sha256_hex(FAKE_ROOT));
    assert_eq!(rec.rasterizer.as_ref().unwrap().fill_mode, 3);
    assert_eq!(rec.flags, Some(7));

    let json = encode_record_file(&file).unwrap();
    let text = String::from_utf8(json.clone()).unwrap();
    assert!(!text.contains("FAKE_VS_BLOB_BYTES"));
    assert!(!text.contains("FAKE_PS_BLOB_BYTES"));
    assert!(!text.contains(&bytes_to_hex(FAKE_VS)));
    assert!(!text.contains(&bytes_to_hex(FAKE_PS)));
    assert!(!text.contains(sodb.path().to_string_lossy().as_ref()));

    let loaded_path = write_temp(".json", &json);
    let loaded = load(loaded_path.path()).unwrap();
    assert_eq!(loaded, file);
}

#[test]
fn extract_is_deterministic_across_insert_order() {
    let make = |keys: &[&[u8]]| {
        create_sodb(|conn| {
            create_core_tables(conn)?;
            for key in keys {
                let flags = i64::from(key[0]);
                let vs = format!("blob-{}", bytes_to_hex(key)).into_bytes();
                insert_pso(conn, key, key, &vs, flags)?;
            }
            Ok(())
        })
    };
    let a = make(&[b"\x03", b"\x01", b"\x02"]);
    let b = make(&[b"\x01", b"\x02", b"\x03"]);
    let left = encode_record_file(&extract(a.path(), BuildIdentity::empty()).unwrap()).unwrap();
    let right = encode_record_file(&extract(b.path(), BuildIdentity::empty()).unwrap()).unwrap();
    assert_eq!(left, right);
    let parsed = extract(a.path(), BuildIdentity::empty()).unwrap();
    let keys: Vec<&str> = parsed.records.iter().map(|r| r.app_key.as_str()).collect();
    assert_eq!(keys, ["01", "02", "03"]);
}

#[test]
fn matching_covered_partial_and_missing() {
    let vs = Sha256::parse_hex(&sha256_hex(FAKE_VS)).unwrap();
    let ps = Sha256::parse_hex(&sha256_hex(FAKE_PS)).unwrap();
    let extra = Sha256::parse_hex(HASH_C).unwrap();

    let sodb = create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"vs", FAKE_VS],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"ps", FAKE_PS],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, ByteCode_PS) VALUES (?1, ?2, ?3)",
            rusqlite::params![b"full", b"vs", b"ps"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, ByteCode_PS) VALUES (?1, ?2, ?3)",
            rusqlite::params![b"part", b"vs", b"ps"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS) VALUES (?1, ?2)",
            rusqlite::params![b"miss", b"ps"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 1)",
            rusqlite::params![b"none"],
        )?;
        Ok(())
    });
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    let mut local = HashSet::new();
    local.insert(vs);
    local.insert(extra);
    let report = match_local_hashes(&file, &local);
    assert_eq!(report.fully_covered.len(), 0);
    assert_eq!(report.partially_covered.len(), 2);
    assert_eq!(report.missing_shaders.len(), 2);
    assert!(report
        .missing_shaders
        .iter()
        .any(|m| m.app_key.as_str() == "6e6f6e65"));
    assert_eq!(report.missing_shaders[0].kind, CoverageKind::MissingShaders);

    let mut both = HashSet::new();
    both.insert(vs);
    both.insert(ps);
    let all = match_local_hashes(&file, &both);
    assert_eq!(all.missing_shaders.len(), 1);
    assert_eq!(all.missing_shaders[0].app_key.as_str(), "6e6f6e65");
    assert_eq!(all.fully_covered.len(), 3);
}

#[test]
fn sha256_rejects_uppercase_and_wrong_length() {
    assert!(matches!(
        Sha256::parse_hex(&HASH_A.to_ascii_uppercase()),
        Err(Error::MalformedHex)
    ));
    assert!(matches!(
        Sha256::parse_hex("aa"),
        Err(Error::WrongHexLength { got: 2 })
    ));
}

#[test]
fn load_rejects_hostile_files() {
    let oversize = write_temp(".json", vec![b'x'; 128]);
    let err = load_with_limits(
        oversize.path(),
        Limits {
            max_file_bytes: 16,
            ..Limits::DEFAULT
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::FileTooLarge { max: 16, .. }),
        "{err:?}"
    );

    let nested = write_temp(
        ".json",
        r#"{"format_version":1,"build":{},"records":[[{"a":1}]]}"#,
    );
    let err = load_with_limits(
        nested.path(),
        Limits {
            max_json_nesting: 2,
            ..Limits::DEFAULT
        },
    )
    .unwrap_err();
    assert!(matches!(err, Error::NestingTooDeep { max: 2 }), "{err:?}");

    let dup = write_temp(
        ".json",
        r#"{"format_version":1,"format_version":1,"build":{},"records":[]}"#,
    );
    let err = load(dup.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "{err:?}");

    let unknown =
        empty_record_json().replacen(r#""records":[]"#, r#""records":[],"extra":true"#, 1);
    let unknown = write_temp(".json", unknown);
    let err = load(unknown.path()).unwrap_err();
    assert!(matches!(err, Error::UnknownField), "{err:?}");

    let ver = empty_record_json().replacen("\"format_version\":1", "\"format_version\":2", 1);
    let ver = write_temp(".json", ver);
    let err = load(ver.path()).unwrap_err();
    assert!(
        matches!(
            err,
            Error::UnsupportedFormatVersion {
                found: 2,
                expected: 1
            }
        ),
        "{err:?}"
    );

    let bad_hex = record_shell(&format!(
        r#"{{"app_key":"01","version":0,"bytecode_hashes":{{"vs":"{HASH_A}zz"}}}}"#
    ));
    let bad_hex = write_temp(".json", bad_hex);
    let err = load(bad_hex.path()).unwrap_err();
    assert!(
        matches!(err, Error::WrongHexLength { got: 66 } | Error::MalformedHex),
        "{err:?}"
    );

    let upper = record_shell(&format!(
        r#"{{"app_key":"01","version":0,"bytecode_hashes":{{"vs":"{}"}}}}"#,
        HASH_A.to_ascii_uppercase()
    ));
    let upper = write_temp(".json", upper);
    let err = load(upper.path()).unwrap_err();
    assert!(matches!(err, Error::MalformedHex), "{err:?}");

    let bom = format!("\u{feff}{}", empty_record_json());
    let bom = write_temp(".json", bom);
    let err = load(bom.path()).unwrap_err();
    assert!(matches!(err, Error::ByteOrderMark), "{err:?}");

    let trail = format!("{}{{}}", empty_record_json());
    let trail = write_temp(".json", trail);
    let err = load(trail.path()).unwrap_err();
    assert!(matches!(err, Error::TrailingGarbage), "{err:?}");

    let dir = temp_path("-dir");
    std::fs::create_dir(&dir).unwrap();
    let err = load(&dir).unwrap_err();
    let _ = std::fs::remove_dir(&dir);
    assert!(matches!(err, Error::NotARegularFile), "{err:?}");
}

#[test]
fn record_cap_is_enforced_in_visitor() {
    let rec = format!(
        r#"{{"app_key":"0{0}","version":0,"bytecode_hashes":{{}}}}"#,
        1
    );
    let rec2 = format!(
        r#"{{"app_key":"0{0}","version":0,"bytecode_hashes":{{}}}}"#,
        2
    );
    let rec3 = format!(
        r#"{{"app_key":"0{0}","version":0,"bytecode_hashes":{{}}}}"#,
        3
    );
    let json = format!(r#"{{"format_version":1,"build":{{}},"records":[{rec},{rec2},{rec3}]}}"#);
    let file = write_temp(".json", json);
    let err = load_with_limits(
        file.path(),
        Limits {
            max_records: 2,
            ..Limits::DEFAULT
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::TooManyRecords { max: 2, got: 3 }),
        "{err:?}"
    );
}

#[test]
fn metadata_size_check_rejects_before_read() {
    reset_read_reached();
    let path = temp_path(".sparse");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(10 * 1024 * 1024).unwrap();
    drop(file);
    let err = load_with_limits(
        &path,
        Limits {
            max_file_bytes: 64,
            ..Limits::DEFAULT
        },
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(matches!(
        err,
        Error::FileTooLarge {
            max: 64,
            got: 10_485_760
        }
    ));
    assert!(!take_read_reached(), "metadata size check missed; read ran");
}

#[test]
fn trailing_whitespace_is_allowed() {
    let json = format!("{}\n", empty_record_json());
    let file = write_temp(".json", json);
    load(file.path()).unwrap();
}

#[test]
fn error_display_never_includes_path() {
    let path = temp_path("-missing.json");
    let err = load(&path).unwrap_err();
    let text = err.to_string();
    assert!(!text.contains(path.to_string_lossy().as_ref()));
    assert!(!text.contains("osdb-records"));
}

#[test]
fn redacted_path_debug_hides_location() {
    let path = RedactedPath::new("/home/os-user/secret/records.json");
    let debug = format!("{path:?}");
    assert_eq!(debug, "<redacted>");
    assert!(!debug.contains("os-user"));
}

#[test]
fn build_labels_are_validated() {
    for game_id in ["has space", "C:\\Games", "ä", &"x".repeat(129)] {
        assert!(matches!(GameId::parse(game_id), Err(Error::InvalidBuildId)));
    }
}

#[test]
fn read_capped_stops_after_max_plus_one() {
    let data = vec![b'x'; 100];
    let mut cursor = std::io::Cursor::new(data);
    let err = read_capped(&mut cursor, 10).unwrap_err();
    assert!(matches!(err, Error::FileTooLarge { max: 10, got: 11 }));
}

#[cfg(unix)]
#[test]
fn symlink_is_not_a_regular_file() {
    let target = write_temp(".json", empty_record_json());
    let link = temp_path(".json");
    std::os::unix::fs::symlink(target.path(), &link).unwrap();
    let err = load(&link).unwrap_err();
    let _ = std::fs::remove_file(&link);
    assert!(matches!(err, Error::NotARegularFile), "{err:?}");
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
    let status = std::process::Command::new(exe)
        .args([test_name, "--exact", "--nocapture"])
        .env("OSDB_RSS_CHILD", "1")
        .env("RUST_TEST_THREADS", "1")
        .status()
        .expect("spawn rss child");
    assert!(status.success(), "isolated RSS child {test_name} failed");
    true
}

#[cfg(target_os = "linux")]
#[test]
fn extract_rss_does_not_scale_with_blob_size() {
    if run_rss_isolated("extract_rss_does_not_scale_with_blob_size") {
        return;
    }
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const ROWS: usize = 4;
    const BLOB_LEN: usize = 16 * 1024 * 1024;
    const TOTAL: u64 = (ROWS * BLOB_LEN) as u64;
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_bc =
                tx.prepare("INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)")?;
            let mut ins_pso =
                tx.prepare("INSERT INTO pipeline_states (Key, ByteCode_VS) VALUES (?1, ?2)")?;
            let mut blob = vec![0_u8; BLOB_LEN];
            for i in 0..ROWS {
                let key = (i as u32).to_le_bytes();
                blob[0] = i as u8;
                ins_bc.execute(rusqlite::params![&key, &blob])?;
                ins_pso.execute(rusqlite::params![&key, &key])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let limits = sodb::Limits {
        max_blob_bytes: BLOB_LEN,
        max_file_bytes: 256 * 1024 * 1024,
        ..sodb::Limits::DEFAULT
    };
    let before = rss_bytes();
    let file = extract_with_sodb_limits(fixture.path(), BuildIdentity::empty(), limits).unwrap();
    let after = rss_bytes();
    assert_eq!(file.records.len(), ROWS);
    let json = encode_record_file(&file).unwrap();
    let text = String::from_utf8(json).unwrap();
    assert!(!text.contains(&"\0".repeat(32)));
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "4x16MiB bytecode extract RSS delta: {} bytes ({:.1} MiB); total blobs {} MiB",
            delta,
            delta as f64 / (1024.0 * 1024.0),
            TOTAL / (1024 * 1024)
        );
        assert!(
            delta < 32 * 1024 * 1024,
            "RSS grew by {delta} bytes; streaming hash missing (total blobs {TOTAL})"
        );
    }
}

fn dxbc_with_parts(parts: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let header_len = 32;
    let offsets_len = parts.len() * 4;
    let mut body = Vec::new();
    let mut offsets = Vec::new();
    let mut cursor = header_len + offsets_len;
    for (fcc, data) in parts {
        offsets.push(cursor as u32);
        body.extend_from_slice(*fcc);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        cursor += 8 + data.len();
    }
    let file_size = (header_len + offsets_len + body.len()) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"DXBC");
    out.extend_from_slice(&[0_u8; 16]);
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&(parts.len() as u32).to_le_bytes());
    for off in offsets {
        out.extend_from_slice(&off.to_le_bytes());
    }
    out.extend(body);
    out
}

fn sodb_with_root_sig(value: &[u8]) -> TempFile {
    create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![b"rs-1", value],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-1", b"rs-1"],
        )?;
        Ok(())
    })
}

#[test]
fn dxil_carrying_root_signature_extracts_rts0_and_marker_is_absent() {
    const MARKER: &[u8] = b"DXIL_MARKER_SHOULD_NOT_LEAK_9f3a";
    let blob = dxbc_with_parts(&[(b"RTS0", FAKE_ROOT), (b"DXIL", MARKER)]);
    let sodb = sodb_with_root_sig(&blob);
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(file.root_signatures.len(), 1);
    let stored = file.root_signatures.values().next().unwrap();
    assert_eq!(stored.as_str(), bytes_to_hex(FAKE_ROOT));
    let json = String::from_utf8(encode_record_file(&file).unwrap()).unwrap();
    assert!(!json.contains("DXIL_MARKER"));
    assert!(!json.contains("9f3a"));
    assert!(!json.contains(&bytes_to_hex(b"DXIL")));
    assert!(!json.contains(&bytes_to_hex(MARKER)));
    assert!(file.records[0].root_signature_skip.is_none());
}

#[test]
fn rts0_only_container_emits_payload_not_container() {
    let blob = dxbc_with_parts(&[(b"RTS0", FAKE_ROOT)]);
    let sodb = sodb_with_root_sig(&blob);
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(file.root_signatures.len(), 1);
    let stored = file.root_signatures.values().next().unwrap();
    assert_eq!(stored.as_str(), bytes_to_hex(FAKE_ROOT));
    let json = String::from_utf8(encode_record_file(&file).unwrap()).unwrap();
    assert!(!json.contains(&bytes_to_hex(b"DXBC")));
    assert!(!json.to_ascii_lowercase().contains("44584243"));
}

#[test]
fn malformed_dxbc_root_signature_is_skipped() {
    let mut blob = dxbc_with_parts(&[(b"RTS0", FAKE_ROOT)]);
    blob.truncate(20);
    extract_skips_root_sig(&blob, RootSignatureSkipReason::UnsupportedContainer);
}

#[test]
fn two_rts0_parts_are_skipped_as_invalid() {
    let blob = dxbc_with_parts(&[(b"RTS0", FAKE_ROOT), (b"RTS0", FAKE_ROOT)]);
    extract_skips_root_sig(&blob, RootSignatureSkipReason::Invalid);
}

#[test]
fn version_1_2_root_signature_is_skipped() {
    let mut bytes = FAKE_ROOT.to_vec();
    bytes[0..4].copy_from_slice(&3_u32.to_le_bytes());
    extract_skips_root_sig(&bytes, RootSignatureSkipReason::UnsupportedVersion);
}

#[test]
fn raw_root_signature_is_accepted() {
    let sodb = sodb_with_root_sig(FAKE_ROOT);
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(
        file.root_signatures.values().next().unwrap().as_str(),
        bytes_to_hex(FAKE_ROOT)
    );
}

#[test]
fn record_file_debug_redacts_descriptor_bytes() {
    let sodb = sodb_with_root_sig(FAKE_ROOT);
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    let debug = format!("{file:?}");
    assert!(!debug.contains(&bytes_to_hex(FAKE_ROOT)));
    assert!(!debug.contains("FAKE"));
}

#[test]
fn unresolved_stage_is_not_fully_covered() {
    let sodb = create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"vs", FAKE_VS],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, ByteCode_PS) VALUES (?1, ?2, ?3)",
            rusqlite::params![b"hang", b"vs", b"missing"],
        )?;
        Ok(())
    });
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(file.records[0].unresolved_stages, vec![ShaderStage::Ps]);
    assert!(file.records[0].bytecode_hashes.ps.is_none());
    let mut local = HashSet::new();
    local.insert(Sha256::parse_hex(&sha256_hex(FAKE_VS)).unwrap());
    let report = match_local_hashes(&file, &local);
    assert_eq!(report.fully_covered.len(), 0);
    assert_eq!(report.partially_covered.len(), 1);
}

#[test]
fn empty_record_is_not_fully_covered() {
    let rec = record_shell(r#"{"app_key":"01","version":0,"bytecode_hashes":{}}"#);
    let file = write_temp(".json", rec);
    let loaded = load(file.path()).unwrap();
    let report = match_local_hashes(&loaded, &HashSet::new());
    assert_eq!(report.fully_covered.len(), 0);
    assert_eq!(report.missing_shaders.len(), 1);
}

#[test]
fn non_finite_float_is_rejected_at_extract() {
    let sodb = create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO rasterizer_descs VALUES (?1, 3, 1, 0, ?2, 0.0, 0.0, 1, 0, 0, 0)",
            rusqlite::params![b"rast-1", f64::INFINITY],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, RasterizerDesc, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-1", b"rast-1"],
        )?;
        Ok(())
    });
    let err = extract(sodb.path(), BuildIdentity::empty()).unwrap_err();
    assert!(
        matches!(err, Error::Sodb(sodb::Error::NonFiniteFloat { .. })),
        "{err:?}"
    );
}

#[test]
fn extract_output_always_loads() {
    let sodb = sodb_with_root_sig(FAKE_ROOT);
    let file = extract(sodb.path(), sample_build()).unwrap();
    let json = encode_record_file(&file).unwrap();
    let loaded_path = write_temp(".json", &json);
    let loaded = load(loaded_path.path()).unwrap();
    assert_eq!(loaded, file);
}

#[test]
fn semantic_name_rejects_path_shaped_value() {
    let sodb = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE input_element_descs (
                Key BLOB PRIMARY KEY,
                SemanticName TEXT,
                SemanticIndex INTEGER,
                Format INTEGER,
                InputSlot INTEGER,
                AlignedByteOffset INTEGER,
                InputSlotClass INTEGER,
                InstanceDataStepRate INTEGER
            );
            CREATE TABLE input_layout_to_input_element_associations (
                InputLayoutKey BLOB,
                InputElementKey BLOB
            );
            CREATE TABLE pipeline_states (
                Key BLOB NOT NULL PRIMARY KEY,
                InputLayout BLOB,
                Flags INTEGER
            );
            ",
        )?;
        conn.execute(
            "INSERT INTO input_element_descs VALUES (?1, ?2, 0, 0, 0, 0, 0, 0)",
            rusqlite::params![b"el-1", "/home/os-user/secret"],
        )?;
        conn.execute(
            "INSERT INTO input_layout_to_input_element_associations VALUES (?1, ?2)",
            rusqlite::params![b"lay-1", b"el-1"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, InputLayout, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-1", b"lay-1"],
        )?;
        Ok(())
    });
    let err = extract(sodb.path(), BuildIdentity::empty()).unwrap_err();
    let text = err.to_string();
    assert!(
        matches!(err, Error::Sodb(sodb::Error::InvalidSemanticName)),
        "{err:?}"
    );
    assert!(!text.contains("os-user"));
    assert!(!text.contains("/home"));
}

#[test]
fn hash_app_keys_replaces_key_with_sha256() {
    let sodb = sodb_with_root_sig(FAKE_ROOT);
    let file = extract_with_options(
        sodb.path(),
        BuildIdentity::empty(),
        sodb::Limits::DEFAULT,
        true,
    )
    .unwrap();
    let expected = sha256_hex(b"pso-1");
    assert_eq!(file.records[0].app_key.as_str(), expected);
    assert_ne!(file.records[0].app_key.as_str(), "70736f2d31");
}

#[test]
fn loader_rejects_duplicate_build_key() {
    let json = empty_record_json().replacen(
        r#""game_id":"synthetic""#,
        r#""game_id":"synthetic","game_id":"other""#,
        1,
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "{err:?}");
}

#[test]
fn loader_rejects_duplicate_record_key() {
    let inner = r#"{"app_key":"01","app_key":"02","version":0,"bytecode_hashes":{}}"#;
    let file = write_temp(".json", record_shell(inner));
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "{err:?}");
}

#[test]
fn loader_rejects_duplicate_bytecode_hash_key() {
    let inner = format!(
        r#"{{"app_key":"01","version":0,"bytecode_hashes":{{"vs":"{HASH_A}","vs":"{HASH_C}"}}}}"#
    );
    let file = write_temp(".json", record_shell(&inner));
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "{err:?}");
}

#[test]
fn loader_rejects_odd_length_hex() {
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{HASH_A}":"000"}},"records":[]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::OddHexLength { got: 3 }), "{err:?}");
}

#[test]
fn loader_rejects_empty_and_unsorted_app_keys() {
    let empty = record_shell(r#"{"app_key":"","version":0,"bytecode_hashes":{}}"#);
    let file = write_temp(".json", empty);
    let err = load(file.path()).unwrap_err();
    assert!(
        matches!(err, Error::EmptyAppKey | Error::MalformedHex),
        "{err:?}"
    );

    let unsorted = r#"{"format_version":1,"build":{},"records":[{"app_key":"02","version":0,"bytecode_hashes":{}},{"app_key":"01","version":0,"bytecode_hashes":{}}]}"#;
    let file = write_temp(".json", unsorted);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::AppKeysNotSorted), "{err:?}");
}

#[test]
fn output_cap_fails_without_partial_vec_growth() {
    let mut file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: Vec::new(),
    };
    let huge = vec![0_u8; 33 * 1024 * 1024];
    let hash = crate::hash::sha256_bytes(&huge);
    file.root_signatures
        .insert(hash, HexBytes::from_bytes(&huge));
    let err = encode_record_file(&file).unwrap_err();
    assert!(
        matches!(err, Error::OutputTooLarge { max: 67_108_864 }),
        "{err:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn shared_root_signature_fanout_stays_under_rss_and_output_caps() {
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const STATES: usize = 200;
    const BLOB_LEN: usize = 8 * 1024 * 1024;
    let fixture = create_sodb(|conn| {
        create_core_tables(conn)?;
        let blob = FAKE_ROOT.to_vec();
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![b"rs-shared", &blob],
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (?1, ?2, 0)",
            )?;
            for i in 0..STATES {
                let key = (i as u32).to_le_bytes();
                ins.execute(rusqlite::params![&key, b"rs-shared"])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let limits = sodb::Limits {
        max_blob_bytes: BLOB_LEN,
        max_root_signature_bytes: BLOB_LEN,
        max_file_bytes: 256 * 1024 * 1024,
        ..sodb::Limits::DEFAULT
    };
    let before = rss_bytes();
    let file = extract_with_sodb_limits(fixture.path(), BuildIdentity::empty(), limits).unwrap();
    let json = encode_record_file(&file).unwrap();
    let after = rss_bytes();
    assert_eq!(file.records.len(), STATES);
    assert_eq!(file.root_signatures.len(), 1);
    assert!(
        json.len() < 20 * 1024 * 1024,
        "interned output should stay small, got {}",
        json.len()
    );
    if let (Some(before), Some(after)) = (before, after) {
        let peak = after.max(before);
        eprintln!(
            "200x8MiB shared root-signature extract peak RSS: {} bytes ({:.1} MiB); output {} bytes",
            peak,
            peak as f64 / (1024.0 * 1024.0),
            json.len()
        );
        let delta = after.saturating_sub(before);
        assert!(
            peak < 64 * 1024 * 1024 || delta < 64 * 1024 * 1024,
            "peak RSS {peak} delta {delta} exceeded 64 MiB"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn shared_input_layout_fanout_stays_small() {
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const STATES: usize = 200;
    const ELEMS: usize = 4000;
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE input_element_descs (
                Key BLOB PRIMARY KEY,
                SemanticName TEXT,
                SemanticIndex INTEGER,
                Format INTEGER,
                InputSlot INTEGER,
                AlignedByteOffset INTEGER,
                InputSlotClass INTEGER,
                InstanceDataStepRate INTEGER
            );
            CREATE TABLE input_layout_to_input_element_associations (
                InputLayoutKey BLOB,
                InputElementKey BLOB
            );
            CREATE TABLE pipeline_states (
                Key BLOB NOT NULL PRIMARY KEY,
                InputLayout BLOB,
                Flags INTEGER
            );
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO input_element_descs VALUES (?1, 'POSITION', ?2, 0, 0, 0, 0, 0)",
            )?;
            let mut assoc = tx.prepare(
                "INSERT INTO input_layout_to_input_element_associations VALUES (?1, ?2)",
            )?;
            for i in 0..ELEMS {
                let key = (i as u32).to_le_bytes();
                ins.execute(rusqlite::params![&key, i as i64])?;
                assoc.execute(rusqlite::params![b"lay-1", &key])?;
            }
            let mut pso = tx.prepare(
                "INSERT INTO pipeline_states (Key, InputLayout, Flags) VALUES (?1, ?2, 0)",
            )?;
            for i in 0..STATES {
                let key = ((ELEMS as u32) + i as u32).to_le_bytes();
                pso.execute(rusqlite::params![&key, b"lay-1"])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let before = rss_bytes();
    let file = extract(fixture.path(), BuildIdentity::empty()).unwrap();
    let json = encode_record_file(&file).unwrap();
    let after = rss_bytes();
    assert_eq!(file.records.len(), STATES);
    assert_eq!(file.input_layouts.len(), 1);
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "200x4000 shared input-layout extract RSS delta: {} bytes ({:.1} MiB); output {} bytes",
            delta,
            delta as f64 / (1024.0 * 1024.0),
            json.len()
        );
        assert!(
            delta < 64 * 1024 * 1024,
            "RSS grew by {delta} bytes for interned input layouts"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn extract_rss_200_mib_bytecode() {
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const ROWS: usize = 25;
    const BLOB_LEN: usize = 8 * 1024 * 1024;
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_bc =
                tx.prepare("INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)")?;
            let mut ins_pso =
                tx.prepare("INSERT INTO pipeline_states (Key, ByteCode_VS) VALUES (?1, ?2)")?;
            let mut blob = vec![0_u8; BLOB_LEN];
            for i in 0..ROWS {
                let key = (i as u32).to_le_bytes();
                blob[0] = i as u8;
                ins_bc.execute(rusqlite::params![&key, &blob])?;
                ins_pso.execute(rusqlite::params![&key, &key])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let limits = sodb::Limits {
        max_blob_bytes: BLOB_LEN,
        max_file_bytes: 256 * 1024 * 1024,
        ..sodb::Limits::DEFAULT
    };
    let before = rss_bytes();
    let file = extract_with_sodb_limits(fixture.path(), BuildIdentity::empty(), limits).unwrap();
    let after = rss_bytes();
    assert_eq!(file.records.len(), ROWS);
    if let (Some(before), Some(after)) = (before, after) {
        let peak = after.max(before);
        eprintln!(
            "25x8MiB bytecode extract peak RSS: {} bytes ({:.1} MiB)",
            peak,
            peak as f64 / (1024.0 * 1024.0)
        );
        assert!(
            after.saturating_sub(before) < 64 * 1024 * 1024,
            "200 MiB blob extract RSS grew too much"
        );
    }
}

const MARKER: &[u8] = b"DXIL_MARKER_SHOULD_NOT_LEAK_9f3a";

fn marker_absent(err: &Error) {
    let text = format!("{err}{err:?}");
    assert!(!text.contains("DXIL_MARKER"), "{text}");
    assert!(!text.contains("9f3a"), "{text}");
}

fn extract_skips_root_sig(value: &[u8], reason: RootSignatureSkipReason) {
    let sodb = sodb_with_root_sig(value);
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(file.records.len(), 1);
    assert!(file.records[0].root_signature.is_none());
    assert_eq!(file.records[0].root_signature_skip, Some(reason));
    assert!(file.root_signatures.is_empty());
    let json = String::from_utf8(encode_record_file(&file).unwrap()).unwrap();
    assert!(!json.contains("DXIL_MARKER"));
    assert!(!json.contains("9f3a"));
    assert!(json.contains(&format!("\"root_signature_skip\": \"{}\"", reason.as_str())));
    let loaded_path = write_temp(".json", &json);
    assert_eq!(load(loaded_path.path()).unwrap(), file);
}

#[test]
fn raw_smuggle_is_skipped_and_marker_absent() {
    let mut blob = vec![1, 0, 0, 0];
    blob.extend_from_slice(MARKER);
    extract_skips_root_sig(&blob, RootSignatureSkipReason::Invalid);
}

#[test]
fn raw_smuggle2_is_skipped_and_marker_absent() {
    let mut blob = vec![2, 0, 0, 0];
    blob.extend_from_slice(MARKER);
    extract_skips_root_sig(&blob, RootSignatureSkipReason::Invalid);
}

#[test]
fn rts0_smuggle_is_skipped_and_marker_absent() {
    let mut payload = vec![1, 0, 0, 0];
    payload.extend_from_slice(MARKER);
    let blob = dxbc_with_parts(&[(b"RTS0", &payload)]);
    extract_skips_root_sig(&blob, RootSignatureSkipReason::Invalid);
}

#[test]
fn rts0_v99_is_skipped_and_marker_absent() {
    let mut payload = vec![99, 0, 0, 0];
    payload.extend_from_slice(MARKER);
    let blob = dxbc_with_parts(&[(b"RTS0", &payload)]);
    extract_skips_root_sig(&blob, RootSignatureSkipReason::UnsupportedVersion);
}

#[test]
fn skipped_root_signature_does_not_affect_sibling_record() {
    let mut hostile = vec![1, 0, 0, 0];
    hostile.extend_from_slice(MARKER);
    let sodb = create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![b"rs-bad", &hostile],
        )?;
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![b"rs-good", FAKE_ROOT],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-bad", b"rs-bad"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-good", b"rs-good"],
        )?;
        Ok(())
    });
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(file.records.len(), 2);
    assert_eq!(file.root_signatures.len(), 1);
    assert_eq!(
        file.root_signatures.values().next().unwrap().as_str(),
        bytes_to_hex(FAKE_ROOT)
    );
    let bad = file
        .records
        .iter()
        .find(|r| r.app_key.as_str() == bytes_to_hex(b"pso-bad"))
        .unwrap();
    assert!(bad.root_signature.is_none());
    assert_eq!(
        bad.root_signature_skip,
        Some(RootSignatureSkipReason::Invalid)
    );
    let good = file
        .records
        .iter()
        .find(|r| r.app_key.as_str() == bytes_to_hex(b"pso-good"))
        .unwrap();
    assert!(good.root_signature.is_some());
    assert!(good.root_signature_skip.is_none());
    let json = String::from_utf8(encode_record_file(&file).unwrap()).unwrap();
    assert!(!json.contains("DXIL_MARKER"));
    assert!(!json.contains("9f3a"));
}

fn extract_and_load_root_sig(bytes: &[u8]) {
    let sodb = sodb_with_root_sig(bytes);
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(
        file.root_signatures.values().next().unwrap().as_str(),
        bytes_to_hex(bytes)
    );
    let json = encode_record_file(&file).unwrap();
    let loaded_path = write_temp(".json", &json);
    let loaded = load(loaded_path.path()).unwrap();
    assert_eq!(loaded, file);
}

#[test]
fn spec_faithful_root_signatures_extract_and_load() {
    let samples = [
        ("empty_v1_0", sodb::empty_root_signature_v1_0()),
        ("empty_v1_1", sodb::empty_root_signature_v1_1()),
        ("cbv_v1_0", sodb::one_cbv_root_signature_v1_0()),
        ("constants_v1_0", sodb::one_constants_root_signature_v1_0()),
        ("cbv_v1_1", sodb::one_cbv_root_signature_v1_1()),
        ("table_v1_0", sodb::table_three_ranges_v1_0()),
        ("table_v1_1", sodb::one_table_root_signature_v1_1()),
        ("table_two_v1_1", sodb::table_two_ranges_v1_1()),
        ("sampler_border4", sodb::one_sampler_border4_v1_0()),
        (
            "flag_0x80000000",
            sodb::empty_root_signature_v1_0_heap_flag(),
        ),
    ];
    for (name, bytes) in samples {
        extract_and_load_root_sig(&bytes);
        let rts0 = dxbc_with_parts(&[(b"RTS0", &bytes)]);
        let sodb = sodb_with_root_sig(&rts0);
        let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
        assert_eq!(
            file.root_signatures.values().next().unwrap().as_str(),
            bytes_to_hex(&bytes),
            "{name} RTS0"
        );
        let json = encode_record_file(&file).unwrap();
        let loaded_path = write_temp(".json", &json);
        assert_eq!(load(loaded_path.path()).unwrap(), file, "{name} RTS0 load");
    }
}

#[test]
fn extract_rewrites_non_canonical_payload_order() {
    let sodb = sodb_with_root_sig(&sodb::one_cbv_v1_0_payload_before_params());
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert_eq!(
        file.root_signatures.values().next().unwrap().as_str(),
        bytes_to_hex(sodb::one_cbv_root_signature_v1_0())
    );
}

#[test]
fn loader_rejects_non_canonical_payload_order() {
    let bytes = sodb::one_cbv_v1_0_payload_before_params();
    let hex = bytes_to_hex(&bytes);
    let hash = sha256_hex(&bytes);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":"{hash}"}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::RootSignatureMalformed), "{err:?}");
}

#[test]
fn loader_rejects_zero_offset_empty_as_non_canonical() {
    let bytes = [
        1_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let hex = bytes_to_hex(bytes);
    let hash = sha256_hex(&bytes);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":"{hash}"}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::RootSignatureMalformed), "{err:?}");
}

#[test]
fn unreferenced_hostile_root_signature_is_not_copied() {
    let mut hostile = vec![1, 0, 0, 0];
    hostile.extend_from_slice(MARKER);
    let sodb = create_sodb(|conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
            rusqlite::params![b"rs-bad", &hostile],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"pso-1"],
        )?;
        Ok(())
    });
    let file = extract(sodb.path(), BuildIdentity::empty()).unwrap();
    assert!(file.root_signatures.is_empty());
    let json = String::from_utf8(encode_record_file(&file).unwrap()).unwrap();
    assert!(!json.contains("DXIL_MARKER"));
    assert!(!json.contains(&bytes_to_hex(MARKER)));
}

#[test]
fn loader_rejects_dxil_carrying_referenced_root_signature() {
    let mut hostile = vec![1, 0, 0, 0];
    hostile.extend_from_slice(MARKER);
    let hex = bytes_to_hex(&hostile);
    let hash = sha256_hex(&hostile);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":"{hash}"}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    marker_absent(&err);
    assert!(matches!(err, Error::RootSignatureMalformed), "{err:?}");
}

#[test]
fn loader_rejects_dxil_carrying_unreferenced_root_signature() {
    let mut hostile = vec![1, 0, 0, 0];
    hostile.extend_from_slice(MARKER);
    let hex = bytes_to_hex(&hostile);
    let hash = sha256_hex(&hostile);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    marker_absent(&err);
    assert!(
        matches!(
            err,
            Error::RootSignatureMalformed | Error::UnreferencedDescriptor
        ),
        "{err:?}"
    );
}

#[test]
fn loader_rejects_unreferenced_canonical_root_signature() {
    let hex = bytes_to_hex(FAKE_ROOT);
    let hash = sha256_hex(FAKE_ROOT);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::UnreferencedDescriptor), "{err:?}");
}

fn minimal_record(
    app_key: &str,
    input_layout: Option<Sha256>,
    root_signature: Option<Sha256>,
) -> crate::types::PipelineStateRecord {
    crate::types::PipelineStateRecord {
        app_key: crate::types::AppKeyHex::parse_hex(app_key).unwrap(),
        version: 0,
        bytecode_hashes: crate::types::StageHashes {
            vs: None,
            ps: None,
            hs: None,
            ds: None,
            gs: None,
            as_stage: None,
            ms: None,
            cs: None,
        },
        unresolved_stages: Vec::new(),
        root_signature,
        root_signature_skip: None,
        input_layout,
        depth_stencil: None,
        blend: None,
        rasterizer: None,
        render_target_formats: None,
        view_instancing: None,
        stream_out: None,
        sample_desc: None,
        sample_mask: None,
        ib_strip_cut_value: None,
        primitive_topology: None,
        dsv_format: None,
        node_mask: None,
        flags: None,
    }
}

#[test]
fn loader_rejects_path_shaped_semantic_name() {
    let elems = vec![InputElementDesc {
        semantic_name: "C:\\Windows\\secret".into(),
        semantic_index: 0,
        format: 0,
        input_slot: 0,
        aligned_byte_offset: 0,
        input_slot_class: 0,
        instance_data_step_rate: 0,
    }];
    let hash = sha256_json(&elems).unwrap();
    let mut file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: vec![],
    };
    file.input_layouts.insert(hash, elems);
    file.records.push(minimal_record("01", Some(hash), None));
    let json = serde_json::to_vec(&file).unwrap();
    let path = write_temp(".json", json);
    let err = load(path.path()).unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, Error::InvalidSemanticName), "{err:?}");
    assert!(!text.contains("Windows"));
    assert!(!text.contains("secret"));
}

#[test]
fn loader_rejects_unreferenced_input_layout() {
    let elems = vec![InputElementDesc {
        semantic_name: "POSITION".into(),
        semantic_index: 0,
        format: 0,
        input_slot: 0,
        aligned_byte_offset: 0,
        input_slot_class: 0,
        instance_data_step_rate: 0,
    }];
    let hash = sha256_json(&elems).unwrap();
    let mut file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: Vec::new(),
    };
    file.input_layouts.insert(hash, elems);
    let json = serde_json::to_vec(&file).unwrap();
    let path = write_temp(".json", json);
    let err = load(path.path()).unwrap_err();
    assert!(matches!(err, Error::UnreferencedDescriptor), "{err:?}");
}

#[test]
fn loader_rejects_unreferenced_stream_output() {
    let so = StreamOutDesc {
        buffer_stride: [0, 0, 0, 0],
        num_strides: 0,
        rasterized_stream: 0,
        declarations: vec![SoDeclaration {
            stream: 0,
            semantic_name: "TEXCOORD".into(),
            semantic_index: 0,
            start_component: 0,
            component_count: 1,
            output_slot: 0,
        }],
    };
    let hash = sha256_json(&so).unwrap();
    let mut file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: Vec::new(),
    };
    file.stream_outputs.insert(hash, so);
    let json = serde_json::to_vec(&file).unwrap();
    let path = write_temp(".json", json);
    let err = load(path.path()).unwrap_err();
    assert!(matches!(err, Error::UnreferencedDescriptor), "{err:?}");
}

#[test]
fn loader_rejects_root_signature_hash_mismatch() {
    let hex = bytes_to_hex(FAKE_ROOT);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{HASH_A}":"{hex}"}},"records":[]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DescriptorHashMismatch), "{err:?}");
}

#[test]
fn loader_rejects_root_signature_over_cap() {
    let hex = bytes_to_hex(FAKE_ROOT);
    let hash = sha256_hex(FAKE_ROOT);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":"{hash}"}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load_with_limits(
        file.path(),
        Limits {
            max_root_signature_bytes: 10,
            ..Limits::DEFAULT
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::RootSignatureTooLarge { max: 10, .. }),
        "{err:?}"
    );
}

#[test]
fn loader_rejects_duplicate_root_signature_key() {
    let hex = bytes_to_hex(FAKE_ROOT);
    let hash = sha256_hex(FAKE_ROOT);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}","{hash}":"{hex}"}},"records":[]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "{err:?}");
}

#[test]
fn loader_rejects_layout_hash_mismatch() {
    let elems = vec![InputElementDesc {
        semantic_name: "POSITION".into(),
        semantic_index: 0,
        format: 0,
        input_slot: 0,
        aligned_byte_offset: 0,
        input_slot_class: 0,
        instance_data_step_rate: 0,
    }];
    let mut file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: Vec::new(),
    };
    let wrong = Sha256::parse_hex(HASH_A).unwrap();
    file.input_layouts.insert(wrong, elems);
    file.records.push(minimal_record("01", Some(wrong), None));
    let json = serde_json::to_vec(&file).unwrap();
    let path = write_temp(".json", json);
    let err = load(path.path()).unwrap_err();
    assert!(matches!(err, Error::DescriptorHashMismatch), "{err:?}");
}

#[test]
fn loader_rejects_missing_descriptor_reference() {
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":"{HASH_A}"}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::UnresolvedDescriptor), "{err:?}");
}

#[test]
fn loader_rejects_duplicate_app_key() {
    let json = r#"{"format_version":1,"build":{},"records":[{"app_key":"01","version":0,"bytecode_hashes":{}},{"app_key":"01","version":0,"bytecode_hashes":{}}]}"#;
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateKey), "{err:?}");
}

#[test]
fn loader_rejects_duplicate_unresolved_stages() {
    let json = r#"{"format_version":1,"build":{},"records":[{"app_key":"01","version":0,"bytecode_hashes":{},"unresolved_stages":["ps","ps"]}]}"#;
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::DuplicateUnresolvedStage), "{err:?}");
}

#[test]
fn loader_rejects_overlapping_unresolved_stages() {
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{"vs":"{HASH_A}"}},"unresolved_stages":["vs"]}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::OverlappingUnresolvedStage), "{err:?}");
}

#[test]
fn encode_to_writer_buffers_before_emitting() {
    let mut file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: Vec::new(),
    };
    let huge = vec![0_u8; 33 * 1024 * 1024];
    let hash = crate::hash::sha256_bytes(&huge);
    file.root_signatures
        .insert(hash, HexBytes::from_bytes(&huge));
    let mut dest = Vec::new();
    let err = encode_record_file_to_writer(&file, &mut dest).unwrap_err();
    assert!(
        matches!(err, Error::OutputTooLarge { max: 67_108_864 }),
        "{err:?}"
    );
    assert!(
        dest.is_empty(),
        "stdout/writer must not see partial JSON, got {} bytes",
        dest.len()
    );
}

#[test]
fn encode_write_error_is_io_not_invalid_json() {
    struct FailWriter;
    impl std::io::Write for FailWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("no"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let file = RecordFile {
        format_version: FORMAT_VERSION,
        build: BuildIdentity::empty(),
        root_signatures: Default::default(),
        input_layouts: Default::default(),
        stream_outputs: Default::default(),
        records: Vec::new(),
    };
    let err = encode_record_file_to_writer(&file, FailWriter).unwrap_err();
    assert!(matches!(err, Error::Io), "{err:?}");
}

#[test]
fn loader_accepts_documented_root_signature_skip_reasons() {
    for reason in [
        RootSignatureSkipReason::UnsupportedVersion,
        RootSignatureSkipReason::Invalid,
        RootSignatureSkipReason::UnsupportedContainer,
    ] {
        let json = format!(
            r#"{{"format_version":1,"build":{{}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":null,"root_signature_skip":"{}"}}]}}"#,
            reason.as_str()
        );
        let file = write_temp(".json", json);
        let loaded = load(file.path()).unwrap();
        assert_eq!(loaded.records[0].root_signature, None);
        assert_eq!(loaded.records[0].root_signature_skip, Some(reason));
    }
}

#[test]
fn loader_rejects_unknown_root_signature_skip() {
    let json = r#"{"format_version":1,"build":{},"records":[{"app_key":"01","version":0,"bytecode_hashes":{},"root_signature":null,"root_signature_skip":"not_a_reason"}]}"#;
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::UnknownRootSignatureSkip), "{err:?}");
}

#[test]
fn loader_rejects_root_signature_skip_with_hash() {
    let hex = bytes_to_hex(FAKE_ROOT);
    let hash = sha256_hex(FAKE_ROOT);
    let json = format!(
        r#"{{"format_version":1,"build":{{}},"root_signatures":{{"{hash}":"{hex}"}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}},"root_signature":"{hash}","root_signature_skip":"invalid"}}]}}"#
    );
    let file = write_temp(".json", json);
    let err = load(file.path()).unwrap_err();
    assert!(matches!(err, Error::RootSignatureSkipConflict), "{err:?}");
}
