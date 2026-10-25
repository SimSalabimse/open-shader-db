// SPDX-License-Identifier: MIT

//! Rebuild tests. Fixtures are synthetic (CC0 / fake DXBC bytes).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::sync::Mutex;
use std::time::{Duration, Instant};

use osdb_records::{extract, extract_with_sodb_limits, BuildIdentity, HexBytes};
use osdb_shaders::{scan_dir, scan_sodb, Limits as ScanLimits};
use rusqlite::Connection;
use sodb::{
    empty_root_signature_v1_0, hash_shader_container, one_sampler_border4_v1_0,
    parse_root_signature, SODB_APPLICATION_ID, SODB_USER_VERSION,
};

use crate::{
    inject_fault, rebuild_sodb, Error, InjectedFault, RebuildOptions, HARD_MAX_OUTPUT_BYTES,
    LOCAL_USE_NOTICE,
};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static RSS_MEASURE_LOCK: Mutex<()> = Mutex::new(());

struct TempPath {
    path: PathBuf,
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn temp_path(suffix: &str) -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("osdb-rebuild-{}-{n}{suffix}", std::process::id()))
}

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

fn create_sodb(setup: impl FnOnce(&mut Connection) -> rusqlite::Result<()>) -> TempPath {
    let path = temp_path(".sodb");
    let mut conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", SODB_USER_VERSION)
        .unwrap();
    conn.pragma_update(None, "application_id", SODB_APPLICATION_ID)
        .unwrap();
    setup(&mut conn).unwrap();
    drop(conn);
    TempPath { path }
}

