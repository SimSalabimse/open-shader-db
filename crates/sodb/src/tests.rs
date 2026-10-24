// SPDX-License-Identifier: MIT

//! Synthetic SODB fixtures. Blobs are fake bytes, never real DXIL.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// RSS is process-wide; serialize tests that assert a peak-memory bound.
static RSS_MEASURE_LOCK: Mutex<()> = Mutex::new(());

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::{
    is_sodb, open, open_with_limits, probe, Error, Limits, ObjectKind, TableId,
    SODB_APPLICATION_ID, SODB_USER_VERSION,
};

const FAKE_VS: &[u8] = b"FAKE_VS_BLOB";
const FAKE_PS: &[u8] = b"FAKE_PS_BLOB";
const FAKE_VS2: &[u8] = b"FAKE_VS_BLOB_2";

/// Full Microsoft spec `pipeline_states` CREATE (1472 bytes). The old
/// 1 KiB `SQLITE_LIMIT_LENGTH` floor rejects this; the 1 MiB floor must not.
const SPEC_PIPELINE_STATES_SQL: &str = "CREATE TABLE pipeline_states (Key BLOB NOT NULL PRIMARY KEY , RootSignature BLOB  ,InputLayout BLOB  ,ByteCode_VS BLOB  ,ByteCode_PS BLOB  ,ByteCode_HS BLOB  ,ByteCode_DS BLOB  ,ByteCode_GS BLOB  ,ByteCode_AS BLOB  ,ByteCode_MS BLOB  ,ByteCode_CS BLOB  ,DepthStencilDesc BLOB  ,RenderTargetFormats BLOB  ,BlendDesc BLOB  ,RasterizerDesc BLOB  ,ViewInstancingDesc BLOB  ,StreamOutDesc BLOB  ,SampleDesc_Count INTEGER  ,SampleDesc_Quality INTEGER  ,SampleMask INTEGER  ,IBStripCutValue INTEGER  ,PrimitiveTopology INTEGER  ,DSVFormat INTEGER  ,NodeMask INTEGER  ,Flags INTEGER  ,FOREIGN KEY(RootSignature) REFERENCES root_signatures(Key),FOREIGN KEY(ByteCode_VS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_PS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_HS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_DS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_GS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_AS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_MS) REFERENCES shader_bytecode(Key),FOREIGN KEY(ByteCode_CS) REFERENCES shader_bytecode(Key),FOREIGN KEY(DepthStencilDesc) REFERENCES depth_stencil_descs(Key),FOREIGN KEY(RenderTargetFormats) REFERENCES render_target_formats(Key),FOREIGN KEY(BlendDesc) REFERENCES blend_descs(Key),FOREIGN KEY(RasterizerDesc) REFERENCES rasterizer_descs(Key),FOREIGN KEY(ViewInstancingDesc) REFERENCES view_instancing_descs(Key),FOREIGN KEY(StreamOutDesc) REFERENCES stream_out_descs(Key));";

const EMPTY_SO_ASSOC_TABLES_SQL: &str = "
        CREATE TABLE so_to_global_rs_associations (
            StateObjectKey BLOB, RootSignatureKey BLOB
        );
        CREATE TABLE so_to_local_rs_associations (
            StateObjectKey BLOB, RootSignatureKey BLOB
        );
        CREATE TABLE so_to_dxil_lib_associations (
            StateObjectKey BLOB, DxilLibKey BLOB, ExportKey BLOB
        );
        CREATE TABLE so_to_existing_so_associations (
            StateObjectKey BLOB, ExistingStateObjectKey BLOB, ExportKey BLOB
        );
        CREATE TABLE so_to_hit_group_associations (
            StateObjectKey BLOB, HitGroupKey BLOB
        );
        CREATE TABLE so_to_rt_shader_config_associations (
            StateObjectKey BLOB, ShaderConfigKey BLOB
        );
        CREATE TABLE so_to_rt_pipeline_config_associations (
            StateObjectKey BLOB, PipelineConfigKey BLOB
        );
        CREATE TABLE so_to_dxil_subobject_to_exports_associations (
            StateObjectKey BLOB, DxilSubobjectToExportsAssociationKey BLOB
        );
        CREATE TABLE so_to_subobject_to_exports_associations (
            StateObjectKey BLOB, SubobjectToExportsAssociationKey BLOB
        );
        CREATE TABLE so_to_generic_program_associations (
            StateObjectKey BLOB, GenericProgramKey BLOB
        );
        CREATE TABLE so_to_work_graph_associations (
            StateObjectKey BLOB, WorkGraphKey BLOB
        );
        ";

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn temp_path() -> PathBuf {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("osdb-sodb-{}-{n}.sodb", std::process::id()))
}

fn create_sodb(
    user_version: i32,
    application_id: i32,
    setup: impl FnOnce(&mut Connection) -> rusqlite::Result<()>,
) -> Fixture {
    let path = temp_path();
    let mut conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "OFF").unwrap();
    conn.pragma_update(None, "user_version", user_version)
        .unwrap();
    conn.pragma_update(None, "application_id", application_id)
        .unwrap();
    setup(&mut conn).unwrap();
    drop(conn);
    Fixture { path }
}

fn create_core_tables(conn: &mut Connection) -> rusqlite::Result<()> {
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
        CREATE TABLE shader_bytecode (
            Key BLOB NOT NULL PRIMARY KEY,
            Type TEXT,
            Bytecode BLOB
        );
        CREATE TABLE root_signatures (
            Key BLOB NOT NULL PRIMARY KEY,
            value BLOB NOT NULL
        );
        CREATE TABLE pipeline_states (
            Key BLOB NOT NULL PRIMARY KEY,
            RootSignature BLOB,
            InputLayout BLOB,
            ByteCode_VS BLOB,
            ByteCode_PS BLOB,
            ByteCode_HS BLOB,
            ByteCode_DS BLOB,
            ByteCode_GS BLOB,
            ByteCode_AS BLOB,
            ByteCode_MS BLOB,
            ByteCode_CS BLOB,
            DepthStencilDesc BLOB,
            RenderTargetFormats BLOB,
            BlendDesc BLOB,
            RasterizerDesc BLOB,
            ViewInstancingDesc BLOB,
            StreamOutDesc BLOB,
            SampleDesc_Count INTEGER,
            SampleDesc_Quality INTEGER,
            SampleMask INTEGER,
            IBStripCutValue INTEGER,
            PrimitiveTopology INTEGER,
            DSVFormat INTEGER,
            NodeMask INTEGER,
            Flags INTEGER
        );
        CREATE TABLE state_objects (
            Key BLOB NOT NULL PRIMARY KEY,
            Type INTEGER,
            NodeMask INTEGER,
            Flags INTEGER,
            AddToStateObjectParent BLOB
        );
        CREATE TABLE groups (
            Key BLOB NOT NULL PRIMARY KEY,
            Version INTEGER NOT NULL,
            PSOKey BLOB,
            SOKey BLOB
        );
        CREATE TABLE so_to_dxil_lib_associations (
            StateObjectKey BLOB NOT NULL,
            DxilLibKey BLOB NOT NULL,
            ExportKey BLOB
        );
        CREATE TABLE rasterizer_descs (
            Key BLOB NOT NULL PRIMARY KEY,
            FillMode INTEGER NOT NULL,
            CullMode INTEGER NOT NULL,
            FrontCounterClockwise INTEGER NOT NULL,
            DepthBias REAL NOT NULL,
            DepthBiasClamp REAL NOT NULL,
            SlopeScaledDepthBias REAL NOT NULL,
            DepthClipEnable INTEGER NOT NULL,
            LineRasterizationMode INTEGER NOT NULL,
            ForcedSampleCount INTEGER NOT NULL,
            ConservativeRaster INTEGER NOT NULL
        );
        ",
    )?;
    Ok(())
}

