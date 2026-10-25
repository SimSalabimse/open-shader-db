// SPDX-License-Identifier: MIT

//! Temp-dir fixtures. Paths in this module stay in-process; they are never
//! serialized by the types under test.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

use crate::scan::ScanLimits;
use crate::vdf::parse_vdf_with_limits;
use crate::{
    discover, discover_with_scan, parse_vdf, parse_vdf_file, AppId, BuildId, Error, GameIdentity,
    GameListing, GameName, LocalPath, UploadSafeDiscovery, Warning, WarningKind, MAX_VDF_DEPTH,
    MAX_VDF_TOKENS,
};
use sodb::{SODB_APPLICATION_ID, SODB_USER_VERSION};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Distinctive directory name used by the path-leak test (a fake username).
const LEAK_MARKER: &str = "user-alice-fixture";

struct Fixture {
    base: PathBuf,
    steam: PathBuf,
    lib2: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn new_base() -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{LEAK_MARKER}-{}-{n}", std::process::id()))
}

fn empty_fixture() -> Fixture {
    let base = new_base();
    let steam = base.join("steam");
    let lib2 = base.join("lib2");
    fs::create_dir_all(&steam).unwrap();
    fs::create_dir_all(&lib2).unwrap();
    Fixture { base, steam, lib2 }
}

fn vdf_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn write_libraryfolders(steam: &Path, libraries: &[&Path]) {
    let steamapps = steam.join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    let mut body = String::from("\"libraryfolders\"\n{\n");
    for (i, lib) in libraries.iter().enumerate() {
        body.push_str(&format!(
            "\t\"{i}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n",
            vdf_escape(&lib.to_string_lossy())
        ));
    }
    body.push_str("}\n");
    fs::write(steamapps.join("libraryfolders.vdf"), body).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn write_manifest(
    steamapps: &Path,
    app: u32,
    name: &str,
    installdir: &str,
    build: Option<u64>,
    include_installdir: bool,
    include_name: bool,
    include_appid: bool,
) {
    fs::create_dir_all(steamapps).unwrap();
    let mut body = String::from("\"AppState\"\n{\n");
    if include_appid {
        body.push_str(&format!("\t\"appid\"\t\t\"{app}\"\n"));
    }
    if include_name {
        body.push_str(&format!("\t\"name\"\t\t\"{}\"\n", vdf_escape(name)));
    }
    if include_installdir {
        body.push_str(&format!(
            "\t\"installdir\"\t\t\"{}\"\n",
            vdf_escape(installdir)
        ));
    }
    if let Some(build) = build {
        body.push_str(&format!("\t\"buildid\"\t\t\"{build}\"\n"));
    }
    body.push_str("}\n");
    fs::write(steamapps.join(format!("appmanifest_{app}.acf")), body).unwrap();
}

fn write_sodb_header(path: &Path, user_version: i32, application_id: i32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut buf = vec![0_u8; 100];
    buf[..16].copy_from_slice(b"SQLite format 3\0");
    buf[18] = 1;
    buf[19] = 1;
    buf[60..64].copy_from_slice(&user_version.to_be_bytes());
    buf[68..72].copy_from_slice(&application_id.to_be_bytes());
    fs::write(path, buf).unwrap();
}

fn two_libs_three_games() -> Fixture {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam, &fx.lib2]);

    let sa1 = fx.steam.join("steamapps");
    write_manifest(&sa1, 100, "Game One", "GameOne", Some(11), true, true, true);
    fs::create_dir_all(sa1.join("common").join("GameOne")).unwrap();

    let sa2 = fx.lib2.join("steamapps");
    write_manifest(&sa2, 200, "Game Two", "GameTwo", Some(22), true, true, true);
    write_manifest(
        &sa2,
        50,
        "Game Three",
        "GameThree",
        Some(33),
        true,
        true,
        true,
    );
    fs::create_dir_all(sa2.join("common").join("GameTwo")).unwrap();
    fs::create_dir_all(sa2.join("common").join("GameThree")).unwrap();
    fx
}

#[test]
fn vdf_quotes_escapes_comments_and_nesting() {
    let text = r#"
        // line comment
        "root" /* block */ {
            "key" "value with \"quotes\" and \\slash"
            "child" {
                "n" "1"
            }
            bare unquoted
        }
    "#;
    let doc = parse_vdf(text).unwrap();
    assert_eq!(doc.root_key, "root");
    assert_eq!(
        doc.root.get("key").and_then(|v| v.as_str()),
        Some("value with \"quotes\" and \\slash")
    );
    assert_eq!(
        doc.root
            .get("child")
            .and_then(|v| v.get("n"))
            .and_then(|v| v.as_str()),
        Some("1")
    );
    assert_eq!(
        doc.root.get("bare").and_then(|v| v.as_str()),
        Some("unquoted")
    );
}

