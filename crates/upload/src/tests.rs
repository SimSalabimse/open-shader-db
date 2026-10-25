// SPDX-License-Identifier: MIT

//! Hostile-input, privacy, determinism, and schema tests.

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use osdb_records::{
    AppKeyHex, BuildIdentity, PipelineStateRecord, RecordFile, Sha256, StageHashes,
};
use osdb_steam::{AppId, GameIdentity, GameName};

use crate::build::{BuildId, ExeHash, GameId, GameVersion};
use crate::builder::ManifestBuilder;
use crate::client::ClientVersion;
use crate::hash::Sha256Hex;
use crate::manifest::{HashSet, UploadManifest, SCHEMA_VERSION};
use crate::ownership::OwnershipProof;
use crate::preview::{UploadPreview, NOTHING_SENT_LINE, THESE_EXACT_BYTES_LINE, UNCHALLENGED_LINE};
use crate::schema::{document, MANIFEST_FIELDS, SCHEMA_JSON};
use crate::time::{FixedClock, UtcHour};
use crate::{
    Error, Limits, BYTECODE_STAGES_PER_RECORD, DEFAULT_MAX_BYTECODE_HASHES,
    DEFAULT_MAX_DESCRIPTION_HASHES, DEFAULT_MAX_HASHES_PER_LIST, DEFAULT_MAX_JSON_BYTES,
    DEFAULT_PER_BUILD_UPLOAD_CAP, DESCRIPTION_HASHES_PER_RECORD, LIMITS_JSON,
    MAX_NUMBER_TOKEN_CHARS, WIRE_MAX_JSON_BYTES,
};

const CANARY_USER: &str = "user-alice-fixture";
const CANARY_PATH: &str = "/home/user-alice-fixture/Games/ExampleTitle";
const CANARY_IP: &str = "203.0.113.77";
const CANARY_HOST: &str = "alice-laptop.local";
const CANARY_HWID: &str = "hwid-DEADBEEF-cpu0";
const CANARY_FULL_TS: &str = "2026-10-25T14:37:19Z";

fn canaries() -> [&'static str; 6] {
    [
        CANARY_USER,
        CANARY_PATH,
        CANARY_IP,
        CANARY_HOST,
        CANARY_HWID,
        CANARY_FULL_TS,
    ]
}

fn assert_no_canaries(text: &str) {
    for marker in canaries() {
        assert!(
            !text.contains(marker),
            "canary {marker:?} leaked in {text:?}"
        );
    }
    assert!(
        !text.contains("14:37:19"),
        "full timestamp leaked in {text:?}"
    );
}

fn hex64(n: u64) -> String {
    format!("{n:064x}")
}

fn sha_rec(n: u64) -> Sha256 {
    Sha256::parse_hex(&hex64(n)).unwrap()
}

fn sha_up(n: u64) -> Sha256Hex {
    Sha256Hex::parse(&hex64(n)).unwrap()
}

fn sample_exe() -> ExeHash {
    ExeHash::parse(&hex64(0xee)).unwrap()
}

fn sample_build() -> BuildId {
    BuildId::new(
        GameId::parse("synthetic").unwrap(),
        GameVersion::parse("1.0.0").unwrap(),
        sample_exe(),
    )
}

