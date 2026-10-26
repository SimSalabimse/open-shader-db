// SPDX-License-Identifier: MIT

//! Temp-dir fixtures. Paths in this module stay in-process; they are never
//! serialized by the types under test.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::mpsc;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use crate::scan::ScanLimits;
use crate::vdf::parse_vdf_with_limits;
use crate::{
    discover, discover_with_options, discover_with_scan, parse_vdf, parse_vdf_file,
    scan_bound_summary, AppId, BuildId, DiscoverOptions, Error, FolderScanKind, GameIdentity,
    GameListing, GameName, LocalPath, UploadSafeDiscovery, WalkBound, Warning, WarningKind,
    MAX_VDF_DEPTH, MAX_VDF_TOKENS, NO_SODB_STEAM_NOTE,
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
    let ids: Vec<u32> = found.games.iter().map(|g| g.identity.app_id().0).collect();
    assert_eq!(ids, vec![50, 100, 200], "games={:?}", found.games);
    assert_eq!(found.games[0].identity.name().as_str(), "Game Three");
    assert_eq!(found.games[1].identity.name().as_str(), "Game One");
    assert_eq!(found.games[2].identity.name().as_str(), "Game Two");
    assert_eq!(found.games[0].identity.build_id(), BuildId(33));
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
        &install.join("ALSO.SODB"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    // Header matches, extension does not: must not be opened or counted.
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
        .find(|g| g.identity.app_id() == AppId(200))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 2);
    assert!(game
        .sodb_files
        .iter()
        .any(|p| p.as_path().ends_with("cache.sodb")));
    assert!(game
        .sodb_files
        .iter()
        .any(|p| p.as_path().ends_with("ALSO.SODB")));
}

#[test]
fn missing_steam_root_is_empty_not_a_crash() {
    let missing = new_base().join("does-not-exist");
    let found = discover(&missing);
    assert!(found.games.is_empty());
    assert!(found
        .warnings
        .iter()
        .any(|w| w.kind() == WarningKind::MissingSteamRoot));
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
        .any(|w| w.kind() == WarningKind::MalformedVdf));
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
    let ids: Vec<u32> = found.games.iter().map(|g| g.identity.app_id().0).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    let no_name = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(2))
        .unwrap();
    assert_eq!(no_name.identity.name().as_str(), "unknown");
    let no_build = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(3))
        .unwrap();
    assert_eq!(no_build.identity.build_id(), BuildId(0));
    assert!(!found.games.iter().any(|g| g.identity.app_id() == AppId(4)));
    assert!(found
        .warnings
        .iter()
        .any(|w| w.kind() == WarningKind::IncompleteManifest && w.app_id() == Some(AppId(4))));
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
        .find(|g| g.identity.app_id() == AppId(100))
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
    let fifo_for_thread = fifo.clone();
    let err = run_with_timeout(Duration::from_secs(2), move || {
        parse_vdf_file(&fifo_for_thread).unwrap_err()
    });
    assert!(matches!(err, Error::NotARegularFile), "{err:?}");
}

#[cfg(unix)]
fn run_with_timeout<T: Send + 'static>(
    timeout: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(timeout)
        .unwrap_or_else(|_| panic!("guard timed out (open or walk blocked)"))
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
    blobs.push(serde_json::to_string(&safe.games()).unwrap());
    blobs.push(serde_json::to_string(&safe.warnings()).unwrap());

    let identity = GameIdentity::new(AppId(1), BuildId(2), GameName::new("Unit"));
    blobs.push(serde_json::to_string(&identity).unwrap());
    blobs.push(serde_json::to_string(&GameListing::new(identity.clone(), 3)).unwrap());
    blobs.push(serde_json::to_string(&AppId(9)).unwrap());
    blobs.push(serde_json::to_string(&BuildId(8)).unwrap());
    blobs.push(serde_json::to_string(&GameName::new("N")).unwrap());
    blobs.push(
        serde_json::to_string(&Warning::new(
            WarningKind::MissingSteamRoot,
            Some(AppId(1)),
            "Steam root does not exist or is not a directory",
        ))
        .unwrap(),
    );
    blobs.push(
        serde_json::to_string(&UploadSafeDiscovery::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
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
            ..ScanLimits::default()
        },
    );
    let limited: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind() == WarningKind::SodbScanLimited)
        .collect();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].app_id(), Some(AppId(100)));
    assert_eq!(limited[0].message(), WalkBound::Files.message());
    let summary = scan_bound_summary(&found.warnings).unwrap();
    assert_eq!(
        summary,
        "stopped SODB scan after a walk bound (files=1, entries=0, time=0)"
    );
    assert!(!summary.contains(fx.base.to_string_lossy().as_ref()));
    assert!(!summary.contains(LEAK_MARKER));
}

