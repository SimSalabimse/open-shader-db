// SPDX-License-Identifier: MIT

//! Scanner tests. Fixtures are synthetic (CC0 / fake DXBC bytes).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::sync::Mutex;

use osdb_records::{extract, match_local_hashes, BuildIdentity, Sha256};
use rusqlite::Connection;
use sodb::{hash_shader_container, SODB_APPLICATION_ID, SODB_USER_VERSION};

use crate::{match_records, scan, scan_dir, scan_sodb, Error, Limits, ScanCap, ScanInput};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static RSS_MEASURE_LOCK: Mutex<()> = Mutex::new(());

struct TempDir {
    path: PathBuf,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn temp_dir() -> TempDir {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("osdb-shaders-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TempDir { path }
}

fn temp_file(suffix: &str) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("osdb-shaders-{}-{n}{suffix}", std::process::id()))
}

/// Minimal DXBC container of `size` bytes (`FileSize` matches the buffer).
fn dxbc_of_len(size: usize, fill: u8) -> Vec<u8> {
    assert!(size >= 32);
    let mut out = vec![fill; size];
    out[..4].copy_from_slice(b"DXBC");
    out[4..20].fill(0);
    out[20..24].copy_from_slice(&1_u32.to_le_bytes());
    out[24..28].copy_from_slice(&(size as u32).to_le_bytes());
    out[28..32].copy_from_slice(&0_u32.to_le_bytes());
    out
}

fn write_dxbc(path: &Path, size: usize, fill: u8) {
    fs::write(path, dxbc_of_len(size, fill)).unwrap();
}

fn create_sodb(setup: impl FnOnce(&mut Connection) -> rusqlite::Result<()>) -> PathBuf {
    let path = temp_file(".sodb");
    let mut conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)
        .unwrap();
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)
        .unwrap();
    setup(&mut conn).unwrap();
    drop(conn);
    path
}

fn default_limits() -> Limits {
    Limits::DEFAULT
}

#[test]
fn hash_matches_records_extract_and_loose_scan() {
    let blob = dxbc_of_len(64, 0x5a);
    let expected = Sha256::from_sodb(hash_shader_container(&blob));
    let sodb_path = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-1", &blob],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS) VALUES (?1, ?2)",
            rusqlite::params![b"pso-1", b"bc-1"],
        )?;
        Ok(())
    });
    let records = extract(&sodb_path, BuildIdentity::empty()).unwrap();
    let from_records = records.records[0].bytecode_hashes.vs.unwrap();
    assert_eq!(from_records, expected);

    let (index, report) = scan_sodb(&sodb_path, default_limits()).unwrap();
    assert_eq!(report.shaders_indexed, 1);
    assert!(index.contains(&expected));

    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 64, 0x5a);
    let (loose, loose_report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(loose_report.shaders_indexed, 1);
    assert!(loose.contains(&expected));
    let via_reader =
        Sha256::from_sodb(sodb::hash_shader_reader(&mut std::io::Cursor::new(&blob)).unwrap());
    assert_eq!(via_reader, expected);
    let _ = fs::remove_file(&sodb_path);
}

#[test]
fn scan_report_has_counts_only_no_paths() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 32, 1);
    let (_index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    let json = serde_json::to_string(&report).unwrap();
    let human = report.format_human();
    assert!(!json.contains(dir.path.to_string_lossy().as_ref()));
    assert!(!human.contains(dir.path.to_string_lossy().as_ref()));
    assert!(json.contains("shaders_indexed"));
}

#[test]
fn missing_root_is_typed_error() {
    let missing = temp_file("-no-such-dir");
    let err = scan_dir(&missing, default_limits()).unwrap_err();
    assert!(matches!(err, Error::MissingRoot));
    assert!(!format!("{err}").contains(missing.to_string_lossy().as_ref()));
}

#[test]
fn unreadable_root_is_typed_error() {
    let dir = temp_dir();
    let nested = dir.path.join("locked");
    fs::create_dir(&nested).unwrap();
    write_dxbc(&nested.join("a.dxbc"), 32, 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o000)).unwrap();
        let err = scan_dir(&nested, default_limits()).unwrap_err();
        let _ = fs::set_permissions(&nested, fs::Permissions::from_mode(0o755));
        assert!(matches!(err, Error::UnreadableRoot));
    }
}

