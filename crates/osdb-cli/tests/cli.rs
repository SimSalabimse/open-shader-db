// SPDX-License-Identifier: MIT

//! CLI smoke tests. Fixtures are synthetic (CC0 / fake SODB blobs).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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
    assert!(stdout.contains("unmatched hashes omitted"));
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