fn hour_clock(year: i32, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> FixedClock {
    let secs = unix_secs(year, month, day, hour, min, sec);
    FixedClock::new(UNIX_EPOCH + Duration::from_secs(secs))
}

fn unix_secs(year: i32, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> u64 {
    let mut days = 0_u64;
    for y in 1970..year {
        let yu = u32::try_from(y).unwrap();
        days += if yu.is_multiple_of(4) && (!yu.is_multiple_of(100) || yu.is_multiple_of(400)) {
            366
        } else {
            365
        };
    }
    let yu = u32::try_from(year).unwrap();
    for m in 1..month {
        days += u64::from(days_in_month(yu, m));
    }
    days += u64::from(day.saturating_sub(1));
    days * 86_400 + u64::from(hour) * 3600 + u64::from(min) * 60 + u64::from(sec)
}

fn days_in_month(year: u32, month: u8) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

fn blank_record(app_key: &str) -> PipelineStateRecord {
    PipelineStateRecord {
        app_key: AppKeyHex::parse_hex(app_key).unwrap(),
        version: 0,
        bytecode_hashes: StageHashes::default(),
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
    }
}

fn record_file(records: Vec<PipelineStateRecord>) -> RecordFile {
    RecordFile {
        format_version: 1,
        build: BuildIdentity::empty(),
        root_signatures: BTreeMap::new(),
        input_layouts: BTreeMap::new(),
        stream_outputs: BTreeMap::new(),
        records,
    }
}

fn build_manifest(file: &RecordFile) -> UploadManifest {
    ManifestBuilder::new(file)
        .build_id(sample_build())
        .clock(&hour_clock(2026, 10, 25, 14, 37, 19))
        .unwrap()
        .build()
        .unwrap()
}

fn canary_app_key_hex() -> String {
    AppKeyHex::from_bytes(CANARY_USER.as_bytes())
        .as_str()
        .to_string()
}

#[test]
fn canary_never_appears_in_manifest_preview_errors_or_debug() {
    let app_key = canary_app_key_hex();
    let mut rec = blank_record(&app_key);
    rec.bytecode_hashes.vs = Some(sha_rec(1));
    rec.root_signature = Some(sha_rec(2));
    rec.input_layout = Some(sha_rec(3));
    rec.stream_out = Some(sha_rec(4));
    let file = record_file(vec![rec]);
    let identity = GameIdentity::new(
        AppId(1),
        osdb_steam::BuildId(99),
        GameName::new(format!(
            "{CANARY_USER} {CANARY_IP} {CANARY_HOST} {CANARY_HWID}"
        )),
    );
    let builder = ManifestBuilder::new(&file)
        .build_id(sample_build())
        .steam_identity(&identity)
        .clock(&hour_clock(2026, 10, 25, 14, 37, 19))
        .unwrap();
    assert_no_canaries(&format!("{builder:?}"));
    let manifest = builder.build().unwrap();
    let bytes = manifest.to_canonical_bytes().unwrap();
    let json = String::from_utf8(bytes.clone()).unwrap();
    let preview = UploadPreview::from_manifest(&manifest).unwrap();
    assert_eq!(preview.canonical_bytes(), bytes.as_slice());
    assert_no_canaries(&json);
    assert_no_canaries(preview.as_text());
    assert_no_canaries(&format!("{manifest:?}"));
    assert_no_canaries(&format!("{preview:?}"));
    assert!(!json.contains(&app_key));
    assert!(!preview.as_text().contains(&app_key));
    assert!(!json.contains(CANARY_PATH));
    assert!(json.contains("2026-10-25T14:00:00Z"));
    assert!(!json.contains(CANARY_FULL_TS));

    let hostile = format!(
        r#"{{"{CANARY_USER}":"{CANARY_PATH}","ip":"{CANARY_IP}","host":"{CANARY_HOST}","ts":"{CANARY_FULL_TS}"}}"#
    );
    let err = UploadManifest::parse(hostile.as_bytes()).unwrap_err();
    assert_no_canaries(&err.to_string());
    assert_no_canaries(&format!("{err:?}"));
}

#[test]
fn empty_records_yield_empty_hash_lists() {
    let file = record_file(Vec::new());
    let manifest = build_manifest(&file);
    assert!(manifest.description_hashes.is_empty());
    assert!(manifest.bytecode_hashes.is_empty());
    assert_eq!(manifest.ownership, OwnershipProof::unchallenged());
}

#[test]
fn duplicate_hashes_are_deduplicated_and_sorted() {
    let mut a = blank_record("01");
    a.bytecode_hashes.vs = Some(sha_rec(9));
    a.bytecode_hashes.ps = Some(sha_rec(3));
    a.root_signature = Some(sha_rec(5));
    let mut b = blank_record("02");
    b.bytecode_hashes.cs = Some(sha_rec(3));
    b.root_signature = Some(sha_rec(5));
    b.input_layout = Some(sha_rec(1));
    let file = record_file(vec![b, a]);
    let manifest = build_manifest(&file);
    let desc: Vec<_> = manifest
        .description_hashes
        .as_slice()
        .iter()
        .map(Sha256Hex::to_hex)
        .collect();
    let code: Vec<_> = manifest
        .bytecode_hashes
        .as_slice()
        .iter()
        .map(Sha256Hex::to_hex)
        .collect();
    assert_eq!(desc, vec![hex64(1), hex64(5)]);
    assert_eq!(code, vec![hex64(3), hex64(9)]);
}

#[test]
fn wrong_case_and_length_hashes_are_rejected() {
    assert!(matches!(
        Sha256Hex::parse(&"A".repeat(64)),
        Err(Error::MalformedHex)
    ));
    assert!(matches!(
        Sha256Hex::parse(&"aa".repeat(31)),
        Err(Error::WrongHexLength { got: 62 })
    ));
    let mut json = sample_json();
    let obj = json.as_object_mut().unwrap();
    obj.insert(
        "bytecode_hashes".into(),
        serde_json::json!(["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"]),
    );
    let err = UploadManifest::parse(serde_json::to_vec(&json).unwrap().as_slice()).unwrap_err();
    assert!(matches!(err, Error::MalformedHex));
    assert_no_canaries(&err.to_string());
}

#[test]
fn parse_rejects_non_utf8_bom_trailing_and_deep_nesting() {
    assert!(matches!(
        UploadManifest::parse(&[0xff, 0xfe, 0x00]),
        Err(Error::InvalidJson)
    ));
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(&sample_canonical());
    assert!(matches!(
        UploadManifest::parse(&bom),
        Err(Error::ByteOrderMark)
    ));
    let mut trailing = sample_canonical();
    trailing.extend_from_slice(b" {}");
    assert!(matches!(
        UploadManifest::parse(&trailing),
        Err(Error::TrailingGarbage)
    ));
    let deep = format!("{}null{}", "[".repeat(9), "]".repeat(9));
    assert!(matches!(
        UploadManifest::parse(deep.as_bytes()),
        Err(Error::NestingTooDeep { max: 8 })
    ));
}

#[test]
fn oversized_manifest_and_hash_lists_are_rejected() {
    let tiny = Limits {
        max_json_bytes: 8,
        max_description_hashes: 2,
        max_bytecode_hashes: 2,
        max_json_nesting: 8,
    };
    assert!(matches!(
        UploadManifest::parse_with_limits(b"{\"a\":1,\"b\":2}", tiny),
        Err(Error::ManifestTooLarge { max: 8, .. })
    ));
    let mut recs = Vec::new();
    for i in 0..3_u64 {
        let mut rec = blank_record(&format!("{i:02x}"));
        rec.bytecode_hashes.vs = Some(sha_rec(i + 1));
        recs.push(rec);
    }
    let file = record_file(recs);
    let err = ManifestBuilder::new(&file)
        .build_id(sample_build())
        .limits(tiny)
        .clock(&hour_clock(2026, 10, 25, 14, 0, 0))
        .unwrap()
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::TooManyHashes { max: 2, got: 3 }));
}

#[test]
fn deny_unknown_fields_and_missing_fields() {
    let mut extra = sample_json();
    extra
        .as_object_mut()
        .unwrap()
        .insert("username".into(), serde_json::json!(CANARY_USER));
    let err = UploadManifest::parse(serde_json::to_vec(&extra).unwrap().as_slice()).unwrap_err();
    assert!(matches!(err, Error::UnknownField));
    assert_no_canaries(&err.to_string());

    let mut missing = sample_json();
    missing.as_object_mut().unwrap().remove("ownership");
    let err = UploadManifest::parse(serde_json::to_vec(&missing).unwrap().as_slice()).unwrap_err();
    assert!(matches!(err, Error::MissingField));

    let build = extra_build_field();
    let err = UploadManifest::parse(serde_json::to_vec(&build).unwrap().as_slice()).unwrap_err();
    assert!(matches!(err, Error::UnknownField));
}

