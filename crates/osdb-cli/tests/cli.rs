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
fn help_lists_inspect_discover_coverage_records_shaders_rebuild() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("discover"));
    assert!(stdout.contains("coverage"));
    assert!(stdout.contains("records"));
    assert!(stdout.contains("shaders"));
    assert!(stdout.contains("rebuild"));
    assert!(stdout.contains("upload"));
}

#[test]
fn rebuild_help_lists_max_output_bytes() {
    let out = bin().arg("rebuild").arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("max-output-bytes"));
    assert!(stdout.contains("8 GiB"));
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

fn write_sodb_header(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut buf = vec![0_u8; 100];
    buf[..16].copy_from_slice(b"SQLite format 3\0");
    buf[18] = 1;
    buf[19] = 1;
    buf[60..64].copy_from_slice(&SODB_USER_VERSION.to_be_bytes());
    buf[68..72].copy_from_slice(&SODB_APPLICATION_ID.to_be_bytes());
    std::fs::write(path, buf).unwrap();
}

#[test]
fn discover_help_lists_sodb_dir_and_xbox_games() {
    let out = bin().args(["discover", "--help"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("--sodb-dir"));
    assert!(stdout.contains("--xbox-games"));
    assert!(!stdout.contains("os-user"));
}

#[test]
fn discover_human_says_no_sodb_once_and_collapses_stale_libraries() {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let base =
        std::env::temp_dir().join(format!("user-alice-fixture-cli-{}-{n}", std::process::id()));
    let steam = base.join("steam");
    let gone = base.join("gone-drive");
    let steamapps = steam.join("steamapps");
    std::fs::create_dir_all(steamapps.join("common").join("GameOne")).unwrap();
    let esc = |path: &std::path::Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let vdf = format!(
        "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
        esc(&gone),
        esc(&steam)
    );
    std::fs::write(steamapps.join("libraryfolders.vdf"), vdf).unwrap();
    std::fs::write(
        steamapps.join("appmanifest_100.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"100\"\n\t\"name\"\t\t\"GameOne\"\n\t\"installdir\"\t\t\"GameOne\"\n\t\"buildid\"\t\t\"1\"\n}\n",
    )
    .unwrap();

    let out = bin()
        .arg("discover")
        .arg("--steam-root")
        .arg(&steam)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stdout.contains("sodb_files=0"));
    assert_eq!(
        stdout
            .matches("no SODB files found (expected for games from Steam today)")
            .count(),
        1
    );
    assert!(stdout.contains("info: 1 library path listed in VDF is not a directory"));
    assert!(!stdout.contains("gone-drive"));
    assert!(!stdout.contains("user-alice-fixture"));
    assert!(!stderr.contains("gone-drive"));
    assert!(!stderr.contains("user-alice-fixture"));
    assert!(!stderr.contains("warning: 1 library path"));
    assert!(!stdout.contains("warning:"));
}

#[test]
fn discover_sodb_dir_counts_extension_and_hides_the_path() {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("osdb-cli-capture-{}-{n}", std::process::id()));
    let capture = base.join("capture");
    write_sodb_header(&capture.join("cap.sodb"));
    write_sodb_header(&capture.join("not.db"));
    let out = bin()
        .arg("discover")
        .arg("--steam-root")
        .arg(base.join("missing-steam"))
        .arg("--sodb-dir")
        .arg(&capture)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stdout.contains("explicit SODB folder  sodb_files=1"));
    assert!(!stdout.contains("capture"));
    assert!(!stderr.contains("capture"));
    assert!(!stdout.contains(base.to_string_lossy().as_ref()));
}

#[test]
fn discover_xbox_games_flag_runs_without_printing_paths() {
    let missing = temp_path("-no-steam");
    let folder = temp_path("-xbox");
    std::fs::create_dir_all(&folder).unwrap();
    let out = bin()
        .arg("discover")
        .arg("--steam-root")
        .arg(&missing)
        .arg("--xbox-games")
        .arg(&folder)
        .arg("--json")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&folder);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stdout.contains(missing.to_string_lossy().as_ref()));
    assert!(!stdout.contains(folder.to_string_lossy().as_ref()));
    assert!(!stderr.contains(folder.to_string_lossy().as_ref()));
    assert!(stdout.contains("folder_scans"));
    assert!(stdout.contains("xbox_games"));
}