fn replace_pipeline_states_with_spec_sql(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DROP TABLE IF EXISTS pipeline_states", [])?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS depth_stencil_descs (Key BLOB PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS render_target_formats (Key BLOB PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS blend_descs (Key BLOB PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS view_instancing_descs (Key BLOB PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS stream_out_descs (Key BLOB PRIMARY KEY);
        ",
    )?;
    conn.execute_batch(SPEC_PIPELINE_STATES_SQL)?;
    Ok(())
}

fn insert_valid_rows(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_id (id, exe, app_name, engine_name, app_version, engine_version)
         VALUES (1, 'ExampleApp.exe', 'Example App', 'Example Engine', 281474976710656, 562958543355904)",
        [],
    )?;
    conn.execute(
        "INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, 'vs', ?2)",
        rusqlite::params![b"bc-vs-1", FAKE_VS],
    )?;
    conn.execute(
        "INSERT INTO shader_bytecode (Key, Type, Bytecode) VALUES (?1, 'ps', ?2)",
        rusqlite::params![b"bc-ps-1", FAKE_PS],
    )?;
    conn.execute(
        "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
        rusqlite::params![b"rs-1", b"FAKE_ROOT_SIG"],
    )?;
    conn.execute(
        "INSERT INTO rasterizer_descs (
            Key, FillMode, CullMode, FrontCounterClockwise, DepthBias, DepthBiasClamp,
            SlopeScaledDepthBias, DepthClipEnable, LineRasterizationMode, ForcedSampleCount,
            ConservativeRaster
         ) VALUES (?1, 3, 1, 0, 0.0, 0.0, 0.0, 1, 0, 0, 0)",
        rusqlite::params![b"rast-1"],
    )?;
    conn.execute(
        "INSERT INTO pipeline_states (
            Key, RootSignature, ByteCode_VS, ByteCode_PS, RasterizerDesc,
            SampleDesc_Count, SampleMask, Flags
         ) VALUES (?1, ?2, ?3, ?4, ?5, 1, 4294967295, 0)",
        rusqlite::params![b"pso-key-1", b"rs-1", b"bc-vs-1", b"bc-ps-1", b"rast-1"],
    )?;
    conn.execute(
        "INSERT INTO state_objects (Key, Type, NodeMask, Flags) VALUES (?1, 2, 0, 0)",
        rusqlite::params![b"so-key-1"],
    )?;
    conn.execute(
        "INSERT INTO groups (Key, Version, PSOKey, SOKey) VALUES (?1, 3, ?2, NULL)",
        rusqlite::params![b"grp-pso", b"pso-key-1"],
    )?;
    conn.execute(
        "INSERT INTO groups (Key, Version, PSOKey, SOKey) VALUES (?1, 7, NULL, ?2)",
        rusqlite::params![b"grp-so", b"so-key-1"],
    )?;
    conn.execute(
        "INSERT INTO so_to_dxil_lib_associations (StateObjectKey, DxilLibKey) VALUES (?1, ?2)",
        rusqlite::params![b"so-key-1", b"bc-vs-1"],
    )?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_digits(Sha256::digest(bytes))
}

fn hex_digits(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn valid_file_reads_keys_and_hashes_only() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        insert_valid_rows(conn)
    });

    let summary = open(fixture.path()).unwrap();
    assert_eq!(summary.user_version, SODB_USER_VERSION);
    assert_eq!(summary.application_id, SODB_APPLICATION_ID);
    assert!(!summary.wal_ignored);
    assert!(summary.missing_tables.is_empty());

    assert_eq!(summary.app_id.len(), 1);
    assert_eq!(summary.app_id[0].exe.as_str(), "ExampleApp.exe");
    assert_eq!(summary.app_id[0].app_name.as_str(), "Example App");

    assert_eq!(summary.shader_bytecode.len(), 2);
    let vs_hash = sha256_hex(FAKE_VS);
    let ps_hash = sha256_hex(FAKE_PS);
    let hashes: Vec<String> = summary
        .shader_bytecode
        .iter()
        .map(|b| b.bytecode_hash.to_hex())
        .collect();
    assert!(hashes.contains(&vs_hash));
    assert!(hashes.contains(&ps_hash));

    assert_eq!(summary.pipeline_states.len(), 1);
    let pso = &summary.pipeline_states[0];
    assert_eq!(pso.kind, ObjectKind::PipelineState);
    assert_eq!(pso.key.as_bytes(), b"pso-key-1");
    assert_eq!(pso.version.get(), 3);
    assert_eq!(pso.bytecode_hashes.len(), 2);
    assert_eq!(pso.desc_hash.as_bytes().len(), 32);

    assert_eq!(summary.state_objects.len(), 1);
    let so = &summary.state_objects[0];
    assert_eq!(so.kind, ObjectKind::StateObject);
    assert_eq!(so.key.as_bytes(), b"so-key-1");
    assert_eq!(so.version.get(), 7);
    assert_eq!(so.bytecode_hashes[0].to_hex(), vs_hash);

    let again = open(fixture.path()).unwrap();
    assert_eq!(again.pipeline_states[0].desc_hash, pso.desc_hash);
}

