// SPDX-License-Identifier: MIT

//! Synthetic SODB and record-file fixtures. Blobs are fake bytes, never real DXIL.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
#[cfg(unix)]
use std::time::Duration;

use rusqlite::Connection;
use sha2::{Digest, Sha256 as Sha256Digest};

use crate::build::{BuildIdentity, ExeHash, GameId, GameVersion};
use crate::extract::{encode_record_file, extract, extract_with_sodb_limits};
#[cfg(unix)]
use crate::file::open_regular_nonblocking;
use crate::file::{
    load, load_with_limits, read_capped, reset_read_reached, take_read_reached, RedactedPath,
    FORMAT_VERSION,
};
use crate::hash::Sha256;
use crate::matching::{match_local_hashes, CoverageKind};
use crate::{Error, Limits};

use sodb::{SODB_APPLICATION_ID, SODB_USER_VERSION};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
static RSS_MEASURE_LOCK: Mutex<()> = Mutex::new(());

const ZERO_EXE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FAKE_VS: &[u8] = b"FAKE_VS_BLOB_BYTES";
const FAKE_PS: &[u8] = b"FAKE_PS_BLOB_BYTES";
const FAKE_ROOT: &[u8] = b"FAKE_ROOT_SIG_BYTES";

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
    assert_eq!(
        rec.root_signature.as_ref().unwrap().as_str(),
        bytes_to_hex(FAKE_ROOT)
    );
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
    assert_eq!(report.fully_covered.len(), 1);
    assert_eq!(report.fully_covered[0].app_key.as_str(), "6e6f6e65");
    assert_eq!(report.partially_covered.len(), 2);
    assert_eq!(report.missing_shaders.len(), 1);
    assert_eq!(report.missing_shaders[0].kind, CoverageKind::MissingShaders);

    let mut both = HashSet::new();
    both.insert(vs);
    both.insert(ps);
    let all = match_local_hashes(&file, &both);
    assert_eq!(all.missing_shaders.len(), 0);
    assert_eq!(all.fully_covered.len(), 4);
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
#[test]
fn extract_rss_does_not_scale_with_blob_size() {
    let _rss = RSS_MEASURE_LOCK.lock().unwrap();
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
            delta < 8 * 1024 * 1024,
            "RSS grew by {delta} bytes; streaming hash missing (total blobs {TOTAL})"
        );
    }
}