#[test]
fn empty_and_truncated_are_corrupt() {
    let dir = temp_dir();
    fs::write(dir.path.join("empty.bin"), []).unwrap();
    fs::write(dir.path.join("trunc.dxbc"), b"DXBC").unwrap();
    let (_index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(report.skipped_corrupt, 2);
    assert_eq!(report.corrupt_empty, 1);
    assert_eq!(report.corrupt_truncated, 1);
    assert_eq!(report.shaders_indexed, 0);
}

#[test]
fn size_field_larger_than_file_is_corrupt() {
    let dir = temp_dir();
    let mut bytes = dxbc_of_len(32, 0);
    bytes[24..28].copy_from_slice(&1000_u32.to_le_bytes());
    fs::write(dir.path.join("short.dxbc"), &bytes).unwrap();
    let (_index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(report.corrupt_size_exceeds_file, 1);
    assert_eq!(report.shaders_indexed, 0);
}

#[test]
fn trailing_bytes_are_not_hashed() {
    let dir = temp_dir();
    let mut bytes = dxbc_of_len(32, 7);
    bytes.extend_from_slice(&[0xAA; 1000]);
    fs::write(dir.path.join("trail.dxbc"), &bytes).unwrap();
    let (index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(report.shaders_indexed, 1);
    let expected = Sha256::from_sodb(hash_shader_container(&bytes[..32]));
    let whole = Sha256::from_sodb(hash_shader_container(&bytes));
    assert_ne!(expected, whole);
    assert!(index.contains(&expected));
    assert!(!index.contains(&whole));
}

#[test]
fn duplicates_are_locations() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 32, 3);
    write_dxbc(&dir.path.join("b.dxbc"), 32, 3);
    let (index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(report.shaders_indexed, 2);
    assert_eq!(report.unique_hashes, 1);
    assert_eq!(report.duplicates, 1);
    let hash = index.hashes().next().unwrap();
    assert_eq!(index.locations(&hash).len(), 2);
}

#[test]
fn deterministic_directory_order() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("z.dxbc"), 32, 1);
    write_dxbc(&dir.path.join("a.dxbc"), 32, 2);
    fs::create_dir(dir.path.join("mid")).unwrap();
    write_dxbc(&dir.path.join("mid").join("m.dxbc"), 32, 3);
    let (first, r1) = scan_dir(&dir.path, default_limits()).unwrap();
    let (second, r2) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(r1, r2);
    let a: Vec<_> = first.hashes().collect();
    let b: Vec<_> = second.hashes().collect();
    assert_eq!(a, b);
}

#[test]
fn per_file_cap_near_miss() {
    let dir = temp_dir();
    let limits = Limits {
        max_file_bytes: 64,
        ..Limits::DEFAULT
    };
    write_dxbc(&dir.path.join("ok.dxbc"), 64, 1);
    write_dxbc(&dir.path.join("big.dxbc"), 65, 2);
    let (index, report) = scan_dir(&dir.path, limits).unwrap();
    assert_eq!(report.shaders_indexed, 1);
    assert_eq!(report.skipped_too_large, 1);
    assert_eq!(index.unique_len(), 1);
}

#[test]
fn total_files_cap_near_miss() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 40, 1);
    write_dxbc(&dir.path.join("b.dxbc"), 41, 2);
    write_dxbc(&dir.path.join("c.dxbc"), 42, 3);
    let ok = Limits {
        max_files: 3,
        ..Limits::DEFAULT
    };
    let (index, report) = scan_dir(&dir.path, ok).unwrap();
    assert_eq!(report.files_seen, 3);
    assert_eq!(index.unique_len(), 3);
    let fail = Limits {
        max_files: 2,
        ..Limits::DEFAULT
    };
    let err = scan_dir(&dir.path, fail).unwrap_err();
    assert!(matches!(
        err,
        Error::CapExceeded {
            cap: ScanCap::TotalFiles,
            max: 2,
            ..
        }
    ));
}

#[test]
fn depth_cap_near_miss() {
    let dir = temp_dir();
    fs::create_dir(dir.path.join("d1")).unwrap();
    write_dxbc(&dir.path.join("d1").join("a.dxbc"), 32, 1);
    let ok = Limits {
        max_depth: 1,
        ..Limits::DEFAULT
    };
    let (_index, report) = scan_dir(&dir.path, ok).unwrap();
    assert_eq!(report.shaders_indexed, 1);
    let fail = Limits {
        max_depth: 0,
        ..Limits::DEFAULT
    };
    let err = scan_dir(&dir.path, fail).unwrap_err();
    assert!(matches!(
        err,
        Error::CapExceeded {
            cap: ScanCap::Depth,
            max: 0,
            ..
        }
    ));
}