#[test]
fn discover_sodb_dir_missing_or_file_warns_without_a_path() {
    let missing_steam = temp_path("-no-steam");
    let missing_dir = temp_path("-missing-sodb-dir");
    let file = temp_path(".sodb");
    write_sodb_header(&file);
    let out = bin()
        .arg("discover")
        .arg("--steam-root")
        .arg(&missing_steam)
        .arg("--sodb-dir")
        .arg(&missing_dir)
        .arg("--sodb-dir")
        .arg(&file)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&file);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("warning: --sodb-dir folder not found or not a directory"));
    assert!(stderr.contains("warning: --sodb-dir must be a folder"));
    assert!(!stdout.contains(missing_dir.to_string_lossy().as_ref()));
    assert!(!stderr.contains(missing_dir.to_string_lossy().as_ref()));
    assert!(!stdout.contains(file.to_string_lossy().as_ref()));
    assert!(!stderr.contains(file.to_string_lossy().as_ref()));
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

fn create_sodb_with_skipped_root_sig() -> TempFile {
    let path = temp_path(".sodb");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)
        .unwrap();
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)
        .unwrap();
    conn.execute_batch(
        "
        CREATE TABLE root_signatures (Key BLOB NOT NULL PRIMARY KEY, value BLOB);
        CREATE TABLE pipeline_states (
            Key BLOB NOT NULL PRIMARY KEY, RootSignature BLOB, Flags INTEGER
        );
        INSERT INTO root_signatures (Key, value) VALUES (x'01', x'0100000044454144');
        INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (x'01', x'01', 0);
        INSERT INTO pipeline_states (Key, Flags) VALUES (x'02', 0);
        ",
    )
    .unwrap();
    drop(conn);
    TempFile { path }
}

#[test]
fn records_extract_prints_skipped_root_signature_count() {
    let sodb = create_sodb_with_skipped_root_sig();
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
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(
        stderr.contains("skipped 1 root signature"),
        "stderr was {stderr:?}"
    );
    let text = std::fs::read_to_string(&out_path.path).unwrap();
    assert!(text.contains("\"root_signature_skip\": \"invalid\""));
    assert!(text.contains("\"app_key\": \"02\""));
    assert!(!text.contains("DEAD"));
}

#[test]
fn records_extract_refused_out_does_not_print_skip_count() {
    let sodb = create_sodb_with_skipped_root_sig();
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
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        !stderr.contains("skipped"),
        "skip count must print only after a successful write, got {stderr:?}"
    );
    assert_eq!(std::fs::read(&dest.path).unwrap(), b"keep-me");
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

fn write_minimal_dxbc(path: &Path) {
    let mut bytes = vec![0_u8; 32];
    bytes[..4].copy_from_slice(b"DXBC");
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&32_u32.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn shaders_scan_json_has_counts_no_paths() {
    let dir = temp_path("-shaders");
    std::fs::create_dir_all(&dir).unwrap();
    write_minimal_dxbc(&dir.join("a.dxbc"));
    let out = bin()
        .arg("shaders")
        .arg("scan")
        .arg("--dir")
        .arg(&dir)
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
    assert_eq!(value["shaders_indexed"], 1);
    assert!(!stdout.contains(dir.to_string_lossy().as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rebuild_round_trip_cli_prints_local_notice() {
    let sodb = create_sodb();
    let records = TempFile {
        path: temp_path(".json"),
    };
    let extracted = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&records.path)
        .output()
        .unwrap();
    assert!(
        extracted.status.success(),
        "{}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    assert!(
        records.path.is_file(),
        "extract did not write a regular record file"
    );
    let dest = TempFile {
        path: temp_path("-rebuilt.sodb"),
    };
    let out = bin()
        .arg("rebuild")
        .arg("--records")
        .arg(&records.path)
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&dest.path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("local use only"));
    assert!(!stderr.contains(sodb.path.to_string_lossy().as_ref()));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(value["rebuilt"].as_u64().unwrap() >= 1);
    let inspect = bin().arg("inspect").arg(&dest.path).output().unwrap();
    assert!(inspect.status.success());
    let summary: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    let hex = summary["pipeline_states"][0]["desc_hash"].as_str().unwrap();
    let index = TempFile {
        path: temp_path("-cov.json"),
    };
    write_index(&index.path, &[hex]);
    let coverage = bin()
        .arg("coverage")
        .arg("--sodb")
        .arg(&dest.path)
        .arg("--index")
        .arg(&index.path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        coverage.status.success(),
        "{}",
        String::from_utf8_lossy(&coverage.stderr)
    );
}

#[test]
fn rebuild_max_output_bytes_is_enforced() {
    let sodb = create_sodb();
    let records = TempFile {
        path: temp_path(".json"),
    };
    let extracted = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&records.path)
        .output()
        .unwrap();
    assert!(
        extracted.status.success(),
        "{}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    let dest = TempFile {
        path: temp_path("-capped.sodb"),
    };
    let out = bin()
        .arg("rebuild")
        .arg("--records")
        .arg(&records.path)
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&dest.path)
        .arg("--max-output-bytes")
        .arg("16")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("exceeds size limit") || stderr.contains("max-output-bytes"),
        "stderr={stderr}"
    );
    assert!(!dest.path.exists());
}

