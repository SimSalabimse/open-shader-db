// SPDX-License-Identifier: MIT

//! Behavior tests. Directories are removed when the test value drops.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use osdb_upload::{
    ClientVersion, ExeHash, GameId, GameVersion, HashSet, OwnershipProof, Sha256Hex,
    UploadManifest, UtcHour,
};

use crate::consensus::HoldReason;
use crate::staging::{
    is_safe_ident, object_hash_from_name, sha256_of, split_build_dir, IngestPath, LoadLimits,
    SkipReason, StagedUpload, StagingRoot,
};
use crate::threshold::{self, AgreementThreshold, CountingPolicy};
use crate::{build_consensus, list_builds, load_build, AgreementUnit, BuildId, Error};

fn h(byte: u8) -> Sha256Hex {
    Sha256Hex::from_bytes([byte; 32])
}

fn build(game: &str, version: &str, exe: u8) -> BuildId {
    BuildId::new(
        GameId::parse(game).unwrap(),
        GameVersion::parse(version).unwrap(),
        ExeHash::from_sha256(h(exe)),
    )
}

fn hash_set(items: &[Sha256Hex]) -> HashSet {
    let mut owned = items.to_vec();
    owned.sort();
    owned.dedup();
    HashSet::from_sorted_unique(owned, 10_000).unwrap()
}

fn manifest_for(
    build_id: &BuildId,
    client: &str,
    hour: u8,
    pipelines: Option<&[Sha256Hex]>,
    bytecode: &[Sha256Hex],
    descriptors: &[Sha256Hex],
    ownership: OwnershipProof,
) -> UploadManifest {
    UploadManifest {
        schema_version: if pipelines.is_some() { 2 } else { 1 },
        build_id: build_id.clone(),
        client_version: ClientVersion::parse(client).unwrap(),
        collected_hour: UtcHour::from_ymd_h(2026, 10, 25, hour).unwrap(),
        description_hashes: hash_set(descriptors),
        bytecode_hashes: hash_set(bytecode),
        ownership,
        pipeline_hashes: pipelines.map(hash_set),
    }
}

fn staged(
    build_id: &BuildId,
    client: &str,
    hour: u8,
    pipelines: Option<&[Sha256Hex]>,
    bytecode: &[Sha256Hex],
    descriptors: &[Sha256Hex],
    ownership: OwnershipProof,
) -> StagedUpload {
    let manifest = manifest_for(
        build_id,
        client,
        hour,
        pipelines,
        bytecode,
        descriptors,
        ownership,
    );
    let bytes = manifest.to_canonical_bytes().unwrap();
    StagedUpload {
        object_sha256: sha256_of(&bytes),
        manifest,
        via: IngestPath::ReviewedPullRequest,
    }
}

fn with_via(upload: StagedUpload, via: IngestPath) -> StagedUpload {
    StagedUpload { via, ..upload }
}

fn limits(files: u64, bytes: u64, entries: u64) -> LoadLimits {
    LoadLimits {
        max_files_per_build: files,
        max_file_bytes: bytes,
        max_entries_scanned: entries,
        manifest: osdb_upload::Limits::DEFAULT,
    }
}

fn skipped(report: &crate::LoadReport, reason: SkipReason) -> u64 {
    report.skipped.get(&reason).copied().unwrap_or(0)
}

struct Temp {
    path: PathBuf,
}

