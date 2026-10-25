// SPDX-License-Identifier: MIT

//! CLI smoke tests. Fixtures are synthetic (CC0 / fake SODB blobs).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

use rusqlite::Connection;
use sodb::{SODB_APPLICATION_ID, SODB_USER_VERSION};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempFile {
    path: PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn temp_path(suffix: &str) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("osdb-cli-{}-{n}{suffix}", std::process::id()))
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_osdb-cli"))
}

#[test]
fn help_lists_inspect_discover_coverage_and_records() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("discover"));
    assert!(stdout.contains("coverage"));
    assert!(stdout.contains("records"));
}

#[test]
fn discover_json_runs_against_missing_root() {
    let missing = temp_path("-no-steam");
    let out = bin()
        .arg("discover")
        .arg("--steam-root")
        .arg(&missing)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(value["games"].as_array().is_some());
    assert!(!stdout.contains(missing.to_string_lossy().as_ref()));
}

fn create_sodb() -> TempFile {
    let path = temp_path(".sodb");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)
        .unwrap();
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)
        .unwrap();
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
        CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Type TEXT, Bytecode BLOB);
        CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB, Flags INTEGER);
        CREATE TABLE state_objects (
            Key BLOB NOT NULL PRIMARY KEY, Type INTEGER, NodeMask INTEGER, Flags INTEGER,
            AddToStateObjectParent BLOB
        );
        CREATE TABLE groups (Key BLOB NOT NULL PRIMARY KEY, Version INTEGER NOT NULL, PSOKey BLOB, SOKey BLOB);
        INSERT INTO app_id (id, exe, app_name, app_version, engine_version)
            VALUES (1, 'synthetic.exe', 'synthetic', 1, 0);
        INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (x'01', 'vs', x'46414b45');
        INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (x'01', x'01', 0);
        INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (x'02', x'01', 1);
        INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (x'03', x'01', 2);
        ",
    )
    .unwrap();
    drop(conn);
    TempFile { path }
}

fn write_index(path: &Path, hashes: &[&str]) {
    let list = hashes
        .iter()
        .map(|h| format!("\"{h}\""))
        .collect::<Vec<_>>()
        .join(",");
    let body = format!(
        r#"{{"format_version":1,"build_id":{{"game_id":"synthetic","version":"1","exe_hash":"0000000000000000000000000000000000000000000000000000000000000000"}},"hashes":[{list}],"upload_count":1,"consensus_count":1,"generated_at":"2026-10-25T00:00:00Z"}}"#
    );
    std::fs::write(path, body).unwrap();
}

#[test]
fn inspect_still_prints_json() {
    let sodb = create_sodb();
    let out = bin()
        .arg("inspect")
        .arg(sodb.path.as_os_str())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("pipeline_states"));
    assert!(stdout.contains("desc_hash"));
}

#[test]
fn coverage_human_and_json() {
    let sodb = create_sodb();
    let summary = sodb::open(&sodb.path).unwrap();
    let hex = summary.pipeline_states[0].desc_hash.to_hex();
    let index = TempFile {
        path: temp_path(".json"),
    };
    write_index(&index.path, &[&hex]);

    let out = bin()
        .arg("coverage")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--index")
        .arg(&index.path)
        .arg("--max-unmatched")
        .arg("1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Match rate:"));
    assert!(
        stdout.contains("unmatched hash omitted") || stdout.contains("unmatched hashes omitted")
    );
    assert!(stdout.contains(osdb_coverage::UNSIGNED_INDEX_LINE));
    assert!(!stdout.contains(sodb.path.to_string_lossy().as_ref()));

    let out = bin()
        .arg("coverage")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--index")
        .arg(&index.path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["index_verified"], false);
    assert_eq!(value["index_unsigned"], true);
}