fn extra_build_field() -> serde_json::Value {
    let mut json = sample_json();
    json["build_id"]["path"] = serde_json::json!(CANARY_PATH);
    json
}

#[test]
fn duplicate_json_keys_are_rejected() {
    let json = r#"{"schema_version":1,"schema_version":1,"build_id":{"game_id":"synthetic","version":"1.0.0","exe_hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"}}"#;
    assert!(matches!(
        UploadManifest::parse(json.as_bytes()),
        Err(Error::DuplicateKey)
    ));
}

#[test]
fn determinism_same_input_any_order_same_bytes() {
    let mut a = blank_record("aa");
    a.bytecode_hashes.vs = Some(sha_rec(2));
    a.root_signature = Some(sha_rec(8));
    let mut b = blank_record("bb");
    b.bytecode_hashes.ps = Some(sha_rec(1));
    b.input_layout = Some(sha_rec(7));
    let first = build_manifest(&record_file(vec![a.clone(), b.clone()]));
    let second = build_manifest(&record_file(vec![b, a]));
    let left = first.to_canonical_bytes().unwrap();
    let right = second.to_canonical_bytes().unwrap();
    assert_eq!(left, right);
    let again = first.to_canonical_bytes().unwrap();
    assert_eq!(left, again);
}

#[test]
fn timestamp_rounds_down_at_hour_boundaries() {
    let on_hour = UtcHour::from_system_time(
        UNIX_EPOCH + Duration::from_secs(unix_secs(2026, 10, 25, 14, 0, 0)),
    )
    .unwrap();
    assert_eq!(on_hour.as_str(), "2026-10-25T14:00:00Z");
    let last_second = UtcHour::from_system_time(
        UNIX_EPOCH + Duration::from_secs(unix_secs(2026, 10, 25, 14, 59, 59)),
    )
    .unwrap();
    assert_eq!(last_second.as_str(), "2026-10-25T14:00:00Z");
    let next = UtcHour::from_system_time(
        UNIX_EPOCH + Duration::from_secs(unix_secs(2026, 10, 25, 15, 0, 0)),
    )
    .unwrap();
    assert_eq!(next.as_str(), "2026-10-25T15:00:00Z");
    let dst = UtcHour::from_system_time(
        UNIX_EPOCH + Duration::from_secs(unix_secs(2026, 3, 8, 7, 30, 0)),
    )
    .unwrap();
    assert_eq!(dst.as_str(), "2026-03-08T07:00:00Z");
    assert!(UtcHour::parse("2026-10-25T14:37:19Z").is_err());
    assert!(UtcHour::parse(CANARY_FULL_TS).is_err());
}

#[test]
fn clock_injection_does_not_use_realtime() {
    let file = record_file(Vec::new());
    let manifest = ManifestBuilder::new(&file)
        .build_id(sample_build())
        .clock(&hour_clock(1970, 1, 1, 0, 59, 59))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(manifest.collected_hour.as_str(), "1970-01-01T00:00:00Z");
}

#[test]
fn preview_is_produced_from_canonical_bytes() {
    let file = {
        let mut rec = blank_record("01");
        rec.bytecode_hashes.vs = Some(sha_rec(1));
        rec.root_signature = Some(sha_rec(2));
        record_file(vec![rec])
    };
    let manifest = build_manifest(&file);
    let canonical = manifest.to_canonical_bytes().unwrap();
    let preview = UploadPreview::from_manifest(&manifest).unwrap();
    assert_eq!(preview.canonical_bytes(), canonical.as_slice());
    assert!(preview.as_text().contains(THESE_EXACT_BYTES_LINE));
    assert!(preview.as_text().contains(NOTHING_SENT_LINE));
    assert!(preview.as_text().contains(UNCHALLENGED_LINE));
    let json = std::str::from_utf8(canonical.as_slice()).unwrap();
    assert!(preview.as_text().contains(json));
    let value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let keys = field_paths(&value, "");
    let listed = preview_listed_paths(preview.as_text());
    assert_eq!(
        listed, keys,
        "preview field list must equal every sent JSON path"
    );
    assert!(listed.iter().any(|p| p == "build_id.exe_hash"));
    assert!(listed.iter().any(|p| p == "bytecode_hashes"));
    assert!(listed.iter().any(|p| p == "description_hashes"));
}

fn preview_listed_paths(text: &str) -> Vec<String> {
    let mut listed = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if rest.starts_with("Ownership") || rest.starts_with("Nothing") {
            continue;
        }
        let name = rest.split_once(':').map_or(rest, |(n, _)| n);
        let name = name.split_once(' ').map_or(name, |(n, _)| n);
        listed.push(name.to_string());
    }
    listed.sort();
    listed
}

fn field_paths(value: &serde_json::Value, prefix: &str) -> Vec<String> {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = Vec::new();
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match child {
                    serde_json::Value::Object(_) => out.extend(field_paths(child, &path)),
                    serde_json::Value::Array(_) => out.push(path),
                    _ => out.push(path),
                }
            }
            out.sort();
            out
        }
        _ => Vec::new(),
    }
}

#[test]
fn schema_file_matches_rust_document() {
    let from_file: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let from_rust = document();
    assert_eq!(from_file, from_rust);
    let mut pretty = serde_json::to_string_pretty(&from_rust).unwrap();
    pretty.push('\n');
    // Git on Windows may check the file out with CRLF; compare LF form.
    let checked_in = SCHEMA_JSON.replace("\r\n", "\n");
    assert_eq!(pretty, checked_in);
}

#[test]
fn schema_required_fields_match_serialized_manifest() {
    let manifest = build_manifest(&record_file(Vec::new()));
    let value = serde_json::to_value(&manifest).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, MANIFEST_FIELDS);
    assert_eq!(manifest.schema_version, SCHEMA_VERSION);
}

