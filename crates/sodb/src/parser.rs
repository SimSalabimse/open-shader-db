// SPDX-License-Identifier: MIT

//! Read-only SODB parser. Bytecode is hashed and discarded.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, DatabaseName, OpenFlags};
use sha2::{Digest, Sha256};

use crate::extract::{
    DescriptorBlob, ExtractedBlend, ExtractedDepthStencil, ExtractedDepthStencilOp,
    ExtractedInputElement, ExtractedPipelineState, ExtractedRasterizer, ExtractedRenderTargetBlend,
    ExtractedRenderTargetFormats, ExtractedSoDeclaration, ExtractedStageHashes, ExtractedStreamOut,
    ExtractedViewInstancing,
};
use crate::hash::CanonicalHasher;
use crate::schema::{
    keyed_spec, APP_ID_COLUMNS, ASSOC_SPECS, COL_BYTECODE, COL_DXIL_LIB_KEY, COL_KEY, COL_PSO_KEY,
    COL_SO_KEY, COL_VERSION, KEYED_SPECS, PIPELINE_BYTECODE_COLUMNS, PIPELINE_DESC_COLUMNS,
    PIPELINE_KEYED_FOLLOWS, REPORTED_IF_MISSING, SODB_APPLICATION_ID, SODB_USER_VERSION,
};
use crate::{
    AppIdentity, AppKey, AppText, Error, HashedBytecode, HashedObject, Limits, ObjectKind,
    ObjectVersion, Sha256Hash, SodbSummary, TableId,
};

/// Read `PRAGMA`-equivalent header fields without opening tables or hashing blobs.
///
/// This reads the 100-byte SQLite header only. It does not parse schema, load
/// rows, or retain bytecode. Use [`is_sodb`] when the caller only needs a yes/no.
#[must_use = "header probe result should be used"]
pub fn probe(path: impl AsRef<Path>) -> Result<crate::SodbHeader, Error> {
    let path = path.as_ref();
    let mut file = open_read_file(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(Error::NotARegularFile);
    }
    if opened.len() < 100 {
        return Err(Error::NotAnSodb);
    }
    let mut header = [0_u8; 100];
    file.read_exact(&mut header).map_err(|_| Error::NotAnSodb)?;
    if header.get(..16) != Some(b"SQLite format 3\0") {
        return Err(Error::NotAnSodb);
    }
    Ok(crate::SodbHeader {
        user_version: i32::from_be_bytes([header[60], header[61], header[62], header[63]]),
        application_id: i32::from_be_bytes([header[68], header[69], header[70], header[71]]),
    })
}

/// `true` when `path` is a regular file whose SQLite header is SODB v2.
#[must_use]
pub fn is_sodb(path: impl AsRef<Path>) -> bool {
    match probe(path) {
        Ok(header) => {
            header.user_version == SODB_USER_VERSION && header.application_id == SODB_APPLICATION_ID
        }
        Err(_) => false,
    }
}

/// Open `path` with [`Limits::DEFAULT`].
#[must_use = "parsed SODB summary should be used"]
pub fn open(path: impl AsRef<Path>) -> Result<SodbSummary, Error> {
    open_with_limits(path, Limits::DEFAULT)
}

/// Open `path` with caller-supplied resource caps.
#[must_use = "parsed SODB summary should be used"]
pub fn open_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<SodbSummary, Error> {
    let opened = open_db(path.as_ref(), limits)?;
    summary_from_opened(opened, limits)
}

/// Extract pipeline-state descriptors and per-stage bytecode hashes.
///
/// Uses the same read-only open, size/row limits, cycle detection, and key
/// intern as [`open`]. DXIL bytes are hashed incrementally and discarded.
#[must_use = "extracted pipeline states should be used"]
pub fn extract_pipeline_states(
    path: impl AsRef<Path>,
) -> Result<Vec<ExtractedPipelineState>, Error> {
    extract_pipeline_states_with_limits(path, Limits::DEFAULT)
}

/// Extract pipeline states with caller-supplied resource caps.
#[must_use = "extracted pipeline states should be used"]
pub fn extract_pipeline_states_with_limits(
    path: impl AsRef<Path>,
    limits: Limits,
) -> Result<Vec<ExtractedPipelineState>, Error> {
    let opened = open_db(path.as_ref(), limits)?;
    extract_from_opened(opened, limits)
}

struct OpenedSodb {
    conn: Connection,
    catalog: Catalog,
    bytecode_by_key: BTreeMap<AppKey, Sha256Hash>,
    wal_ignored: bool,
    budget: Budget,
}

fn open_db(path: &Path, limits: Limits) -> Result<OpenedSodb, Error> {
    let header = inspect_path(path, limits.max_file_bytes)?;

    let conn = if header.wal {
        let uri = immutable_uri(path)?;
        Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(Error::from_open)?
    } else {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(Error::from_open)?
    };

    // SQLITE_LIMIT_LENGTH also applies to schema strings. Floor at 1 MiB so
    // tiny test caps still open a valid spec-schema file.
    const SQLITE_LENGTH_FLOOR: usize = 1 << 20;
    let length_cap = i32::try_from(
        limits
            .max_blob_bytes
            .max(limits.max_text_bytes)
            .max(limits.max_key_bytes)
            .max(SQLITE_LENGTH_FLOOR)
            .saturating_add(1),
    )
    .unwrap_or(i32::MAX);
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, length_cap);

    if let Err(source) = conn.pragma_update(None, "trusted_schema", false) {
        return Err(Error::from_open(source));
    }

    let user_version = pragma_i64(&conn, "user_version")?;
    if user_version != i64::from(SODB_USER_VERSION) {
        return Err(Error::UnsupportedUserVersion {
            found: user_version,
            expected: SODB_USER_VERSION,
        });
    }

    let application_id = pragma_i64(&conn, "application_id")?;
    if application_id != i64::from(SODB_APPLICATION_ID) {
        return Err(Error::UnsupportedApplicationId {
            found: application_id,
            expected: SODB_APPLICATION_ID,
        });
    }

    let mut budget = Budget::new(limits);
    let catalog = Catalog::load(&conn, &mut budget, limits)?;

    let bytecode_by_key = if catalog.present.contains(&TableId::ShaderBytecode) {
        read_shader_bytecode(&conn, &catalog, &mut budget, limits)?
    } else {
        BTreeMap::new()
    };

    Ok(OpenedSodb {
        conn,
        catalog,
        bytecode_by_key,
        wal_ignored: header.wal_ignored,
        budget,
    })
}

fn summary_from_opened(opened: OpenedSodb, limits: Limits) -> Result<SodbSummary, Error> {
    let OpenedSodb {
        conn,
        catalog,
        bytecode_by_key,
        wal_ignored,
        mut budget,
    } = opened;
    let missing_tables = REPORTED_IF_MISSING
        .iter()
        .copied()
        .filter(|table| !catalog.present.contains(table))
        .collect();

    let app_id = if catalog.present.contains(&TableId::AppId) {
        read_app_id(&conn, &catalog, &mut budget, limits)?
    } else {
        Vec::new()
    };

    let pipeline_states = if catalog.present.contains(&TableId::PipelineStates) {
        read_pipeline_states(&conn, &catalog, &mut budget, &bytecode_by_key, limits)?
    } else {
        Vec::new()
    };

    let state_objects = if catalog.present.contains(&TableId::StateObjects) {
        emit_state_objects(&catalog, &bytecode_by_key)?
    } else {
        Vec::new()
    };

    let shader_bytecode = bytecode_by_key
        .into_iter()
        .map(|(key, bytecode_hash)| HashedBytecode { key, bytecode_hash })
        .collect();

    Ok(SodbSummary {
        user_version: SODB_USER_VERSION,
        application_id: SODB_APPLICATION_ID,
        wal_ignored,
        app_id,
        missing_tables,
        pipeline_states,
        state_objects,
        shader_bytecode,
    })
}

fn extract_from_opened(
    opened: OpenedSodb,
    limits: Limits,
) -> Result<Vec<ExtractedPipelineState>, Error> {
    let OpenedSodb {
        conn,
        catalog,
        bytecode_by_key,
        wal_ignored: _,
        mut budget,
    } = opened;
    if !catalog.present.contains(&TableId::PipelineStates) {
        return Ok(Vec::new());
    }
    read_pipeline_extract(&conn, &catalog, &mut budget, &bytecode_by_key, limits)
}

struct FileHeader {
    wal: bool,
    wal_ignored: bool,
}

fn sidecar_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// SQLite WAL header is 32 bytes; frames follow. A header-only sibling is not
/// ignored unflushed data.
const WAL_HEADER_BYTES: u64 = 32;

fn wal_sidecar_has_frames(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() > WAL_HEADER_BYTES)
        .unwrap_or(false)
}

fn opened_path_for_wal(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Open for read. On Unix uses `O_NOFOLLOW | O_NONBLOCK | O_NOCTTY` so a
/// FIFO or symlink does not block or redirect. File type is taken from the
/// opened handle.
fn open_read_file(path: &Path) -> Result<File, Error> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(Error::NotARegularFile);
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::NotARegularFile);
        }
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(Error::NotARegularFile);
        }
        Ok(file)
    }
}

fn inspect_path(path: &Path, max_file_bytes: u64) -> Result<FileHeader, Error> {
    let file = open_read_file(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(Error::NotARegularFile);
    }
    let got = opened.len();
    if got > max_file_bytes {
        return Err(Error::FileTooLarge {
            max: max_file_bytes,
            got,
        });
    }
    if got < 100 {
        return Err(Error::NotAnSodb);
    }
    let mut header = [0_u8; 20];
    let mut file = file;
    file.read_exact(&mut header).map_err(|_| Error::NotAnSodb)?;
    if header.get(..16) != Some(b"SQLite format 3\0") {
        return Err(Error::NotAnSodb);
    }
    let write_ver = header.get(18).copied().unwrap_or(0);
    let read_ver = header.get(19).copied().unwrap_or(0);
    let wal = write_ver == 2 || read_ver == 2;
    // WAL opens use the canonical path; look for `-wal` next to that file.
    let wal_base = if wal {
        opened_path_for_wal(path)
    } else {
        path.to_path_buf()
    };
    let wal_ignored = wal && wal_sidecar_has_frames(&sidecar_with_suffix(&wal_base, "-wal"));
    Ok(FileHeader { wal, wal_ignored })
}