#[test]
fn desc_hash_ignores_bytecode_and_changes_with_state() {
    let same_desc = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-vs-1", FAKE_VS],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-vs-2", FAKE_VS2],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-a", b"bc-vs-1"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-b", b"bc-vs-2"],
        )?;
        Ok(())
    });
    let same = open(same_desc.path()).unwrap();
    assert_ne!(
        same.pipeline_states[0].bytecode_hashes,
        same.pipeline_states[1].bytecode_hashes
    );
    // Keys differ, so desc hashes differ; compare a pair that shares flags only
    // after hashing without the key... we include the app key in the desc hash.
    // A Flags change must still change the hash for the same key:
    drop(same_desc);

    let flags_a = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"pso-same"],
        )?;
        Ok(())
    });
    let flags_b = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 1)",
            rusqlite::params![b"pso-same"],
        )?;
        Ok(())
    });
    let a = open(flags_a.path()).unwrap();
    let b = open(flags_b.path()).unwrap();
    assert_ne!(
        a.pipeline_states[0].desc_hash,
        b.pipeline_states[0].desc_hash
    );

    let bc_a = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-1", FAKE_VS],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-same", b"bc-1"],
        )?;
        Ok(())
    });
    let bc_b = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-1", FAKE_PS],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, ByteCode_VS, Flags) VALUES (?1, ?2, 0)",
            rusqlite::params![b"pso-same", b"bc-1"],
        )?;
        Ok(())
    });
    let left = open(bc_a.path()).unwrap();
    let right = open(bc_b.path()).unwrap();
    assert_eq!(
        left.pipeline_states[0].desc_hash,
        right.pipeline_states[0].desc_hash
    );
    assert_ne!(
        left.pipeline_states[0].bytecode_hashes,
        right.pipeline_states[0].bytecode_hashes
    );
}

#[test]
fn probe_reads_header_without_tables() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |_| Ok(()));
    let header = probe(fixture.path()).unwrap();
    assert_eq!(header.user_version, SODB_USER_VERSION);
    assert_eq!(header.application_id, SODB_APPLICATION_ID);
    assert!(is_sodb(fixture.path()));
}

#[test]
fn probe_rejects_wrong_magic_and_versions() {
    let wrong_ver = create_sodb(1, SODB_APPLICATION_ID, |_| Ok(()));
    let header = probe(wrong_ver.path()).unwrap();
    assert_eq!(header.user_version, 1);
    assert!(!is_sodb(wrong_ver.path()));

    let wrong_app = create_sodb(SODB_USER_VERSION, 0, |_| Ok(()));
    assert_eq!(probe(wrong_app.path()).unwrap().application_id, 0);
    assert!(!is_sodb(wrong_app.path()));

    let garbage = temp_path();
    std::fs::write(&garbage, b"not a sqlite database").unwrap();
    assert!(matches!(probe(&garbage), Err(Error::NotAnSodb)));
    assert!(!is_sodb(&garbage));
    let _ = std::fs::remove_file(&garbage);
}

#[test]
fn rejects_wrong_user_version() {
    let fixture = create_sodb(1, SODB_APPLICATION_ID, |_| Ok(()));
    let err = open(fixture.path()).unwrap_err();
    assert!(matches!(
        err,
        Error::UnsupportedUserVersion {
            found: 1,
            expected: 2
        }
    ));
}

#[test]
fn rejects_wrong_application_id() {
    let fixture = create_sodb(SODB_USER_VERSION, 0, |_| Ok(()));
    let err = open(fixture.path()).unwrap_err();
    assert!(matches!(
        err,
        Error::UnsupportedApplicationId {
            found: 0,
            expected: SODB_APPLICATION_ID
        }
    ));
}

#[test]
fn missing_tables_are_reported() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |_| Ok(()));
    let summary = open(fixture.path()).unwrap();
    assert!(summary.app_id.is_empty());
    assert!(summary.pipeline_states.is_empty());
    assert!(summary.state_objects.is_empty());
    assert!(summary.shader_bytecode.is_empty());
    for required in [
        TableId::AppId,
        TableId::PipelineStates,
        TableId::StateObjects,
        TableId::ShaderBytecode,
    ] {
        assert!(
            summary.missing_tables.contains(&required),
            "missing {required}"
        );
    }
}

#[test]
fn empty_tables_succeed() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, create_core_tables);
    let summary = open(fixture.path()).unwrap();
    assert!(summary.missing_tables.is_empty());
    assert!(summary.app_id.is_empty());
    assert!(summary.pipeline_states.is_empty());
    assert!(summary.state_objects.is_empty());
    assert!(summary.shader_bytecode.is_empty());
}

#[test]
fn truncated_file_is_malformed() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, create_core_tables);
    let contents = std::fs::read(fixture.path()).unwrap();
    assert!(contents.len() > 16);
    let truncated = temp_path();
    // Keep only the SQLite magic prefix so the file is a truncated header, not a full DB.
    std::fs::write(&truncated, &contents[..16]).unwrap();
    let truncated_result = open(&truncated);
    let _ = std::fs::remove_file(&truncated);
    match truncated_result {
        Err(Error::NotAnSodb | Error::Malformed { .. }) => {}
        other => panic!("expected NotAnSodb/Malformed for truncated file, got {other:?}"),
    }

    let garbage = temp_path();
    std::fs::write(&garbage, b"not a sqlite database").unwrap();
    let garbage_result = open(&garbage);
    let _ = std::fs::remove_file(&garbage);
    match garbage_result {
        Err(Error::NotAnSodb | Error::Malformed { .. }) => {}
        other => panic!("expected NotAnSodb/Malformed for garbage file, got {other:?}"),
    }
}

#[test]
fn oversized_blob_is_rejected() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"bc-big", b"0123456789"],
        )?;
        Ok(())
    });
    let limits = Limits {
        max_blob_bytes: 8,
        ..Limits::DEFAULT
    };
    let err = open_with_limits(fixture.path(), limits).unwrap_err();
    assert!(
        matches!(
            err,
            Error::BlobTooLarge {
                table: TableId::ShaderBytecode,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn duplicate_keys_are_rejected() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch("CREATE TABLE shader_bytecode (Key BLOB, Type TEXT, Bytecode BLOB);")?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"dup", FAKE_VS],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
            rusqlite::params![b"dup", FAKE_PS],
        )?;
        Ok(())
    });
    let err = open(fixture.path()).unwrap_err();
    assert!(matches!(
        err,
        Error::DuplicateKey {
            table: TableId::ShaderBytecode
        }
    ));
}

#[test]
fn json_summary_never_contains_bytecode_bytes() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        insert_valid_rows(conn)
    });
    let summary = open(fixture.path()).unwrap();
    let json = serde_json::to_string(&summary).unwrap();
    assert!(!json.contains("FAKE_VS_BLOB"));
    assert!(!json.contains("FAKE_PS_BLOB"));
    assert!(!json.contains("FAKE_ROOT_SIG"));
}

#[test]
fn empty_file_is_not_an_sodb() {
    let path = temp_path();
    std::fs::write(&path, []).unwrap();
    let err = open(&path).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(matches!(err, Error::NotAnSodb), "got {err:?}");
}

#[test]
fn directory_is_not_a_regular_file() {
    let dir = std::env::temp_dir();
    let err = open(&dir).unwrap_err();
    assert!(matches!(err, Error::NotARegularFile), "got {err:?}");
}

#[cfg(unix)]
#[test]
fn fifo_is_not_a_regular_file() {
    let path = temp_path();
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let err = open(&path).unwrap_err();
    let _ = std::fs::remove_file(&path);
    assert!(matches!(err, Error::NotARegularFile), "got {err:?}");
}