#[test]
fn exe_binding_v1_parses_but_is_not_built_by_default() {
    let mut json = sample_json();
    json["ownership"] = serde_json::json!({
        "kind": "exe_binding_v1",
        "challenge": hex64(10),
        "response": hex64(11)
    });
    let parsed = UploadManifest::parse(serde_json::to_vec(&json).unwrap().as_slice()).unwrap();
    assert_eq!(
        parsed.ownership,
        OwnershipProof::exe_binding_v1(sha_up(10), sha_up(11))
    );
    let built = build_manifest(&record_file(Vec::new()));
    assert_eq!(built.ownership, OwnershipProof::unchallenged());
}

#[test]
fn incomplete_build_id_is_rejected() {
    let file = record_file(Vec::new());
    let err = ManifestBuilder::new(&file).build().unwrap_err();
    assert!(matches!(err, Error::IncompleteBuildId));
    assert_eq!(err.to_string(), "build identity is incomplete");
}

#[test]
fn path_shaped_game_id_is_rejected() {
    assert!(matches!(
        GameId::parse(CANARY_PATH),
        Err(Error::InvalidBuildId)
    ));
    let err = GameId::parse(CANARY_PATH).unwrap_err();
    assert_no_canaries(&err.to_string());
}

#[test]
fn client_version_rejects_non_semver_digits() {
    assert!(ClientVersion::parse("0.1.0").is_ok());
    assert!(ClientVersion::parse("1.2").is_err());
    assert!(ClientVersion::parse("01.02.03-beta").is_err());
}

#[test]
fn app_keys_and_inline_descriptors_are_not_hashed() {
    let mut rec = blank_record(&canary_app_key_hex());
    rec.flags = Some(7);
    rec.sample_mask = Some(1);
    let manifest = build_manifest(&record_file(vec![rec]));
    assert!(manifest.description_hashes.is_empty());
    assert!(manifest.bytecode_hashes.is_empty());
    let json = String::from_utf8(manifest.to_canonical_bytes().unwrap()).unwrap();
    assert!(!json.contains(&canary_app_key_hex()));
}

#[test]
fn canonical_json_keys_are_sorted() {
    let json = String::from_utf8(
        build_manifest(&record_file(Vec::new()))
            .to_canonical_bytes()
            .unwrap(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert!(!json.contains(' '));
}

fn sample_json() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "build_id": {
            "game_id": "synthetic",
            "version": "1.0.0",
            "exe_hash": hex64(0xee)
        },
        "client_version": "0.1.0",
        "collected_hour": "2026-10-25T14:00:00Z",
        "description_hashes": [],
        "bytecode_hashes": [],
        "ownership": {
            "kind": "unchallenged",
            "reason": "no_server_challenge"
        }
    })
}

fn sample_canonical() -> Vec<u8> {
    let file = record_file(Vec::new());
    ManifestBuilder::new(&file)
        .build_id(sample_build())
        .client_version(ClientVersion::parse("0.1.0").unwrap())
        .collected_hour(UtcHour::parse("2026-10-25T14:00:00Z").unwrap())
        .build()
        .unwrap()
        .to_canonical_bytes()
        .unwrap()
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
        "isolated RSS child {qualified} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test"),
        "RSS child matched the wrong filter:\n{stdout}"
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "RSS child did not run exactly one test:\n{stdout}"
    );
    true
}

#[cfg(target_os = "linux")]
#[test]
fn builder_rss_stays_flat_with_duplicate_hashes() {
    if run_rss_isolated("builder_rss_stays_flat_with_duplicate_hashes") {
        return;
    }
    let mut recs = Vec::with_capacity(20_000);
    for i in 0..20_000_u32 {
        let mut rec = blank_record(&format!("{i:08x}"));
        rec.bytecode_hashes.vs = Some(sha_rec(1));
        rec.root_signature = Some(sha_rec(2));
        recs.push(rec);
    }
    let file = record_file(recs);
    let before = rss_bytes().expect("rss");
    let manifest = ManifestBuilder::new(&file)
        .build_id(sample_build())
        .clock(&hour_clock(2026, 10, 25, 14, 0, 0))
        .unwrap()
        .build()
        .unwrap();
    let after = rss_bytes().expect("rss");
    assert_eq!(manifest.bytecode_hashes.len(), 1);
    assert_eq!(manifest.description_hashes.len(), 1);
    let delta = after.saturating_sub(before);
    assert!(
        delta < 8 * 1024 * 1024,
        "builder RSS grew by {delta} bytes for duplicate hashes"
    );
}

#[test]
fn hashset_debug_prints_count_only() {
    let set = HashSet::from_unsorted(vec![sha_up(1), sha_up(2)], 10).unwrap();
    let text = format!("{set:?}");
    assert!(text.contains("len"));
    assert!(!text.contains(&hex64(1)));
}

fn record_file_with_build(
    records: Vec<PipelineStateRecord>,
    game_id: &str,
    version: &str,
    exe: u64,
) -> RecordFile {
    let mut file = record_file(records);
    file.build = BuildIdentity::new(
        Some(osdb_records::GameId::parse(game_id).unwrap()),
        Some(osdb_records::GameVersion::parse(version).unwrap()),
        Some(osdb_records::ExeHash::parse_hex(&hex64(exe)).unwrap()),
    );
    file
}