fn hex_upper(nibble: u8) -> char {
    let nibble = nibble & 0x0f;
    if nibble < 10 {
        char::from(b'0' + nibble)
    } else {
        char::from(b'A' + (nibble - 10))
    }
}

fn append_encoded(uri: &mut String, bytes: &[u8], windows: bool) {
    for &byte in bytes {
        match byte {
            b'\\' if windows => uri.push('/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'.' | b'-' | b'_' => {
                uri.push(char::from(byte));
            }
            b => {
                uri.push('%');
                uri.push(hex_upper(b >> 4));
                uri.push(hex_upper(b & 0x0f));
            }
        }
    }
}

/// Build `file:<path>?immutable=1` from absolute OS path bytes.
///
/// `windows` selects drive / UNC rules (`\\?\C:\` and `\\?\UNC\server\share`).
pub(crate) fn immutable_uri_from_os_bytes(bytes: &[u8], windows: bool) -> String {
    let mut rest = bytes;
    if windows {
        if rest.starts_with(br"\\?\") {
            rest = rest.get(4..).unwrap_or(&[]);
        }
        let unc = rest.starts_with(b"UNC\\") || rest.starts_with(b"UNC/");
        if unc {
            let after = rest.get(4..).unwrap_or(&[]);
            // Bundled SQLite is not built with SQLITE_ALLOW_URI_AUTHORITY, so
            // `file://server/share` is rejected. Four slashes keep the authority
            // empty and put the UNC path in the URI path (`file:////server/...`).
            let mut uri = String::from("file:////");
            append_encoded(&mut uri, after, true);
            uri.push_str("?immutable=1");
            return uri;
        }
    }

    let mut uri = String::from("file:");
    let is_drive = windows
        && rest.first().is_some_and(|c| c.is_ascii_alphabetic())
        && rest.get(1) == Some(&b':');
    if is_drive {
        uri.push('/');
        uri.push('/');
        uri.push('/');
    } else if !rest.first().is_some_and(|c| *c == b'/' || *c == b'\\') {
        uri.push('/');
    }
    append_encoded(&mut uri, rest, windows);
    uri.push_str("?immutable=1");
    uri
}

/// Convert Windows wide path units to UTF-8 bytes. Unpaired surrogates are an
/// error so the URI cannot silently open a different path via U+FFFD.
#[cfg(any(test, windows))]
pub(crate) fn wide_path_to_os_bytes(wide: &[u16]) -> Result<Vec<u8>, Error> {
    String::from_utf16(wide)
        .map(String::into_bytes)
        .map_err(|_| Error::InvalidWidePath)
}