fn rich_fixture() -> TempPath {
    let vs = dxbc_of_len(64, 0x11);
    let ps = dxbc_of_len(64, 0x22);
    let rs = empty_root_signature_v1_0();
    create_sodb(move |conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Type TEXT, Bytecode BLOB);
            CREATE TABLE root_signatures (Key BLOB NOT NULL PRIMARY KEY, value BLOB NOT NULL);
            CREATE TABLE rasterizer_descs (
                Key BLOB NOT NULL PRIMARY KEY, FillMode INTEGER NOT NULL, CullMode INTEGER NOT NULL,
                FrontCounterClockwise INTEGER NOT NULL, DepthBias REAL NOT NULL, DepthBiasClamp REAL NOT NULL,
                SlopeScaledDepthBias REAL NOT NULL, DepthClipEnable INTEGER NOT NULL,
                LineRasterizationMode INTEGER NOT NULL, ForcedSampleCount INTEGER NOT NULL,
                ConservativeRaster INTEGER NOT NULL
            );
            CREATE TABLE depth_stencil_op_descs (
                Key BLOB NOT NULL PRIMARY KEY, StencilFailOp INTEGER NOT NULL, StencilDepthFailOp INTEGER NOT NULL,
                StencilPassOp INTEGER NOT NULL, StencilFunc INTEGER NOT NULL, StencilReadMask INTEGER NOT NULL,
                StencilWriteMask INTEGER NOT NULL
            );
            CREATE TABLE depth_stencil_descs (
                Key BLOB NOT NULL PRIMARY KEY, DepthEnable INTEGER NOT NULL, DepthWriteMask INTEGER NOT NULL,
                DepthFunc INTEGER NOT NULL, StencilEnable INTEGER NOT NULL, FrontFace BLOB, BackFace BLOB,
                DepthBoundsTestEnable INTEGER NOT NULL
            );
            CREATE TABLE render_target_blend_descs (
                Key BLOB NOT NULL PRIMARY KEY, BlendEnable INTEGER NOT NULL, LogicOpEnable INTEGER NOT NULL,
                SrcBlend INTEGER NOT NULL, DestBlend INTEGER NOT NULL, BlendOp INTEGER NOT NULL,
                SrcBlendAlpha INTEGER NOT NULL, DestBlendAlpha INTEGER NOT NULL, BlendOpAlpha INTEGER NOT NULL,
                LogicOp INTEGER NOT NULL, RenderTargetWriteMask INTEGER NOT NULL
            );
            CREATE TABLE blend_descs (
                Key BLOB NOT NULL PRIMARY KEY, AlphaToCoverageEnable INTEGER NOT NULL,
                IndependentBlendEnable INTEGER NOT NULL, RenderTarget0 BLOB, RenderTarget1 BLOB,
                RenderTarget2 BLOB, RenderTarget3 BLOB, RenderTarget4 BLOB, RenderTarget5 BLOB,
                RenderTarget6 BLOB, RenderTarget7 BLOB
            );
            CREATE TABLE render_target_formats (
                Key BLOB NOT NULL PRIMARY KEY, RTFormat0 INTEGER NOT NULL, RTFormat1 INTEGER NOT NULL,
                RTFormat2 INTEGER NOT NULL, RTFormat3 INTEGER NOT NULL, RTFormat4 INTEGER NOT NULL,
                RTFormat5 INTEGER NOT NULL, RTFormat6 INTEGER NOT NULL, RTFormat7 INTEGER NOT NULL,
                NumRenderTargets INTEGER NOT NULL
            );
            CREATE TABLE input_element_descs (
                Key BLOB NOT NULL PRIMARY KEY, SemanticName TEXT NOT NULL, SemanticIndex INTEGER NOT NULL,
                Format INTEGER NOT NULL, InputSlot INTEGER NOT NULL, AlignedByteOffset INTEGER NOT NULL,
                InputSlotClass INTEGER NOT NULL, InstanceDataStepRate INTEGER NOT NULL
            );
            CREATE TABLE input_layout_to_input_element_associations (
                InputLayoutKey BLOB NOT NULL, InputElementKey BLOB NOT NULL
            );
            CREATE TABLE pipeline_states (
                Key BLOB NOT NULL PRIMARY KEY, RootSignature BLOB, InputLayout BLOB,
                ByteCode_VS BLOB, ByteCode_PS BLOB, DepthStencilDesc BLOB, BlendDesc BLOB,
                RasterizerDesc BLOB, RenderTargetFormats BLOB, SampleDesc_Count INTEGER,
                SampleMask INTEGER, Flags INTEGER
            );
            CREATE TABLE groups (Key BLOB NOT NULL PRIMARY KEY, Version INTEGER NOT NULL, PSOKey BLOB, SOKey BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, 'vs', ?2)",
            rusqlite::params![b"bc-vs", &vs],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, 'ps', ?2)",
            rusqlite::params![b"bc-ps", &ps],
        )?;
        conn.execute(
            "INSERT INTO root_signatures VALUES (?1, ?2)",
            rusqlite::params![b"rs-1", &rs],
        )?;
        conn.execute(
            "INSERT INTO rasterizer_descs VALUES (?1, 3, 1, 0, 0.0, 0.0, 0.0, 1, 0, 0, 0)",
            rusqlite::params![b"rast-1"],
        )?;
        conn.execute(
            "INSERT INTO depth_stencil_op_descs VALUES (?1, 1, 1, 1, 1, 255, 255)",
            rusqlite::params![b"dsop-1"],
        )?;
        conn.execute(
            "INSERT INTO depth_stencil_descs VALUES (?1, 1, 1, 2, 0, ?2, ?2, 0)",
            rusqlite::params![b"ds-1", b"dsop-1"],
        )?;
        conn.execute(
            "INSERT INTO render_target_blend_descs VALUES (?1, 0, 0, 1, 1, 1, 1, 1, 1, 0, 15)",
            rusqlite::params![b"rtb-1"],
        )?;
        conn.execute(
            "INSERT INTO blend_descs VALUES (?1, 0, 0, ?2, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            rusqlite::params![b"blend-1", b"rtb-1"],
        )?;
        conn.execute(
            "INSERT INTO render_target_formats VALUES (?1, 28, 0, 0, 0, 0, 0, 0, 0, 1)",
            rusqlite::params![b"rtf-1"],
        )?;
        conn.execute(
            "INSERT INTO input_element_descs VALUES (?1, 'POSITION', 0, 6, 0, 0, 0, 0)",
            rusqlite::params![b"ie-1"],
        )?;
        conn.execute(
            "INSERT INTO input_layout_to_input_element_associations VALUES (?1, ?2)",
            rusqlite::params![b"il-1", b"ie-1"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (
                Key, RootSignature, InputLayout, ByteCode_VS, ByteCode_PS, DepthStencilDesc,
                BlendDesc, RasterizerDesc, RenderTargetFormats, SampleDesc_Count, SampleMask, Flags
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 4294967295, 0)",
            rusqlite::params![
                b"pso-1", b"rs-1", b"il-1", b"bc-vs", b"bc-ps", b"ds-1", b"blend-1", b"rast-1",
                b"rtf-1"
            ],
        )?;
        conn.execute(
            "INSERT INTO groups VALUES (?1, 3, ?2, NULL)",
            rusqlite::params![b"grp-1", b"pso-1"],
        )?;
        Ok(())
    })
}

fn opts() -> RebuildOptions {
    RebuildOptions::default()
}

fn leftovers_for(dest: &Path) -> Vec<PathBuf> {
    let Some(parent) = dest.parent() else {
        return Vec::new();
    };
    let Some(name) = dest.file_name() else {
        return Vec::new();
    };
    let needle = format!(".{}.osdb-rebuild-", name.to_string_lossy());
    match fs::read_dir(parent) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(&needle))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn assert_no_leftovers(dest: &Path) {
    let left = leftovers_for(dest);
    assert!(left.is_empty(), "leftover rebuild siblings: {left:?}");
}