fn compact(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn with_bytecode_hashes(hashes: &[&str]) -> Vec<u8> {
    let mut json = sample_json();
    json["bytecode_hashes"] = hashes
        .iter()
        .copied()
        .map(serde_json::Value::from)
        .collect();
    compact(&json)
}

#[test]
fn default_caps_cover_a_full_record_file() {
    assert_eq!(BYTECODE_STAGES_PER_RECORD, 8);
    assert_eq!(DESCRIPTION_HASHES_PER_RECORD, 3);
    assert_eq!(
        DEFAULT_MAX_HASHES_PER_LIST,
        osdb_records::Limits::DEFAULT.max_records * BYTECODE_STAGES_PER_RECORD
    );
    assert_eq!(DEFAULT_MAX_HASHES_PER_LIST, 400_000);
    assert_eq!(DEFAULT_MAX_BYTECODE_HASHES, 400_000);
    assert_eq!(DEFAULT_MAX_DESCRIPTION_HASHES, 150_000);
    assert_eq!(
        DEFAULT_MAX_DESCRIPTION_HASHES,
        osdb_records::Limits::DEFAULT.max_records * DESCRIPTION_HASHES_PER_RECORD
    );
    assert_eq!(DEFAULT_MAX_JSON_BYTES, 64 * 1024 * 1024);
    assert_eq!(WIRE_MAX_JSON_BYTES, 8 * 1024 * 1024);
    assert_eq!(DEFAULT_PER_BUILD_UPLOAD_CAP, 64);
    assert_eq!(Limits::DEFAULT.max_bytecode_hashes, 400_000);
    assert_eq!(Limits::DEFAULT.max_description_hashes, 150_000);
    assert_eq!(Limits::DEFAULT.max_json_bytes, 64 * 1024 * 1024);
    let empty = sample_canonical().len() as u64;
    let with_eight = {
        let hashes: Vec<_> = (1..=8_u64).map(sha_up).collect();
        let mut manifest = build_manifest(&record_file(Vec::new()));
        manifest.bytecode_hashes = HashSet::from_unsorted(hashes, 8).unwrap();
        manifest
            .to_canonical_bytes_with_limits(Limits::DEFAULT)
            .unwrap()
            .len() as u64
    };
    let per_hash = with_eight.saturating_sub(empty).div_ceil(8);
    let worst = empty + 2 * DEFAULT_MAX_HASHES_PER_LIST * (per_hash + 1);
    assert!(
        worst <= DEFAULT_MAX_JSON_BYTES,
        "two full hash lists need {worst} bytes, cap is {DEFAULT_MAX_JSON_BYTES}"
    );
}

#[test]
fn shared_limits_json_matches_rust_constants() {
    let value: serde_json::Value = serde_json::from_str(LIMITS_JSON).unwrap();
    assert_eq!(
        value["local_max_json_bytes"].as_u64().unwrap(),
        DEFAULT_MAX_JSON_BYTES
    );
    assert_eq!(
        value["wire_max_json_bytes"].as_u64().unwrap(),
        WIRE_MAX_JSON_BYTES
    );
    assert_eq!(
        value["max_description_hashes"].as_u64().unwrap(),
        DEFAULT_MAX_DESCRIPTION_HASHES
    );
    assert_eq!(
        value["max_bytecode_hashes"].as_u64().unwrap(),
        DEFAULT_MAX_BYTECODE_HASHES
    );
    assert_eq!(value["max_json_nesting"].as_u64().unwrap(), 8);
    assert_eq!(
        value["max_number_token_chars"].as_u64().unwrap(),
        u64::try_from(MAX_NUMBER_TOKEN_CHARS).unwrap()
    );
    assert_eq!(
        value["per_build_upload_cap"].as_u64().unwrap(),
        DEFAULT_PER_BUILD_UPLOAD_CAP
    );
    assert_eq!(Limits::DEFAULT.max_json_nesting, 8);
    assert_eq!(Error::UnknownField.code(), "UNKNOWN_FIELD");
    assert_eq!(Error::ByteOrderMark.code(), "BYTE_ORDER_MARK");
}

#[test]
fn description_and_bytecode_caps_are_independent() {
    let limits = Limits {
        max_json_bytes: 64 * 1024 * 1024,
        max_description_hashes: 1,
        max_bytecode_hashes: 3,
        max_json_nesting: 8,
    };
    let mut json = sample_json();
    json["description_hashes"] = serde_json::json!([hex64(1), hex64(2)]);
    json["bytecode_hashes"] = serde_json::json!([hex64(1), hex64(2)]);
    let err = UploadManifest::parse_with_limits(&compact(&json), limits).unwrap_err();
    assert!(matches!(err, Error::TooManyHashes { max: 1, got: 2 }));
    json["description_hashes"] = serde_json::json!([hex64(1)]);
    UploadManifest::parse_with_limits(&compact(&json), limits).unwrap();
}

#[test]
fn hash_cap_accepts_max_and_refuses_max_plus_one() {
    let limits = Limits {
        max_json_bytes: 64 * 1024 * 1024,
        max_description_hashes: 2,
        max_bytecode_hashes: 2,
        max_json_nesting: 8,
    };
    let two = with_bytecode_hashes(&[&hex64(1), &hex64(2)]);
    UploadManifest::parse_with_limits(&two, limits).unwrap();
    let three = with_bytecode_hashes(&[&hex64(1), &hex64(2), &hex64(3)]);
    let err = UploadManifest::parse_with_limits(&three, limits).unwrap_err();
    assert!(matches!(err, Error::TooManyHashes { max: 2, got: 3 }));
    assert_eq!(err.to_string(), "hash count exceeds limit of 2 (got 3)");
    HashSet::from_sorted_unique(vec![sha_up(1), sha_up(2)], 2).unwrap();
    assert!(matches!(
        HashSet::from_sorted_unique(vec![sha_up(1), sha_up(2), sha_up(3)], 2),
        Err(Error::TooManyHashes { max: 2, got: 3 })
    ));
}

#[test]
fn builder_insert_cap_accepts_max_and_refuses_max_plus_one() {
    let limits = Limits {
        max_json_bytes: 64 * 1024 * 1024,
        max_description_hashes: 2,
        max_bytecode_hashes: 2,
        max_json_nesting: 8,
    };
    let mut two = Vec::new();
    for i in 0..2_u64 {
        let mut rec = blank_record(&format!("{i:02x}"));
        rec.bytecode_hashes.vs = Some(sha_rec(i + 1));
        two.push(rec);
    }
    ManifestBuilder::new(&record_file(two))
        .build_id(sample_build())
        .limits(limits)
        .clock(&hour_clock(2026, 10, 25, 14, 0, 0))
        .unwrap()
        .build()
        .unwrap();
    let mut three = Vec::new();
    for i in 0..3_u64 {
        let mut rec = blank_record(&format!("{i:02x}"));
        rec.bytecode_hashes.vs = Some(sha_rec(i + 1));
        three.push(rec);
    }
    let err = ManifestBuilder::new(&record_file(three))
        .build_id(sample_build())
        .limits(limits)
        .clock(&hour_clock(2026, 10, 25, 14, 0, 0))
        .unwrap()
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::TooManyHashes { max: 2, got: 3 }));
}