impl Temp {
    fn new(label: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "osdb-consensus-{label}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn build_dir(root: &Path, build_id: &BuildId) -> PathBuf {
    let name = format!(
        "{}--{}",
        build_id.version.as_str(),
        build_id.exe_hash.to_hex()
    );
    let dir = root.join(build_id.game_id.as_str()).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_manifest(dir: &Path, manifest: &UploadManifest) -> Sha256Hex {
    let bytes = manifest.to_canonical_bytes().unwrap();
    let hash = sha256_of(&bytes);
    std::fs::write(dir.join(format!("{}.json", hash.to_hex())), &bytes).unwrap();
    hash
}

fn write_named(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn names_in(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let entries = std::fs::read_dir(dir).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(entry.path()).unwrap();
        let bytes = if meta.file_type().is_file() {
            std::fs::read(entry.path()).unwrap()
        } else {
            Vec::new()
        };
        out.insert(name, bytes);
    }
    out
}

#[test]
fn defaults_are_the_documented_bar() {
    let threshold = AgreementThreshold::default();
    assert_eq!(threshold.min_uploads(), 3);
    assert_eq!(threshold.min_ratio_per_mille(), 500);
    assert_eq!(
        AgreementThreshold::new(3, 500).unwrap(),
        AgreementThreshold::DEFAULT
    );
    assert!(AgreementThreshold::new(0, 500).is_err());
    assert!(AgreementThreshold::new(1, 1001).is_err());
    assert!(AgreementThreshold::new(1, 0).is_ok());
    let policy = CountingPolicy::default();
    assert!(!policy.count_worker_unchallenged);
    assert!(!policy.count_worker_exe_binding);
    assert_eq!(crate::DEFAULT_MAX_FILES_PER_BUILD, 64);
    assert_eq!(crate::WALK_DEPTH, 2);
}

#[test]
fn empty_staging_loads_and_lists_nothing() {
    let temp = Temp::new("empty");
    let root = StagingRoot::new(temp.path());
    let build_id = build("synthetic", "1.0.0", 1);
    let _dir = build_dir(temp.path(), &build_id);
    let report = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(64, 1024, 32),
    )
    .unwrap();
    assert!(report.loaded.is_empty());
    assert!(report.skipped.is_empty());
    assert_eq!(
        list_builds(&root, limits(64, 1024, 32)).unwrap(),
        vec![build_id.clone()]
    );

    let missing = build("synthetic", "9.9.9", 2);
    let missing_report = load_build(
        &root,
        &missing,
        IngestPath::WorkerStaging,
        limits(64, 1024, 32),
    )
    .unwrap();
    assert!(missing_report.loaded.is_empty());
    assert!(!temp
        .path()
        .join("synthetic")
        .join(format!("9.9.9--{}", missing.exe_hash.to_hex()))
        .exists());
}

#[test]
fn missing_root_is_an_io_error_without_creating_it() {
    let temp = Temp::new("missing-root");
    let absent = temp.path().join("no-such-root");
    let root = StagingRoot::new(&absent);
    let err = list_builds(&root, LoadLimits::default()).unwrap_err();
    assert_eq!(err, Error::Io);
    assert!(!absent.exists());
}

#[test]
fn file_root_is_not_a_directory() {
    let temp = Temp::new("file-root");
    let file = temp.path().join("not-a-dir");
    std::fs::write(&file, b"[]").unwrap();
    let root = StagingRoot::new(&file);
    let err = list_builds(&root, LoadLimits::default()).unwrap_err();
    assert_eq!(err, Error::NotADirectory);
    let err = load_build(
        &root,
        &build("synthetic", "1", 1),
        IngestPath::ReviewedPullRequest,
        LoadLimits::default(),
    )
    .unwrap_err();
    assert_eq!(err, Error::NotADirectory);
}

#[test]
fn mixed_valid_staging_counts_every_skip_reason() {
    let temp = Temp::new("mixed");
    let build_id = build("synthetic", "1.0.0", 3);
    let other = build("synthetic", "1.0.0", 4);
    let dir = build_dir(temp.path(), &build_id);
    let good = manifest_for(
        &build_id,
        "0.1.0",
        1,
        Some(&[h(1)]),
        &[h(2)],
        &[h(3)],
        OwnershipProof::unchallenged(),
    );
    let good_hash = write_manifest(&dir, &good);
    std::fs::create_dir(dir.join("nested")).unwrap();
    write_named(&dir.join("nested"), "ignored.json", b"{}");
    write_named(&dir, "notes.txt", b"hello");
    let mismatch_bytes = good.to_canonical_bytes().unwrap();
    write_named(&dir, &format!("{}.json", h(0xee).to_hex()), &mismatch_bytes);
    let mut spaced = good.to_canonical_bytes().unwrap();
    let _ = spaced.splice(0..1, b"{ ".iter().copied());
    let spaced_name = format!("{}.json", sha256_of(&spaced).to_hex());
    write_named(&dir, &spaced_name, &spaced);
    let foreign = manifest_for(
        &other,
        "0.1.2",
        2,
        Some(&[h(9)]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    write_manifest(&dir, &foreign);
    let mut version_nine = good.to_canonical_bytes().unwrap();
    let text = String::from_utf8(version_nine.clone()).unwrap();
    let replaced = text.replace("\"schema_version\":2", "\"schema_version\":9");
    assert_ne!(replaced, text);
    version_nine = replaced.into_bytes();
    write_named(
        &dir,
        &format!("{}.json", sha256_of(&version_nine).to_hex()),
        &version_nine,
    );
    let cap = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .max()
        .unwrap();
    let fat = vec![b'x'; usize::try_from(cap).unwrap() + 8];
    write_named(&dir, &format!("{}.json", h(0xaa).to_hex()), &fat);

    let before = names_in(&dir);
    let root = StagingRoot::new(temp.path());
    let report = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(64, cap, 64),
    )
    .unwrap();
    assert_eq!(names_in(&dir), before, "loader must not mutate staging");
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].object_sha256, good_hash);
    assert_eq!(report.loaded[0].via, IngestPath::ReviewedPullRequest);
    assert_eq!(report.loaded[0].manifest, good);
    assert_eq!(skipped(&report, SkipReason::NotRegularFile), 1);
    assert_eq!(skipped(&report, SkipReason::BadFileName), 1);
    assert_eq!(skipped(&report, SkipReason::NameHashMismatch), 1);
    assert_eq!(skipped(&report, SkipReason::NotCanonical), 1);
    assert_eq!(skipped(&report, SkipReason::BuildIdMismatch), 1);
    assert_eq!(
        skipped(&report, SkipReason::Parse("UNSUPPORTED_SCHEMA_VERSION")),
        1
    );
    assert_eq!(skipped(&report, SkipReason::TooLarge), 1);
    assert_eq!(
        report
            .loaded
            .iter()
            .map(|u| u.object_sha256)
            .collect::<Vec<_>>(),
        {
            let mut hashes = report
                .loaded
                .iter()
                .map(|u| u.object_sha256)
                .collect::<Vec<_>>();
            hashes.sort();
            hashes
        }
    );
}