struct FaultGuard;

impl Drop for FaultGuard {
    fn drop(&mut self) {
        inject_fault(None);
    }
}

fn distinct_dxbc(i: u32, size: usize) -> Vec<u8> {
    let mut bytes = dxbc_of_len(size, 1);
    let tag = i.to_le_bytes();
    let start = 32;
    bytes[start..start + 4].copy_from_slice(&tag);
    bytes
}

fn many_distinct_sodb(count: u32, size: usize) -> TempPath {
    create_sodb(move |conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_bc = tx.prepare("INSERT INTO shader_bytecode VALUES (?1, ?2)")?;
            let mut ins_pso = tx.prepare("INSERT INTO pipeline_states VALUES (?1, ?2)")?;
            for i in 0..count {
                let key = i.to_le_bytes();
                let vs = distinct_dxbc(i, size);
                ins_bc.execute(rusqlite::params![&key, &vs])?;
                ins_pso.execute(rusqlite::params![&key, &key])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
}

#[test]
fn notice_is_local_only() {
    assert!(LOCAL_USE_NOTICE.contains("local use only"));
    assert!(LOCAL_USE_NOTICE.contains("must not be shared"));
}

#[test]
fn round_trip_extract_scan_rebuild_extract() {
    let fixture = rich_fixture();
    let original = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let out = temp_path("-rebuilt.sodb");
    let report = rebuild_sodb(&original, &index, &out, opts()).unwrap();
    assert_eq!(report.rebuilt, 1);
    assert_eq!(report.skipped_missing_stage, 0);
    let rebuilt = extract(&out, BuildIdentity::empty()).unwrap();
    assert_eq!(rebuilt.records, original.records);
    assert_eq!(rebuilt.root_signatures, original.root_signatures);
    assert_eq!(rebuilt.input_layouts, original.input_layouts);
    let _ = fs::remove_file(&out);
}

#[test]
fn rebuild_with_record_input_path_matches_cli() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let rec_path = temp_path("-records.json");
    fs::write(&rec_path, b"{}").unwrap();
    let mut options = opts();
    options.record_input = Some(rec_path.clone());
    let out = temp_path("-cli-shaped.sodb");
    rebuild_sodb(&records, &index, &out, options).unwrap();
    assert!(out.exists());
    let _ = fs::remove_file(&out);
    let _ = fs::remove_file(&rec_path);
}

#[test]
fn rebuild_is_byte_identical() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let a = temp_path("-a.sodb");
    let b = temp_path("-b.sodb");
    rebuild_sodb(&records, &index, &a, opts()).unwrap();
    rebuild_sodb(&records, &index, &b, opts()).unwrap();
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
}

#[test]
fn skipped_records_match_report() {
    let vs = dxbc_of_len(32, 1);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE root_signatures (Key BLOB NOT NULL PRIMARY KEY, value BLOB);
            CREATE TABLE pipeline_states (
                Key BLOB NOT NULL PRIMARY KEY, RootSignature BLOB, ByteCode_VS BLOB, ByteCode_PS BLOB
            );
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, ?2)",
            rusqlite::params![b"bc", &vs],
        )?;
        conn.execute(
            "INSERT INTO root_signatures VALUES (?1, x'0100000044454144')",
            rusqlite::params![b"bad-rs"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states VALUES (?1, NULL, ?2, NULL)",
            rusqlite::params![b"ok", b"bc"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states VALUES (?1, NULL, ?2, ?3)",
            rusqlite::params![b"missing", b"bc", b"gone"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states VALUES (?1, ?2, ?3, NULL)",
            rusqlite::params![b"skiprs", b"bad-rs", b"bc"],
        )?;
        Ok(())
    });
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let out = temp_path("-skip.sodb");
    let report = rebuild_sodb(&records, &index, &out, opts()).unwrap();
    assert_eq!(report.rebuilt, 1);
    assert_eq!(report.skipped_unresolved_stages, 1);
    assert_eq!(report.skipped_root_signature, 1);
    let rebuilt = extract(&out, BuildIdentity::empty()).unwrap();
    assert_eq!(rebuilt.records.len(), 1);
    assert_eq!(rebuilt.records[0].app_key.as_str(), "6f6b"); // "ok"
    let _ = fs::remove_file(&out);
}

#[test]
fn mismatched_hash_is_not_rebuilt() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let dir = temp_path("-empty-dir");
    fs::create_dir_all(&dir).unwrap();
    let (index, _) = scan_dir(&dir, ScanLimits::DEFAULT).unwrap();
    let out = temp_path("-miss.sodb");
    let report = rebuild_sodb(&records, &index, &out, opts()).unwrap();
    assert_eq!(report.rebuilt, 0);
    assert_eq!(report.skipped_missing_stage, 1);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&out);
}

