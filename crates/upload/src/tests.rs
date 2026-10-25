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
use crate::{Error, Limits};

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
    let preview = UploadPreview::from_canonical_bytes(bytes.clone()).unwrap();
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
        max_hashes_per_list: 2,
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
    for line in preview.as_text().lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let name = rest.split_once(':').map_or(rest, |(n, _)| n);
        let name = name.split_once(' ').map_or(name, |(n, _)| n);
        assert!(
            keys.iter().any(|k| k == name),
            "preview field {name:?} is not in the manifest keys {keys:?}"
        );
    }
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
                    _ => out.push(path),
                }
            }
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