fn extract_records_with_build() -> TempFile {
    let sodb = create_sodb();
    let records = TempFile {
        path: temp_path(".json"),
    };
    let extracted = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&records.path)
        .arg("--game-id")
        .arg("synthetic")
        .arg("--version")
        .arg("1.0.0")
        .arg("--exe-hash")
        .arg("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
        .output()
        .unwrap();
    assert!(
        extracted.status.success(),
        "{}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    records
}

fn sha_at(n: u64) -> osdb_records::Sha256 {
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(&n.to_be_bytes());
    osdb_records::Sha256::from_bytes(bytes)
}

/// Enough unique bytecode and pipeline hashes that the canonical manifest
/// exceeds the 8 MiB wire cap. Description tables stay empty.
fn oversize_record_file() -> TempFile {
    let count = 14_100_u64;
    let mut records = Vec::with_capacity(usize::try_from(count).unwrap());
    for i in 0..count {
        let base = i * 8;
        let stages = osdb_records::StageHashes {
            vs: Some(sha_at(base)),
            ps: Some(sha_at(base + 1)),
            hs: Some(sha_at(base + 2)),
            ds: Some(sha_at(base + 3)),
            gs: Some(sha_at(base + 4)),
            as_stage: Some(sha_at(base + 5)),
            ms: Some(sha_at(base + 6)),
            cs: Some(sha_at(base + 7)),
        };
        records.push(osdb_records::PipelineStateRecord {
            app_key: osdb_records::AppKeyHex::parse_hex(&format!("{i:08x}")).unwrap(),
            version: 0,
            bytecode_hashes: stages,
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
            sample_mask: None,
            ib_strip_cut_value: None,
            primitive_topology: None,
            dsv_format: None,
            node_mask: None,
            flags: None,
        });
    }
    let file = osdb_records::RecordFile {
        format_version: osdb_records::FORMAT_VERSION,
        build: osdb_records::BuildIdentity::new(
            Some(osdb_records::GameId::parse("synthetic").unwrap()),
            Some(osdb_records::GameVersion::parse("1.0.0").unwrap()),
            Some(
                osdb_records::ExeHash::parse_hex(
                    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                )
                .unwrap(),
            ),
        ),
        root_signatures: std::collections::BTreeMap::new(),
        input_layouts: std::collections::BTreeMap::new(),
        stream_outputs: std::collections::BTreeMap::new(),
        records,
    };
    let bytes = osdb_records::encode_record_file(&file).unwrap();
    let dest = TempFile {
        path: temp_path("-records.json"),
    };
    std::fs::write(&dest.path, bytes).unwrap();
    dest
}

#[test]
fn upload_prepare_help_lists_out_and_yes_i_reviewed() {
    let out = bin()
        .args(["upload", "prepare", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("--records"));
    assert!(stdout.contains("--build-id"));
    assert!(stdout.contains("--out"));
    assert!(stdout.contains("--yes-i-reviewed"));
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("--allow-oversize"));
    assert!(stdout.contains("Nothing is sent") || stdout.contains("nothing is sent"));
}

#[test]
fn upload_prepare_prints_preview_and_writes_same_bytes() {
    let records = extract_records_with_build();
    let dest = TempFile {
        path: temp_path("-manifest.json"),
    };
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .arg("--yes-i-reviewed")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(osdb_upload::THESE_EXACT_BYTES_LINE));
    assert!(stdout.contains(osdb_upload::NOTHING_SENT_LINE));
    assert!(stdout.contains(osdb_upload::UNCHALLENGED_LINE));
    assert!(stdout.contains(":00:00Z"));
    assert!(!stdout.contains("14:37:19"));
    let file = std::fs::read(&dest.path).unwrap();
    let start = stdout.find("Canonical JSON:").unwrap();
    let json = stdout[start + "Canonical JSON:".len()..].trim();
    assert_eq!(json.as_bytes(), file.as_slice());
    assert!(!stdout.contains(records.path.to_string_lossy().as_ref()));
    assert!(!String::from_utf8_lossy(&out.stderr).contains(records.path.to_string_lossy().as_ref()));
    let parsed = osdb_upload::UploadManifest::parse(&file).unwrap();
    assert_eq!(parsed.schema_version, osdb_upload::SCHEMA_VERSION);
}