#[test]
fn fanout_shares_one_shader() {
    let vs = dxbc_of_len(32, 3);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, ?2)",
            rusqlite::params![b"bc", &vs],
        )?;
        for i in 0_u16..200 {
            let key = i.to_le_bytes();
            conn.execute(
                "INSERT INTO pipeline_states VALUES (?1, ?2)",
                rusqlite::params![&key, b"bc"],
            )?;
        }
        Ok(())
    });
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let out = temp_path("-fanout.sodb");
    let report = rebuild_sodb(&records, &index, &out, opts()).unwrap();
    assert_eq!(report.rebuilt, 200);
    assert_eq!(report.unique_shaders, 1);
    let rebuilt = extract(&out, BuildIdentity::empty()).unwrap();
    assert_eq!(rebuilt.records.len(), 200);
    let _ = fs::remove_file(&out);
}

#[test]
fn one_hundred_thousand_states_share_one_shader() {
    let vs = dxbc_of_len(32, 4);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, ?2)",
            rusqlite::params![b"bc", &vs],
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare("INSERT INTO pipeline_states VALUES (?1, ?2)")?;
            for i in 0_u32..100_000 {
                ins.execute(rusqlite::params![&i.to_le_bytes(), b"bc"])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let sodb_limits = sodb::Limits {
        max_rows_per_table: 100_000,
        max_total_rows: 200_000,
        ..sodb::Limits::DEFAULT
    };
    let records =
        extract_with_sodb_limits(&fixture.path, BuildIdentity::empty(), sodb_limits).unwrap();
    assert_eq!(records.records.len(), 100_000);
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let out = temp_path("-100k.sodb");
    let report = rebuild_sodb(&records, &index, &out, opts()).unwrap();
    assert_eq!(report.rebuilt, 100_000);
    assert_eq!(report.unique_shaders, 1);
    let _ = fs::remove_file(&out);
}

#[test]
fn refuses_same_file_as_input() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let err = rebuild_sodb(&records, &index, &fixture.path, opts()).unwrap_err();
    assert!(matches!(err, Error::SameAsInput));
}

#[test]
fn refuses_existing_without_force() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-exists.sodb");
    fs::write(&dest, b"keep").unwrap();
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::AlreadyExists));
    assert_eq!(fs::read(&dest).unwrap(), b"keep");
    let mut forced = opts();
    forced.force = true;
    rebuild_sodb(&records, &index, &dest, forced).unwrap();
    assert_ne!(fs::read(&dest).unwrap(), b"keep");
    let _ = fs::remove_file(&dest);
}

#[test]
fn output_cap_near_miss() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-cap.sodb");
    let mut tiny = opts();
    tiny.max_output_bytes = 16;
    let err = rebuild_sodb(&records, &index, &dest, tiny).unwrap_err();
    assert!(matches!(err, Error::ChargedPayloadTooLarge { max: 16, .. }));
    assert!(!dest.exists());
    assert_no_leftovers(&dest);
    let mut ok = opts();
    ok.max_output_bytes = 8 * 1024 * 1024;
    rebuild_sodb(&records, &index, &dest, ok).unwrap();
    assert!(dest.exists());
    let _ = fs::remove_file(&dest);
}

#[test]
fn rebuilt_opens_with_inspect_and_coverage() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-inspect.sodb");
    rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    let summary = sodb::open(&dest).unwrap();
    assert_eq!(summary.user_version, SODB_USER_VERSION);
    assert_eq!(summary.application_id, SODB_APPLICATION_ID);
    assert_eq!(summary.pipeline_states.len(), 1);
    assert_eq!(summary.shader_bytecode.len(), 2);
    let _ = fs::remove_file(&dest);
}

#[test]
fn cyclic_sodb_is_rejected_on_scan() {
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE state_objects (
                Key BLOB NOT NULL PRIMARY KEY, Type INTEGER, NodeMask INTEGER, Flags INTEGER,
                AddToStateObjectParent BLOB
            );
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            INSERT INTO state_objects (Key, Type, AddToStateObjectParent) VALUES (x'01', 1, x'02');
            INSERT INTO state_objects (Key, Type, AddToStateObjectParent) VALUES (x'02', 1, x'01');
            ",
        )?;
        Ok(())
    });
    let err = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap_err();
    let text = format!("{err}");
    assert!(
        text.contains("cycle")
            || text.contains("Cycle")
            || matches!(err, osdb_shaders::Error::Sodb(_))
    );
}