#[test]
fn octet_length_counts_bytes_not_characters() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(crate::parser::sqlite_has_octet_length(&conn).unwrap());
    let chars: i64 = conn
        .query_row("SELECT length('中中中中')", [], |row| row.get(0))
        .unwrap();
    let bytes: i64 = conn
        .query_row("SELECT octet_length('中中中中')", [], |row| row.get(0))
        .unwrap();
    assert_eq!(chars, 4);
    assert_eq!(bytes, 12);

    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO app_id (id, exe, app_name, app_version, engine_version)
             VALUES (1, '中中中中', 'n', 1, 0)",
            [],
        )?;
        Ok(())
    });
    let limits = Limits {
        max_text_bytes: 10,
        ..Limits::DEFAULT
    };
    let err = open_with_limits(fixture.path(), limits).unwrap_err();
    assert!(
        matches!(
            err,
            Error::TextTooLarge {
                table: TableId::AppId,
                max: 10,
                got: 12
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn wal_open_creates_no_sidecar_files() {
    let src = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        create_core_tables(conn)?;
        insert_valid_rows(conn)
    });
    let copy = temp_path();
    std::fs::copy(src.path(), &copy).unwrap();
    let summary = open(&copy).unwrap();
    assert_eq!(summary.pipeline_states.len(), 1);
    let wal = PathBuf::from(format!("{}-wal", copy.display()));
    let shm = PathBuf::from(format!("{}-shm", copy.display()));
    assert!(!wal.exists(), "WAL sidecar was created: {wal:?}");
    assert!(!shm.exists(), "SHM sidecar was created: {shm:?}");
    let _ = std::fs::remove_file(&copy);
}

#[test]
fn negative_app_version_is_invalid() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO app_id (id, exe, app_name, app_version, engine_version)
             VALUES (1, 'e', 'n', -1, 0)",
            [],
        )?;
        Ok(())
    });
    let err = open(fixture.path()).unwrap_err();
    assert!(
        matches!(
            err,
            Error::InvalidValue {
                table: TableId::AppId,
                found: -1,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn text_in_integer_column_is_wrong_type() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO app_id (id, exe, app_name, app_version, engine_version)
             VALUES (1, 'e', 'n', 'not-int', 0)",
            [],
        )?;
        Ok(())
    });
    let err = open(fixture.path()).unwrap_err();
    assert!(
        matches!(
            err,
            Error::WrongColumnType {
                table: TableId::AppId,
                ..
            }
        ),
        "got {err:?}"
    );
    let shown = err.to_string();
    assert!(
        !shown.contains("CASE WHEN") && !shown.contains("typeof"),
        "error leaked generated SQL: {shown}"
    );
}

#[test]
fn wrong_key_types_are_rejected() {
    for (sql_value, _label) in [(" 'text-key' ", "text"), ("NULL", "null"), ("1.5", "real")] {
        let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
            conn.execute_batch("CREATE TABLE shader_bytecode (Key, Type TEXT, Bytecode BLOB);")?;
            conn.execute(
                &format!("INSERT INTO shader_bytecode (Key, Bytecode) VALUES ({sql_value}, x'00')"),
                [],
            )?;
            Ok(())
        });
        let err = open(fixture.path()).unwrap_err();
        assert!(
            matches!(
                err,
                Error::WrongColumnType {
                    table: TableId::ShaderBytecode,
                    ..
                } | Error::NullKey {
                    table: TableId::ShaderBytecode
                }
            ),
            "value {sql_value} got {err:?}"
        );
    }
}

#[test]
fn wrong_bytecode_types_are_rejected() {
    for sql_value in ["'text-bc'", "1"] {
        let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
            conn.execute_batch("CREATE TABLE shader_bytecode (Key BLOB, Type TEXT, Bytecode);")?;
            conn.execute(
                &format!("INSERT INTO shader_bytecode (Key, Bytecode) VALUES (x'01', {sql_value})"),
                [],
            )?;
            Ok(())
        });
        let err = open(fixture.path()).unwrap_err();
        assert!(
            matches!(
                err,
                Error::WrongColumnType {
                    table: TableId::ShaderBytecode,
                    ..
                }
            ),
            "value {sql_value} got {err:?}"
        );
    }
}

#[test]
fn row_limit_is_enforced() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"a"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"b"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"c"],
        )?;
        Ok(())
    });
    let limits = Limits {
        max_rows_per_table: 2,
        ..Limits::DEFAULT
    };
    let err = open_with_limits(fixture.path(), limits).unwrap_err();
    assert!(
        matches!(
            err,
            Error::TooManyRows {
                table: TableId::PipelineStates,
                max: 2,
                got: 3
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn total_row_budget_is_enforced() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        for i in 0..8 {
            let key = i_u8(i);
            conn.execute(
                "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
                rusqlite::params![&key],
            )?;
        }
        Ok(())
    });
    let limits = Limits {
        max_total_rows: 5,
        ..Limits::DEFAULT
    };
    let err = open_with_limits(fixture.path(), limits).unwrap_err();
    assert!(
        matches!(err, Error::TooManyTotalRows { max: 5, .. }),
        "got {err:?}"
    );
}

fn i_u8(i: u8) -> [u8; 1] {
    [i]
}