fn write_libraryfolders_keyed(steam: &Path, libraries: &[(&str, &Path)]) {
    let steamapps = steam.join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    let mut body = String::from("\"libraryfolders\"\n{\n");
    for (key, lib) in libraries {
        body.push_str(&format!(
            "\t\"{key}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n",
            vdf_escape(&lib.to_string_lossy())
        ));
    }
    body.push_str("}\n");
    fs::write(steamapps.join("libraryfolders.vdf"), body).unwrap();
}

fn write_manifest_raw(steamapps: &Path, file_app: u32, extra: &str) {
    fs::create_dir_all(steamapps).unwrap();
    let body = format!("\"AppState\"\n{{\n{extra}}}\n");
    fs::write(steamapps.join(format!("appmanifest_{file_app}.acf")), body).unwrap();
}

#[test]
fn installdir_must_be_plain_relative() {
    assert!(!crate::discover::is_plain_relative_installdir(""));
    assert!(!crate::discover::is_plain_relative_installdir(".."));
    assert!(!crate::discover::is_plain_relative_installdir("../outside"));
    assert!(!crate::discover::is_plain_relative_installdir("/tmp/abs"));
    assert!(!crate::discover::is_plain_relative_installdir(r"C:\x"));
    assert!(!crate::discover::is_plain_relative_installdir(r"\\?\C:\x"));
    assert!(crate::discover::is_plain_relative_installdir("GameOne"));
    assert!(crate::discover::is_plain_relative_installdir("Folder/Sub"));

    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    let cases: &[(u32, &str)] = &[
        (1, "/absolute"),
        (2, ".."),
        (3, ""),
        (4, r"C:\x"),
        (5, r"\\?\C:\x"),
    ];
    for (app, dir) in cases {
        write_manifest(&sa, *app, "Bad", dir, Some(1), true, true, true);
    }
    let found = discover(&fx.steam);
    assert!(found.games.is_empty(), "games={:?}", found.games);
    assert_eq!(
        found
            .warnings
            .iter()
            .filter(|w| w.kind() == WarningKind::InvalidInstallDir)
            .count(),
        5
    );
}

#[cfg(unix)]
#[test]
fn escaping_symlink_is_not_followed() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let outside = fx.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    write_sodb_header(
        &outside.join("leaked.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    std::os::unix::fs::symlink(&outside, install.join("escape")).unwrap();
    let found = discover(&fx.steam);
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 0);
}

#[cfg(unix)]
#[test]
fn internal_symlink_counts_target_once() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let inner = install.join("real");
    fs::create_dir_all(&inner).unwrap();
    write_sodb_header(
        &inner.join("one.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    std::os::unix::fs::symlink(&inner, install.join("alias")).unwrap();
    let found = discover(&fx.steam);
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 1);
}

#[test]
fn depth_limit_excludes_depth_five() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let deep4 = install.join("a").join("b").join("c").join("d");
    let deep5 = deep4.join("e");
    write_sodb_header(
        &deep4.join("at4.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    write_sodb_header(
        &deep5.join("at5.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover(&fx.steam);
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 1);
    assert!(game
        .sodb_files
        .iter()
        .any(|p| p.as_path().ends_with("at4.sodb")));
}

#[cfg(unix)]
#[test]
fn symlink_to_self_counts_root_file_once() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    std::os::unix::fs::symlink(&install, install.join("self")).unwrap();
    write_sodb_header(
        &install.join("root.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover(&fx.steam);
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 1);
}

#[cfg(unix)]
#[test]
fn followed_symlink_folder_still_respects_depth() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let level1 = install.join("level1");
    let target = level1.join("hidden_target");
    fs::create_dir_all(&target).unwrap();
    write_sodb_header(
        &target.join("hidden.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    std::os::unix::fs::symlink(&target, level1.join("link")).unwrap();
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_depth: 1,
            ..ScanLimits::default()
        },
    );
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert!(
        game.sodb_files.is_empty(),
        "link at depth == max_depth must not be followed"
    );
}