#[test]
fn upload_prepare_checks_wire_cap_before_writing() {
    let records = oversize_record_file();
    let dest = TempFile {
        path: temp_path("-manifest.json"),
    };
    let path_text = records.path.display().to_string();
    let refused = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .arg("--yes-i-reviewed")
        .output()
        .unwrap();
    assert!(!refused.status.success());
    let stderr = String::from_utf8(refused.stderr).unwrap();
    let stdout = String::from_utf8(refused.stdout).unwrap();
    assert!(stderr
        .contains("this manifest would be rejected by the upload server; nothing was written"));
    assert!(stderr.contains("8388608"));
    assert!(stderr.contains("bytes"));
    assert!(!stderr.contains(&path_text));
    assert!(!stdout.contains("Canonical JSON"));
    assert!(!stdout.contains(&path_text));
    assert!(!dest.path.exists());

    let allowed = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .arg("--yes-i-reviewed")
        .arg("--allow-oversize")
        .output()
        .unwrap();
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let stderr = String::from_utf8(allowed.stderr).unwrap();
    let stdout = String::from_utf8(allowed.stdout).unwrap();
    assert!(stderr.contains("local-only"));
    assert!(stderr.contains("8388608"));
    assert!(stderr.contains("nothing was sent"));
    assert!(!stderr.contains(&path_text));
    assert!(!stdout.contains(&path_text));
    assert!(stdout.contains("Canonical JSON"));
    let file = std::fs::read(&dest.path).unwrap();
    assert!(file.len() > 8 * 1024 * 1024, "wrote {} bytes", file.len());
    let start = stdout.find("Canonical JSON:").unwrap();
    let json = stdout[start + "Canonical JSON:".len()..].trim();
    assert_eq!(json.as_bytes(), file.as_slice());
}

#[test]
fn upload_prepare_allow_oversize_keeps_small_manifests() {
    let records = extract_records_with_build();
    let dest = TempFile {
        path: temp_path("-manifest.json"),
    };
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .arg("--yes-i-reviewed")
        .arg("--allow-oversize")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("wire limit"));
    assert!(!stderr.contains(&records.path.display().to_string()));
    let file = std::fs::read(&dest.path).unwrap();
    assert!(file.len() < 8 * 1024 * 1024);
    assert!(file
        .windows(b"schema_version".len())
        .any(|w| w == b"schema_version"));
}

#[test]
fn upload_prepare_refuses_out_without_review_flag() {
    let records = extract_records_with_build();
    let dest = TempFile {
        path: temp_path("-manifest.json"),
    };
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("--yes-i-reviewed"));
    assert!(!dest.path.exists());
}

#[test]
fn upload_prepare_refuses_existing_out_without_force() {
    let records = extract_records_with_build();
    let dest = TempFile {
        path: temp_path("-manifest.json"),
    };
    std::fs::write(&dest.path, b"keep-me").unwrap();
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .arg("--yes-i-reviewed")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read(&dest.path).unwrap(), b"keep-me");
}