#[test]
fn over_file_cap_keeps_the_lowest_names_and_does_not_open_the_rest() {
    let temp = Temp::new("file-cap");
    let build_id = build("synthetic", "1", 5);
    let dir = build_dir(temp.path(), &build_id);
    let first = manifest_for(
        &build_id,
        "0.1.0",
        1,
        Some(&[h(1)]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    let second = manifest_for(
        &build_id,
        "0.1.1",
        2,
        Some(&[h(2)]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    let hash_a = write_manifest(&dir, &first);
    let hash_b = write_manifest(&dir, &second);
    let mut ordered = [hash_a, hash_b];
    ordered.sort();
    write_named(&dir, "not-a-manifest.txt", b"nope");
    let root = StagingRoot::new(temp.path());
    let report = load_build(
        &root,
        &build_id,
        IngestPath::WorkerStaging,
        limits(1, 1_000_000, 32),
    )
    .unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].object_sha256, ordered[0]);
    assert_eq!(report.loaded[0].via, IngestPath::WorkerStaging);
    assert_eq!(skipped(&report, SkipReason::OverFileCap), 1);
    assert_eq!(skipped(&report, SkipReason::BadFileName), 1);

    let none = load_build(
        &root,
        &build_id,
        IngestPath::WorkerStaging,
        limits(0, 1_000_000, 32),
    )
    .unwrap();
    assert!(none.loaded.is_empty());
    assert_eq!(skipped(&none, SkipReason::OverFileCap), 2);
    assert_eq!(skipped(&none, SkipReason::BadFileName), 1);
}

#[test]
fn over_entries_cap_is_an_error_and_leaves_the_tree() {
    let temp = Temp::new("entries");
    let build_id = build("synthetic", "1", 6);
    let dir = build_dir(temp.path(), &build_id);
    let manifest = manifest_for(
        &build_id,
        "0.1.0",
        1,
        Some(&[]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    write_manifest(&dir, &manifest);
    write_named(&dir, "extra.txt", b"x");
    let before = names_in(&dir);
    let root = StagingRoot::new(temp.path());
    let err = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(64, 1_000_000, 1),
    )
    .unwrap_err();
    assert_eq!(err, Error::TooManyEntries { max: 1 });
    assert_eq!(names_in(&dir), before);
    let empty_cap = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(64, 1_000_000, 0),
    )
    .unwrap_err();
    assert_eq!(empty_cap, Error::TooManyEntries { max: 0 });
}

#[test]
fn list_builds_is_sorted_and_ignores_hostile_names() {
    let temp = Temp::new("list");
    let exe = h(7);
    let later = BuildId::new(
        GameId::parse("zeta").unwrap(),
        GameVersion::parse("1--beta").unwrap(),
        ExeHash::from_sha256(exe),
    );
    let earlier = build("alpha", "1.0.0", 7);
    build_dir(temp.path(), &later);
    build_dir(temp.path(), &earlier);
    let game = temp.path().join("alpha");
    std::fs::create_dir(game.join(format!("..--{}", exe.to_hex()))).unwrap();
    std::fs::write(game.join("notes.txt"), b"x").unwrap();
    std::fs::create_dir(game.join("nested")).unwrap();
    std::fs::create_dir_all(
        temp.path()
            .join("游戏")
            .join(format!("1--{}", exe.to_hex())),
    )
    .unwrap();
    std::fs::create_dir(temp.path().join("a b")).unwrap();
    let long = "a".repeat(129);
    std::fs::create_dir(temp.path().join(&long)).unwrap();
    let ident_128 = "b".repeat(128);
    let wide = BuildId::new(
        GameId::parse(&ident_128).unwrap(),
        GameVersion::parse("1").unwrap(),
        ExeHash::from_sha256(exe),
    );
    build_dir(temp.path(), &wide);
    if std::fs::create_dir(temp.path().join("a:b")).is_ok() {
        std::fs::create_dir(temp.path().join("a:b").join(format!("1--{}", exe.to_hex()))).unwrap();
    }
    let root = StagingRoot::new(temp.path());
    let listed = list_builds(&root, limits(64, 1024, 64)).unwrap();
    assert_eq!(listed, vec![earlier, wide, later]);
    let err = list_builds(&root, limits(64, 1024, 1)).unwrap_err();
    assert_eq!(err, Error::TooManyEntries { max: 1 });
}

#[test]
fn identical_bytes_via_hard_link_appear_once() {
    let temp = Temp::new("hardlink");
    let build_id = build("synthetic", "1", 8);
    let dir = build_dir(temp.path(), &build_id);
    let manifest = manifest_for(
        &build_id,
        "0.1.0",
        4,
        Some(&[h(4)]),
        &[h(5)],
        &[h(6)],
        OwnershipProof::unchallenged(),
    );
    let hash = write_manifest(&dir, &manifest);
    let alias = dir.join(format!("{}.json", h(0x11).to_hex()));
    std::fs::hard_link(dir.join(format!("{}.json", hash.to_hex())), &alias).unwrap();
    let root = StagingRoot::new(temp.path());
    let report = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(8, 1_000_000, 16),
    )
    .unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].object_sha256, hash);
    assert_eq!(skipped(&report, SkipReason::NameHashMismatch), 1);
    assert!(alias.exists());
}