#[test]
fn entry_count_bound_stops_walk() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    for i in 0..80 {
        fs::create_dir_all(install.join(format!("dir-{i}"))).unwrap();
    }
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_entries: 50,
            ..ScanLimits::default()
        },
    );
    let limited: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind() == WarningKind::SodbScanLimited)
        .collect();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].message(), WalkBound::Entries.message());
    let summary = scan_bound_summary(&found.warnings).unwrap();
    assert_eq!(
        summary,
        "stopped SODB scan after a walk bound (files=0, entries=1, time=0)"
    );
    assert!(!summary.contains(LEAK_MARKER));
}

#[test]
fn deadline_bound_stops_walk() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    for i in 0..20 {
        fs::create_dir_all(install.join(format!("d-{i}"))).unwrap();
    }
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_duration: Duration::ZERO,
            ..ScanLimits::default()
        },
    );
    let limited: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.kind() == WarningKind::SodbScanLimited)
        .collect();
    assert_eq!(limited.len(), 3, "each game hits the time bound");
    assert!(limited
        .iter()
        .all(|w| w.message() == WalkBound::Time.message()));
    let summary = scan_bound_summary(&found.warnings).unwrap();
    assert_eq!(
        summary,
        "stopped SODB scan after a walk bound (files=0, entries=0, time=3)"
    );
    assert!(!summary.contains(LEAK_MARKER));
}

#[test]
fn duplicate_app_keeps_higher_build_then_earlier_library() {
    fn build_dup(first_key: &str, second_key: &str, build_a: u64, build_b: u64) -> Vec<u64> {
        let fx = empty_fixture();
        let lib_a = fx.base.join("libA");
        let lib_b = fx.base.join("libB");
        fs::create_dir_all(&lib_a).unwrap();
        fs::create_dir_all(&lib_b).unwrap();
        write_libraryfolders_keyed(&fx.steam, &[(first_key, &lib_a), (second_key, &lib_b)]);
        let sa_a = lib_a.join("steamapps");
        let sa_b = lib_b.join("steamapps");
        write_manifest(&sa_a, 42, "Dup", "DupA", Some(build_a), true, true, true);
        write_manifest(&sa_b, 42, "Dup", "DupB", Some(build_b), true, true, true);
        fs::create_dir_all(sa_a.join("common").join("DupA")).unwrap();
        fs::create_dir_all(sa_b.join("common").join("DupB")).unwrap();
        let found = discover(&fx.steam);
        assert!(found
            .warnings
            .iter()
            .any(|w| w.kind() == WarningKind::DuplicateApp && w.app_id() == Some(AppId(42))));
        assert_eq!(found.games.len(), 1);
        vec![found.games[0].identity.build_id().0]
    }

    let higher_in_ten = build_dup("2", "10", 5, 9);
    let higher_in_two = build_dup("10", "2", 9, 5);
    assert_eq!(higher_in_ten, vec![9]);
    assert_eq!(higher_in_two, vec![9]);

    let same_created_2_first = build_dup("2", "10", 5, 5);
    let same_created_10_first = build_dup("10", "2", 5, 5);
    assert_eq!(same_created_2_first, same_created_10_first);
    assert_eq!(same_created_2_first, vec![5]);
}