#[test]
fn duplicate_null_bytecode_keys_are_rejected() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch("CREATE TABLE shader_bytecode (Key BLOB, Type TEXT, Bytecode BLOB);")?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, NULL)",
            rusqlite::params![b"dup"],
        )?;
        conn.execute(
            "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, NULL)",
            rusqlite::params![b"dup"],
        )?;
        Ok(())
    });
    let err = open(fixture.path()).unwrap_err();
    assert!(
        matches!(
            err,
            Error::DuplicateKey {
                table: TableId::ShaderBytecode
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn desc_hash_covers_nested_descriptor_content() {
    let make = |elem_semantic: &str| {
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            conn.execute_batch(
                "
                CREATE TABLE input_element_descs (
                    Key BLOB NOT NULL PRIMARY KEY,
                    SemanticName TEXT NOT NULL,
                    SemanticIndex INTEGER NOT NULL,
                    Format INTEGER NOT NULL,
                    InputSlot INTEGER NOT NULL,
                    AlignedByteOffset INTEGER NOT NULL,
                    InputSlotClass INTEGER NOT NULL,
                    InstanceDataStepRate INTEGER NOT NULL
                );
                CREATE TABLE input_layout_to_input_element_associations (
                    InputLayoutKey BLOB NOT NULL,
                    InputElementKey BLOB NOT NULL
                );
                ",
            )?;
            conn.execute(
                "INSERT INTO input_element_descs VALUES (?1, ?2, 0, 0, 0, 0, 0, 0)",
                rusqlite::params![b"el-1", elem_semantic],
            )?;
            conn.execute(
                "INSERT INTO input_layout_to_input_element_associations VALUES (?1, ?2)",
                rusqlite::params![b"il-1", b"el-1"],
            )?;
            conn.execute(
                "INSERT INTO pipeline_states (Key, InputLayout, Flags) VALUES (?1, ?2, 0)",
                rusqlite::params![b"pso-same", b"il-1"],
            )?;
            Ok(())
        })
    };
    let a = make("POSITION");
    let b = make("COLOR");
    let left = open(a.path()).unwrap();
    let right = open(b.path()).unwrap();
    assert_ne!(
        left.pipeline_states[0].desc_hash,
        right.pipeline_states[0].desc_hash
    );
}

#[test]
fn desc_hash_covers_state_object_associations() {
    let make = |rs_value: &[u8]| {
        let owned = rs_value.to_vec();
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            conn.execute_batch(
                "
                CREATE TABLE so_to_global_rs_associations (
                    StateObjectKey BLOB NOT NULL,
                    RootSignatureKey BLOB NOT NULL
                );
                ",
            )?;
            conn.execute(
                "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
                rusqlite::params![b"rs-x", &owned],
            )?;
            conn.execute(
                "INSERT INTO state_objects (Key, Type, NodeMask, Flags) VALUES (?1, 1, 0, 0)",
                rusqlite::params![b"so-same"],
            )?;
            conn.execute(
                "INSERT INTO so_to_global_rs_associations VALUES (?1, ?2)",
                rusqlite::params![b"so-same", b"rs-x"],
            )?;
            Ok(())
        })
    };
    let a = make(b"AAA");
    let b = make(b"BBB");
    let left = open(a.path()).unwrap();
    let right = open(b.path()).unwrap();
    assert_ne!(
        left.state_objects[0].desc_hash,
        right.state_objects[0].desc_hash
    );
}

#[test]
fn desc_hash_covers_depth_stencil_ops_and_blend_targets() {
    let make_ds = |fail_op: i64| {
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            conn.execute_batch(
                "
                CREATE TABLE depth_stencil_op_descs (
                    Key BLOB PRIMARY KEY, StencilFailOp INTEGER, StencilDepthFailOp INTEGER,
                    StencilPassOp INTEGER, StencilFunc INTEGER, StencilReadMask INTEGER,
                    StencilWriteMask INTEGER
                );
                CREATE TABLE depth_stencil_descs (
                    Key BLOB PRIMARY KEY, DepthEnable INTEGER, DepthWriteMask INTEGER,
                    DepthFunc INTEGER, StencilEnable INTEGER, FrontFace BLOB, BackFace BLOB,
                    DepthBoundsTestEnable INTEGER
                );
                ",
            )?;
            conn.execute(
                "INSERT INTO depth_stencil_op_descs VALUES (?1, ?2, 0, 0, 0, 0, 0)",
                rusqlite::params![b"op-1", fail_op],
            )?;
            conn.execute(
                "INSERT INTO depth_stencil_descs VALUES (?1, 1, 1, 1, 1, ?2, ?2, 0)",
                rusqlite::params![b"ds-1", b"op-1"],
            )?;
            conn.execute(
                "INSERT INTO pipeline_states (Key, DepthStencilDesc, Flags) VALUES (?1, ?2, 0)",
                rusqlite::params![b"pso-same", b"ds-1"],
            )?;
            Ok(())
        })
    };
    let a = make_ds(1);
    let b = make_ds(2);
    assert_ne!(
        open(a.path()).unwrap().pipeline_states[0].desc_hash,
        open(b.path()).unwrap().pipeline_states[0].desc_hash
    );

    let make_blend = |mask: i64| {
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            conn.execute_batch(
                "
                CREATE TABLE render_target_blend_descs (
                    Key BLOB PRIMARY KEY, BlendEnable INTEGER, LogicOpEnable INTEGER,
                    SrcBlend INTEGER, DestBlend INTEGER, BlendOp INTEGER, SrcBlendAlpha INTEGER,
                    DestBlendAlpha INTEGER, BlendOpAlpha INTEGER, LogicOp INTEGER,
                    RenderTargetWriteMask INTEGER
                );
                CREATE TABLE blend_descs (
                    Key BLOB PRIMARY KEY, AlphaToCoverageEnable INTEGER,
                    IndependentBlendEnable INTEGER, RenderTarget0 BLOB, RenderTarget1 BLOB,
                    RenderTarget2 BLOB, RenderTarget3 BLOB, RenderTarget4 BLOB,
                    RenderTarget5 BLOB, RenderTarget6 BLOB, RenderTarget7 BLOB
                );
                ",
            )?;
            conn.execute(
                "INSERT INTO render_target_blend_descs VALUES (?1, 1, 0, 1, 1, 1, 1, 1, 1, 0, ?2)",
                rusqlite::params![b"rtb-1", mask],
            )?;
            conn.execute(
                "INSERT INTO blend_descs (Key, AlphaToCoverageEnable, IndependentBlendEnable, RenderTarget0)
                 VALUES (?1, 0, 0, ?2)",
                rusqlite::params![b"bl-1", b"rtb-1"],
            )?;
            conn.execute(
                "INSERT INTO pipeline_states (Key, BlendDesc, Flags) VALUES (?1, ?2, 0)",
                rusqlite::params![b"pso-same", b"bl-1"],
            )?;
            Ok(())
        })
    };
    let c = make_blend(0x0f);
    let d = make_blend(0xff);
    assert_ne!(
        open(c.path()).unwrap().pipeline_states[0].desc_hash,
        open(d.path()).unwrap().pipeline_states[0].desc_hash
    );
}

#[test]
fn desc_hash_covers_stream_output_declarations() {
    let make = |semantic: &str| {
        let owned = semantic.to_string();
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            conn.execute_batch(
                "
                CREATE TABLE so_declarations (
                    Key BLOB PRIMARY KEY, Stream INTEGER, SemanticName TEXT, SemanticIndex INTEGER,
                    StartComponent INTEGER, ComponentCount INTEGER, OutputSlot INTEGER
                );
                CREATE TABLE stream_out_descs (
                    Key BLOB PRIMARY KEY, BufferStride0 INTEGER, BufferStride1 INTEGER,
                    BufferStride2 INTEGER, BufferStride3 INTEGER, NumStrides INTEGER,
                    RasterizedStream INTEGER
                );
                CREATE TABLE stream_output_desc_to_stream_output_decl_associations (
                    StreamOutDescKey BLOB, StreamOutDeclKey BLOB
                );
                ",
            )?;
            conn.execute(
                "INSERT INTO so_declarations VALUES (?1, 0, ?2, 0, 0, 4, 0)",
                rusqlite::params![b"decl-1", &owned],
            )?;
            conn.execute(
                "INSERT INTO stream_out_descs VALUES (?1, 16, 0, 0, 0, 1, 0)",
                rusqlite::params![b"so-desc-1"],
            )?;
            conn.execute(
                "INSERT INTO stream_output_desc_to_stream_output_decl_associations VALUES (?1, ?2)",
                rusqlite::params![b"so-desc-1", b"decl-1"],
            )?;
            conn.execute(
                "INSERT INTO pipeline_states (Key, StreamOutDesc, Flags) VALUES (?1, ?2, 0)",
                rusqlite::params![b"pso-same", b"so-desc-1"],
            )?;
            Ok(())
        })
    };
    let a = make("POSITION");
    let b = make("TEXCOORD");
    assert_ne!(
        open(a.path()).unwrap().pipeline_states[0].desc_hash,
        open(b.path()).unwrap().pipeline_states[0].desc_hash
    );
}