#[test]
fn upload_prepare_force_overwrites_regular_file() {
    let records = extract_records_with_build();
    let dest = TempFile {
        path: temp_path("-manifest.json"),
    };
    std::fs::write(&dest.path, b"old").unwrap();
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest.path)
        .arg("--yes-i-reviewed")
        .arg("--force")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&dest.path).unwrap();
    assert!(text.contains("schema_version"));
    assert_ne!(text.as_bytes(), b"old");
}

#[test]
fn upload_prepare_refuses_out_equal_to_records() {
    let records = extract_records_with_build();
    let original = std::fs::read(&records.path).unwrap();
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&records.path)
        .arg("--yes-i-reviewed")
        .arg("--force")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("same file"));
    assert_eq!(std::fs::read(&records.path).unwrap(), original);
}

#[test]
fn upload_prepare_canary_path_stays_out_of_stdio() {
    let sodb = create_sodb();
    let dir = temp_path("-user-alice-fixture");
    std::fs::create_dir_all(&dir).unwrap();
    let records = dir.join("records.json");
    let extracted = bin()
        .arg("records")
        .arg("extract")
        .arg("--sodb")
        .arg(&sodb.path)
        .arg("--out")
        .arg(&records)
        .arg("--game-id")
        .arg("synthetic")
        .arg("--version")
        .arg("1.0.0")
        .arg("--exe-hash")
        .arg("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
        .output()
        .unwrap();
    assert!(extracted.status.success());
    let dest = dir.join("manifest.json");
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&dest)
        .arg("--yes-i-reviewed")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stdout.contains("user-alice-fixture"));
    assert!(!stderr.contains("user-alice-fixture"));
    assert!(!stdout.contains(records.to_string_lossy().as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}

fn write_records_json(game_id: &str, version: &str, exe_hash: &str) -> TempFile {
    let records = TempFile {
        path: temp_path("-records.json"),
    };
    let json = format!(
        r#"{{"format_version":1,"build":{{"game_id":"{game_id}","version":"{version}","exe_hash":"{exe_hash}"}},"records":[{{"app_key":"01","version":0,"bytecode_hashes":{{}}}}]}}"#
    );
    std::fs::write(&records.path, json).unwrap();
    records
}

#[test]
fn upload_prepare_version_and_exe_hash_overrides() {
    let ee = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let aa = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let records = write_records_json("from-records", "1.0.0", ee);
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("from-flag")
        .arg("--version")
        .arg("9.9.9")
        .arg("--exe-hash")
        .arg(aa)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("from-flag"));
    assert!(!stdout.contains("from-records"));
    assert!(stdout.contains("9.9.9"));
    assert!(stdout.contains(aa));
    assert!(!stdout.contains(ee));
}

#[test]
fn upload_prepare_build_id_wins_over_records_game_id() {
    let ee = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let records = write_records_json("from-records", "1.0.0", ee);
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("from-flag")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("from-flag"));
    assert!(!stdout.contains("from-records"));
    assert!(stdout.contains("1.0.0"));
    assert!(stdout.contains(ee));
}

#[test]
fn upload_prepare_malformed_records_error_omits_path() {
    let bad = TempFile {
        path: temp_path("-bad.json"),
    };
    std::fs::write(&bad.path, b"\xff\xfe not-json").unwrap();
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&bad.path)
        .arg("--build-id")
        .arg("synthetic")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains(bad.path.to_string_lossy().as_ref()));
}

#[cfg(unix)]
#[test]
fn upload_prepare_refuses_symlink_out() {
    let records = extract_records_with_build();
    let target = TempFile {
        path: temp_path(".json"),
    };
    std::fs::write(&target.path, b"keep").unwrap();
    let link = temp_path(".json");
    std::os::unix::fs::symlink(&target.path, &link).unwrap();
    let out = bin()
        .arg("upload")
        .arg("prepare")
        .arg("--records")
        .arg(&records.path)
        .arg("--build-id")
        .arg("synthetic")
        .arg("--out")
        .arg(&link)
        .arg("--yes-i-reviewed")
        .arg("--force")
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&link);
    assert!(!out.status.success());
    assert_eq!(std::fs::read(&target.path).unwrap(), b"keep");
}