#[test]
fn two_loads_match_and_do_not_depend_on_write_order() {
    let temp = Temp::new("order");
    let build_id = build("synthetic", "2", 9);
    let dir = build_dir(temp.path(), &build_id);
    for (client, hour, pipe) in [("0.1.2", 3, 3_u8), ("0.1.0", 1, 1), ("0.1.1", 2, 2)] {
        let manifest = manifest_for(
            &build_id,
            client,
            hour,
            Some(&[h(pipe)]),
            &[],
            &[],
            OwnershipProof::unchallenged(),
        );
        write_manifest(&dir, &manifest);
    }
    let root = StagingRoot::new(temp.path());
    let limits = limits(10, 1_000_000, 20);
    let first = load_build(&root, &build_id, IngestPath::ReviewedPullRequest, limits).unwrap();
    let second = load_build(&root, &build_id, IngestPath::ReviewedPullRequest, limits).unwrap();
    assert_eq!(first, second);
    let hashes: Vec<_> = first.loaded.iter().map(|u| u.object_sha256).collect();
    let mut sorted = hashes.clone();
    sorted.sort();
    assert_eq!(hashes, sorted);
    assert_eq!(hashes.len(), 3);
}

#[test]
fn component_rules_reject_dot_dotdot_unicode_and_overlong_names() {
    assert!(is_safe_ident("synthetic"));
    assert!(is_safe_ident(&"a".repeat(128)));
    assert!(is_safe_ident("1--beta"));
    assert!(is_safe_ident("..."));
    for bad in [
        "",
        ".",
        "..",
        &"a".repeat(129),
        "a b",
        "a/b",
        "a\\b",
        "a:b",
        "游戏",
        "a\nb",
        "a\0b",
        "café",
    ] {
        assert!(!is_safe_ident(bad), "{bad:?} must be rejected");
    }
    let exe = "ab".repeat(32);
    assert_eq!(
        split_build_dir(&format!("1.0.0--{exe}")).unwrap().0,
        "1.0.0"
    );
    assert_eq!(
        split_build_dir(&format!("1--beta--{exe}")).unwrap().0,
        "1--beta"
    );
    for bad in [
        format!("..--{exe}"),
        format!(".--{exe}"),
        format!("--{exe}"),
        format!("1.0.0--{}", "ab".repeat(31)),
        format!("1.0.0--{}", "AB".repeat(32)),
        format!("1.0.0--{exe}x"),
        format!("a b--{exe}"),
        format!("游戏--{exe}"),
        exe.clone(),
        format!("../{exe}"),
    ] {
        assert!(split_build_dir(&bad).is_none(), "{bad} must be rejected");
    }
}