#[test]
fn fifty_thousand_rows_without_indexes_finish_quickly() {
    let _rss = RSS_MEASURE_LOCK.lock().unwrap();
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "
            CREATE TABLE pipeline_states (Key BLOB, Flags INTEGER);
            CREATE TABLE groups (Key BLOB, Version INTEGER, PSOKey BLOB, SOKey BLOB);
            CREATE TABLE rasterizer_descs (
                Key BLOB, FillMode INTEGER, CullMode INTEGER, FrontCounterClockwise INTEGER,
                DepthBias REAL, DepthBiasClamp REAL, SlopeScaledDepthBias REAL,
                DepthClipEnable INTEGER, LineRasterizationMode INTEGER,
                ForcedSampleCount INTEGER, ConservativeRaster INTEGER
            );
            CREATE TABLE blend_descs (
                Key BLOB, AlphaToCoverageEnable INTEGER, IndependentBlendEnable INTEGER,
                RenderTarget0 BLOB, RenderTarget1 BLOB, RenderTarget2 BLOB, RenderTarget3 BLOB,
                RenderTarget4 BLOB, RenderTarget5 BLOB, RenderTarget6 BLOB, RenderTarget7 BLOB
            );
            CREATE TABLE depth_stencil_descs (
                Key BLOB, DepthEnable INTEGER, DepthWriteMask INTEGER, DepthFunc INTEGER,
                StencilEnable INTEGER, FrontFace BLOB, BackFace BLOB, DepthBoundsTestEnable INTEGER
            );
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_pso =
                tx.prepare("INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)")?;
            let mut ins_grp =
                tx.prepare("INSERT INTO groups (Key, Version, PSOKey) VALUES (?1, 1, ?2)")?;
            let mut ins_rast = tx.prepare(
                "INSERT INTO rasterizer_descs VALUES (?1, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0)",
            )?;
            for i in 0..50_000_u32 {
                let key = i.to_le_bytes();
                ins_pso.execute(rusqlite::params![&key])?;
                ins_grp.execute(rusqlite::params![&key, &key])?;
                ins_rast.execute(rusqlite::params![&key])?;
            }
        }
        tx.commit()?;
        Ok(())
    });

    let start = Instant::now();
    let summary = open(fixture.path()).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(summary.pipeline_states.len(), 50_000);
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "parse of 50k unindexed rows took {elapsed:?}"
    );
}

// Debug CI (especially Windows) is slower than a release-ish local run.
// 15s still fails the pre-fix hang (tens of seconds / abort).
const PARSE_BUDGET_SECS: f64 = 15.0;

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

fn so_key(i: u32) -> [u8; 4] {
    i.to_le_bytes()
}

#[test]
fn fifty_thousand_state_object_chain_finishes_quickly() {
    let _rss = RSS_MEASURE_LOCK.lock().unwrap();
    let make = |reverse: bool| {
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            conn.execute_batch(
                "CREATE TABLE state_objects (
                    Key BLOB, Type INTEGER, NodeMask INTEGER, Flags INTEGER,
                    AddToStateObjectParent BLOB
                );",
            )?;
            let tx = conn.transaction()?;
            {
                let mut ins = tx.prepare(
                    "INSERT INTO state_objects (Key, Type, NodeMask, Flags, AddToStateObjectParent)
                     VALUES (?1, 1, 0, 0, ?2)",
                )?;
                let range: Box<dyn Iterator<Item = u32>> = if reverse {
                    Box::new((0..50_000_u32).rev())
                } else {
                    Box::new(0..50_000_u32)
                };
                for i in range {
                    let key = so_key(i);
                    let parent = if i + 1 < 50_000 {
                        Some(so_key(i + 1))
                    } else {
                        None
                    };
                    ins.execute(rusqlite::params![&key, parent.as_ref()])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    };
    let fwd = make(false);
    let rev = make(true);
    let start = Instant::now();
    let a = open(fwd.path()).unwrap();
    let elapsed = start.elapsed();
    let b = open(rev.path()).unwrap();
    assert_eq!(a.state_objects.len(), 50_000);
    assert_eq!(a.state_objects[0].desc_hash, b.state_objects[0].desc_hash);
    eprintln!("50k SO chain parse: {elapsed:?}");
    assert!(
        elapsed.as_secs_f64() < PARSE_BUDGET_SECS,
        "50k SO chain took {elapsed:?}"
    );
}

#[test]
fn forty_deep_diamond_finishes_quickly() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "
            CREATE TABLE state_objects (
                Key BLOB, Type INTEGER, NodeMask INTEGER, Flags INTEGER,
                AddToStateObjectParent BLOB
            );
            CREATE TABLE so_to_existing_so_associations (
                StateObjectKey BLOB, ExistingStateObjectKey BLOB, ExportKey BLOB
            );
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_so = tx.prepare(
                "INSERT INTO state_objects (Key, Type, NodeMask, Flags) VALUES (?1, 1, 0, 0)",
            )?;
            let mut ins_assoc = tx.prepare(
                "INSERT INTO so_to_existing_so_associations (StateObjectKey, ExistingStateObjectKey)
                 VALUES (?1, ?2)",
            )?;
            for level in 0..40_u32 {
                for slot in 0..2_u32 {
                    let key = so_key(level * 2 + slot);
                    ins_so.execute(rusqlite::params![&key])?;
                    if level + 1 < 40 {
                        let child_a = so_key((level + 1) * 2);
                        let child_b = so_key((level + 1) * 2 + 1);
                        ins_assoc.execute(rusqlite::params![&key, &child_a])?;
                        ins_assoc.execute(rusqlite::params![&key, &child_b])?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    });
    let start = Instant::now();
    let summary = open(fixture.path()).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(summary.state_objects.len(), 80);
    eprintln!("40-deep diamond parse: {elapsed:?}");
    assert!(
        elapsed.as_secs_f64() < PARSE_BUDGET_SECS,
        "40-deep diamond took {elapsed:?}"
    );
}

#[test]
fn fifty_thousand_shared_layout_finishes_quickly() {
    let _rss = RSS_MEASURE_LOCK.lock().unwrap();
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "
            CREATE TABLE pipeline_states (Key BLOB, InputLayout BLOB, Flags INTEGER);
            CREATE TABLE input_element_descs (
                Key BLOB, SemanticName TEXT, SemanticIndex INTEGER, Format INTEGER,
                InputSlot INTEGER, AlignedByteOffset INTEGER, InputSlotClass INTEGER,
                InstanceDataStepRate INTEGER
            );
            CREATE TABLE input_layout_to_input_element_associations (
                InputLayoutKey BLOB, InputElementKey BLOB
            );
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_el = tx.prepare(
                "INSERT INTO input_element_descs VALUES (?1, 'POSITION', 0, 0, 0, 0, 0, 0)",
            )?;
            let mut ins_assoc = tx.prepare(
                "INSERT INTO input_layout_to_input_element_associations VALUES (?1, ?2)",
            )?;
            let mut ins_pso = tx.prepare(
                "INSERT INTO pipeline_states (Key, InputLayout, Flags) VALUES (?1, ?2, 0)",
            )?;
            let layout = so_key(0);
            for i in 0..50_000_u32 {
                let key = so_key(i);
                ins_el.execute(rusqlite::params![&key])?;
                ins_assoc.execute(rusqlite::params![&layout, &key])?;
                ins_pso.execute(rusqlite::params![&key, &layout])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let before = rss_bytes();
    let start = Instant::now();
    let summary = open(fixture.path()).unwrap();
    let elapsed = start.elapsed();
    let after = rss_bytes();
    assert_eq!(summary.pipeline_states.len(), 50_000);
    eprintln!("50k shared layout parse: {elapsed:?}");
    assert!(
        elapsed.as_secs_f64() < PARSE_BUDGET_SECS,
        "50k shared layout took {elapsed:?}"
    );
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "shared-layout RSS delta: {} bytes ({:.1} MiB)",
            delta,
            delta as f64 / (1024.0 * 1024.0)
        );
        assert!(
            delta < 256 * 1024 * 1024,
            "RSS grew by {delta} bytes (target < 256 MiB)"
        );
    }
}