#[test]
fn invalid_appid_and_filename_mismatch() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    write_manifest_raw(
        &sa,
        8,
        "\t\"appid\"\t\t\"-1\"\n\t\"name\"\t\t\"Neg\"\n\t\"installdir\"\t\t\"Neg\"\n\t\"buildid\"\t\t\"1\"\n",
    );
    write_manifest_raw(
        &sa,
        9,
        "\t\"appid\"\t\t\"99999999999\"\n\t\"name\"\t\t\"Big\"\n\t\"installdir\"\t\t\"Big\"\n\t\"buildid\"\t\t\"1\"\n",
    );
    write_manifest_raw(
        &sa,
        10,
        "\t\"appid\"\t\t\"20\"\n\t\"name\"\t\t\"Mismatch\"\n\t\"installdir\"\t\t\"Mis\"\n\t\"buildid\"\t\t\"1\"\n",
    );
    fs::create_dir_all(sa.join("common").join("Mis")).unwrap();
    let found = discover(&fx.steam);
    assert!(found
        .warnings
        .iter()
        .any(|w| w.message() == "appid is not a valid u32"));
    assert_eq!(found.games.len(), 1);
    assert_eq!(found.games[0].identity.app_id(), AppId(20));
    assert!(found.warnings.iter().any(
        |w| w.message() == "manifest appid differs from the filename; using the manifest value"
    ));
}

#[test]
fn game_name_strips_control_characters() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    write_manifest_raw(
        &sa,
        11,
        "\t\"appid\"\t\t\"11\"\n\t\"name\"\t\t\"Foo\u{0}Bar\nBaz\"\n\t\"installdir\"\t\t\"Ctrl\"\n\t\"buildid\"\t\t\"1\"\n",
    );
    fs::create_dir_all(sa.join("common").join("Ctrl")).unwrap();
    let found = discover(&fx.steam);
    assert_eq!(found.games[0].identity.name().as_str(), "FooBarBaz");
}

#[test]
fn game_name_strips_unicode_format_characters() {
    let name = GameName::new("A\u{202E}B\u{200B}C\u{2066}D\u{FEFF}E");
    assert_eq!(name.as_str(), "ABCDE");
}

#[test]
fn vdf_debug_redacts_string_values() {
    let doc = parse_vdf("\"libraryfolders\" { \"0\" { \"path\" \"/secret/steam/lib\" } }").unwrap();
    let rendered = format!("{doc:?}");
    assert!(rendered.contains("String(<redacted>)"));
    assert!(!rendered.contains("/secret/steam/lib"));
}

#[test]
fn vdf_duplicate_key_keeps_last() {
    let doc = parse_vdf("\"root\" { \"k\" \"first\" \"k\" \"second\" }").unwrap();
    assert_eq!(doc.root.get("k").and_then(|v| v.as_str()), Some("second"));
}

#[test]
fn warnings_are_sorted() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    write_manifest(&sa, 3, "", "A", None, true, false, true);
    write_manifest(&sa, 1, "", "B", None, true, false, true);
    fs::create_dir_all(sa.join("common").join("A")).unwrap();
    fs::create_dir_all(sa.join("common").join("B")).unwrap();
    let found = discover(&fx.steam);
    let keyed: Vec<_> = found
        .warnings
        .iter()
        .filter(|w| w.app_id().is_some())
        .map(|w| (w.app_id(), w.kind(), w.message()))
        .collect();
    let mut sorted = keyed.clone();
    sorted.sort();
    assert_eq!(keyed, sorted);
}