#[test]
fn report_has_counts_only() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-rep.sodb");
    let report = rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    let json = serde_json::to_string(&report).unwrap();
    let human = report.format_human();
    assert!(!json.contains(dest.to_string_lossy().as_ref()));
    assert!(!human.contains(dest.to_string_lossy().as_ref()));
    assert!(json.contains("rebuilt"));
    let _ = fs::remove_file(&dest);
}

#[test]
fn errors_omit_paths_and_bytes() {
    let err = Error::SameAsInput;
    let text = format!("{err}");
    assert!(!text.contains("/"));
    assert!(!text.contains("DXBC"));
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
    let filter = format!("tests::{test_name}");
    let output = std::process::Command::new(exe)
        .args([&filter, "--exact", "--nocapture"])
        .env("OSDB_RSS_CHILD", "1")
        .env("RUST_TEST_THREADS", "1")
        .output()
        .expect("spawn rss child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated RSS child {filter} failed: {stderr}{stdout}"
    );
    assert!(
        stdout.contains("running 1 test"),
        "RSS child must run one test, got stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("RSS_CHILD_RAN") || stderr.contains("RSS_CHILD_RAN"),
        "RSS child did not print RSS_CHILD_RAN: stdout={stdout} stderr={stderr}"
    );
    true
}

#[cfg(target_os = "linux")]
fn dest_family_bytes(dest: &Path) -> u64 {
    let Some(parent) = dest.parent() else {
        return 0;
    };
    match fs::read_dir(parent) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
            .sum(),
        Err(_) => 0,
    }
}

#[cfg(target_os = "linux")]
#[test]
fn rebuild_fanout_rss_stays_bounded() {
    if run_rss_isolated("rebuild_fanout_rss_stays_bounded") {
        return;
    }
    eprintln!("RSS_CHILD_RAN");
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let vs = dxbc_of_len(32, 8);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, ?2)",
            rusqlite::params![b"bc", &vs],
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare("INSERT INTO pipeline_states VALUES (?1, ?2)")?;
            for i in 0_u16..2_000 {
                ins.execute(rusqlite::params![&i.to_le_bytes(), b"bc"])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-rss.sodb");
    let before = rss_bytes();
    let report = rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    let after = rss_bytes();
    assert_eq!(report.unique_shaders, 1);
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "2000-state shared-shader rebuild RSS delta: {} bytes ({:.1} MiB)",
            delta,
            delta as f64 / (1024.0 * 1024.0)
        );
        assert!(
            delta < 96 * 1024 * 1024,
            "RSS grew by {delta} bytes; per-row shader copies?"
        );
    }
    let _ = fs::remove_file(&dest);
}

#[test]
fn rebuild_from_sodb_with_many_distinct_shaders_is_linear() {
    const COUNT: u32 = 3_000;
    let fixture = many_distinct_sodb(COUNT, 64);
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-3k.sodb");
    let start = Instant::now();
    let report = rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(report.rebuilt, u64::from(COUNT));
    assert_eq!(report.unique_shaders, u64::from(COUNT));
    assert!(
        elapsed < Duration::from_secs(25),
        "rebuild of {COUNT} distinct SODB shaders took {elapsed:?}; still quadratic?"
    );
    assert_no_leftovers(&dest);
    let _ = fs::remove_file(&dest);
}

#[test]
fn persist_fault_keeps_existing_output() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-keep.sodb");
    fs::write(&dest, b"previous-output").unwrap();
    let mut forced = opts();
    forced.force = true;
    inject_fault(Some(InjectedFault::Persist));
    let err = rebuild_sodb(&records, &index, &dest, forced).unwrap_err();
    assert!(matches!(err, Error::Io | Error::Sqlite));
    assert_eq!(fs::read(&dest).unwrap(), b"previous-output");
    assert_no_leftovers(&dest);
}

#[test]
fn vacuum_into_fault_leaves_dest_absent() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-vac.sodb");
    inject_fault(Some(InjectedFault::VacuumInto));
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::Sqlite));
    assert!(!dest.exists());
    assert_no_leftovers(&dest);
}

#[test]
fn rename_fault_copy_fallback_is_byte_identical() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let control = temp_path("-copy-ctrl.sodb");
    rebuild_sodb(&records, &index, &control, opts()).unwrap();
    let dest = temp_path("-copy.sodb");
    inject_fault(Some(InjectedFault::Rename));
    rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), fs::read(&control).unwrap());
    assert_no_leftovers(&dest);
    let _ = fs::remove_file(&dest);
    let _ = fs::remove_file(&control);
}