#[test]
fn valid_file_opens_with_64_byte_caps() {
    assert!(
        SPEC_PIPELINE_STATES_SQL.len() > 1024,
        "fixture CREATE SQL must exceed the old 1 KiB SQLITE_LIMIT_LENGTH floor, got {}",
        SPEC_PIPELINE_STATES_SQL.len()
    );
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        replace_pipeline_states_with_spec_sql(conn)?;
        insert_valid_rows(conn)
    });
    let limits = Limits {
        max_blob_bytes: 64,
        max_key_bytes: 64,
        max_text_bytes: 64,
        ..Limits::DEFAULT
    };
    let summary = open_with_limits(fixture.path(), limits).unwrap();
    assert_eq!(summary.pipeline_states.len(), 1);
}

#[test]
fn text_and_blob_hash_differ() {
    let make = |as_text: bool| {
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            if as_text {
                conn.execute(
                    "INSERT INTO root_signatures (Key, value) VALUES (?1, 'hello')",
                    rusqlite::params![b"rs-x"],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)",
                    rusqlite::params![b"rs-x", b"hello"],
                )?;
            }
            conn.execute(
                "INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (?1, ?2, 0)",
                rusqlite::params![b"pso-same", b"rs-x"],
            )?;
            Ok(())
        })
    };
    let text = make(true);
    let blob = make(false);
    assert_ne!(
        open(text.path()).unwrap().pipeline_states[0].desc_hash,
        open(blob.path()).unwrap().pipeline_states[0].desc_hash
    );
}

#[test]
fn wal_header_without_frames_is_not_ignored_data() {
    let src = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        create_core_tables(conn)?;
        insert_valid_rows(conn)
    });
    let copy = temp_path();
    std::fs::copy(src.path(), &copy).unwrap();
    let wal = PathBuf::from(format!("{}-wal", copy.display()));
    std::fs::write(&wal, vec![0_u8; 32]).unwrap();
    let summary = open(&copy).unwrap();
    assert!(
        !summary.wal_ignored,
        "header-only WAL must not be treated as ignored frames"
    );
    let _ = std::fs::remove_file(&wal);
    let _ = std::fs::remove_file(&copy);
}

#[test]
fn nonempty_wal_sidecar_sets_wal_ignored() {
    let src = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        create_core_tables(conn)?;
        insert_valid_rows(conn)
    });
    let copy = temp_path();
    std::fs::copy(src.path(), &copy).unwrap();
    let wal = PathBuf::from(format!("{}-wal", copy.display()));
    std::fs::write(&wal, vec![0_u8; 4096]).unwrap();
    let summary = open(&copy).unwrap();
    assert!(summary.wal_ignored);
    let shm = PathBuf::from(format!("{}-shm", copy.display()));
    assert!(!shm.exists(), "open created a -shm sidecar");
    let _ = std::fs::remove_file(&wal);
    let _ = std::fs::remove_file(&copy);
}

#[test]
fn immutable_uri_encodes_unix_and_windows_paths() {
    let unix = crate::parser::immutable_uri_from_os_bytes(b"/tmp/foo bar.sodb", false);
    assert_eq!(unix, "file:/tmp/foo%20bar.sodb?immutable=1");
    let non_utf8 = crate::parser::immutable_uri_from_os_bytes(b"/tmp/foo\xffbar.sodb", false);
    assert_eq!(non_utf8, "file:/tmp/foo%FFbar.sodb?immutable=1");
    let drive = crate::parser::immutable_uri_from_os_bytes(br"C:\x\y.sodb", true);
    assert_eq!(drive, "file:///C:/x/y.sodb?immutable=1");
    let ext = crate::parser::immutable_uri_from_os_bytes(br"\\?\C:\x\y.sodb", true);
    assert_eq!(ext, "file:///C:/x/y.sodb?immutable=1");
    let unc = crate::parser::immutable_uri_from_os_bytes(br"\\?\UNC\server\share\a.sodb", true);
    assert_eq!(unc, "file:////server/share/a.sodb?immutable=1");
    let unpaired = crate::parser::wide_path_to_os_bytes(&[0xD800]);
    assert!(matches!(unpaired, Err(Error::InvalidWidePath)));
}