#[test]
fn total_bytes_cap_near_miss() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 32, 1);
    write_dxbc(&dir.path.join("b.dxbc"), 32, 2);
    let ok = Limits {
        max_total_bytes: 64,
        ..Limits::DEFAULT
    };
    let (_index, report) = scan_dir(&dir.path, ok).unwrap();
    assert_eq!(report.shaders_indexed, 2);
    let fail = Limits {
        max_total_bytes: 32,
        ..Limits::DEFAULT
    };
    let err = scan_dir(&dir.path, fail).unwrap_err();
    assert!(matches!(
        err,
        Error::CapExceeded {
            cap: ScanCap::TotalBytes,
            max: 32,
            ..
        }
    ));
}

#[test]
fn index_entry_cap_near_miss() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 32, 1);
    write_dxbc(&dir.path.join("b.dxbc"), 32, 2);
    let ok = Limits {
        max_index_entries: 2,
        ..Limits::DEFAULT
    };
    let (_index, report) = scan_dir(&dir.path, ok).unwrap();
    assert_eq!(report.shaders_indexed, 2);
    let fail = Limits {
        max_index_entries: 1,
        ..Limits::DEFAULT
    };
    let err = scan_dir(&dir.path, fail).unwrap_err();
    assert!(matches!(
        err,
        Error::CapExceeded {
            cap: ScanCap::IndexEntries,
            max: 1,
            ..
        }
    ));
}

#[test]
fn non_dxbc_regular_file_is_seen_not_indexed() {
    let dir = temp_dir();
    fs::write(dir.path.join("notes.txt"), b"hello").unwrap();
    write_dxbc(&dir.path.join("a.dxbc"), 32, 1);
    let (_index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(report.files_seen, 2);
    assert_eq!(report.shaders_indexed, 1);
    assert_eq!(report.skipped_corrupt, 0);
}

#[test]
fn match_records_reuses_hash_rules() {
    let blob = dxbc_of_len(48, 9);
    let sodb_path = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB, ByteCode_PS BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-1", &blob],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS) VALUES (?1, ?2)",
            rusqlite::params![b"full", b"bc-1"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, ByteCode_PS) VALUES (?1, ?2, ?3)",
            rusqlite::params![b"part", b"bc-1", b"missing"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key) VALUES (?1)",
            rusqlite::params![b"empty"],
        )?;
        Ok(())
    });
    let records = extract(&sodb_path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&sodb_path, default_limits()).unwrap();
    let report = match_records(&records, &index);
    let hashes: std::collections::HashSet<_> = index.hashes().collect();
    let expected = match_local_hashes(&records, &hashes);
    assert_eq!(report, expected);
    assert_eq!(report.fully_covered.len(), 1);
    assert_eq!(report.partially_covered.len(), 1);
    assert_eq!(report.missing_shaders.len(), 1);
    let _ = fs::remove_file(&sodb_path);
}

#[test]
fn reopen_detects_changed_loose_file() {
    let dir = temp_dir();
    let path = dir.path.join("a.dxbc");
    write_dxbc(&path, 40, 1);
    let (index, _) = scan_dir(&dir.path, default_limits()).unwrap();
    let hash = index.hashes().next().unwrap();
    let before = index.read_verified(&hash).unwrap();
    assert_eq!(before.len(), 40);
    write_dxbc(&path, 40, 2);
    let err = index.read_verified(&hash).unwrap_err();
    assert!(matches!(
        err,
        Error::ShaderHashMismatch | Error::ShaderChanged
    ));
}

#[test]
fn debug_and_error_omit_paths() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 32, 1);
    let (index, _) = scan_dir(&dir.path, default_limits()).unwrap();
    let text = format!("{index:?}");
    assert!(!text.contains(dir.path.to_string_lossy().as_ref()));
    let err = Error::MissingRoot;
    assert!(!format!("{err:?}").contains("/"));
}