#[test]
fn output_cap_after_compact_is_typed_and_clean() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let measure = temp_path("-cap-measure.sodb");
    rebuild_sodb(&records, &index, &measure, opts()).unwrap();
    let compact = fs::metadata(&measure).unwrap().len();
    let _ = fs::remove_file(&measure);
    assert!(
        compact > 1024,
        "expected compacted file to exceed charged blobs"
    );
    let dest = temp_path("-cap-final.sodb");
    let mut mid = opts();
    mid.max_output_bytes = compact.saturating_sub(1);
    let err = rebuild_sodb(&records, &index, &dest, mid).unwrap_err();
    assert!(matches!(
        err,
        Error::OutputTooLarge {
            max,
            got
        } if max == compact.saturating_sub(1) && got == compact
    ));
    assert!(!dest.exists());
    assert_no_leftovers(&dest);
}

#[test]
fn refuses_output_equal_to_records_file() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let rec = temp_path("-records.json");
    fs::write(&rec, b"{}").unwrap();
    let mut options = opts();
    options.record_input = Some(rec.clone());
    let err = rebuild_sodb(&records, &index, &rec, options).unwrap_err();
    assert!(matches!(err, Error::SameAsInput));
    assert_eq!(fs::read(&rec).unwrap(), b"{}");
    let _ = fs::remove_file(&rec);
}

#[cfg(unix)]
#[test]
fn refuses_output_symlink() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let target = temp_path("-link-target.sodb");
    fs::write(&target, b"keep").unwrap();
    let dest = temp_path("-link.sodb");
    std::os::unix::fs::symlink(&target, &dest).unwrap();
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::NotRegular));
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    let _ = fs::remove_file(&dest);
    let _ = fs::remove_file(&target);
}

#[test]
fn refuses_output_inside_scan_root() {
    let dir = temp_path("-scan-root");
    fs::create_dir_all(&dir).unwrap();
    let vs = dxbc_of_len(32, 5);
    fs::write(dir.join("a.dxbc"), &vs).unwrap();
    let (index, _) = scan_dir(&dir, ScanLimits::DEFAULT).unwrap();
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let dest = dir.join("out.sodb");
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::InsideScanRoot));
    assert!(!dest.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_output_parent_is_typed() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-no-parent").join("out.sodb");
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::OutputParentMissing));
}

#[cfg(unix)]
#[test]
fn unreadable_output_parent_is_io() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dir = temp_path("-denied");
    fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("out.sodb");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o755));
    assert!(matches!(err, Error::Io));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn non_utf8_output_path_is_typed() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dir = temp_path("-utf8-dir");
    fs::create_dir_all(&dir).unwrap();
    let dest = dir.join(std::ffi::OsString::from_vec(vec![b'o', 0xff, b'.', b's']));
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    let _ = fs::remove_dir_all(&dir);
    assert!(matches!(err, Error::OutputPathNotUtf8));
}

#[test]
fn hard_max_output_bytes_is_rejected() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-hard.sodb");
    let mut huge = opts();
    huge.max_output_bytes = HARD_MAX_OUTPUT_BYTES.saturating_add(1);
    let err = rebuild_sodb(&records, &index, &dest, huge).unwrap_err();
    assert!(matches!(
        err,
        Error::MaxOutputBytesTooLarge {
            max: HARD_MAX_OUTPUT_BYTES,
            ..
        }
    ));
    assert!(!dest.exists());
}

#[test]
fn planted_temp_sibling_is_not_written_through() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-planted.sodb");
    let seq = crate::write::current_temp_seq();
    let planted = crate::write::predicted_sibling(&dest, seq);
    fs::write(&planted, b"planted-sibling").unwrap();
    rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    assert_eq!(fs::read(&planted).unwrap(), b"planted-sibling");
    assert!(dest.exists());
    let _ = fs::remove_file(&planted);
    assert_no_leftovers(&dest);
    let _ = fs::remove_file(&dest);
}

#[test]
fn noncanonical_root_signature_is_skipped_not_copied() {
    let vs = dxbc_of_len(32, 6);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB);
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, ?2)",
            rusqlite::params![b"bc", &vs],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states VALUES (?1, ?2)",
            rusqlite::params![b"ok", b"bc"],
        )?;
        Ok(())
    });
    let mut records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let mut raw = one_sampler_border4_v1_0();
    raw[24 + 16..24 + 20].copy_from_slice(&(-0.0_f32).to_bits().to_le_bytes());
    let canonical = parse_root_signature(&raw).unwrap().serialize();
    assert_ne!(canonical, raw);
    let hash = osdb_records::Sha256::from_sodb(hash_shader_container(&raw));
    records
        .root_signatures
        .insert(hash, HexBytes::from_bytes(&raw));
    records.records[0].root_signature = Some(hash);
    let dest = temp_path("-noncanon.sodb");
    let report = rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    assert_eq!(report.rebuilt, 0);
    assert_eq!(report.skipped_noncanonical_root_signature, 1);
    assert_no_leftovers(&dest);
    if dest.exists() {
        let bytes = fs::read(&dest).unwrap();
        assert!(
            !bytes.windows(raw.len()).any(|w| w == raw.as_slice()),
            "non-canonical root signature was copied raw"
        );
    }
    let _ = fs::remove_file(&dest);
}