#[test]
fn file_name_table_accepts_only_lowercase_object_hashes() {
    let ok = format!("{}.json", "ab".repeat(32));
    assert!(object_hash_from_name(&ok).is_some());
    let upper = format!("{}.json", "AB".repeat(32));
    let short = format!("{}.json", "a".repeat(63));
    let long = format!("{}.json", "a".repeat(65));
    let not_hex = format!("{}.json", "g".repeat(64));
    let bare = "a".repeat(64);
    let cases = [
        "",
        ".json",
        "abc.json",
        ".",
        "..",
        "../x.json",
        "foo/bar.json",
        "foo\\bar.json",
        "has space.json",
        "UPPER.json",
        upper.as_str(),
        short.as_str(),
        long.as_str(),
        not_hex.as_str(),
        "name.json.bak",
        bare.as_str(),
        "文件.json",
        "a/../../etc.json",
    ];
    for name in cases {
        assert!(
            object_hash_from_name(name).is_none(),
            "{name:?} must be rejected"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlink_entries_are_not_followed() {
    let temp = Temp::new("symlink");
    let build_id = build("synthetic", "1", 10);
    let dir = build_dir(temp.path(), &build_id);
    let manifest = manifest_for(
        &build_id,
        "0.1.0",
        1,
        Some(&[h(1)]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    let outside = Temp::new("symlink-target");
    let outside_dir = build_dir(outside.path(), &build_id);
    let hash = write_manifest(&outside_dir, &manifest);
    let link = dir.join(format!("{}.json", hash.to_hex()));
    std::os::unix::fs::symlink(outside_dir.join(format!("{}.json", hash.to_hex())), &link).unwrap();
    let dir_link = dir.join("sub");
    std::os::unix::fs::symlink(&outside_dir, &dir_link).unwrap();
    let root = StagingRoot::new(temp.path());
    let report = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(8, 1_000_000, 16),
    )
    .unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(skipped(&report, SkipReason::Symlink), 2);
    assert!(link.exists());

    let game_link_root = Temp::new("game-link");
    std::os::unix::fs::symlink(
        temp.path().join("synthetic"),
        game_link_root.path().join("synthetic"),
    )
    .unwrap();
    let linked = StagingRoot::new(game_link_root.path());
    let err = load_build(
        &linked,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(8, 1_000_000, 16),
    )
    .unwrap_err();
    assert_eq!(err, Error::Symlink);
    assert!(list_builds(&linked, limits(8, 1024, 16))
        .unwrap()
        .is_empty());

    let root_link_parent = Temp::new("root-link");
    let root_link = root_link_parent.path().join("root");
    std::os::unix::fs::symlink(temp.path(), &root_link).unwrap();
    let as_root = StagingRoot::new(&root_link);
    assert_eq!(
        list_builds(&as_root, LoadLimits::default()).unwrap_err(),
        Error::Symlink
    );
}

#[cfg(unix)]
#[test]
fn fifo_is_not_a_regular_file_and_does_not_block() {
    let temp = Temp::new("fifo");
    let build_id = build("synthetic", "1", 11);
    let dir = build_dir(temp.path(), &build_id);
    let fifo = dir.join(format!("{}.json", "ab".repeat(32)));
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let root_path = temp.path().to_path_buf();
    let build_for_thread = build_id.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let root = StagingRoot::new(&root_path);
        let result = load_build(
            &root,
            &build_for_thread,
            IngestPath::ReviewedPullRequest,
            limits(8, 1_000_000, 8),
        );
        let _ = tx.send(result);
    });
    let received = rx.recv_timeout(std::time::Duration::from_secs(2));
    let report = received.expect("FIFO open hung; regular-file / O_NONBLOCK check regressed");
    let report = report.unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(skipped(&report, SkipReason::NotRegularFile), 1);
    handle.join().unwrap();
    assert!(fifo.exists());
}

#[test]
fn reports_never_contain_a_path_or_file_body() {
    let marker_path = "OSDB_PATH_SENTINEL_7f3a";
    let marker_body = "OSDB_BODY_SENTINEL_7f3a";
    let temp = Temp::new(marker_path);
    assert!(temp.path().to_string_lossy().contains(marker_path));
    let build_id = build("synthetic", "1", 12);
    let dir = build_dir(temp.path(), &build_id);
    let good = manifest_for(
        &build_id,
        "0.1.0",
        1,
        Some(&[h(1)]),
        &[h(2)],
        &[h(3)],
        OwnershipProof::unchallenged(),
    );
    write_manifest(&dir, &good);
    let body = format!("{marker_body}-not-a-manifest");
    let body_bytes = body.as_bytes();
    write_named(
        &dir,
        &format!("{}.json", sha256_of(body_bytes).to_hex()),
        body_bytes,
    );
    let root = StagingRoot::new(temp.path());
    let report = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(8, 1_000_000, 8),
    )
    .unwrap();
    assert_eq!(report.loaded.len(), 1);
    assert!(skipped(&report, SkipReason::Parse("INVALID_JSON")) >= 1);
    let missing = StagingRoot::new(temp.path().join(marker_path));
    let io_err = list_builds(&missing, LoadLimits::default()).unwrap_err();
    let consensus = build_consensus(
        &build_id,
        &report.loaded,
        AgreementThreshold::default(),
        CountingPolicy::default(),
    );
    let mut rendered = String::new();
    rendered.push_str(&format!("{root:?}"));
    rendered.push_str(&format!("{report:?}"));
    rendered.push_str(&format!("{consensus:?}"));
    rendered.push_str(&format!("{io_err} {io_err:?}"));
    for err in [
        Error::Io,
        Error::NotADirectory,
        Error::Symlink,
        Error::TooManyEntries { max: 3 },
        Error::InvalidBuildId,
        Error::InvalidThreshold,
    ] {
        rendered.push_str(&format!("{err} {err:?}"));
    }
    for reason in report.skipped.keys() {
        rendered.push_str(&format!("{reason} {reason:?}"));
    }
    let absolute = temp.path().to_string_lossy().into_owned();
    assert!(!rendered.contains(marker_path), "{rendered}");
    assert!(!rendered.contains(marker_body), "{rendered}");
    assert!(!rendered.contains(&absolute), "{rendered}");
    assert_eq!(format!("{root:?}"), "<redacted>");
}

#[test]
fn threshold_boundaries_and_ratio_edges() {
    let build_id = build("synthetic", "1", 13);
    let pipe = h(0x21);
    let uploads: Vec<_> = (0..3_u8)
        .map(|index| {
            staged(
                &build_id,
                &format!("0.1.{index}"),
                index,
                Some(&[pipe]),
                &[h(0x22)],
                &[h(0x23)],
                OwnershipProof::unchallenged(),
            )
        })
        .collect();
    let two = build_consensus(
        &build_id,
        &uploads[..2],
        AgreementThreshold::new(3, 1000).unwrap(),
        CountingPolicy::default(),
    );
    assert!(two.accepted.is_empty());
    assert_eq!(two.counted_uploads, 2);
    assert_eq!(
        two.rejected_counts.get(&AgreementUnit::PipelineHash),
        Some(&1)
    );
    assert!(!format!("{two:?}").contains(&pipe.to_hex()));

    let three = build_consensus(
        &build_id,
        &uploads,
        AgreementThreshold::new(3, 1000).unwrap(),
        CountingPolicy::default(),
    );
    assert_eq!(three.accepted.len(), 3);
    assert!(three.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::PipelineHash && tally.hash == pipe && tally.uploads == 3
    }));

    let mut half = Vec::new();
    for index in 0..4_u8 {
        let pipelines: &[Sha256Hex] = if index < 2 { &[pipe] } else { &[] };
        half.push(staged(
            &build_id,
            &format!("0.2.{index}"),
            index,
            Some(pipelines),
            &[],
            &[],
            OwnershipProof::unchallenged(),
        ));
    }
    let exact = build_consensus(
        &build_id,
        &half,
        AgreementThreshold::new(2, 500).unwrap(),
        CountingPolicy::default(),
    );
    assert!(exact.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::PipelineHash && tally.hash == pipe && tally.uploads == 2
    }));
    let below = build_consensus(
        &build_id,
        &half,
        AgreementThreshold::new(2, 501).unwrap(),
        CountingPolicy::default(),
    );
    assert!(below
        .accepted
        .iter()
        .all(|tally| tally.unit != AgreementUnit::PipelineHash));
    assert_eq!(
        below.rejected_counts.get(&AgreementUnit::PipelineHash),
        Some(&1)
    );
    assert!(!format!("{below:?}").contains(&pipe.to_hex()));
}