#[cfg(unix)]
#[test]
fn symlink_outside_root_is_skipped() {
    let outside = temp_dir();
    write_dxbc(&outside.path.join("secret.dxbc"), 32, 9);
    let dir = temp_dir();
    let link = dir.path.join("escape.dxbc");
    std::os::unix::fs::symlink(outside.path.join("secret.dxbc"), &link).unwrap();
    let (index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    assert_eq!(report.skipped_symlink, 1);
    assert_eq!(report.shaders_indexed, 0);
    assert_eq!(index.unique_len(), 0);
}

#[cfg(unix)]
#[test]
fn fifo_is_skipped_without_blocking() {
    let dir = temp_dir();
    let fifo = dir.path.join("pipe.fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo failed");
    write_dxbc(&dir.path.join("ok.dxbc"), 32, 1);
    let (tx, rx) = std::sync::mpsc::channel();
    let root = dir.path.clone();
    let handle = std::thread::spawn(move || {
        let result = scan_dir(&root, default_limits());
        let _ = tx.send(result);
    });
    let received = rx.recv_timeout(std::time::Duration::from_secs(2));
    let result = received.expect("FIFO open hung; O_NONBLOCK or is_file check regressed");
    let (_index, report) = result.unwrap();
    assert_eq!(report.skipped_special_file, 1);
    assert_eq!(report.shaders_indexed, 1);
    handle.join().unwrap();
}

#[cfg(target_os = "linux")]
fn rss_bytes() -> Option<u64> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
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
fn scan_rss_stays_flat_on_large_files() {
    if run_rss_isolated("scan_rss_stays_flat_on_large_files") {
        return;
    }
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const FILES: usize = 4;
    const SIZE: usize = 8 * 1024 * 1024;
    let dir = temp_dir();
    for i in 0..FILES {
        write_dxbc(&dir.path.join(format!("{i}.dxbc")), SIZE, i as u8 + 1);
    }
    let before = rss_bytes();
    let (index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    let after = rss_bytes();
    assert_eq!(report.shaders_indexed, FILES as u64);
    assert_eq!(index.unique_len(), FILES);
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "4x8MiB loose DXBC scan RSS delta: {} bytes ({:.1} MiB)",
            delta,
            delta as f64 / (1024.0 * 1024.0)
        );
        assert!(
            delta < 48 * 1024 * 1024,
            "RSS grew by {delta} bytes; streaming hash missing"
        );
    }
}

#[test]
fn scan_combines_sodb_and_dir() {
    let blob = dxbc_of_len(32, 4);
    let sodb_path = create_sodb(|conn| {
        conn.execute_batch(
            "CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"k", &blob],
        )?;
        Ok(())
    });
    let dir = temp_dir();
    write_dxbc(&dir.path.join("extra.dxbc"), 40, 5);
    let (index, report) = scan(
        &[
            ScanInput::Sodb(sodb_path.clone()),
            ScanInput::Directory(dir.path.clone()),
        ],
        default_limits(),
    )
    .unwrap();
    assert_eq!(report.sources, 2);
    assert_eq!(report.shaders_indexed, 2);
    assert_eq!(index.unique_len(), 2);
    let _ = fs::remove_file(&sodb_path);
}

#[test]
fn insert_counter_matches_location_sum() {
    use crate::types::{ByteLen, ShaderLocation, ShaderSourceKind};
    use crate::ScanReport;

    let mut index = crate::LocalShaderIndex::new();
    let mut report = ScanReport::default();
    let source = index.add_source(ShaderSourceKind::LooseFile, PathBuf::from("synthetic"));
    let loc = ShaderLocation::loose(source, ByteLen(32));
    for i in 0_u32..4_000 {
        let mut bytes = [0_u8; 32];
        bytes[..4].copy_from_slice(&i.to_le_bytes());
        let hash = Sha256::from_sodb(hash_shader_container(&bytes));
        index
            .insert(hash, loc.clone(), &mut report, 1_000_000)
            .unwrap();
    }
    let dup = Sha256::from_sodb(hash_shader_container(&[7_u8; 32]));
    for _ in 0..200 {
        index
            .insert(dup, loc.clone(), &mut report, 1_000_000)
            .unwrap();
    }
    let sum: u64 = index
        .hashes()
        .map(|h| index.locations(&h).len() as u64)
        .sum();
    assert_eq!(index.entry_count(), sum);
    assert_eq!(index.entry_count(), 4_200);
    assert_eq!(index.unique_len(), 4_001);
    assert_eq!(report.duplicates, 199);
}

#[test]
fn read_verified_falls_back_to_later_duplicate() {
    let dir = temp_dir();
    write_dxbc(&dir.path.join("a.dxbc"), 48, 3);
    write_dxbc(&dir.path.join("b.dxbc"), 48, 3);
    assert_ne!(dxbc_of_len(48, 3), dxbc_of_len(48, 9));
    let (index, _) = scan_dir(&dir.path, default_limits()).unwrap();
    let hash = index.hashes().next().unwrap();
    assert_eq!(index.locations(&hash).len(), 2);
    write_dxbc(&dir.path.join("a.dxbc"), 48, 9);
    let bytes = index.read_verified(&hash).unwrap();
    assert_eq!(bytes, dxbc_of_len(48, 3));
}