#[test]
fn parse_rejects_unsorted_and_duplicate_hashes() {
    let unsorted = with_bytecode_hashes(&[&hex64(2), &hex64(1)]);
    assert!(matches!(
        UploadManifest::parse(&unsorted),
        Err(Error::UnsortedOrDuplicateHashes)
    ));
    let dup = with_bytecode_hashes(&[&hex64(1), &hex64(1)]);
    assert!(matches!(
        UploadManifest::parse(&dup),
        Err(Error::UnsortedOrDuplicateHashes)
    ));
    let sorted = HashSet::from_unsorted(vec![sha_up(2), sha_up(1), sha_up(1)], 10).unwrap();
    assert_eq!(
        sorted
            .as_slice()
            .iter()
            .map(Sha256Hex::to_hex)
            .collect::<Vec<_>>(),
        vec![hex64(1), hex64(2)]
    );
}

#[test]
fn encode_enforces_json_size_cap() {
    let manifest = build_manifest(&record_file(Vec::new()));
    let bytes = manifest.to_canonical_bytes().unwrap();
    let tiny = Limits {
        max_json_bytes: u64::try_from(bytes.len()).unwrap().saturating_sub(1),
        max_description_hashes: 8,
        max_bytecode_hashes: 8,
        max_json_nesting: 8,
    };
    let err = manifest.to_canonical_bytes_with_limits(tiny).unwrap_err();
    assert!(matches!(err, Error::ManifestTooLarge { .. }));
}

#[test]
fn from_canonical_bytes_rejects_non_canonical() {
    let canonical = sample_canonical();
    UploadPreview::from_canonical_bytes(canonical.clone()).unwrap();
    let pretty = serde_json::to_string_pretty(
        &serde_json::from_slice::<serde_json::Value>(&canonical).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        UploadPreview::from_canonical_bytes(pretty.into_bytes()),
        Err(Error::NonCanonicalJson)
    ));
    let mut value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    value["bytecode_hashes"] = serde_json::json!([hex64(2), hex64(1)]);
    assert!(UploadPreview::from_canonical_bytes(compact(&value)).is_err());
}

#[test]
fn nested_duplicate_keys_are_rejected() {
    let exe = hex64(0xee);
    let build = format!(
        r#"{{"schema_version":1,"build_id":{{"game_id":"synthetic","game_id":"other","version":"1.0.0","exe_hash":"{exe}"}},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{{"kind":"unchallenged","reason":"no_server_challenge"}}}}"#
    );
    assert!(matches!(
        UploadManifest::parse(build.as_bytes()),
        Err(Error::DuplicateKey)
    ));
    let ownership = format!(
        r#"{{"schema_version":1,"build_id":{{"game_id":"synthetic","version":"1.0.0","exe_hash":"{exe}"}},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{{"kind":"unchallenged","kind":"unchallenged","reason":"no_server_challenge"}}}}"#
    );
    assert!(matches!(
        UploadManifest::parse(ownership.as_bytes()),
        Err(Error::DuplicateKey)
    ));
}

#[test]
fn ownership_fields_are_strict() {
    let mut unknown = sample_json();
    unknown["ownership"] = serde_json::json!({
        "kind": "unchallenged",
        "reason": "no_server_challenge",
        "extra": true
    });
    assert!(matches!(
        UploadManifest::parse(&compact(&unknown)),
        Err(Error::UnknownField)
    ));

    let mut challenged = sample_json();
    challenged["ownership"] = serde_json::json!({
        "kind": "unchallenged",
        "reason": "no_server_challenge",
        "challenge": hex64(1)
    });
    assert!(matches!(
        UploadManifest::parse(&compact(&challenged)),
        Err(Error::UnknownField)
    ));

    let mut bad_reason = sample_json();
    bad_reason["ownership"] = serde_json::json!({
        "kind": "unchallenged",
        "reason": "because"
    });
    let err = UploadManifest::parse(&compact(&bad_reason)).unwrap_err();
    assert!(matches!(err, Error::UnknownOwnershipReason));
    assert_eq!(err.to_string(), "unknown ownership reason");

    let mut extra_reason = sample_json();
    extra_reason["ownership"] = serde_json::json!({
        "kind": "exe_binding_v1",
        "challenge": hex64(10),
        "response": hex64(11),
        "reason": "no_server_challenge"
    });
    assert!(matches!(
        UploadManifest::parse(&compact(&extra_reason)),
        Err(Error::UnknownField)
    ));

    let mut bad_kind = sample_json();
    bad_kind["ownership"] = serde_json::json!({
        "kind": "not_a_kind",
        "reason": "no_server_challenge"
    });
    assert!(matches!(
        UploadManifest::parse(&compact(&bad_kind)),
        Err(Error::UnknownOwnershipKind)
    ));
}

#[test]
fn schema_version_must_be_one() {
    for raw in [
        r#"{"schema_version":0,"build_id":{"game_id":"synthetic","version":"1.0.0","exe_hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"}}"#,
        r#"{"schema_version":2,"build_id":{"game_id":"synthetic","version":"1.0.0","exe_hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"}}"#,
        r#"{"schema_version":"1","build_id":{"game_id":"synthetic","version":"1.0.0","exe_hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"}}"#,
        r#"{"schema_version":1.0,"build_id":{"game_id":"synthetic","version":"1.0.0","exe_hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"}}"#,
        r#"{"schema_version":-1,"build_id":{"game_id":"synthetic","version":"1.0.0","exe_hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},"client_version":"0.1.0","collected_hour":"2026-10-25T14:00:00Z","description_hashes":[],"bytecode_hashes":[],"ownership":{"kind":"unchallenged","reason":"no_server_challenge"}}"#,
    ] {
        let err = UploadManifest::parse(raw.as_bytes()).unwrap_err();
        assert!(
            matches!(
                err,
                Error::UnsupportedSchemaVersion { .. } | Error::InvalidInteger
            ),
            "unexpected {err:?} for {raw}"
        );
    }
    let mut ok = sample_json();
    ok["schema_version"] = serde_json::json!(1);
    UploadManifest::parse(&compact(&ok)).unwrap();
}