#[test]
fn two_libraries_three_games_sorted_by_app_id() {
    let fx = two_libs_three_games();
    let found = discover(&fx.steam);
    let ids: Vec<u32> = found.games.iter().map(|g| g.identity.app_id.0).collect();
    assert_eq!(ids, vec![50, 100, 200], "games={:?}", found.games);
    assert_eq!(found.games[0].identity.name.as_str(), "Game Three");
    assert_eq!(found.games[1].identity.name.as_str(), "Game One");
    assert_eq!(found.games[2].identity.name.as_str(), "Game Two");
    assert_eq!(found.games[0].identity.build_id, BuildId(33));
}

#[test]
fn game_with_sodb_file() {
    let fx = two_libs_three_games();
    let install = fx.lib2.join("steamapps").join("common").join("GameTwo");
    write_sodb_header(
        &install.join("cache.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    write_sodb_header(
        &install.join("also.db"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    write_sodb_header(&install.join("wrong.sodb"), 1, SODB_APPLICATION_ID);
    fs::write(install.join("readme.txt"), b"not sqlite").unwrap();

    let found = discover(&fx.steam);
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id == AppId(200))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 2);
}

#[test]
fn missing_steam_root_is_empty_not_a_crash() {
    let missing = new_base().join("does-not-exist");
    let found = discover(&missing);
    assert!(found.games.is_empty());
    assert!(found
        .warnings
        .iter()
        .any(|w| w.kind == WarningKind::MissingSteamRoot));
}

#[test]
fn malformed_vdf_is_a_warning() {
    let fx = empty_fixture();
    let steamapps = fx.steam.join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    fs::write(steamapps.join("libraryfolders.vdf"), "\"root\" { \"x\" \"").unwrap();
    write_manifest(&steamapps, 7, "Solo", "Solo", Some(1), true, true, true);
    fs::create_dir_all(steamapps.join("common").join("Solo")).unwrap();
    let found = discover(&fx.steam);
    assert!(found
        .warnings
        .iter()
        .any(|w| w.kind == WarningKind::MalformedVdf));
    assert_eq!(found.games.len(), 1);
}

#[test]
fn hostile_vdf_deep_nesting() {
    let mut text = String::from("\"root\" ");
    for _ in 0..=MAX_VDF_DEPTH {
        text.push_str("{ \"k\" ");
    }
    text.push_str("\"leaf\"");
    for _ in 0..=MAX_VDF_DEPTH {
        text.push('}');
    }
    let err = parse_vdf(&text).unwrap_err();
    assert!(matches!(err, Error::TooDeep { .. }), "{err:?}");
}

#[test]
fn hostile_vdf_huge_file() {
    let fx = empty_fixture();
    let path = fx.base.join("huge.vdf");
    let mut file = File::create(&path).unwrap();
    file.write_all(b"\"root\" { \"k\" \"").unwrap();
    let chunk = vec![b'x'; 64 * 1024];
    let mut written = 14_u64;
    while written <= crate::MAX_VDF_BYTES {
        file.write_all(&chunk).unwrap();
        written += chunk.len() as u64;
    }
    file.write_all(b"\" }").unwrap();
    drop(file);
    let err = parse_vdf_file(&path).unwrap_err();
    assert!(matches!(err, Error::FileTooLarge { .. }), "{err:?}");
}

#[test]
fn hostile_vdf_huge_token_count() {
    let mut text = String::from("\"root\" {\n");
    for i in 0..40_000 {
        text.push_str(&format!("\"k{i}\" \"v\"\n"));
    }
    text.push('}');
    let err = parse_vdf_with_limits(&text, MAX_VDF_DEPTH, MAX_VDF_TOKENS).unwrap_err();
    assert!(matches!(err, Error::TooManyTokens { .. }), "{err:?}");
}

#[test]
fn missing_manifest_fields() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    write_manifest(&sa, 1, "HasAll", "HasAll", Some(9), true, true, true);
    fs::create_dir_all(sa.join("common").join("HasAll")).unwrap();
    write_manifest(&sa, 2, "", "NoName", Some(9), true, false, true);
    fs::create_dir_all(sa.join("common").join("NoName")).unwrap();
    write_manifest(&sa, 3, "NoBuild", "NoBuild", None, true, true, true);
    fs::create_dir_all(sa.join("common").join("NoBuild")).unwrap();
    write_manifest(&sa, 4, "NoDir", "unused", Some(9), false, true, true);

    let found = discover(&fx.steam);
    let ids: Vec<u32> = found.games.iter().map(|g| g.identity.app_id.0).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    let no_name = found
        .games
        .iter()
        .find(|g| g.identity.app_id == AppId(2))
        .unwrap();
    assert_eq!(no_name.identity.name.as_str(), "unknown");
    let no_build = found
        .games
        .iter()
        .find(|g| g.identity.app_id == AppId(3))
        .unwrap();
    assert_eq!(no_build.identity.build_id, BuildId(0));
    assert!(!found.games.iter().any(|g| g.identity.app_id == AppId(4)));
    assert!(found
        .warnings
        .iter()
        .any(|w| w.kind == WarningKind::IncompleteManifest && w.app_id == Some(AppId(4))));
}