#[test]
fn records_extract_writes_json_without_bytecode_or_paths() {
    let sodb = create_sodb();
    let out_path = TempFile {
        path: temp_path(".json"),
    };
    let result = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&out_path.path)
        .arg("--game-id")
        .arg("synthetic")
        .arg("--version")
        .arg("1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = std::fs::read_to_string(&out_path.path).unwrap();
    assert!(text.contains("\"format_version\": 1"));
    assert!(text.contains("bytecode_hashes"));
    assert!(!text.contains("FAKE"));
    assert!(!text.contains(sodb.path.to_string_lossy().as_ref()));
    assert!(!String::from_utf8_lossy(&result.stderr).contains(sodb.path.to_string_lossy().as_ref()));
}

#[test]
fn coverage_load_error_is_nonzero() {
    let sodb = create_sodb();
    let missing = temp_path("-missing.json");
    let out = bin()
        .arg("coverage")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--index")
        .arg(&missing)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1);
    assert!(!stderr.contains(missing.to_string_lossy().as_ref()));
}

#[test]
fn records_extract_refuses_out_equal_to_sodb() {
    let sodb = create_sodb();
    let original = std::fs::read(&sodb.path).unwrap();
    let out = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&sodb.path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("same file"));
    assert_eq!(std::fs::read(&sodb.path).unwrap(), original);
}

#[test]
fn records_extract_refuses_existing_out_without_force() {
    let sodb = create_sodb();
    let dest = TempFile {
        path: temp_path(".json"),
    };
    std::fs::write(&dest.path, b"keep-me").unwrap();
    let out = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&dest.path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read(&dest.path).unwrap(), b"keep-me");
}

#[test]
fn records_extract_force_overwrites_regular_file() {
    let sodb = create_sodb();
    let dest = TempFile {
        path: temp_path(".json"),
    };
    std::fs::write(&dest.path, b"old").unwrap();
    let out = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&dest.path)
        .arg("--force")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&dest.path).unwrap();
    assert!(text.contains("format_version"));
    assert_ne!(text.as_bytes(), b"old");
}

#[test]
fn records_extract_hash_app_keys() {
    let sodb = create_sodb();
    let dest = TempFile {
        path: temp_path(".json"),
    };
    let out = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&dest.path)
        .arg("--hash-app-keys")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&dest.path).unwrap();
    assert!(!text.contains("\"app_key\": \"01\""));
}

#[test]
fn help_lists_force_and_hash_app_keys() {
    let out = bin()
        .args(["records", "extract", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("--hash-app-keys"));
}

#[cfg(unix)]
#[test]
fn records_extract_refuses_hardlink_to_sodb() {
    let sodb = create_sodb();
    let original = std::fs::read(&sodb.path).unwrap();
    let dest = temp_path(".sodb");
    std::fs::hard_link(&sodb.path, &dest).unwrap();
    let out = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&dest)
        .arg("--force")
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&dest);
    assert!(!out.status.success());
    assert_eq!(std::fs::read(&sodb.path).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn records_extract_refuses_symlink_out() {
    let sodb = create_sodb();
    let target = TempFile {
        path: temp_path(".json"),
    };
    std::fs::write(&target.path, b"keep").unwrap();
    let link = temp_path(".json");
    std::os::unix::fs::symlink(&target.path, &link).unwrap();
    let out = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&link)
        .arg("--force")
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&link);
    assert!(!out.status.success());
    assert_eq!(std::fs::read(&target.path).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn records_extract_refuses_fifo_out_without_hanging() {
    struct KillOnDrop(Option<std::process::Child>);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    let sodb = create_sodb();
    let fifo = temp_path(".json");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    let child = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&fifo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut guard = KillOnDrop(Some(child));
    let started = Instant::now();
    let status = loop {
        match guard.0.as_mut().and_then(|c| c.try_wait().ok()).flatten() {
            Some(status) => {
                guard.0 = None;
                break status;
            }
            None if started.elapsed() < Duration::from_secs(3) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            None => panic!("FIFO --out hung"),
        }
    };
    let _ = std::fs::remove_file(&fifo);
    assert!(!status.success());
}