#[test]
fn rebuilt_root_signature_is_canonical() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-canon-rs.sodb");
    rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    let conn = Connection::open(&dest).unwrap();
    let stored: Vec<u8> = conn
        .query_row("SELECT value FROM root_signatures LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let parsed = parse_root_signature(&stored).unwrap();
    assert_eq!(parsed.serialize(), stored);
    assert_eq!(stored, empty_root_signature_v1_0());
    let _ = fs::remove_file(&dest);
}

#[cfg(target_os = "linux")]
#[test]
fn rebuild_distinct_shader_rss_stays_bounded() {
    if run_rss_isolated("rebuild_distinct_shader_rss_stays_bounded") {
        return;
    }
    eprintln!("RSS_CHILD_RAN");
    let _rss = RSS_MEASURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    const COUNT: u32 = 160;
    const SIZE: usize = 256 * 1024;
    let fixture = many_distinct_sodb(COUNT, SIZE);
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dir = temp_path("-rss-distinct-dir");
    fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("out.sodb");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dest_s = dest.clone();
    let stop_s = std::sync::Arc::clone(&stop);
    let sampler = std::thread::spawn(move || {
        let mut peak = 0_u64;
        let mut peak_rss = rss_bytes().unwrap_or(0);
        while !stop_s.load(std::sync::atomic::Ordering::Relaxed) {
            peak = peak.max(dest_family_bytes(&dest_s));
            if let Some(now) = rss_bytes() {
                peak_rss = peak_rss.max(now);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        (
            peak.max(dest_family_bytes(&dest_s)),
            peak_rss.max(rss_bytes().unwrap_or(0)),
        )
    });
    let before = rss_bytes();
    let report = rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (sampled_disk, peak_rss) = sampler.join().unwrap();
    let peak_disk = sampled_disk.saturating_add(crate::write::injected_extra_disk());
    let after = rss_bytes();
    let out_len = fs::metadata(&dest).unwrap().len();
    assert_eq!(report.unique_shaders, u64::from(COUNT));
    assert!(
        out_len > 24 * 1024 * 1024,
        "expected tens of MB of output, got {out_len}"
    );
    eprintln!(
        "distinct-shader rebuild output={} peak_disk={} ({:.2}x) extra_disk={}",
        out_len,
        peak_disk,
        peak_disk as f64 / out_len as f64,
        crate::write::injected_extra_disk()
    );
    assert!(
        !crate::write::saw_memory_temp_store(),
        "temp_store=MEMORY was set; RSS/disk guard must fail"
    );
    assert!(
        crate::write::injected_extra_disk() == 0,
        "plain VACUUM charged extra disk {}; disk-peak guard must fail",
        crate::write::injected_extra_disk()
    );
    assert!(
        peak_disk < out_len.saturating_mul(5) / 2 + 4 * 1024 * 1024,
        "peak disk {peak_disk} tracks 3x output {out_len}; extra VACUUM?"
    );
    if let Some(before) = before {
        let delta = peak_rss.max(after.unwrap_or(0)).saturating_sub(before);
        eprintln!(
            "distinct-shader rebuild RSS peak-delta={} ({:.1} MiB)",
            delta,
            delta as f64 / (1024.0 * 1024.0)
        );
        assert!(
            delta < 28 * 1024 * 1024,
            "RSS grew by {delta} bytes; temp_store=MEMORY?"
        );
        assert!(
            delta < out_len / 2,
            "RSS grew by {delta} bytes tracking output {out_len}"
        );
    }
    let _ = fs::remove_file(&dest);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn without_rowid_source_rebuilds() {
    let vs = dxbc_of_len(48, 5);
    let ps = dxbc_of_len(48, 6);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (
                Key BLOB NOT NULL PRIMARY KEY, Type TEXT, Bytecode BLOB
            ) WITHOUT ROWID;
            CREATE TABLE pipeline_states (
                Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB, ByteCode_PS BLOB
            );
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, 'vs', ?2)",
            rusqlite::params![b"bc-vs", &vs],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, 'ps', ?2)",
            rusqlite::params![b"bc-ps", &ps],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states VALUES (?1, ?2, ?3)",
            rusqlite::params![b"pso-1", b"bc-vs", b"bc-ps"],
        )?;
        Ok(())
    });
    let original = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-wr.sodb");
    let report = rebuild_sodb(&original, &index, &dest, opts()).unwrap();
    assert_eq!(report.rebuilt, 1);
    assert_eq!(report.unique_shaders, 2);
    let rebuilt = extract(&dest, BuildIdentity::empty()).unwrap();
    assert_eq!(rebuilt.records, original.records);
    assert_no_leftovers(&dest);
    let _ = fs::remove_file(&dest);
}