#[cfg(unix)]
#[test]
fn symlink_loop_does_not_hang() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let a = install.join("loop_a");
    let b = install.join("loop_b");
    std::os::unix::fs::symlink(&b, &a).unwrap();
    std::os::unix::fs::symlink(&a, &b).unwrap();
    write_sodb_header(
        &install.join("ok.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );

    let start = Instant::now();
    let found = discover(&fx.steam);
    assert!(start.elapsed() < Duration::from_secs(5), "walk hung");
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 1);
}

#[cfg(unix)]
#[test]
fn fifo_is_not_opened() {
    let fx = empty_fixture();
    let fifo = fx.base.join("pipe.vdf");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let start = Instant::now();
    let err = parse_vdf_file(&fifo).unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(matches!(err, Error::NotARegularFile), "{err:?}");
}

/// Serialize every public `Serialize` type and assert the fixture path and
/// fake username never appear. [`LocalPath`] / [`crate::InstalledGame`] are
/// not `Serialize` (see the module docs on path isolation).
#[test]
fn serialized_payload_never_contains_paths() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    write_sodb_header(
        &install.join("game.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover(&fx.steam);
    assert!(!found.games.is_empty());
    assert!(found.games.iter().any(|g| !g.sodb_files.is_empty()));

    let leak_path = fx.base.to_string_lossy().into_owned();
    let mut blobs = Vec::new();

    let safe = found.upload_safe();
    blobs.push(serde_json::to_string(&safe).unwrap());
    blobs.push(serde_json::to_string(&safe.games).unwrap());
    blobs.push(serde_json::to_string(&safe.warnings).unwrap());

    let identity = GameIdentity {
        app_id: AppId(1),
        build_id: BuildId(2),
        name: GameName(String::from("Unit")),
    };
    blobs.push(serde_json::to_string(&identity).unwrap());
    blobs.push(
        serde_json::to_string(&GameListing {
            identity: identity.clone(),
            sodb_file_count: 3,
        })
        .unwrap(),
    );
    blobs.push(serde_json::to_string(&AppId(9)).unwrap());
    blobs.push(serde_json::to_string(&BuildId(8)).unwrap());
    blobs.push(serde_json::to_string(&GameName(String::from("N"))).unwrap());
    blobs.push(
        serde_json::to_string(&Warning {
            kind: WarningKind::MissingSteamRoot,
            app_id: Some(AppId(1)),
            message: String::from("Steam root does not exist or is not a directory"),
        })
        .unwrap(),
    );
    blobs.push(
        serde_json::to_string(&UploadSafeDiscovery {
            games: Vec::new(),
            warnings: Vec::new(),
        })
        .unwrap(),
    );

    let debugs = format!("{:?} {:?}", found, LocalPath::new(fx.base.clone()));
    assert!(
        debugs.contains("LocalPath(<redacted>)"),
        "Debug must redact paths: {debugs}"
    );
    assert!(
        !debugs.contains(&leak_path),
        "Debug leaked fixture path: {debugs}"
    );
    assert!(
        !debugs.contains(LEAK_MARKER),
        "Debug leaked username marker: {debugs}"
    );

    for blob in &blobs {
        assert!(
            !blob.contains(&leak_path),
            "serialized payload leaked path: {blob}"
        );
        assert!(
            !blob.contains(LEAK_MARKER),
            "serialized payload leaked username marker: {blob}"
        );
        assert!(!blob.contains("GameOne"), "install dir name leaked: {blob}");
    }
}

#[test]
fn scan_file_count_bound_emits_warning() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    for i in 0..8 {
        fs::write(install.join(format!("pad-{i}.txt")), b"x").unwrap();
    }
    write_sodb_header(
        &install.join("late.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_depth: 4,
            max_files: 2,
        },
    );
    assert!(found
        .warnings
        .iter()
        .any(|w| w.kind == WarningKind::SodbScanLimited));
}