#[test]
fn threshold_is_not_lowered_for_a_single_upload() {
    let build_id = build("synthetic", "1", 14);
    let pipe = h(0x31);
    let upload = staged(
        &build_id,
        "0.1.0",
        1,
        Some(&[pipe]),
        &[h(0x32)],
        &[h(0x33)],
        OwnershipProof::unchallenged(),
    );
    let result = build_consensus(
        &build_id,
        &[upload],
        AgreementThreshold::default(),
        CountingPolicy::default(),
    );
    assert_eq!(result.upload_count, 1);
    assert_eq!(result.counted_uploads, 1);
    assert!(result.accepted.is_empty());
    assert!(result.held.is_empty());
    assert_eq!(
        result.rejected_counts.get(&AgreementUnit::PipelineHash),
        Some(&1)
    );
}

#[test]
fn v1_manifests_do_not_enter_pipeline_agreement() {
    let build_id = build("synthetic", "1", 15);
    let bytecode = h(0x41);
    let descriptor = h(0x42);
    let mut uploads = Vec::new();
    for index in 0..3_u8 {
        uploads.push(staged(
            &build_id,
            &format!("0.3.{index}"),
            index,
            None,
            &[bytecode],
            &[descriptor],
            OwnershipProof::unchallenged(),
        ));
    }
    let result = build_consensus(
        &build_id,
        &uploads,
        AgreementThreshold::new(3, 1000).unwrap(),
        CountingPolicy::default(),
    );
    assert!(result
        .accepted
        .iter()
        .all(|tally| tally.unit != AgreementUnit::PipelineHash));
    assert!(!result
        .rejected_counts
        .contains_key(&AgreementUnit::PipelineHash));
    assert!(result.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::BytecodeHash && tally.hash == bytecode && tally.uploads == 3
    }));
    assert!(result.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::DescriptorKey && tally.hash == descriptor && tally.uploads == 3
    }));
}

#[test]
fn mixed_v1_and_v2_use_separate_denominators() {
    let build_id = build("synthetic", "1", 16);
    let pipe = h(0x51);
    let bytecode = h(0x52);
    let mut uploads = Vec::new();
    for index in 0..2_u8 {
        uploads.push(staged(
            &build_id,
            &format!("0.4.{index}"),
            index,
            Some(&[pipe]),
            &[bytecode],
            &[],
            OwnershipProof::unchallenged(),
        ));
    }
    for index in 2..4_u8 {
        uploads.push(staged(
            &build_id,
            &format!("0.4.{index}"),
            index,
            None,
            &[bytecode],
            &[],
            OwnershipProof::unchallenged(),
        ));
    }
    let result = build_consensus(
        &build_id,
        &uploads,
        AgreementThreshold::new(2, 1000).unwrap(),
        CountingPolicy::default(),
    );
    assert!(result.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::PipelineHash && tally.hash == pipe && tally.uploads == 2
    }));
    assert!(result.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::BytecodeHash && tally.hash == bytecode && tally.uploads == 4
    }));
}