#[test]
fn utc_hour_range_leap_round_trip_and_pre_epoch() {
    assert!(UtcHour::from_ymd_h(1970, 1, 1, 0).is_ok());
    assert!(UtcHour::from_ymd_h(9999, 12, 31, 23).is_ok());
    assert!(matches!(
        UtcHour::from_ymd_h(1969, 12, 31, 23),
        Err(Error::TimestampOutOfRange)
    ));
    assert!(matches!(
        UtcHour::from_ymd_h(10_000, 1, 1, 0),
        Err(Error::TimestampOutOfRange)
    ));
    let hour = UtcHour::from_ymd_h(2026, 10, 25, 14).unwrap();
    let parsed = UtcHour::parse(hour.as_str()).unwrap();
    assert_eq!(parsed.as_str(), hour.as_str());
    assert_eq!(parsed.as_str(), "2026-10-25T14:00:00Z");
    assert!(UtcHour::parse("2000-02-29T00:00:00Z").is_ok());
    assert!(UtcHour::parse("2024-02-29T00:00:00Z").is_ok());
    assert!(UtcHour::parse("2026-10-25T14:00:00z").is_err());
    assert!(UtcHour::parse("2100-02-29T00:00:00Z").is_err());
    assert!(UtcHour::parse("1900-02-28T00:00:00Z").is_err());
    assert!(UtcHour::parse("1969-12-31T23:00:00Z").is_err());
    let pre = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        UtcHour::from_system_time(pre),
        Err(Error::TimestampOutOfRange)
    ));
}

#[test]
fn client_version_rejects_four_parts() {
    assert!(ClientVersion::parse("1.2.3").is_ok());
    assert!(matches!(
        ClientVersion::parse("1.2.3.4"),
        Err(Error::InvalidClientVersion)
    ));
}

#[test]
fn identifiers_reject_colon_and_overlong() {
    assert!(GameId::parse("synthetic").is_ok());
    assert!(matches!(
        GameId::parse("game:id"),
        Err(Error::InvalidBuildId)
    ));
    assert!(matches!(
        GameVersion::parse("1:0"),
        Err(Error::InvalidBuildId)
    ));
    let long = "a".repeat(129);
    assert!(matches!(GameId::parse(&long), Err(Error::InvalidBuildId)));
    assert!(GameId::parse(&"a".repeat(128)).is_ok());
}

#[test]
fn stream_out_and_later_stage_hashes_reach_the_manifest() {
    let mut rec = blank_record("01");
    rec.root_signature = Some(sha_rec(10));
    rec.input_layout = Some(sha_rec(11));
    rec.stream_out = Some(sha_rec(12));
    rec.bytecode_hashes.vs = Some(sha_rec(21));
    rec.bytecode_hashes.ps = Some(sha_rec(22));
    rec.bytecode_hashes.hs = Some(sha_rec(23));
    rec.bytecode_hashes.ds = Some(sha_rec(24));
    rec.bytecode_hashes.gs = Some(sha_rec(25));
    rec.bytecode_hashes.as_stage = Some(sha_rec(26));
    rec.bytecode_hashes.ms = Some(sha_rec(27));
    rec.bytecode_hashes.cs = Some(sha_rec(28));
    let manifest = build_manifest(&record_file(vec![rec]));
    let desc: Vec<_> = manifest
        .description_hashes
        .as_slice()
        .iter()
        .map(Sha256Hex::to_hex)
        .collect();
    let code: Vec<_> = manifest
        .bytecode_hashes
        .as_slice()
        .iter()
        .map(Sha256Hex::to_hex)
        .collect();
    assert_eq!(desc, vec![hex64(10), hex64(11), hex64(12)]);
    assert_eq!(
        code,
        vec![
            hex64(21),
            hex64(22),
            hex64(23),
            hex64(24),
            hex64(25),
            hex64(26),
            hex64(27),
            hex64(28)
        ]
    );
}

#[test]
fn builder_overrides_win_over_record_build_fields() {
    let file = record_file_with_build(Vec::new(), "from-records", "1.0.0", 0xee);
    let manifest = ManifestBuilder::new(&file)
        .game_id(GameId::parse("from-flag").unwrap())
        .version(GameVersion::parse("9.9.9").unwrap())
        .exe_hash(ExeHash::parse(&hex64(0xaa)).unwrap())
        .clock(&hour_clock(2026, 10, 25, 14, 0, 0))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(manifest.build_id.game_id.as_str(), "from-flag");
    assert_eq!(manifest.build_id.version.as_str(), "9.9.9");
    assert_eq!(manifest.build_id.exe_hash.to_hex(), hex64(0xaa));
}