#[test]
fn null_app_version_is_invalid() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "CREATE TABLE app_id (
                id INTEGER, exe TEXT, app_name TEXT, engine_name TEXT,
                app_version INTEGER, engine_version INTEGER
            );",
        )?;
        conn.execute(
            "INSERT INTO app_id (id, exe, app_name, app_version, engine_version)
             VALUES (1, 'e', 'n', NULL, 0)",
            [],
        )?;
        Ok(())
    });
    let err = open(fixture.path()).unwrap_err();
    assert!(
        matches!(
            err,
            Error::InvalidValue {
                table: TableId::AppId,
                found: 0,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn state_object_parent_cycle_is_a_typed_error() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "CREATE TABLE state_objects (
                Key BLOB, Type INTEGER, NodeMask INTEGER, Flags INTEGER,
                AddToStateObjectParent BLOB
            );",
        )?;
        conn.execute(
            "INSERT INTO state_objects VALUES (?1, 1, 0, 0, ?2)",
            rusqlite::params![b"so-a", b"so-b"],
        )?;
        conn.execute(
            "INSERT INTO state_objects VALUES (?1, 1, 0, 0, ?2)",
            rusqlite::params![b"so-b", b"so-a"],
        )?;
        Ok(())
    });
    let err = open(fixture.path()).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CycleDetected {
                table: TableId::StateObjects
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn sqlite_master_rows_are_not_charged_to_total_budget() {
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        create_core_tables(conn)?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"a"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"b"],
        )?;
        conn.execute(
            "INSERT INTO pipeline_states (Key, Flags) VALUES (?1, 0)",
            rusqlite::params![b"c"],
        )?;
        Ok(())
    });
    let limits = Limits {
        max_total_rows: 3,
        ..Limits::DEFAULT
    };
    let summary = open_with_limits(fixture.path(), limits).unwrap();
    assert_eq!(summary.pipeline_states.len(), 3);
}

#[test]
fn dxil_lib_insert_order_does_not_change_bytecode_hashes() {
    let make = |first: &[u8], second: &[u8]| {
        let a = first.to_vec();
        let b = second.to_vec();
        create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, move |conn| {
            create_core_tables(conn)?;
            conn.execute(
                "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
                rusqlite::params![b"lib-a", FAKE_VS],
            )?;
            conn.execute(
                "INSERT INTO shader_bytecode (Key, Bytecode) VALUES (?1, ?2)",
                rusqlite::params![b"lib-b", FAKE_PS],
            )?;
            conn.execute(
                "INSERT INTO state_objects (Key, Type, NodeMask, Flags) VALUES (?1, 1, 0, 0)",
                rusqlite::params![b"so-same"],
            )?;
            conn.execute(
                "INSERT INTO so_to_dxil_lib_associations (StateObjectKey, DxilLibKey) VALUES (?1, ?2)",
                rusqlite::params![b"so-same", &a],
            )?;
            conn.execute(
                "INSERT INTO so_to_dxil_lib_associations (StateObjectKey, DxilLibKey) VALUES (?1, ?2)",
                rusqlite::params![b"so-same", &b],
            )?;
            Ok(())
        })
    };
    let fwd = make(b"lib-a", b"lib-b");
    let rev = make(b"lib-b", b"lib-a");
    let left = open(fwd.path()).unwrap();
    let right = open(rev.path()).unwrap();
    assert_eq!(
        left.state_objects[0].bytecode_hashes,
        right.state_objects[0].bytecode_hashes
    );
    assert_eq!(left.state_objects[0].bytecode_hashes.len(), 2);
}

const LARGE_KEY_MEMORY_LIMIT: u64 = 512 * 1024 * 1024;

#[test]
fn fifty_thousand_wide_keys_with_empty_assocs_stay_under_memory_cap() {
    let _rss = RSS_MEASURE_LOCK.lock().unwrap();
    const ROWS: u32 = 50_000;
    const KEY_LEN: usize = 4 * 1024;
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "CREATE TABLE state_objects (
                Key BLOB, Type INTEGER, NodeMask INTEGER, Flags INTEGER,
                AddToStateObjectParent BLOB
            );",
        )?;
        conn.execute_batch(EMPTY_SO_ASSOC_TABLES_SQL)?;
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO state_objects (Key, Type, NodeMask, Flags) VALUES (?1, 1, 0, 0)",
            )?;
            let mut key = vec![0_u8; KEY_LEN];
            for i in 0..ROWS {
                key[..4].copy_from_slice(&i.to_le_bytes());
                ins.execute(rusqlite::params![&key])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let before = rss_bytes();
    let start = Instant::now();
    let summary = open(fixture.path()).unwrap();
    let elapsed = start.elapsed();
    let after = rss_bytes();
    assert_eq!(summary.state_objects.len(), ROWS as usize);
    eprintln!("50k 4KiB-key empty-assoc parse: {elapsed:?}");
    assert!(
        elapsed.as_secs_f64() < 30.0,
        "50k 4KiB-key empty-assoc parse took {elapsed:?}"
    );
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "50k 4KiB-key empty-assoc RSS delta: {} bytes ({:.1} MiB)",
            delta,
            delta as f64 / (1024.0 * 1024.0)
        );
        assert!(
            delta < LARGE_KEY_MEMORY_LIMIT,
            "RSS grew by {delta} bytes (limit {LARGE_KEY_MEMORY_LIMIT})"
        );
    }
}

const ROOT_SIG_BLOB_MEMORY_LIMIT: u64 = 64 * 1024 * 1024;

#[test]
fn thirty_large_root_signatures_stay_under_memory_cap() {
    let _rss = RSS_MEASURE_LOCK.lock().unwrap();
    const ROWS: usize = 30;
    const BLOB_LEN: usize = 8 * 1024 * 1024;
    let fixture = create_sodb(SODB_USER_VERSION, SODB_APPLICATION_ID, |conn| {
        conn.execute_batch(
            "
            CREATE TABLE root_signatures (Key BLOB, value BLOB);
            CREATE TABLE pipeline_states (Key BLOB, RootSignature BLOB, Flags INTEGER);
            ",
        )?;
        let tx = conn.transaction()?;
        {
            let mut ins_rs =
                tx.prepare("INSERT INTO root_signatures (Key, value) VALUES (?1, ?2)")?;
            let mut blob = vec![0_u8; BLOB_LEN];
            for i in 0..ROWS {
                let key = (i as u32).to_le_bytes();
                blob[0] = i as u8;
                ins_rs.execute(rusqlite::params![&key, &blob])?;
            }
            tx.prepare(
                "INSERT INTO pipeline_states (Key, RootSignature, Flags) VALUES (?1, ?2, 0)",
            )?
            .execute(rusqlite::params![
                &0_u32.to_le_bytes(),
                &0_u32.to_le_bytes()
            ])?;
        }
        tx.commit()?;
        Ok(())
    });
    let before = rss_bytes();
    let summary = open(fixture.path()).unwrap();
    let after = rss_bytes();
    assert_eq!(summary.pipeline_states.len(), 1);
    if let (Some(before), Some(after)) = (before, after) {
        let delta = after.saturating_sub(before);
        eprintln!(
            "30x8MiB root_signatures RSS delta: {} bytes ({:.1} MiB)",
            delta,
            delta as f64 / (1024.0 * 1024.0)
        );
        assert!(
            delta < ROOT_SIG_BLOB_MEMORY_LIMIT,
            "RSS grew by {delta} bytes (limit {ROOT_SIG_BLOB_MEMORY_LIMIT})"
        );
    }
}