#[test]
fn empty_v2_pipeline_list_still_counts_in_the_denominator() {
    let build_id = build("synthetic", "1", 17);
    let pipe = h(0x61);
    let mut uploads = Vec::new();
    uploads.push(staged(
        &build_id,
        "0.5.0",
        1,
        Some(&[pipe]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    ));
    for index in 1..4_u8 {
        uploads.push(staged(
            &build_id,
            &format!("0.5.{index}"),
            index,
            Some(&[]),
            &[],
            &[],
            OwnershipProof::unchallenged(),
        ));
    }
    let exact = build_consensus(
        &build_id,
        &uploads,
        AgreementThreshold::new(1, 250).unwrap(),
        CountingPolicy::default(),
    );
    assert!(exact.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::PipelineHash && tally.hash == pipe && tally.uploads == 1
    }));
    let below = build_consensus(
        &build_id,
        &uploads,
        AgreementThreshold::new(1, 251).unwrap(),
        CountingPolicy::default(),
    );
    assert!(below.accepted.is_empty());
    assert_eq!(
        below.rejected_counts.get(&AgreementUnit::PipelineHash),
        Some(&1)
    );
}

#[test]
fn worker_unchallenged_uploads_are_held_and_reviewed_prs_count() {
    let build_id = build("synthetic", "1", 18);
    let pipe = h(0x71);
    let worker = with_via(
        staged(
            &build_id,
            "0.1.0",
            1,
            Some(&[pipe]),
            &[h(0x72)],
            &[],
            OwnershipProof::unchallenged(),
        ),
        IngestPath::WorkerStaging,
    );
    let reviewed = staged(
        &build_id,
        "0.1.1",
        2,
        Some(&[pipe]),
        &[h(0x72)],
        &[],
        OwnershipProof::unchallenged(),
    );
    let binding = with_via(
        staged(
            &build_id,
            "0.1.2",
            3,
            Some(&[h(0x73)]),
            &[],
            &[],
            OwnershipProof::exe_binding_v1(h(0x74), h(0x75)),
        ),
        IngestPath::WorkerStaging,
    );
    let held = build_consensus(
        &build_id,
        &[worker.clone(), binding.clone()],
        AgreementThreshold::new(1, 1000).unwrap(),
        CountingPolicy::default(),
    );
    assert_eq!(held.counted_uploads, 0);
    assert_eq!(held.upload_count, 2);
    assert!(held.accepted.is_empty());
    assert!(held.rejected_counts.is_empty());
    assert_eq!(held.held.len(), 2);
    assert!(held
        .held
        .iter()
        .any(|item| item.reason == HoldReason::WorkerUnchallenged));
    assert!(held
        .held
        .iter()
        .any(|item| item.reason == HoldReason::WorkerExeBindingUnverified));
    assert!(!format!("{held:?}").contains(&pipe.to_hex()));
    assert!(!format!("{held:?}").contains(&h(0x73).to_hex()));

    let counted = build_consensus(
        &build_id,
        &[reviewed.clone(), worker.clone()],
        AgreementThreshold::new(1, 1000).unwrap(),
        CountingPolicy::default(),
    );
    assert_eq!(counted.counted_uploads, 1);
    assert_eq!(counted.held.len(), 1);
    assert_eq!(counted.held[0].reason, HoldReason::WorkerUnchallenged);
    assert!(counted.accepted.iter().any(|tally| {
        tally.unit == AgreementUnit::PipelineHash && tally.hash == pipe && tally.uploads == 1
    }));

    let allow = CountingPolicy {
        count_worker_unchallenged: true,
        ..CountingPolicy::default()
    };
    let both = build_consensus(
        &build_id,
        &[worker, reviewed],
        AgreementThreshold::new(2, 1000).unwrap(),
        allow,
    );
    assert_eq!(both.counted_uploads, 2);
    assert!(both.held.is_empty());
    assert!(both
        .accepted
        .iter()
        .any(|tally| tally.uploads == 2 && tally.hash == pipe));
}

#[test]
fn consensus_output_is_sorted_and_independent_of_input_order() {
    let build_id = build("synthetic", "1", 19);
    let other = build("other", "1", 19);
    let mut uploads = vec![
        staged(
            &build_id,
            "0.1.0",
            1,
            Some(&[h(0x02), h(0xff)]),
            &[h(0x10), h(0x01)],
            &[h(0x20)],
            OwnershipProof::unchallenged(),
        ),
        with_via(
            staged(
                &other,
                "0.1.1",
                2,
                Some(&[h(0x99)]),
                &[],
                &[],
                OwnershipProof::unchallenged(),
            ),
            IngestPath::WorkerStaging,
        ),
    ];
    let threshold = AgreementThreshold::new(1, 1000).unwrap();
    let forward = build_consensus(&build_id, &uploads, threshold, CountingPolicy::default());
    uploads.reverse();
    let backward = build_consensus(&build_id, &uploads, threshold, CountingPolicy::default());
    assert_eq!(forward, backward);
    assert_eq!(forward.upload_count, 2);
    assert_eq!(forward.counted_uploads, 1);
    assert_eq!(forward.held.len(), 1);
    assert_eq!(forward.held[0].reason, HoldReason::BuildMismatch);
    assert!(!format!("{forward:?}").contains(&h(0x99).to_hex()));
    let units: Vec<_> = forward.accepted.iter().map(|tally| tally.unit).collect();
    assert_eq!(
        units,
        vec![
            AgreementUnit::BytecodeHash,
            AgreementUnit::BytecodeHash,
            AgreementUnit::DescriptorKey,
            AgreementUnit::PipelineHash,
            AgreementUnit::PipelineHash,
        ]
    );
    let bytecode: Vec<_> = forward
        .accepted
        .iter()
        .filter(|tally| tally.unit == AgreementUnit::BytecodeHash)
        .map(|tally| tally.hash)
        .collect();
    assert_eq!(bytecode, vec![h(0x01), h(0x10)]);
    assert_eq!(forward.held.len() + forward.counted_uploads as usize, 2);
    assert_eq!(
        forward
            .accepted
            .iter()
            .find(|tally| tally.hash == h(0x02))
            .unwrap()
            .uploads,
        1
    );
}