#[test]
fn duplicate_locations_follow_sorted_directory_order() {
    let dir = temp_dir();
    // Create in non-sorted order so a missing directory sort is visible.
    let created = ["m2.dxbc", "z1.dxbc", "a9.dxbc", "q0.dxbc", "b3.dxbc"];
    for name in created {
        write_dxbc(&dir.path.join(name), 48, 3);
    }
    let mut expected: Vec<String> = created.iter().map(|n| (*n).to_owned()).collect();
    expected.sort();
    let (index, _) = scan_dir(&dir.path, default_limits()).unwrap();
    let hash = index.hashes().next().unwrap();
    let locs = index.locations(&hash);
    assert_eq!(locs.len(), created.len());
    let names: Vec<_> = locs
        .iter()
        .map(|loc| {
            index
                .source_path(loc.source)
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap()
        })
        .collect();
    assert_eq!(names, expected);
    let first = index.read_location(&locs[0], &hash).unwrap();
    assert_eq!(first, dxbc_of_len(48, 3));
}

#[test]
fn reopen_detects_changed_sodb_blob() {
    let blob = dxbc_of_len(40, 4);
    let sodb_path = create_sodb(|conn| {
        conn.execute_batch(
            "CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"k", &blob],
        )?;
        Ok(())
    });
    let (index, _) = scan_sodb(&sodb_path, default_limits()).unwrap();
    let hash = index.hashes().next().unwrap();
    assert_eq!(index.read_verified(&hash).unwrap(), blob);
    let conn = Connection::open(&sodb_path).unwrap();
    conn.execute(
        "UPDATE shader_bytecode SET Bytecode = ?1",
        rusqlite::params![dxbc_of_len(40, 99)],
    )
    .unwrap();
    drop(conn);
    let err = index.read_verified(&hash).unwrap_err();
    assert!(matches!(
        err,
        Error::ShaderHashMismatch | Error::ShaderChanged
    ));
    let _ = fs::remove_file(&sodb_path);
}

#[test]
fn without_rowid_sodb_stores_key_locator() {
    use crate::ShaderLocator;
    let blob = dxbc_of_len(40, 4);
    let sodb_path = create_sodb(|conn| {
        conn.execute_batch(
            "CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB) WITHOUT ROWID;",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"k", &blob],
        )?;
        Ok(())
    });
    let (index, _) = scan_sodb(&sodb_path, default_limits()).unwrap();
    let hash = index.hashes().next().unwrap();
    match &index.locations(&hash)[0].locator {
        ShaderLocator::SodbKey { key, len } => {
            assert_eq!(key.as_slice(), b"k");
            assert_eq!(len.get(), 40);
        }
        other => panic!("expected SodbKey, got {other:?}"),
    }
    assert_eq!(index.read_verified(&hash).unwrap(), blob);
    let conn = Connection::open(&sodb_path).unwrap();
    conn.execute(
        "UPDATE shader_bytecode SET Bytecode = ?1",
        rusqlite::params![dxbc_of_len(40, 99)],
    )
    .unwrap();
    drop(conn);
    let err = index.read_verified(&hash).unwrap_err();
    assert!(matches!(
        err,
        Error::ShaderHashMismatch | Error::ShaderChanged
    ));
    let _ = fs::remove_file(&sodb_path);
}

#[cfg(unix)]
#[test]
fn growing_loose_file_is_bounded_by_filesize() {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::Arc;
    let dir = temp_dir();
    let path = dir.path.join("grow.dxbc");
    let original = dxbc_of_len(256 * 1024, 7);
    fs::write(&path, &original).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_path = path.clone();
    let stop_w = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        if let Ok(mut file) = fs::OpenOptions::new().append(true).open(&writer_path) {
            let buf = [0xAA_u8; 65_536];
            while !stop_w.load(AtomicOrdering::Relaxed) {
                if file.write_all(&buf).is_err() {
                    break;
                }
            }
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(10));
    let (index, report) = scan_dir(&dir.path, default_limits()).unwrap();
    stop.store(true, AtomicOrdering::Relaxed);
    let _ = handle.join();
    assert_eq!(report.shaders_indexed, 1);
    let hash = index.hashes().next().unwrap();
    assert_eq!(hash, Sha256::from_sodb(hash_shader_container(&original)));
    assert_eq!(index.read_verified(&hash).unwrap(), original);
}