fn immutable_uri(path: &Path) -> Result<String, Error> {
    let abs = std::fs::canonicalize(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(immutable_uri_from_os_bytes(
            abs.as_os_str().as_bytes(),
            false,
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = abs.as_os_str().encode_wide().collect();
        let bytes = wide_path_to_os_bytes(&wide)?;
        Ok(immutable_uri_from_os_bytes(&bytes, true))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(immutable_uri_from_os_bytes(
            abs.to_string_lossy().as_bytes(),
            false,
        ))
    }
}

fn pragma_i64(conn: &Connection, name: &str) -> Result<i64, Error> {
    conn.pragma_query_value(None, name, |row| row.get(0))
        .map_err(Error::from_open)
}

struct Budget {
    limits: Limits,
    total: u64,
}

impl Budget {
    fn new(limits: Limits) -> Self {
        Self { limits, total: 0 }
    }

    fn charge(&mut self, table: TableId, n: u64) -> Result<(), Error> {
        if n > self.limits.max_rows_per_table {
            return Err(Error::TooManyRows {
                table,
                max: self.limits.max_rows_per_table,
                got: n,
            });
        }
        self.total = self.total.saturating_add(n);
        if self.total > self.limits.max_total_rows {
            return Err(Error::TooManyTotalRows {
                max: self.limits.max_total_rows,
                got: self.total,
            });
        }
        Ok(())
    }
}

enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

struct NamedCell {
    name: String,
    value: Cell,
}

/// Interned application-key id. Bytes live once in [`KeyIntern`].
type KeyId = u32;

struct KeyIntern {
    bytes: Vec<Box<[u8]>>,
    /// Content hash → interned ids (short collision lists).
    index: HashMap<u64, Vec<KeyId>>,
}

impl KeyIntern {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn intern(&mut self, bytes: &[u8]) -> Result<KeyId, Error> {
        let h = hash_key_bytes(bytes);
        if let Some(ids) = self.index.get(&h) {
            for &id in ids {
                if self.bytes_for(id)? == bytes {
                    return Ok(id);
                }
            }
        }
        let id = u32::try_from(self.bytes.len()).map_err(|_| Error::TooManyTotalRows {
            max: u64::from(u32::MAX),
            got: self.bytes.len() as u64,
        })?;
        self.bytes.push(bytes.to_vec().into_boxed_slice());
        self.index.entry(h).or_default().push(id);
        Ok(id)
    }

    fn find(&self, bytes: &[u8]) -> Result<Option<KeyId>, Error> {
        let h = hash_key_bytes(bytes);
        let Some(ids) = self.index.get(&h) else {
            return Ok(None);
        };
        for &id in ids {
            if self.bytes_for(id)? == bytes {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn bytes_for(&self, id: KeyId) -> Result<&[u8], Error> {
        self.bytes
            .get(id as usize)
            .map(|b| b.as_ref())
            .ok_or(Error::InternInconsistency)
    }

    fn get(&self, id: KeyId) -> Result<&[u8], Error> {
        self.bytes_for(id)
    }

    fn app_key(&self, id: KeyId) -> Result<AppKey, Error> {
        Ok(AppKey::new(self.get(id)?.to_vec()))
    }
}

fn hash_key_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Copy)]
enum Follow {
    Keyed { table: TableId, key: KeyId },
    Assoc { table: TableId, owner: KeyId },
}

impl Follow {
    fn id(self) -> (TableId, KeyId) {
        match self {
            Follow::Keyed { table, key } => (table, key),
            Follow::Assoc { table, owner } => (table, owner),
        }
    }
}

struct KeyedNode {
    own: Sha256Hash,
    follows: Vec<Follow>,
}

struct AssocNode {
    own: Sha256Hash,
    follows: Vec<Follow>,
}

struct Catalog {
    intern: KeyIntern,
    present: HashSet<TableId>,
    columns: HashMap<TableId, Vec<String>>,
    keyed: HashMap<TableId, HashMap<KeyId, KeyedNode>>,
    assoc: HashMap<TableId, HashMap<KeyId, Vec<AssocNode>>>,
    pso_version: HashMap<KeyId, ObjectVersion>,
    so_version: HashMap<KeyId, ObjectVersion>,
    subhashes: HashMap<TableId, HashMap<KeyId, Sha256Hash>>,
    so_dxil: HashMap<KeyId, Vec<KeyId>>,
}

impl Catalog {
    fn load(conn: &Connection, budget: &mut Budget, limits: Limits) -> Result<Self, Error> {
        let present = listed_tables(conn, budget)?;
        let mut columns = HashMap::new();
        for table in &present {
            columns.insert(*table, column_names(conn, *table)?);
        }

        let mut catalog = Self {
            intern: KeyIntern::new(),
            present,
            columns,
            keyed: HashMap::new(),
            assoc: HashMap::new(),
            pso_version: HashMap::new(),
            so_version: HashMap::new(),
            subhashes: HashMap::new(),
            so_dxil: HashMap::new(),
        };

        if catalog.present.contains(&TableId::Groups) {
            load_groups(conn, &mut catalog, budget, limits)?;
        }
        for spec in KEYED_SPECS {
            if catalog.present.contains(&spec.table) {
                load_keyed_table(conn, &mut catalog, spec.table, spec.columns, budget, limits)?;
            }
        }
        for spec in ASSOC_SPECS {
            if catalog.present.contains(&spec.table) {
                load_assoc_table(conn, &mut catalog, spec, budget, limits)?;
            }
        }
        catalog.attach_existing_assoc_follows();
        catalog.sort_so_dxil()?;
        catalog.finalize_subhashes()?;
        Ok(catalog)
    }

    fn cols(&self, table: TableId) -> &[String] {
        self.columns.get(&table).map_or(&[], Vec::as_slice)
    }

    fn subhash(&self, table: TableId, key: &AppKey) -> Result<Sha256Hash, Error> {
        if let Some(id) = self.intern.find(key.as_bytes())? {
            if let Some(hash) = self.subhashes.get(&table).and_then(|m| m.get(&id)) {
                return Ok(*hash);
            }
        }
        Ok(miss_hash(table, key.as_bytes()))
    }

    fn attach_existing_assoc_follows(&mut self) {
        let present_assoc: HashSet<(TableId, KeyId)> = self
            .assoc
            .iter()
            .flat_map(|(table, owners)| owners.keys().map(|id| (*table, *id)))
            .collect();

        for map in self.keyed.values_mut() {
            for node in map.values_mut() {
                node.follows.retain(|follow| match follow {
                    Follow::Assoc { table, owner } => present_assoc.contains(&(*table, *owner)),
                    Follow::Keyed { .. } => true,
                });
            }
        }

        for spec in KEYED_SPECS {
            let Some(map) = self.keyed.get_mut(&spec.table) else {
                continue;
            };
            for (key_id, node) in map.iter_mut() {
                for &assoc_id in spec.assocs {
                    if present_assoc.contains(&(assoc_id, *key_id)) {
                        node.follows.push(Follow::Assoc {
                            table: assoc_id,
                            owner: *key_id,
                        });
                    }
                }
            }
        }
    }

    fn sort_so_dxil(&mut self) -> Result<(), Error> {
        for keys in &mut self.so_dxil {
            let intern = &self.intern;
            let mut keyed = Vec::with_capacity(keys.1.len());
            for id in keys.1.iter() {
                keyed.push((intern.get(*id)?.to_vec(), *id));
            }
            keyed.sort_by(|a, b| a.0.cmp(&b.0));
            *keys.1 = keyed.into_iter().map(|(_, id)| id).collect();
        }
        Ok(())
    }

    fn finalize_subhashes(&mut self) -> Result<(), Error> {
        let mut starts = Vec::new();
        for (table, rows) in &self.keyed {
            for &key in rows.keys() {
                starts.push(Follow::Keyed { table: *table, key });
            }
        }
        for (table, owners) in &self.assoc {
            for &owner in owners.keys() {
                starts.push(Follow::Assoc {
                    table: *table,
                    owner,
                });
            }
        }
        let mut cache: HashMap<(TableId, KeyId), Sha256Hash> = HashMap::new();
        for start in starts {
            compute_subhash(self, &mut cache, start)?;
        }
        let mut nested: HashMap<TableId, HashMap<KeyId, Sha256Hash>> = HashMap::new();
        for ((table, key), hash) in cache {
            nested.entry(table).or_default().insert(key, hash);
        }
        self.subhashes = nested;
        self.keyed.clear();
        self.assoc.clear();
        Ok(())
    }
}

fn miss_hash(table: TableId, key: &[u8]) -> Sha256Hash {
    let mut hasher = CanonicalHasher::new();
    hasher.tag(b"follow-miss");
    hasher.tag(table.sql_name().as_bytes());
    hasher.bytes(key);
    hasher.finish()
}

type SubHashCache = HashMap<(TableId, KeyId), Sha256Hash>;

fn cache_get(cache: &SubHashCache, table: TableId, key: KeyId) -> Option<Sha256Hash> {
    cache.get(&(table, key)).copied()
}

fn cache_insert(cache: &mut SubHashCache, table: TableId, key: KeyId, hash: Sha256Hash) {
    cache.insert((table, key), hash);
}

fn lookup_hash(
    catalog: &Catalog,
    cache: &SubHashCache,
    table: TableId,
    key: KeyId,
) -> Result<Sha256Hash, Error> {
    if let Some(hash) = cache_get(cache, table, key) {
        return Ok(hash);
    }
    Ok(miss_hash(table, catalog.intern.get(key)?))
}

fn follow_exists(catalog: &Catalog, follow: Follow) -> bool {
    match follow {
        Follow::Keyed { table, key } => catalog
            .keyed
            .get(&table)
            .is_some_and(|m| m.contains_key(&key)),
        Follow::Assoc { table, owner } => catalog
            .assoc
            .get(&table)
            .is_some_and(|m| m.contains_key(&owner)),
    }
}

fn children_of(catalog: &Catalog, follow: Follow) -> Vec<Follow> {
    match follow {
        Follow::Keyed { table, key } => catalog
            .keyed
            .get(&table)
            .and_then(|m| m.get(&key))
            .map(|n| n.follows.clone())
            .unwrap_or_default(),
        Follow::Assoc { table, owner } => catalog
            .assoc
            .get(&table)
            .and_then(|m| m.get(&owner))
            .map(|rows| {
                rows.iter()
                    .flat_map(|r| r.follows.iter().copied())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn finish_hash(
    catalog: &Catalog,
    cache: &SubHashCache,
    follow: Follow,
) -> Result<Sha256Hash, Error> {
    match follow {
        Follow::Keyed { table, key } => {
            let Some(node) = catalog.keyed.get(&table).and_then(|m| m.get(&key)) else {
                return Ok(miss_hash(table, catalog.intern.get(key)?));
            };
            let mut hasher = CanonicalHasher::new();
            hasher.tag(b"keyed-sub");
            hasher.bytes(node.own.as_bytes());
            for child in &node.follows {
                hasher.tag(b"follow");
                let (t, k) = child.id();
                hasher.bytes(lookup_hash(catalog, cache, t, k)?.as_bytes());
            }
            Ok(hasher.finish())
        }
        Follow::Assoc { table, owner } => {
            let mut hasher = CanonicalHasher::new();
            hasher.tag(b"assoc-sub");
            hasher.tag(table.sql_name().as_bytes());
            hasher.bytes(catalog.intern.get(owner)?);
            match catalog.assoc.get(&table).and_then(|m| m.get(&owner)) {
                None => hasher.tag(b"assoc-miss"),
                Some(rows) => {
                    for row in rows {
                        hasher.tag(b"assoc-row");
                        hasher.bytes(row.own.as_bytes());
                        for child in &row.follows {
                            hasher.tag(b"follow");
                            let (t, k) = child.id();
                            hasher.bytes(lookup_hash(catalog, cache, t, k)?.as_bytes());
                        }
                    }
                }
            }
            Ok(hasher.finish())
        }
    }
}

#[derive(Clone, Copy)]
enum Task {
    Enter(Follow),
    Exit(Follow),
}

fn compute_subhash(
    catalog: &Catalog,
    cache: &mut SubHashCache,
    start: Follow,
) -> Result<Sha256Hash, Error> {
    let (start_table, start_key) = start.id();
    if let Some(hash) = cache_get(cache, start_table, start_key) {
        return Ok(hash);
    }
    let mut stack = vec![Task::Enter(start)];
    let mut gray = HashSet::new();
    while let Some(task) = stack.pop() {
        match task {
            Task::Enter(follow) => {
                let (table, key) = follow.id();
                if cache_get(cache, table, key).is_some() {
                    continue;
                }
                if !gray.insert((table, key)) {
                    return Err(Error::CycleDetected { table });
                }
                let kids = children_of(catalog, follow);
                stack.push(Task::Exit(follow));
                for child in kids.into_iter().rev() {
                    if !follow_exists(catalog, child) {
                        continue;
                    }
                    let (ct, ck) = child.id();
                    if cache_get(cache, ct, ck).is_none() {
                        stack.push(Task::Enter(child));
                    }
                }
            }
            Task::Exit(follow) => {
                let (table, key) = follow.id();
                let hash = match follow {
                    Follow::Keyed { table, key } => {
                        let node = catalog.keyed.get(&table).and_then(|m| m.get(&key));
                        match node {
                            Some(node) if node.follows.is_empty() => node.own,
                            _ => finish_hash(catalog, cache, follow)?,
                        }
                    }
                    Follow::Assoc { .. } => finish_hash(catalog, cache, follow)?,
                };
                gray.remove(&(table, key));
                cache_insert(cache, table, key, hash);
            }
        }
    }
    lookup_hash(catalog, cache, start_table, start_key)
}

fn listed_tables(conn: &Connection, _budget: &mut Budget) -> Result<HashSet<TableId>, Error> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(Error::from_open)?;
    let mut rows = stmt.query([]).map_err(Error::from_open)?;
    let mut present = HashSet::new();
    while let Some(row) = rows.next().map_err(Error::from_open)? {
        let name = row
            .get::<_, String>(0)
            .map_err(|source| Error::from_sqlite(TableId::SqliteMaster, source))?;
        if let Some(id) = TableId::from_sql_name(&name) {
            present.insert(id);
        }
    }
    Ok(present)
}

fn column_names(conn: &Connection, table: TableId) -> Result<Vec<String>, Error> {
    let sql = format!("PRAGMA table_info({})", quote_ident(table.sql_name()));
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut names = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let name = row
            .get::<_, String>(1)
            .map_err(|source| Error::from_sqlite(table, source))?;
        names.push(name);
    }
    Ok(names)
}

fn has_column(columns: &[String], wanted: &str) -> bool {
    columns.iter().any(|c| c.eq_ignore_ascii_case(wanted))
}

fn actual_column<'a>(columns: &'a [String], wanted: &str) -> Option<&'a str> {
    columns
        .iter()
        .find(|c| c.eq_ignore_ascii_case(wanted))
        .map(String::as_str)
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn row_count(conn: &Connection, table: TableId) -> Result<u64, Error> {
    let sql = format!("SELECT COUNT(*) FROM {}", quote_ident(table.sql_name()));
    let count = conn
        .query_row(&sql, [], |row| row.get::<_, i64>(0))
        .map_err(|source| Error::from_sqlite(table, source))?;
    if count < 0 {
        return Err(Error::InvalidValue {
            table,
            column: String::from("COUNT(*)"),
            found: count,
        });
    }
    Ok(count as u64)
}

fn sql_cutoff(limits: Limits) -> usize {
    limits
        .max_blob_bytes
        .max(limits.max_text_bytes)
        .max(limits.max_key_bytes)
}

#[derive(Clone)]
struct SelectedCol {
    name: String,
    typeof_idx: usize,
    len_idx: usize,
    val_idx: usize,
}

fn push_selected(parts: &mut Vec<String>, cols: &mut Vec<SelectedCol>, name: &str, max: usize) {
    let q = quote_ident(name);
    let typeof_idx = parts.len();
    parts.push(format!("typeof({q})"));
    let len_idx = parts.len();
    parts.push(format!("octet_length({q})"));
    let val_idx = parts.len();
    parts.push(format!(
        "CASE WHEN typeof({q}) IN ('blob','text') AND octet_length({q}) > {max} THEN NULL ELSE {q} END"
    ));
    cols.push(SelectedCol {
        name: name.to_string(),
        typeof_idx,
        len_idx,
        val_idx,
    });
}

struct SelectList {
    parts: Vec<String>,
    cols: Vec<SelectedCol>,
    key: Option<SelectedCol>,
}

impl SelectList {
    fn new() -> Self {
        Self {
            parts: Vec::new(),
            cols: Vec::new(),
            key: None,
        }
    }

    fn push(&mut self, name: &str, max: usize) -> SelectedCol {
        let typeof_idx = self.parts.len();
        push_selected(&mut self.parts, &mut self.cols, name, max);
        SelectedCol {
            name: name.to_string(),
            typeof_idx,
            len_idx: typeof_idx + 1,
            val_idx: typeof_idx + 2,
        }
    }

    fn push_key(&mut self, name: &str, max: usize) {
        self.key = Some(self.push(name, max));
    }

    fn sql(&self, table: TableId) -> String {
        format!(
            "SELECT {} FROM {}",
            self.parts.join(", "),
            quote_ident(table.sql_name())
        )
    }

    fn key_col(&self, table: TableId) -> Result<&SelectedCol, Error> {
        self.key.as_ref().ok_or(Error::MissingKeyColumn { table })
    }

    fn find(&self, name: &str) -> Option<&SelectedCol> {
        self.cols.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

fn enforce_len(
    table: TableId,
    name: &str,
    ty: &str,
    len: Option<i64>,
    limits: Limits,
) -> Result<(), Error> {
    let Some(len) = len else {
        return Ok(());
    };
    if len < 0 {
        return Err(Error::InvalidValue {
            table,
            column: name.to_string(),
            found: len,
        });
    }
    let got = len as u64;
    let is_key = name.eq_ignore_ascii_case(COL_KEY);
    if is_key && got > limits.max_key_bytes as u64 {
        return Err(Error::KeyTooLarge {
            table,
            max: limits.max_key_bytes as u64,
            got,
        });
    }
    match ty {
        "blob" if got > limits.max_blob_bytes as u64 => Err(Error::BlobTooLarge {
            table,
            max: limits.max_blob_bytes as u64,
            got,
        }),
        "text" if got > limits.max_text_bytes as u64 => Err(Error::TextTooLarge {
            table,
            max: limits.max_text_bytes as u64,
            got,
        }),
        _ => Ok(()),
    }
}

fn read_ty_len(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
) -> Result<(String, Option<i64>), Error> {
    let ty = row
        .get::<_, String>(col.typeof_idx)
        .map_err(|source| Error::from_sqlite(table, source))?;
    let len = row
        .get::<_, Option<i64>>(col.len_idx)
        .map_err(|source| Error::from_sqlite(table, source))?;
    Ok((ty, len))
}

fn read_selected_key(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
    limits: Limits,
) -> Result<AppKey, Error> {
    let (ty, len) = read_ty_len(row, col, table)?;
    match ty.as_str() {
        "null" => return Err(Error::NullKey { table }),
        "blob" => {}
        other => {
            return Err(Error::WrongColumnType {
                table,
                column: col.name.clone(),
                got: other.to_string(),
            });
        }
    }
    enforce_len(table, &col.name, &ty, len, limits)?;
    let bytes = row
        .get::<_, Option<Vec<u8>>>(col.val_idx)
        .map_err(|source| Error::from_sqlite(table, source))?;
    match bytes {
        Some(bytes) => Ok(AppKey::new(bytes)),
        None => Err(Error::NullKey { table }),
    }
}

fn read_opt_blob(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
    limits: Limits,
) -> Result<Option<Vec<u8>>, Error> {
    let (ty, len) = read_ty_len(row, col, table)?;
    enforce_len(table, &col.name, &ty, len, limits)?;
    match ty.as_str() {
        "null" => Ok(None),
        "blob" => row
            .get(col.val_idx)
            .map_err(|source| Error::from_sqlite(table, source)),
        other => Err(Error::WrongColumnType {
            table,
            column: col.name.clone(),
            got: other.to_string(),
        }),
    }
}

fn read_opt_text(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
    limits: Limits,
) -> Result<Option<AppText>, Error> {
    let (ty, len) = read_ty_len(row, col, table)?;
    enforce_len(table, &col.name, &ty, len, limits)?;
    match ty.as_str() {
        "null" => Ok(None),
        "text" => {
            let text = row
                .get::<_, Option<String>>(col.val_idx)
                .map_err(|source| Error::from_sqlite(table, source))?;
            Ok(text.map(AppText::new))
        }
        other => Err(Error::WrongColumnType {
            table,
            column: col.name.clone(),
            got: other.to_string(),
        }),
    }
}

fn read_opt_f64(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
    limits: Limits,
) -> Result<Option<f64>, Error> {
    let (ty, len) = read_ty_len(row, col, table)?;
    match ty.as_str() {
        "null" => Ok(None),
        "real" | "integer" => row
            .get(col.val_idx)
            .map_err(|source| Error::from_sqlite(table, source)),
        "text" => {
            enforce_len(table, &col.name, &ty, len, limits)?;
            Err(Error::WrongColumnType {
                table,
                column: col.name.clone(),
                got: String::from("text"),
            })
        }
        other => Err(Error::WrongColumnType {
            table,
            column: col.name.clone(),
            got: other.to_string(),
        }),
    }
}

fn read_opt_i64(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
    limits: Limits,
) -> Result<Option<i64>, Error> {
    let (ty, len) = read_ty_len(row, col, table)?;
    match ty.as_str() {
        "null" => Ok(None),
        "integer" => row
            .get(col.val_idx)
            .map_err(|source| Error::from_sqlite(table, source)),
        "text" => {
            enforce_len(table, &col.name, &ty, len, limits)?;
            Err(Error::WrongColumnType {
                table,
                column: col.name.clone(),
                got: String::from("text"),
            })
        }
        other => Err(Error::WrongColumnType {
            table,
            column: col.name.clone(),
            got: other.to_string(),
        }),
    }
}

fn require_u64(table: TableId, column: &str, value: Option<i64>) -> Result<u64, Error> {
    match value {
        None => Err(Error::InvalidValue {
            table,
            column: column.to_string(),
            found: 0,
        }),
        Some(v) => u64::try_from(v).map_err(|_| Error::InvalidValue {
            table,
            column: column.to_string(),
            found: v,
        }),
    }
}

fn object_version(table: TableId, column: &str, value: i64) -> Result<ObjectVersion, Error> {
    if (0..=i64::from(u32::MAX)).contains(&value) {
        Ok(ObjectVersion::new(value as u32))
    } else {
        Err(Error::InvalidValue {
            table,
            column: column.to_string(),
            found: value,
        })
    }
}

fn read_cell(
    row: &rusqlite::Row<'_>,
    col: &SelectedCol,
    table: TableId,
    limits: Limits,
) -> Result<NamedCell, Error> {
    let (ty, len) = read_ty_len(row, col, table)?;
    enforce_len(table, &col.name, &ty, len, limits)?;
    let value = match ty.as_str() {
        "null" => Cell::Null,
        "integer" => {
            let v = row
                .get::<_, i64>(col.val_idx)
                .map_err(|source| Error::from_sqlite(table, source))?;
            Cell::Integer(v)
        }
        "real" => {
            let v = row
                .get::<_, f64>(col.val_idx)
                .map_err(|source| Error::from_sqlite(table, source))?;
            Cell::Real(v)
        }
        "text" => {
            let v = row
                .get::<_, String>(col.val_idx)
                .map_err(|source| Error::from_sqlite(table, source))?;
            Cell::Text(v.into_bytes())
        }
        "blob" => {
            let v = row
                .get::<_, Vec<u8>>(col.val_idx)
                .map_err(|source| Error::from_sqlite(table, source))?;
            Cell::Blob(v)
        }
        other => {
            return Err(Error::WrongColumnType {
                table,
                column: col.name.clone(),
                got: other.to_string(),
            });
        }
    };
    Ok(NamedCell {
        name: col.name.clone(),
        value,
    })
}

fn feed_cell(hasher: &mut CanonicalHasher, value: &Cell) {
    match value {
        Cell::Null => hasher.feed_null(),
        Cell::Integer(v) => hasher.opt_i64(Some(*v)),
        Cell::Real(v) => hasher.opt_f64(Some(*v)),
        Cell::Text(t) => hasher.feed_text(t),
        Cell::Blob(b) => hasher.feed_blob(b),
    }
}

fn load_groups(
    conn: &Connection,
    catalog: &mut Catalog,
    budget: &mut Budget,
    limits: Limits,
) -> Result<(), Error> {
    let table = TableId::Groups;
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table).to_vec();
    if !has_column(&columns, COL_VERSION) {
        return Ok(());
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(&columns, COL_VERSION) {
        select.push(name, max);
    }
    let has_pso = actual_column(&columns, COL_PSO_KEY).map(|n| {
        select.push(n, max);
        true
    });
    let has_so = actual_column(&columns, COL_SO_KEY).map(|n| {
        select.push(n, max);
        true
    });
    if select.parts.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let version_col = select.find(COL_VERSION);
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let version = match version_col {
            Some(col) => match read_opt_i64(row, col, table, limits)? {
                Some(v) => object_version(table, COL_VERSION, v)?,
                None => ObjectVersion::new(0),
            },
            None => ObjectVersion::new(0),
        };
        if has_pso.is_some() {
            if let Some(col) = select.find(COL_PSO_KEY) {
                if let Some(key) = read_opt_blob(row, col, table, limits)? {
                    let id = catalog.intern.intern(&key)?;
                    catalog.pso_version.entry(id).or_insert(version);
                }
            }
        }
        if has_so.is_some() {
            if let Some(col) = select.find(COL_SO_KEY) {
                if let Some(key) = read_opt_blob(row, col, table, limits)? {
                    let id = catalog.intern.intern(&key)?;
                    catalog.so_version.entry(id).or_insert(version);
                }
            }
        }
    }
    Ok(())
}

fn keyed_fk_table(table: TableId, column: &str) -> Option<TableId> {
    keyed_spec(table).and_then(|spec| {
        spec.keyed_fks
            .iter()
            .find(|fk| fk.column.eq_ignore_ascii_case(column))
            .map(|fk| fk.table)
    })
}

fn load_keyed_table(
    conn: &Connection,
    catalog: &mut Catalog,
    table: TableId,
    wanted: &[&str],
    budget: &mut Budget,
    limits: Limits,
) -> Result<(), Error> {
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table).to_vec();
    if !has_column(&columns, COL_KEY) {
        return Err(Error::MissingKeyColumn { table });
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(&columns, COL_KEY) {
        select.push_key(name, max);
    }
    for col in wanted {
        if col.eq_ignore_ascii_case(COL_KEY) {
            continue;
        }
        if let Some(name) = actual_column(&columns, col) {
            if select.find(name).is_none() {
                select.push(name, max);
            }
        }
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut map = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let key = read_selected_key(row, select.key_col(table)?, table, limits)?;
        let key_id = catalog.intern.intern(key.as_bytes())?;
        if map.contains_key(&key_id) {
            return Err(Error::DuplicateKey { table });
        }
        let mut hasher = CanonicalHasher::new();
        hasher.tag(b"keyed-own");
        hasher.tag(table.sql_name().as_bytes());
        hasher.bytes(key.as_bytes());
        let mut follows = Vec::new();
        for col in &select.cols {
            let cell = read_cell(row, col, table, limits)?;
            hasher.tag(cell.name.as_bytes());
            feed_cell(&mut hasher, &cell.value);
            if let Cell::Blob(bytes) = &cell.value {
                let fk = keyed_fk_table(table, &cell.name);
                let il = cell.name.eq_ignore_ascii_case("InputLayout");
                if fk.is_none() && !il {
                    continue;
                }
                let child_id = catalog.intern.intern(bytes)?;
                if let Some(fk_table) = fk {
                    follows.push(Follow::Keyed {
                        table: fk_table,
                        key: child_id,
                    });
                }
                if il {
                    follows.push(Follow::Assoc {
                        table: TableId::InputLayoutToInputElementAssociations,
                        owner: child_id,
                    });
                }
            }
        }
        map.insert(
            key_id,
            KeyedNode {
                own: hasher.finish(),
                follows,
            },
        );
    }
    catalog.keyed.insert(table, map);
    Ok(())
}

fn load_assoc_table(
    conn: &Connection,
    catalog: &mut Catalog,
    spec: &crate::schema::AssocSpec,
    budget: &mut Budget,
    limits: Limits,
) -> Result<(), Error> {
    let table = spec.table;
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table).to_vec();
    if !has_column(&columns, spec.owner_col) {
        return Ok(());
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    for col in spec.columns {
        if let Some(name) = actual_column(&columns, col) {
            select.push(name, max);
        }
    }
    if select.parts.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut map: HashMap<KeyId, Vec<AssocNode>> = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let mut hasher = CanonicalHasher::new();
        hasher.tag(b"assoc-own");
        hasher.tag(table.sql_name().as_bytes());
        let mut follows = Vec::new();
        let mut owner = None;
        let mut dxil = None;
        for col in &select.cols {
            let cell = read_cell(row, col, table, limits)?;
            hasher.tag(cell.name.as_bytes());
            feed_cell(&mut hasher, &cell.value);
            if let Cell::Blob(bytes) = &cell.value {
                if cell.name.eq_ignore_ascii_case(spec.owner_col) {
                    owner = Some(catalog.intern.intern(bytes)?);
                }
                if table == TableId::SoToDxilLibAssociations
                    && cell.name.eq_ignore_ascii_case(COL_DXIL_LIB_KEY)
                {
                    dxil = Some(catalog.intern.intern(bytes)?);
                }
                for fk in spec.follow_fks {
                    if cell.name.eq_ignore_ascii_case(fk.column) {
                        follows.push(Follow::Keyed {
                            table: fk.table,
                            key: catalog.intern.intern(bytes)?,
                        });
                    }
                }
            }
        }
        let Some(owner) = owner else {
            continue;
        };
        if let Some(dxil) = dxil {
            catalog.so_dxil.entry(owner).or_default().push(dxil);
        }
        map.entry(owner).or_default().push(AssocNode {
            own: hasher.finish(),
            follows,
        });
    }
    for rows in map.values_mut() {
        rows.sort_by_key(|row| *row.own.as_bytes());
    }
    catalog.assoc.insert(table, map);
    Ok(())
}

fn read_app_id(
    conn: &Connection,
    catalog: &Catalog,
    budget: &mut Budget,
    limits: Limits,
) -> Result<Vec<AppIdentity>, Error> {
    let table = TableId::AppId;
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table).to_vec();
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    for wanted in APP_ID_COLUMNS {
        if let Some(actual) = actual_column(&columns, wanted) {
            select.push(actual, max);
        }
    }
    if select.parts.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let id = match select.find("id") {
            Some(c) => read_opt_i64(row, c, table, limits)?.unwrap_or(0),
            None => 0,
        };
        let exe = match select.find("exe") {
            Some(c) => read_opt_text(row, c, table, limits)?.ok_or(Error::NullKey { table })?,
            None => AppText::new(String::new()),
        };
        let app_name = match select.find("app_name") {
            Some(c) => read_opt_text(row, c, table, limits)?.ok_or(Error::NullKey { table })?,
            None => AppText::new(String::new()),
        };
        let engine_name = match select.find("engine_name") {
            Some(c) => read_opt_text(row, c, table, limits)?,
            None => None,
        };
        let app_version = match select.find("app_version") {
            Some(c) => require_u64(table, "app_version", read_opt_i64(row, c, table, limits)?)?,
            None => 0,
        };
        let engine_version = match select.find("engine_version") {
            Some(c) => require_u64(
                table,
                "engine_version",
                read_opt_i64(row, c, table, limits)?,
            )?,
            None => 0,
        };
        out.push(AppIdentity {
            id,
            exe,
            app_name,
            engine_name,
            app_version,
            engine_version,
        });
    }
    Ok(out)
}

fn read_shader_bytecode(
    conn: &Connection,
    catalog: &Catalog,
    budget: &mut Budget,
    limits: Limits,
) -> Result<BTreeMap<AppKey, Sha256Hash>, Error> {
    let table = TableId::ShaderBytecode;
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table).to_vec();
    if !has_column(&columns, COL_KEY) {
        return Err(Error::MissingKeyColumn { table });
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(&columns, COL_KEY) {
        select.push_key(name, max);
    }
    let bytecode_name = actual_column(&columns, COL_BYTECODE).map(str::to_string);
    let mut bc_typeof_idx = None;
    let mut bc_len_idx = None;
    let mut bc_rowid_idx = None;
    if let Some(name) = bytecode_name.as_deref() {
        // Length + typeof only; the blob is hashed via incremental I/O.
        let q = quote_ident(name);
        bc_typeof_idx = Some(select.parts.len());
        select.parts.push(format!("typeof({q})"));
        bc_len_idx = Some(select.parts.len());
        select.parts.push(format!("octet_length({q})"));
        bc_rowid_idx = Some(select.parts.len());
        select.parts.push(String::from("rowid"));
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut out = BTreeMap::new();
    let mut seen = HashSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let key = read_selected_key(row, select.key_col(table)?, table, limits)?;
        if !seen.insert(key.clone()) {
            return Err(Error::DuplicateKey { table });
        }
        let Some(name) = bytecode_name.as_deref() else {
            continue;
        };
        let (Some(ty_idx), Some(len_idx), Some(rowid_idx)) =
            (bc_typeof_idx, bc_len_idx, bc_rowid_idx)
        else {
            continue;
        };
        let ty = row
            .get::<_, String>(ty_idx)
            .map_err(|source| Error::from_sqlite(table, source))?;
        let len = row
            .get::<_, Option<i64>>(len_idx)
            .map_err(|source| Error::from_sqlite(table, source))?;
        match ty.as_str() {
            "null" => continue,
            "blob" => {}
            other => {
                return Err(Error::WrongColumnType {
                    table,
                    column: name.to_string(),
                    got: other.to_string(),
                });
            }
        }
        enforce_len(table, name, &ty, len, limits)?;
        let rowid = row
            .get::<_, i64>(rowid_idx)
            .map_err(|source| Error::from_sqlite(table, source))?;
        let bytecode_hash = hash_bytecode_blob(conn, name, rowid, table)?;
        out.insert(key, bytecode_hash);
    }
    Ok(out)
}

/// SHA-256 of a `shader_bytecode.Bytecode` blob without retaining the bytes.
fn hash_bytecode_blob(
    conn: &Connection,
    column: &str,
    rowid: i64,
    table: TableId,
) -> Result<Sha256Hash, Error> {
    let mut blob = conn
        .blob_open(DatabaseName::Main, table.sql_name(), column, rowid, true)
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let n = std::io::Read::read(&mut blob, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Sha256Hash::from_bytes(hasher.finalize().into()))
}

fn hashes_for_keys(
    keys: &[Option<AppKey>],
    bytecode_by_key: &BTreeMap<AppKey, Sha256Hash>,
) -> Vec<Sha256Hash> {
    let mut out = Vec::new();
    for key in keys.iter().flatten() {
        if let Some(hash) = bytecode_by_key.get(key) {
            if !out.contains(hash) {
                out.push(*hash);
            }
        }
    }
    out
}

fn feed_value(hasher: &mut CanonicalHasher, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.feed_null(),
        ValueRef::Integer(v) => hasher.opt_i64(Some(v)),
        ValueRef::Real(v) => hasher.opt_f64(Some(v)),
        ValueRef::Text(t) => hasher.feed_text(t),
        ValueRef::Blob(b) => hasher.feed_blob(b),
    }
}

fn feed_cached(
    hasher: &mut CanonicalHasher,
    catalog: &Catalog,
    table: TableId,
    key: &AppKey,
) -> Result<(), Error> {
    hasher.tag(b"follow");
    hasher.tag(table.sql_name().as_bytes());
    hasher.bytes(catalog.subhash(table, key)?.as_bytes());
    Ok(())
}

fn read_pipeline_states(
    conn: &Connection,
    catalog: &Catalog,
    budget: &mut Budget,
    bytecode_by_key: &BTreeMap<AppKey, Sha256Hash>,
    limits: Limits,
) -> Result<Vec<HashedObject>, Error> {
    let table = TableId::PipelineStates;
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table);
    if !has_column(columns, COL_KEY) {
        return Err(Error::MissingKeyColumn { table });
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(columns, COL_KEY) {
        select.push_key(name, max);
    }
    for wanted in PIPELINE_DESC_COLUMNS
        .iter()
        .chain(PIPELINE_BYTECODE_COLUMNS.iter())
    {
        if let Some(actual) = actual_column(columns, wanted) {
            select.push(actual, max);
        }
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut out = Vec::new();
    let mut seen_keys = HashSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let key = read_selected_key(row, select.key_col(table)?, table, limits)?;
        if !seen_keys.insert(key.clone()) {
            return Err(Error::DuplicateKey { table });
        }

        let mut hasher = CanonicalHasher::new();
        hasher.tag(b"pso_desc_v1");
        hasher.bytes(key.as_bytes());

        let mut bytecode_keys = Vec::new();
        for col in &select.cols {
            if select
                .key
                .as_ref()
                .is_some_and(|k| k.name.eq_ignore_ascii_case(&col.name))
            {
                continue;
            }
            let (ty, len) = read_ty_len(row, col, table)?;
            enforce_len(table, &col.name, &ty, len, limits)?;
            let is_bytecode = PIPELINE_BYTECODE_COLUMNS
                .iter()
                .any(|name| col.name.eq_ignore_ascii_case(name));
            if is_bytecode {
                bytecode_keys.push(read_opt_blob(row, col, table, limits)?.map(AppKey::new));
                continue;
            }
            hasher.tag(col.name.as_bytes());
            let value = row
                .get_ref(col.val_idx)
                .map_err(|source| Error::from_sqlite(table, source))?;
            feed_value(&mut hasher, value);
        }

        for fk in PIPELINE_KEYED_FOLLOWS {
            if let Some(col) = select.find(fk.column) {
                if let Some(fk_key) = read_opt_blob(row, col, table, limits)?.map(AppKey::new) {
                    feed_cached(&mut hasher, catalog, fk.table, &fk_key)?;
                }
            }
        }
        if let Some(col) = select.find("InputLayout") {
            if let Some(layout) = read_opt_blob(row, col, table, limits)?.map(AppKey::new) {
                feed_cached(
                    &mut hasher,
                    catalog,
                    TableId::InputLayoutToInputElementAssociations,
                    &layout,
                )?;
            }
        }

        let version = match catalog.intern.find(key.as_bytes())? {
            Some(id) => catalog
                .pso_version
                .get(&id)
                .copied()
                .unwrap_or(ObjectVersion::new(0)),
            None => ObjectVersion::new(0),
        };
        out.push(HashedObject {
            kind: ObjectKind::PipelineState,
            key,
            version,
            desc_hash: hasher.finish(),
            bytecode_hashes: hashes_for_keys(&bytecode_keys, bytecode_by_key),
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

fn so_bytecode_keys(catalog: &Catalog, so_key: KeyId) -> Result<Vec<Option<AppKey>>, Error> {
    let Some(keys) = catalog.so_dxil.get(&so_key) else {
        return Ok(Vec::new());
    };
    keys.iter()
        .map(|id| catalog.intern.app_key(*id).map(Some))
        .collect()
}

fn emit_state_objects(
    catalog: &Catalog,
    bytecode_by_key: &BTreeMap<AppKey, Sha256Hash>,
) -> Result<Vec<HashedObject>, Error> {
    let mut out = Vec::new();
    if let Some(map) = catalog.subhashes.get(&TableId::StateObjects) {
        out.reserve(map.len());
        for (key, desc_hash) in map {
            let bytecode_keys = so_bytecode_keys(catalog, *key)?;
            let version = catalog
                .so_version
                .get(key)
                .copied()
                .unwrap_or(ObjectVersion::new(0));
            out.push(HashedObject {
                kind: ObjectKind::StateObject,
                key: catalog.intern.app_key(*key)?,
                version,
                desc_hash: *desc_hash,
                bytecode_hashes: hashes_for_keys(&bytecode_keys, bytecode_by_key),
            });
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

fn require_i64(table: TableId, column: &str, value: Option<i64>) -> Result<i64, Error> {
    value.ok_or(Error::InvalidValue {
        table,
        column: column.to_string(),
        found: 0,
    })
}

fn require_f64(table: TableId, column: &str, value: Option<f64>) -> Result<f64, Error> {
    value.ok_or(Error::InvalidValue {
        table,
        column: column.to_string(),
        found: 0,
    })
}

fn require_text(table: TableId, column: &str, value: Option<AppText>) -> Result<String, Error> {
    match value {
        Some(text) => Ok(text.as_str().to_string()),
        None => Err(Error::InvalidValue {
            table,
            column: column.to_string(),
            found: 0,
        }),
    }
}

fn stage_hash(
    bytecode_by_key: &BTreeMap<AppKey, Sha256Hash>,
    key: Option<AppKey>,
) -> Option<Sha256Hash> {
    key.and_then(|k| bytecode_by_key.get(&k).copied())
}

fn read_pipeline_extract(
    conn: &Connection,
    catalog: &Catalog,
    budget: &mut Budget,
    bytecode_by_key: &BTreeMap<AppKey, Sha256Hash>,
    limits: Limits,
) -> Result<Vec<ExtractedPipelineState>, Error> {
    let table = TableId::PipelineStates;
    let count = row_count(conn, table)?;
    budget.charge(table, count)?;
    let columns = catalog.cols(table);
    if !has_column(columns, COL_KEY) {
        return Err(Error::MissingKeyColumn { table });
    }

    let root_sigs = load_root_signatures(conn, catalog, limits)?;
    let input_elements = load_input_elements(conn, catalog, limits)?;
    let layout_assocs = load_layout_assocs(conn, catalog, limits)?;
    let ds_ops = load_depth_stencil_ops(conn, catalog, limits)?;
    let depth_stencils = load_depth_stencils(conn, catalog, limits, &ds_ops)?;
    let rt_blends = load_rt_blends(conn, catalog, limits)?;
    let blends = load_blends(conn, catalog, limits, &rt_blends)?;
    let rasters = load_rasters(conn, catalog, limits)?;
    let rt_formats = load_rt_formats(conn, catalog, limits)?;
    let views = load_view_instancing(conn, catalog, limits)?;
    let so_decls = load_so_decls(conn, catalog, limits)?;
    let stream_outs = load_stream_outs(conn, catalog, limits, &so_decls)?;

    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(columns, COL_KEY) {
        select.push_key(name, max);
    }
    for wanted in PIPELINE_DESC_COLUMNS
        .iter()
        .chain(PIPELINE_BYTECODE_COLUMNS.iter())
    {
        if let Some(actual) = actual_column(columns, wanted) {
            if select.find(actual).is_none() {
                select.push(actual, max);
            }
        }
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut out = Vec::new();
    let mut seen_keys = HashSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let key = read_selected_key(row, select.key_col(table)?, table, limits)?;
        if !seen_keys.insert(key.clone()) {
            return Err(Error::DuplicateKey { table });
        }
        let version = match catalog.intern.find(key.as_bytes())? {
            Some(id) => catalog
                .pso_version
                .get(&id)
                .copied()
                .unwrap_or(ObjectVersion::new(0)),
            None => ObjectVersion::new(0),
        };
        let mut vs = None;
        let mut ps = None;
        let mut hs = None;
        let mut ds = None;
        let mut gs = None;
        let mut amplification = None;
        let mut ms = None;
        let mut cs = None;
        for (col_name, dest) in [
            ("ByteCode_VS", &mut vs),
            ("ByteCode_PS", &mut ps),
            ("ByteCode_HS", &mut hs),
            ("ByteCode_DS", &mut ds),
            ("ByteCode_GS", &mut gs),
            ("ByteCode_AS", &mut amplification),
            ("ByteCode_MS", &mut ms),
            ("ByteCode_CS", &mut cs),
        ] {
            if let Some(col) = select.find(col_name) {
                *dest = stage_hash(
                    bytecode_by_key,
                    read_opt_blob(row, col, table, limits)?.map(AppKey::new),
                );
            }
        }
        let root_signature = lookup_blob(
            &root_sigs,
            &opt_key(row, &select, table, limits, "RootSignature")?,
        );
        let input_layout = opt_key(row, &select, table, limits, "InputLayout")?.and_then(|k| {
            layout_assocs.get(&k).map(|ids| {
                let mut elems: Vec<ExtractedInputElement> = ids
                    .iter()
                    .filter_map(|id| input_elements.get(id).cloned())
                    .collect();
                elems.sort_by(|a, b| {
                    a.semantic_name
                        .cmp(&b.semantic_name)
                        .then(a.semantic_index.cmp(&b.semantic_index))
                });
                elems
            })
        });
        let depth_stencil = lookup_clone(
            &depth_stencils,
            &opt_key(row, &select, table, limits, "DepthStencilDesc")?,
        );
        let blend = lookup_clone(&blends, &opt_key(row, &select, table, limits, "BlendDesc")?);
        let rasterizer = lookup_clone(
            &rasters,
            &opt_key(row, &select, table, limits, "RasterizerDesc")?,
        );
        let render_target_formats = lookup_clone(
            &rt_formats,
            &opt_key(row, &select, table, limits, "RenderTargetFormats")?,
        );
        let view_instancing = lookup_clone(
            &views,
            &opt_key(row, &select, table, limits, "ViewInstancingDesc")?,
        );
        let stream_out = lookup_clone(
            &stream_outs,
            &opt_key(row, &select, table, limits, "StreamOutDesc")?,
        );
        out.push(ExtractedPipelineState {
            key,
            version,
            bytecode: ExtractedStageHashes {
                vs,
                ps,
                hs,
                ds,
                gs,
                amplification,
                ms,
                cs,
            },
            root_signature,
            input_layout,
            depth_stencil,
            blend,
            rasterizer,
            render_target_formats,
            view_instancing,
            stream_out,
            sample_count: opt_i64(row, &select, table, limits, "SampleDesc_Count")?,
            sample_quality: opt_i64(row, &select, table, limits, "SampleDesc_Quality")?,
            sample_mask: opt_i64(row, &select, table, limits, "SampleMask")?,
            ib_strip_cut_value: opt_i64(row, &select, table, limits, "IBStripCutValue")?,
            primitive_topology: opt_i64(row, &select, table, limits, "PrimitiveTopology")?,
            dsv_format: opt_i64(row, &select, table, limits, "DSVFormat")?,
            node_mask: opt_i64(row, &select, table, limits, "NodeMask")?,
            flags: opt_i64(row, &select, table, limits, "Flags")?,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

fn opt_key(
    row: &rusqlite::Row<'_>,
    select: &SelectList,
    table: TableId,
    limits: Limits,
    name: &str,
) -> Result<Option<AppKey>, Error> {
    match select.find(name) {
        Some(col) => Ok(read_opt_blob(row, col, table, limits)?.map(AppKey::new)),
        None => Ok(None),
    }
}

fn opt_i64(
    row: &rusqlite::Row<'_>,
    select: &SelectList,
    table: TableId,
    limits: Limits,
    name: &str,
) -> Result<Option<i64>, Error> {
    match select.find(name) {
        Some(col) => read_opt_i64(row, col, table, limits),
        None => Ok(None),
    }
}

fn lookup_blob(
    map: &HashMap<AppKey, DescriptorBlob>,
    key: &Option<AppKey>,
) -> Option<DescriptorBlob> {
    key.as_ref().and_then(|k| map.get(k).cloned())
}

fn lookup_clone<T: Clone>(map: &HashMap<AppKey, T>, key: &Option<AppKey>) -> Option<T> {
    key.as_ref().and_then(|k| map.get(k).cloned())
}

fn load_root_signatures(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, DescriptorBlob>, Error> {
    let table = TableId::RootSignatures;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &["Key", "value"],
        |key, row, select| {
            let value = match select.find("value") {
                Some(col) => read_opt_blob(row, col, table, limits)?,
                None => None,
            };
            if let Some(bytes) = value {
                out.insert(key, DescriptorBlob::new(bytes));
            }
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_input_elements(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedInputElement>, Error> {
    let table = TableId::InputElementDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "SemanticName",
            "SemanticIndex",
            "Format",
            "InputSlot",
            "AlignedByteOffset",
            "InputSlotClass",
            "InstanceDataStepRate",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedInputElement {
                    semantic_name: require_text(
                        table,
                        "SemanticName",
                        opt_text(row, select, table, limits, "SemanticName")?,
                    )?,
                    semantic_index: require_i64(
                        table,
                        "SemanticIndex",
                        opt_i64(row, select, table, limits, "SemanticIndex")?,
                    )?,
                    format: require_i64(
                        table,
                        "Format",
                        opt_i64(row, select, table, limits, "Format")?,
                    )?,
                    input_slot: require_i64(
                        table,
                        "InputSlot",
                        opt_i64(row, select, table, limits, "InputSlot")?,
                    )?,
                    aligned_byte_offset: require_i64(
                        table,
                        "AlignedByteOffset",
                        opt_i64(row, select, table, limits, "AlignedByteOffset")?,
                    )?,
                    input_slot_class: require_i64(
                        table,
                        "InputSlotClass",
                        opt_i64(row, select, table, limits, "InputSlotClass")?,
                    )?,
                    instance_data_step_rate: require_i64(
                        table,
                        "InstanceDataStepRate",
                        opt_i64(row, select, table, limits, "InstanceDataStepRate")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn opt_text(
    row: &rusqlite::Row<'_>,
    select: &SelectList,
    table: TableId,
    limits: Limits,
    name: &str,
) -> Result<Option<AppText>, Error> {
    match select.find(name) {
        Some(col) => read_opt_text(row, col, table, limits),
        None => Ok(None),
    }
}

fn load_layout_assocs(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, Vec<AppKey>>, Error> {
    let table = TableId::InputLayoutToInputElementAssociations;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    assoc_pairs(
        conn,
        catalog,
        table,
        limits,
        "InputLayoutKey",
        "InputElementKey",
    )
}

fn load_depth_stencil_ops(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedDepthStencilOp>, Error> {
    let table = TableId::DepthStencilOpDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "StencilFailOp",
            "StencilDepthFailOp",
            "StencilPassOp",
            "StencilFunc",
            "StencilReadMask",
            "StencilWriteMask",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedDepthStencilOp {
                    stencil_fail_op: require_i64(
                        table,
                        "StencilFailOp",
                        opt_i64(row, select, table, limits, "StencilFailOp")?,
                    )?,
                    stencil_depth_fail_op: require_i64(
                        table,
                        "StencilDepthFailOp",
                        opt_i64(row, select, table, limits, "StencilDepthFailOp")?,
                    )?,
                    stencil_pass_op: require_i64(
                        table,
                        "StencilPassOp",
                        opt_i64(row, select, table, limits, "StencilPassOp")?,
                    )?,
                    stencil_func: require_i64(
                        table,
                        "StencilFunc",
                        opt_i64(row, select, table, limits, "StencilFunc")?,
                    )?,
                    stencil_read_mask: require_i64(
                        table,
                        "StencilReadMask",
                        opt_i64(row, select, table, limits, "StencilReadMask")?,
                    )?,
                    stencil_write_mask: require_i64(
                        table,
                        "StencilWriteMask",
                        opt_i64(row, select, table, limits, "StencilWriteMask")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_depth_stencils(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
    ops: &HashMap<AppKey, ExtractedDepthStencilOp>,
) -> Result<HashMap<AppKey, ExtractedDepthStencil>, Error> {
    let table = TableId::DepthStencilDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "DepthEnable",
            "DepthWriteMask",
            "DepthFunc",
            "StencilEnable",
            "FrontFace",
            "BackFace",
            "DepthBoundsTestEnable",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedDepthStencil {
                    depth_enable: require_i64(
                        table,
                        "DepthEnable",
                        opt_i64(row, select, table, limits, "DepthEnable")?,
                    )?,
                    depth_write_mask: require_i64(
                        table,
                        "DepthWriteMask",
                        opt_i64(row, select, table, limits, "DepthWriteMask")?,
                    )?,
                    depth_func: require_i64(
                        table,
                        "DepthFunc",
                        opt_i64(row, select, table, limits, "DepthFunc")?,
                    )?,
                    stencil_enable: require_i64(
                        table,
                        "StencilEnable",
                        opt_i64(row, select, table, limits, "StencilEnable")?,
                    )?,
                    front_face: lookup_clone(
                        ops,
                        &opt_key(row, select, table, limits, "FrontFace")?,
                    ),
                    back_face: lookup_clone(ops, &opt_key(row, select, table, limits, "BackFace")?),
                    depth_bounds_test_enable: require_i64(
                        table,
                        "DepthBoundsTestEnable",
                        opt_i64(row, select, table, limits, "DepthBoundsTestEnable")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_rt_blends(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedRenderTargetBlend>, Error> {
    let table = TableId::RenderTargetBlendDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "BlendEnable",
            "LogicOpEnable",
            "SrcBlend",
            "DestBlend",
            "BlendOp",
            "SrcBlendAlpha",
            "DestBlendAlpha",
            "BlendOpAlpha",
            "LogicOp",
            "RenderTargetWriteMask",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedRenderTargetBlend {
                    blend_enable: require_i64(
                        table,
                        "BlendEnable",
                        opt_i64(row, select, table, limits, "BlendEnable")?,
                    )?,
                    logic_op_enable: require_i64(
                        table,
                        "LogicOpEnable",
                        opt_i64(row, select, table, limits, "LogicOpEnable")?,
                    )?,
                    src_blend: require_i64(
                        table,
                        "SrcBlend",
                        opt_i64(row, select, table, limits, "SrcBlend")?,
                    )?,
                    dest_blend: require_i64(
                        table,
                        "DestBlend",
                        opt_i64(row, select, table, limits, "DestBlend")?,
                    )?,
                    blend_op: require_i64(
                        table,
                        "BlendOp",
                        opt_i64(row, select, table, limits, "BlendOp")?,
                    )?,
                    src_blend_alpha: require_i64(
                        table,
                        "SrcBlendAlpha",
                        opt_i64(row, select, table, limits, "SrcBlendAlpha")?,
                    )?,
                    dest_blend_alpha: require_i64(
                        table,
                        "DestBlendAlpha",
                        opt_i64(row, select, table, limits, "DestBlendAlpha")?,
                    )?,
                    blend_op_alpha: require_i64(
                        table,
                        "BlendOpAlpha",
                        opt_i64(row, select, table, limits, "BlendOpAlpha")?,
                    )?,
                    logic_op: require_i64(
                        table,
                        "LogicOp",
                        opt_i64(row, select, table, limits, "LogicOp")?,
                    )?,
                    render_target_write_mask: require_i64(
                        table,
                        "RenderTargetWriteMask",
                        opt_i64(row, select, table, limits, "RenderTargetWriteMask")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_blends(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
    rts: &HashMap<AppKey, ExtractedRenderTargetBlend>,
) -> Result<HashMap<AppKey, ExtractedBlend>, Error> {
    let table = TableId::BlendDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "AlphaToCoverageEnable",
            "IndependentBlendEnable",
            "RenderTarget0",
            "RenderTarget1",
            "RenderTarget2",
            "RenderTarget3",
            "RenderTarget4",
            "RenderTarget5",
            "RenderTarget6",
            "RenderTarget7",
        ],
        |key, row, select| {
            let mut render_targets = [None, None, None, None, None, None, None, None];
            for (i, name) in [
                "RenderTarget0",
                "RenderTarget1",
                "RenderTarget2",
                "RenderTarget3",
                "RenderTarget4",
                "RenderTarget5",
                "RenderTarget6",
                "RenderTarget7",
            ]
            .into_iter()
            .enumerate()
            {
                render_targets[i] = lookup_clone(rts, &opt_key(row, select, table, limits, name)?);
            }
            out.insert(
                key,
                ExtractedBlend {
                    alpha_to_coverage_enable: require_i64(
                        table,
                        "AlphaToCoverageEnable",
                        opt_i64(row, select, table, limits, "AlphaToCoverageEnable")?,
                    )?,
                    independent_blend_enable: require_i64(
                        table,
                        "IndependentBlendEnable",
                        opt_i64(row, select, table, limits, "IndependentBlendEnable")?,
                    )?,
                    render_targets,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_rasters(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedRasterizer>, Error> {
    let table = TableId::RasterizerDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "FillMode",
            "CullMode",
            "FrontCounterClockwise",
            "DepthBias",
            "DepthBiasClamp",
            "SlopeScaledDepthBias",
            "DepthClipEnable",
            "LineRasterizationMode",
            "ForcedSampleCount",
            "ConservativeRaster",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedRasterizer {
                    fill_mode: require_i64(
                        table,
                        "FillMode",
                        opt_i64(row, select, table, limits, "FillMode")?,
                    )?,
                    cull_mode: require_i64(
                        table,
                        "CullMode",
                        opt_i64(row, select, table, limits, "CullMode")?,
                    )?,
                    front_counter_clockwise: require_i64(
                        table,
                        "FrontCounterClockwise",
                        opt_i64(row, select, table, limits, "FrontCounterClockwise")?,
                    )?,
                    depth_bias: require_f64(
                        table,
                        "DepthBias",
                        opt_f64(row, select, table, limits, "DepthBias")?,
                    )?,
                    depth_bias_clamp: require_f64(
                        table,
                        "DepthBiasClamp",
                        opt_f64(row, select, table, limits, "DepthBiasClamp")?,
                    )?,
                    slope_scaled_depth_bias: require_f64(
                        table,
                        "SlopeScaledDepthBias",
                        opt_f64(row, select, table, limits, "SlopeScaledDepthBias")?,
                    )?,
                    depth_clip_enable: require_i64(
                        table,
                        "DepthClipEnable",
                        opt_i64(row, select, table, limits, "DepthClipEnable")?,
                    )?,
                    line_rasterization_mode: require_i64(
                        table,
                        "LineRasterizationMode",
                        opt_i64(row, select, table, limits, "LineRasterizationMode")?,
                    )?,
                    forced_sample_count: require_i64(
                        table,
                        "ForcedSampleCount",
                        opt_i64(row, select, table, limits, "ForcedSampleCount")?,
                    )?,
                    conservative_raster: require_i64(
                        table,
                        "ConservativeRaster",
                        opt_i64(row, select, table, limits, "ConservativeRaster")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn opt_f64(
    row: &rusqlite::Row<'_>,
    select: &SelectList,
    table: TableId,
    limits: Limits,
    name: &str,
) -> Result<Option<f64>, Error> {
    match select.find(name) {
        Some(col) => read_opt_f64(row, col, table, limits),
        None => Ok(None),
    }
}

fn load_rt_formats(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedRenderTargetFormats>, Error> {
    let table = TableId::RenderTargetFormats;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "RTFormat0",
            "RTFormat1",
            "RTFormat2",
            "RTFormat3",
            "RTFormat4",
            "RTFormat5",
            "RTFormat6",
            "RTFormat7",
            "NumRenderTargets",
        ],
        |key, row, select| {
            let mut rt_formats = [0_i64; 8];
            for (i, name) in [
                "RTFormat0",
                "RTFormat1",
                "RTFormat2",
                "RTFormat3",
                "RTFormat4",
                "RTFormat5",
                "RTFormat6",
                "RTFormat7",
            ]
            .into_iter()
            .enumerate()
            {
                rt_formats[i] =
                    require_i64(table, name, opt_i64(row, select, table, limits, name)?)?;
            }
            out.insert(
                key,
                ExtractedRenderTargetFormats {
                    rt_formats,
                    num_render_targets: require_i64(
                        table,
                        "NumRenderTargets",
                        opt_i64(row, select, table, limits, "NumRenderTargets")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_view_instancing(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedViewInstancing>, Error> {
    let table = TableId::ViewInstancingDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "ViewInstanceCount",
            "RenderFlags",
            "ViewportArrayIndex0",
            "RenderTargetArrayIndex0",
            "ViewportArrayIndex1",
            "RenderTargetArrayIndex1",
            "ViewportArrayIndex2",
            "RenderTargetArrayIndex2",
            "ViewportArrayIndex3",
            "RenderTargetArrayIndex3",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedViewInstancing {
                    view_instance_count: require_i64(
                        table,
                        "ViewInstanceCount",
                        opt_i64(row, select, table, limits, "ViewInstanceCount")?,
                    )?,
                    render_flags: require_i64(
                        table,
                        "RenderFlags",
                        opt_i64(row, select, table, limits, "RenderFlags")?,
                    )?,
                    viewport_array_index: [
                        opt_i64(row, select, table, limits, "ViewportArrayIndex0")?,
                        opt_i64(row, select, table, limits, "ViewportArrayIndex1")?,
                        opt_i64(row, select, table, limits, "ViewportArrayIndex2")?,
                        opt_i64(row, select, table, limits, "ViewportArrayIndex3")?,
                    ],
                    render_target_array_index: [
                        opt_i64(row, select, table, limits, "RenderTargetArrayIndex0")?,
                        opt_i64(row, select, table, limits, "RenderTargetArrayIndex1")?,
                        opt_i64(row, select, table, limits, "RenderTargetArrayIndex2")?,
                        opt_i64(row, select, table, limits, "RenderTargetArrayIndex3")?,
                    ],
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_so_decls(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
) -> Result<HashMap<AppKey, ExtractedSoDeclaration>, Error> {
    let table = TableId::SoDeclarations;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "Stream",
            "SemanticName",
            "SemanticIndex",
            "StartComponent",
            "ComponentCount",
            "OutputSlot",
        ],
        |key, row, select| {
            out.insert(
                key,
                ExtractedSoDeclaration {
                    stream: require_i64(
                        table,
                        "Stream",
                        opt_i64(row, select, table, limits, "Stream")?,
                    )?,
                    semantic_name: require_text(
                        table,
                        "SemanticName",
                        opt_text(row, select, table, limits, "SemanticName")?,
                    )?,
                    semantic_index: require_i64(
                        table,
                        "SemanticIndex",
                        opt_i64(row, select, table, limits, "SemanticIndex")?,
                    )?,
                    start_component: require_i64(
                        table,
                        "StartComponent",
                        opt_i64(row, select, table, limits, "StartComponent")?,
                    )?,
                    component_count: require_i64(
                        table,
                        "ComponentCount",
                        opt_i64(row, select, table, limits, "ComponentCount")?,
                    )?,
                    output_slot: require_i64(
                        table,
                        "OutputSlot",
                        opt_i64(row, select, table, limits, "OutputSlot")?,
                    )?,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn load_stream_outs(
    conn: &Connection,
    catalog: &Catalog,
    limits: Limits,
    decls: &HashMap<AppKey, ExtractedSoDeclaration>,
) -> Result<HashMap<AppKey, ExtractedStreamOut>, Error> {
    let table = TableId::StreamOutDescs;
    if !catalog.present.contains(&table) {
        return Ok(HashMap::new());
    }
    let pairs = if catalog
        .present
        .contains(&TableId::StreamOutputDescToDeclAssociations)
    {
        assoc_pairs(
            conn,
            catalog,
            TableId::StreamOutputDescToDeclAssociations,
            limits,
            "StreamOutDescKey",
            "StreamOutDeclKey",
        )?
    } else {
        HashMap::new()
    };
    let mut out = HashMap::new();
    keyed_scan(
        conn,
        catalog,
        table,
        limits,
        &[
            "Key",
            "BufferStride0",
            "BufferStride1",
            "BufferStride2",
            "BufferStride3",
            "NumStrides",
            "RasterizedStream",
        ],
        |key, row, select| {
            let mut declarations: Vec<ExtractedSoDeclaration> = pairs
                .get(&key)
                .into_iter()
                .flatten()
                .filter_map(|id| decls.get(id).cloned())
                .collect();
            declarations.sort_by(|a, b| {
                a.stream
                    .cmp(&b.stream)
                    .then(a.semantic_name.cmp(&b.semantic_name))
                    .then(a.semantic_index.cmp(&b.semantic_index))
            });
            out.insert(
                key,
                ExtractedStreamOut {
                    buffer_stride: [
                        require_i64(
                            table,
                            "BufferStride0",
                            opt_i64(row, select, table, limits, "BufferStride0")?,
                        )?,
                        require_i64(
                            table,
                            "BufferStride1",
                            opt_i64(row, select, table, limits, "BufferStride1")?,
                        )?,
                        require_i64(
                            table,
                            "BufferStride2",
                            opt_i64(row, select, table, limits, "BufferStride2")?,
                        )?,
                        require_i64(
                            table,
                            "BufferStride3",
                            opt_i64(row, select, table, limits, "BufferStride3")?,
                        )?,
                    ],
                    num_strides: require_i64(
                        table,
                        "NumStrides",
                        opt_i64(row, select, table, limits, "NumStrides")?,
                    )?,
                    rasterized_stream: require_i64(
                        table,
                        "RasterizedStream",
                        opt_i64(row, select, table, limits, "RasterizedStream")?,
                    )?,
                    declarations,
                },
            );
            Ok(())
        },
    )?;
    Ok(out)
}

fn keyed_scan(
    conn: &Connection,
    catalog: &Catalog,
    table: TableId,
    limits: Limits,
    wanted: &[&str],
    mut on_row: impl FnMut(AppKey, &rusqlite::Row<'_>, &SelectList) -> Result<(), Error>,
) -> Result<(), Error> {
    let columns = catalog.cols(table);
    if !has_column(columns, COL_KEY) {
        return Err(Error::MissingKeyColumn { table });
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(columns, COL_KEY) {
        select.push_key(name, max);
    }
    for col in wanted {
        if col.eq_ignore_ascii_case(COL_KEY) {
            continue;
        }
        if let Some(name) = actual_column(columns, col) {
            if select.find(name).is_none() {
                select.push(name, max);
            }
        }
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let key = read_selected_key(row, select.key_col(table)?, table, limits)?;
        on_row(key, row, &select)?;
    }
    Ok(())
}

fn assoc_pairs(
    conn: &Connection,
    catalog: &Catalog,
    table: TableId,
    limits: Limits,
    owner_col: &str,
    child_col: &str,
) -> Result<HashMap<AppKey, Vec<AppKey>>, Error> {
    let columns = catalog.cols(table);
    if !has_column(columns, owner_col) || !has_column(columns, child_col) {
        return Ok(HashMap::new());
    }
    let mut select = SelectList::new();
    let max = sql_cutoff(limits);
    if let Some(name) = actual_column(columns, owner_col) {
        select.push(name, max);
    }
    if let Some(name) = actual_column(columns, child_col) {
        select.push(name, max);
    }
    let mut stmt = conn
        .prepare(&select.sql(table))
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut rows = stmt
        .query([])
        .map_err(|source| Error::from_sqlite(table, source))?;
    let mut out: HashMap<AppKey, Vec<AppKey>> = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|source| Error::from_sqlite(table, source))?
    {
        let owner = match select.find(owner_col) {
            Some(col) => read_opt_blob(row, col, table, limits)?,
            None => None,
        };
        let child = match select.find(child_col) {
            Some(col) => read_opt_blob(row, col, table, limits)?,
            None => None,
        };
        if let (Some(owner), Some(child)) = (owner, child) {
            out.entry(AppKey::new(owner))
                .or_default()
                .push(AppKey::new(child));
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) fn sqlite_has_octet_length(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row("SELECT octet_length(x'616263')", [], |row| row.get(0))?;
    Ok(n == 3)
}