#[test]
fn empty_input_keeps_the_requested_build() {
    let build_id = build("synthetic", "1", 20);
    let result = build_consensus(
        &build_id,
        &[],
        AgreementThreshold::default(),
        CountingPolicy::default(),
    );
    assert_eq!(result.build, build_id);
    assert_eq!(result.upload_count, 0);
    assert_eq!(result.counted_uploads, 0);
    assert!(result.accepted.is_empty());
    assert!(result.held.is_empty());
    assert!(result.rejected_counts.is_empty());
}

#[test]
fn ratio_math_does_not_wrap_u32() {
    let full = AgreementThreshold::new(1, 1000).unwrap();
    assert!(threshold::hash_accepted(u32::MAX, u32::MAX, full));
    assert!(threshold::hash_accepted(4_294_968, 4_294_968, full));
    // `1 * 1000 >= 1000 * 4_294_968` is false. A wrapping `u32` multiply of the
    // right-hand side would flip the comparison.
    assert!(!threshold::hash_accepted(1, 4_294_968, full));
    let high_bar = AgreementThreshold::new(u32::MAX, 1000).unwrap();
    assert!(!threshold::hash_accepted(u32::MAX - 1, u32::MAX, high_bar));
    assert!(threshold::hash_accepted(u32::MAX, u32::MAX, high_bar));
    let exact = AgreementThreshold::new(1, 500).unwrap();
    assert!(threshold::hash_accepted(1, 2, exact));
    let just_under = AgreementThreshold::new(1, 501).unwrap();
    assert!(!threshold::hash_accepted(1, 2, just_under));
    assert!(!threshold::hash_accepted(5, 0, full));
}

#[test]
fn each_upload_counts_a_hash_once() {
    let build_id = build("synthetic", "1", 21);
    let pipe = h(0x81);
    let one = staged(
        &build_id,
        "0.1.0",
        1,
        Some(&[pipe, pipe]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    assert_eq!(one.manifest.pipeline_hashes.as_ref().unwrap().len(), 1);
    let result = build_consensus(
        &build_id,
        &[one],
        AgreementThreshold::new(1, 1000).unwrap(),
        CountingPolicy::default(),
    );
    let tally = result
        .accepted
        .iter()
        .find(|tally| tally.unit == AgreementUnit::PipelineHash)
        .unwrap();
    assert_eq!(tally.uploads, 1);
}

#[test]
fn staging_root_debug_is_redacted() {
    let root = StagingRoot::new("/home/os-user/secret/staging");
    let debug = format!("{root:?}");
    assert_eq!(debug, "<redacted>");
    assert!(!debug.contains("os-user"));
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("staging"));
}

#[test]
fn game_directory_that_is_a_file_is_not_a_directory() {
    let temp = Temp::new("game-file");
    std::fs::write(temp.path().join("synthetic"), b"not-a-directory").unwrap();
    let root = StagingRoot::new(temp.path());
    let err = load_build(
        &root,
        &build("synthetic", "1", 22),
        IngestPath::ReviewedPullRequest,
        LoadLimits::default(),
    )
    .unwrap_err();
    assert_eq!(err, Error::NotADirectory);
    assert!(list_builds(&root, LoadLimits::default())
        .unwrap()
        .is_empty());
}

#[test]
fn loader_does_not_descend_past_the_build_directory() {
    let temp = Temp::new("depth");
    let build_id = build("synthetic", "1", 23);
    let dir = build_dir(temp.path(), &build_id);
    let nested = dir.join("deeper").join("still");
    std::fs::create_dir_all(&nested).unwrap();
    let manifest = manifest_for(
        &build_id,
        "0.1.0",
        1,
        Some(&[h(1)]),
        &[],
        &[],
        OwnershipProof::unchallenged(),
    );
    write_manifest(&nested, &manifest);
    let root = StagingRoot::new(temp.path());
    let report = load_build(
        &root,
        &build_id,
        IngestPath::ReviewedPullRequest,
        limits(8, 1_000_000, 16),
    )
    .unwrap();
    assert!(report.loaded.is_empty());
    assert_eq!(skipped(&report, SkipReason::NotRegularFile), 1);
    assert_eq!(
        list_builds(&root, limits(8, 1024, 16)).unwrap(),
        vec![build_id]
    );
}