#[cfg(unix)]
#[test]
fn symlinked_install_dir_is_rejected() {
    let fx = two_libs_three_games();
    let common = fx.steam.join("steamapps").join("common");
    let outside = fx.base.join("outside-lib");
    fs::create_dir_all(&outside).unwrap();
    write_sodb_header(
        &outside.join("leaked.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let sibling = common.join("OtherGame");
    fs::create_dir_all(&sibling).unwrap();
    write_sodb_header(
        &sibling.join("sib.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    std::os::unix::fs::symlink(&outside, common.join("LinkOut")).unwrap();
    std::os::unix::fs::symlink(&sibling, common.join("LinkSib")).unwrap();
    let sa = fx.steam.join("steamapps");
    write_manifest(&sa, 300, "Out", "LinkOut", Some(1), true, true, true);
    write_manifest(&sa, 301, "Sib", "LinkSib", Some(1), true, true, true);
    let found = discover(&fx.steam);
    assert!(!found
        .games
        .iter()
        .any(|g| g.identity.app_id() == AppId(300)));
    assert!(!found
        .games
        .iter()
        .any(|g| g.identity.app_id() == AppId(301)));
    assert_eq!(
        found
            .warnings
            .iter()
            .filter(|w| w.message() == "install dir resolves outside the library; game skipped")
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn symlink_loop_detected_without_hitting_entry_cap() {
    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let loop_b = install.join("loop_b");
    fs::create_dir_all(&loop_b).unwrap();
    std::os::unix::fs::symlink(&loop_b, install.join("loop_a")).unwrap();
    std::os::unix::fs::symlink(install.join("loop_a"), loop_b.join("back")).unwrap();
    write_sodb_header(
        &install.join("ok.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_depth: 32,
            max_entries: 20,
            ..ScanLimits::default()
        },
    );
    assert!(
        found
            .warnings
            .iter()
            .all(|w| w.kind() != WarningKind::SodbScanLimited),
        "loop must be detected, not stopped by the entry cap"
    );
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 1);
}

#[cfg(unix)]
#[test]
fn walk_swap_race_never_counts_outside_file() {
    use std::sync::atomic::{AtomicBool, Ordering as AtomOrdering};
    use std::sync::Arc;

    let fx = two_libs_three_games();
    let install = fx.steam.join("steamapps").join("common").join("GameOne");
    let slot = install.join("slot");
    fs::create_dir_all(&slot).unwrap();
    let outside = fx.base.join("race-outside");
    fs::create_dir_all(&outside).unwrap();
    write_sodb_header(
        &outside.join("leaked.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );

    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = Arc::clone(&stop);
    let slot_t = slot.clone();
    let outside_t = outside.clone();
    let handle = std::thread::spawn(move || {
        while !stop_t.load(AtomOrdering::Relaxed) {
            let _ = fs::remove_dir_all(&slot_t);
            let _ = fs::remove_file(&slot_t);
            let _ = std::os::unix::fs::symlink(&outside_t, &slot_t);
            let _ = fs::remove_file(&slot_t);
            let _ = fs::create_dir_all(&slot_t);
        }
    });

    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        let out = crate::scan::find_sodb_files(
            &install,
            ScanLimits {
                max_duration: Duration::from_millis(50),
                ..ScanLimits::default()
            },
            None,
        );
        assert!(
            out.files
                .iter()
                .all(|p| !p.as_path().ends_with("leaked.sodb")),
            "outside SODB leaked through a swapped symlink"
        );
    }
    stop.store(true, AtomOrdering::Relaxed);
    let _ = handle.join();
}

#[test]
fn no_sodb_note_is_once_for_the_whole_library() {
    let fx = two_libs_three_games();
    let found = discover(&fx.steam);
    assert_eq!(
        found
            .notes
            .iter()
            .filter(|note| note.message() == NO_SODB_STEAM_NOTE)
            .count(),
        1
    );
    assert!(found
        .warnings
        .iter()
        .all(|w| !w.message().contains("no SODB")));
    let json = serde_json::to_string(&found.upload_safe()).unwrap();
    assert_eq!(json.matches(NO_SODB_STEAM_NOTE).count(), 1);
    assert!(!json.contains(LEAK_MARKER));
}

#[test]
fn no_sodb_note_absent_when_a_game_has_a_file() {
    let fx = two_libs_three_games();
    let install = fx.lib2.join("steamapps").join("common").join("GameTwo");
    write_sodb_header(
        &install.join("cache.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover(&fx.steam);
    assert!(found
        .notes
        .iter()
        .all(|note| note.message() != NO_SODB_STEAM_NOTE));
    assert!(!found.games.iter().all(|g| g.sodb_files.is_empty()));
}

#[test]
fn stale_library_paths_collapse_to_one_note_without_paths() {
    let fx = empty_fixture();
    let gone = fx.base.join("gone-drive");
    let also = fx.base.join("also-gone");
    write_libraryfolders_keyed(&fx.steam, &[("0", &gone), ("1", &also), ("2", &fx.steam)]);
    let sa = fx.steam.join("steamapps");
    write_manifest(&sa, 100, "GameOne", "GameOne", Some(1), true, true, true);
    fs::create_dir_all(sa.join("common").join("GameOne")).unwrap();
    let found = discover(&fx.steam);
    assert_eq!(
        found
            .notes
            .iter()
            .filter(|note| note.message().contains("library path"))
            .count(),
        1
    );
    assert_eq!(
        found.notes[0].message(),
        "2 library paths listed in VDF are not directories"
    );
    assert!(found
        .warnings
        .iter()
        .all(|w| w.message() != "a library path listed in VDF is not a directory"));
    let rendered = format!("{found:?}");
    let json = serde_json::to_string(&found.upload_safe()).unwrap();
    for secret in ["gone-drive", "also-gone", LEAK_MARKER] {
        assert!(
            !rendered.contains(secret),
            "debug leaked {secret}: {rendered}"
        );
        assert!(!json.contains(secret), "json leaked {secret}: {json}");
        assert!(found
            .notes
            .iter()
            .all(|note| !note.message().contains(secret)));
        assert!(found.warnings.iter().all(|w| !w.message().contains(secret)));
    }
}

#[test]
fn breadth_first_finds_depth1_sodb_before_deep_tree_is_exhausted() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    write_manifest(&sa, 100, "GameOne", "GameOne", Some(1), true, true, true);
    let install = sa.join("common").join("GameOne");
    fs::create_dir_all(&install).unwrap();
    for side in ["aaa", "zzz"] {
        let leaf = install.join(side).join("d1").join("d2");
        fs::create_dir_all(&leaf).unwrap();
        for i in 0..30 {
            fs::write(leaf.join(format!("pad-{i}.txt")), b"x").unwrap();
        }
    }
    write_sodb_header(
        &install.join("mmm").join("hit.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_files: 5,
            ..ScanLimits::default()
        },
    );
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert!(
        game.sodb_files
            .iter()
            .any(|p| p.as_path().ends_with("hit.sodb")),
        "depth-1 SODB missing; warnings={:?}",
        found.warnings
    );
}

#[test]
fn exe_dir_is_scanned_before_sibling_files_exhaust_the_budget() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    fs::create_dir_all(&sa).unwrap();
    fs::write(
        sa.join("appmanifest_100.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"100\"\n\t\"name\"\t\t\"GameOne\"\n\t\"installdir\"\t\t\"GameOne\"\n\t\"buildid\"\t\t\"1\"\n\t\"Executable\"\t\t\"zzz\\\\game.exe\"\n}\n",
    )
    .unwrap();
    let install = sa.join("common").join("GameOne");
    fs::create_dir_all(install.join("zzz")).unwrap();
    fs::create_dir_all(install.join("aaa")).unwrap();
    write_sodb_header(
        &install.join("zzz").join("hit.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    for i in 0..20 {
        fs::write(install.join("aaa").join(format!("pad-{i}.txt")), b"x").unwrap();
    }
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_files: 3,
            ..ScanLimits::default()
        },
    );
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert!(
        game.sodb_files
            .iter()
            .any(|p| p.as_path().ends_with("hit.sodb")),
        "exe folder was not scanned before sibling files"
    );
}

#[test]
fn exe_dir_deeper_than_one_is_scanned_before_depth1_files() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    fs::create_dir_all(&sa).unwrap();
    fs::write(
        sa.join("appmanifest_100.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"100\"\n\t\"name\"\t\t\"GameOne\"\n\t\"installdir\"\t\t\"GameOne\"\n\t\"buildid\"\t\t\"1\"\n\t\"Executable\"\t\t\"Content/Bin/game.exe\"\n}\n",
    )
    .unwrap();
    let install = sa.join("common").join("GameOne");
    fs::create_dir_all(install.join("Content").join("Bin")).unwrap();
    fs::create_dir_all(install.join("aaa")).unwrap();
    write_sodb_header(
        &install.join("Content").join("Bin").join("hit.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    for i in 0..20 {
        fs::write(install.join("aaa").join(format!("pad-{i}.txt")), b"x").unwrap();
    }
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_files: 3,
            ..ScanLimits::default()
        },
    );
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert!(
        game.sodb_files
            .iter()
            .any(|p| p.as_path().ends_with("hit.sodb")),
        "executable directory was not read before depth-1 files"
    );
}

#[test]
fn huge_asset_dirs_are_skipped_and_a_shallow_sodb_is_kept() {
    let fx = empty_fixture();
    write_libraryfolders(&fx.steam, &[&fx.steam]);
    let sa = fx.steam.join("steamapps");
    write_manifest(&sa, 100, "GameOne", "GameOne", Some(1), true, true, true);
    let install = sa.join("common").join("GameOne");
    fs::create_dir_all(&install).unwrap();
    for name in [
        "Content",
        "Paks",
        "Movies",
        "Videos",
        "Audio",
        "Sounds",
        "Textures",
        "Streaming",
    ] {
        let dir = install.join(name);
        fs::create_dir_all(&dir).unwrap();
        write_sodb_header(
            &dir.join("hidden.sodb"),
            SODB_USER_VERSION,
            SODB_APPLICATION_ID,
        );
        for i in 0..10 {
            fs::write(dir.join(format!("pad-{i}.bin")), b"x").unwrap();
        }
    }
    write_sodb_header(
        &install.join("keep").join("hit.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let found = discover_with_scan(
        &fx.steam,
        ScanLimits {
            max_files: 4,
            ..ScanLimits::default()
        },
    );
    let game = found
        .games
        .iter()
        .find(|g| g.identity.app_id() == AppId(100))
        .unwrap();
    assert_eq!(game.sodb_files.len(), 1);
    assert!(game
        .sodb_files
        .iter()
        .any(|p| p.as_path().ends_with("hit.sodb")));
    assert!(game
        .sodb_files
        .iter()
        .all(|p| !p.as_path().ends_with("hidden.sodb")));
}

#[test]
fn explicit_sodb_dir_and_xbox_roots_are_path_free_in_json() {
    let fx = empty_fixture();
    let capture = fx.base.join("capture");
    write_sodb_header(
        &capture.join("cap.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    write_sodb_header(
        &capture.join("not.db"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let too_deep = capture.join("a").join("b").join("c").join("d").join("e");
    write_sodb_header(
        &too_deep.join("too-deep.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    let drive_c = fx.base.join("C");
    let drive_d = fx.base.join("D");
    let drive_e = fx.base.join("E");
    fs::create_dir_all(drive_c.join("XboxGames").join("Title")).unwrap();
    write_sodb_header(
        &drive_c.join("XboxGames").join("Title").join("game.sodb"),
        SODB_USER_VERSION,
        SODB_APPLICATION_ID,
    );
    fs::create_dir_all(drive_d.join("XboxGames")).unwrap();
    fs::create_dir_all(&drive_e).unwrap();
    let found = discover_with_options(
        None,
        &DiscoverOptions {
            sodb_dirs: vec![capture],
            xbox_drive_roots: vec![drive_c, drive_d, drive_e, fx.base.join("missing-drive")],
            ..DiscoverOptions::default()
        },
    );
    assert!(found.games.is_empty());
    assert!(found
        .warnings
        .iter()
        .all(|w| w.kind() != WarningKind::MissingSteamRoot));
    assert_eq!(found.folder_scans.len(), 3);
    assert_eq!(found.folder_scans[0].kind, FolderScanKind::ExplicitDir);
    assert_eq!(found.folder_scans[0].sodb_files.len(), 1);
    assert_eq!(found.folder_scans[1].kind, FolderScanKind::XboxGames);
    assert_eq!(found.folder_scans[1].sodb_files.len(), 1);
    assert_eq!(found.folder_scans[2].sodb_files.len(), 0);
    let json = serde_json::to_string(&found.upload_safe()).unwrap();
    assert!(json.contains("explicit_dir"));
    assert!(json.contains("xbox_games"));
    assert!(!json.contains(LEAK_MARKER));
    assert!(!json.contains("capture"));
    assert!(!json.contains("XboxGames"));
    let rendered = format!("{found:?}");
    assert!(!rendered.contains(LEAK_MARKER));
    assert!(!rendered.contains("capture"));
}

#[cfg(not(windows))]
#[test]
fn default_fixed_drive_roots_are_empty_off_windows() {
    assert!(crate::default_fixed_drive_roots().is_empty());
}