#[test]
fn first_spec_order_stage_type_wins() {
    let blob = dxbc_of_len(48, 8);
    let fixture = create_sodb(|conn| {
        conn.execute_batch(
            "
            CREATE TABLE shader_bytecode (Key BLOB NOT NULL PRIMARY KEY, Bytecode BLOB);
            CREATE TABLE pipeline_states (
                Key BLOB NOT NULL PRIMARY KEY, ByteCode_VS BLOB, ByteCode_PS BLOB
            );
            ",
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode VALUES (?1, ?2)",
            rusqlite::params![b"shared", &blob],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states VALUES (?1, ?2, ?2)",
            rusqlite::params![b"pso", b"shared"],
        )?;
        Ok(())
    });
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    assert_eq!(
        records.records[0].bytecode_hashes.vs,
        records.records[0].bytecode_hashes.ps
    );
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-type.sodb");
    rebuild_sodb(&records, &index, &dest, opts()).unwrap();
    let conn = Connection::open(&dest).unwrap();
    let ty: String = conn
        .query_row("SELECT Type FROM shader_bytecode LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        ty, "vs",
        "first spec-order stage name wins for a shared hash"
    );
    let _ = fs::remove_file(&dest);
}

#[test]
fn reserve_compact_fault_leaves_no_leftovers() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-reserve.sodb");
    inject_fault(Some(InjectedFault::ReserveCompact));
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::TempFileExists));
    assert!(!dest.exists());
    assert_no_leftovers(&dest);
}

#[test]
fn dest_fsync_after_publish_is_warning() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-fsync.sodb");
    fs::write(&dest, b"previous-output").unwrap();
    let mut forced = opts();
    forced.force = true;
    inject_fault(Some(InjectedFault::DestFsync));
    let report = rebuild_sodb(&records, &index, &dest, forced).unwrap();
    assert!(report.publish_warnings >= 1);
    assert_ne!(fs::read(&dest).unwrap(), b"previous-output");
    assert_no_leftovers(&dest);
    let _ = fs::remove_file(&dest);
}

#[test]
fn fallback_remove_tmp_after_publish_is_warning() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-rmtmp.sodb");
    fs::write(&dest, b"previous-output").unwrap();
    let mut forced = opts();
    forced.force = true;
    inject_fault(Some(InjectedFault::FallbackRemoveTmp));
    let report = rebuild_sodb(&records, &index, &dest, forced).unwrap();
    assert!(report.publish_warnings >= 1);
    assert_ne!(fs::read(&dest).unwrap(), b"previous-output");
    let _ = fs::remove_file(&dest);
}

#[test]
fn persist_before_publish_keeps_old_file() {
    let _fault = FaultGuard;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dest = temp_path("-prepub.sodb");
    fs::write(&dest, b"previous-output").unwrap();
    let mut forced = opts();
    forced.force = true;
    inject_fault(Some(InjectedFault::Persist));
    let err = rebuild_sodb(&records, &index, &dest, forced).unwrap_err();
    assert!(matches!(err, Error::Io | Error::Sqlite));
    assert_eq!(fs::read(&dest).unwrap(), b"previous-output");
    assert_no_leftovers(&dest);
}

#[test]
fn parent_that_is_a_file_is_not_regular() {
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let parent = temp_path("-not-a-dir");
    fs::write(&parent, b"file").unwrap();
    let dest = parent.join("out.sodb");
    let err = rebuild_sodb(&records, &index, &dest, opts()).unwrap_err();
    assert!(matches!(err, Error::NotRegular));
    let _ = fs::remove_file(&parent);
}

#[cfg(unix)]
#[test]
fn unreadable_record_input_is_io() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = rich_fixture();
    let records = extract(&fixture.path, BuildIdentity::empty()).unwrap();
    let (index, _) = scan_sodb(&fixture.path, ScanLimits::DEFAULT).unwrap();
    let dir = temp_path("-rec-denied");
    fs::create_dir_all(&dir).unwrap();
    let rec = dir.join("records.json");
    fs::write(&rec, b"{}").unwrap();
    let dest = temp_path("-rec-denied-out.sodb");
    fs::write(&dest, b"previous-output").unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let mut options = opts();
    options.force = true;
    options.record_input = Some(rec);
    let err = rebuild_sodb(&records, &index, &dest, options).unwrap_err();
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(&dir);
    assert!(matches!(err, Error::Io));
    assert_eq!(fs::read(&dest).unwrap(), b"previous-output");
    let _ = fs::remove_file(&dest);
}