#[test]
fn schema_and_rust_agree_on_generated_and_hostile_manifests() {
    let mut rec = blank_record("01");
    rec.stream_out = Some(sha_rec(4));
    rec.bytecode_hashes.ps = Some(sha_rec(5));
    let generated = build_manifest(&record_file(vec![rec]));
    let generated_bytes = generated.to_canonical_bytes().unwrap();
    let generated_value: serde_json::Value = serde_json::from_slice(&generated_bytes).unwrap();
    assert!(
        schema_accepts(&generated_value),
        "generated manifest must satisfy the checked-in schema"
    );
    UploadManifest::parse(&generated_bytes).unwrap();

    struct Case {
        name: &'static str,
        patch: fn(&mut serde_json::Value),
    }
    let cases = [
        Case {
            name: "colon in game_id",
            patch: |v| v["build_id"]["game_id"] = serde_json::json!("game:id"),
        },
        Case {
            name: "overlong game_id",
            patch: |v| v["build_id"]["game_id"] = serde_json::json!("a".repeat(129)),
        },
        Case {
            name: "uppercase hex",
            patch: |v| {
                v["bytecode_hashes"] = serde_json::json!([
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                ]);
            },
        },
        Case {
            name: "duplicate hashes",
            patch: |v| v["bytecode_hashes"] = serde_json::json!([hex64(1), hex64(1)]),
        },
        Case {
            name: "unknown ownership field",
            patch: |v| {
                v["ownership"] = serde_json::json!({
                    "kind": "unchallenged",
                    "reason": "no_server_challenge",
                    "extra": 1
                });
            },
        },
        Case {
            name: "unchallenged carrying challenge",
            patch: |v| {
                v["ownership"] = serde_json::json!({
                    "kind": "unchallenged",
                    "reason": "no_server_challenge",
                    "challenge": hex64(1)
                });
            },
        },
        Case {
            name: "bad ownership reason",
            patch: |v| {
                v["ownership"] = serde_json::json!({
                    "kind": "unchallenged",
                    "reason": "because"
                });
            },
        },
        Case {
            name: "exe_binding extra reason",
            patch: |v| {
                v["ownership"] = serde_json::json!({
                    "kind": "exe_binding_v1",
                    "challenge": hex64(1),
                    "response": hex64(2),
                    "reason": "no_server_challenge"
                });
            },
        },
        Case {
            name: "four-part client_version",
            patch: |v| v["client_version"] = serde_json::json!("1.2.3.4"),
        },
        Case {
            name: "schema_version 2",
            patch: |v| v["schema_version"] = serde_json::json!(2),
        },
    ];
    for case in cases {
        let mut value = sample_json();
        (case.patch)(&mut value);
        let rust_err = UploadManifest::parse(&compact(&value)).unwrap_err();
        assert!(
            !schema_accepts(&value),
            "schema accepted {} which Rust rejected as {rust_err:?}",
            case.name
        );
    }
}

fn schema_accepts(value: &serde_json::Value) -> bool {
    let schema = document();
    validate_schema_node(&schema, value)
}

fn validate_schema_node(schema: &serde_json::Value, value: &serde_json::Value) -> bool {
    if let Some(const_v) = schema.get("const") {
        if value != const_v {
            return false;
        }
    }
    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => {
            let Some(obj) = value.as_object() else {
                return false;
            };
            if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
                for key in required {
                    let Some(name) = key.as_str() else {
                        return false;
                    };
                    if !obj.contains_key(name) {
                        return false;
                    }
                }
            }
            let props = schema
                .get("properties")
                .and_then(|p| p.as_object())
                .cloned()
                .unwrap_or_default();
            if schema.get("additionalProperties") == Some(&serde_json::json!(false)) {
                for key in obj.keys() {
                    if !props.contains_key(key) {
                        return false;
                    }
                }
            }
            for (key, child) in obj {
                if let Some(child_schema) = props.get(key) {
                    if !validate_schema_node(child_schema, child) {
                        return false;
                    }
                }
            }
            true
        }
        Some("array") => {
            let Some(items) = value.as_array() else {
                return false;
            };
            if let Some(max) = schema.get("maxItems").and_then(serde_json::Value::as_u64) {
                if u64::try_from(items.len()).unwrap_or(u64::MAX) > max {
                    return false;
                }
            }
            if schema.get("uniqueItems") == Some(&serde_json::json!(true)) {
                for (i, item) in items.iter().enumerate() {
                    if items[..i].contains(item) {
                        return false;
                    }
                }
            }
            if let Some(item_schema) = schema.get("items") {
                items
                    .iter()
                    .all(|item| validate_schema_node(item_schema, item))
            } else {
                true
            }
        }
        Some("string") => {
            let Some(text) = value.as_str() else {
                return false;
            };
            if let Some(min) = schema.get("minLength").and_then(serde_json::Value::as_u64) {
                if u64::try_from(text.len()).unwrap_or(u64::MAX) < min {
                    return false;
                }
            }
            if let Some(max) = schema.get("maxLength").and_then(serde_json::Value::as_u64) {
                if u64::try_from(text.len()).unwrap_or(u64::MAX) > max {
                    return false;
                }
            }
            if let Some(pattern) = schema.get("pattern").and_then(serde_json::Value::as_str) {
                string_matches_schema_pattern(pattern, text)
            } else {
                true
            }
        }
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => {
            if let Some(one_of) = schema.get("oneOf").and_then(|v| v.as_array()) {
                one_of
                    .iter()
                    .filter(|alt| validate_schema_node(alt, value))
                    .count()
                    == 1
            } else {
                true
            }
        }
    }
}

fn string_matches_schema_pattern(pattern: &str, text: &str) -> bool {
    match pattern {
        crate::schema::SHA256_HEX_PATTERN => {
            text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }
        crate::schema::UTC_HOUR_PATTERN => {
            let b = text.as_bytes();
            b.len() == 20
                && b[4] == b'-'
                && b[7] == b'-'
                && b[10] == b'T'
                && b[13] == b':'
                && &b[14..16] == b"00"
                && b[16] == b':'
                && &b[17..19] == b"00"
                && b[19] == b'Z'
                && b[0..4].iter().all(u8::is_ascii_digit)
                && b[5..7].iter().all(u8::is_ascii_digit)
                && b[8..10].iter().all(u8::is_ascii_digit)
                && b[11..13].iter().all(u8::is_ascii_digit)
        }
        crate::schema::CLIENT_VERSION_PATTERN => {
            let mut parts = text.split('.');
            let triple = [parts.next(), parts.next(), parts.next()];
            parts.next().is_none()
                && triple.iter().all(|part| {
                    part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                })
        }
        crate::schema::IDENT_PATTERN => {
            text != "."
                && text != ".."
                && (1..=128).contains(&text.len())
                && text.bytes().all(|b| {
                    (0x21..=0x2e).contains(&b)
                        || (0x30..=0x39).contains(&b)
                        || (0x3b..=0x5b).contains(&b)
                        || (0x5d..=0x7e).contains(&b)
                })
        }
        _ => false,
    }
}
