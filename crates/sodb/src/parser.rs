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
use rusqlite::{Connection, OpenFlags};

use crate::hash::{sha256, CanonicalHasher};
use crate::schema::{
    keyed_spec, APP_ID_COLUMNS, ASSOC_SPECS, COL_BYTECODE, COL_DXIL_LIB_KEY, COL_KEY, COL_PSO_KEY,
    COL_SO_KEY, COL_VERSION, KEYED_SPECS, PIPELINE_BYTECODE_COLUMNS, PIPELINE_DESC_COLUMNS,
    PIPELINE_KEYED_FOLLOWS, REPORTED_IF_MISSING, SODB_APPLICATION_ID, SODB_USER_VERSION,
};
use crate::{
    AppIdentity, AppKey, AppText, Error, HashedBytecode, HashedObject, Limits, ObjectKind,
    ObjectVersion, Sha256Hash, SodbSummary, TableId,
};

/// Open `path` with [`Limits::DEFAULT`].
#[must_use = "parsed SODB summary should be used"]
pub fn open(path: impl AsRef<Path>) -> Result<SodbSummary, Error> {
    open_with_limits(path, Limits::DEFAULT)
}

/// Open `path` with caller-supplied resource caps.
#[must_use = "parsed SODB summary should be used"]
pub fn open_with_limits(path: impl AsRef<Path>, limits: Limits) -> Result<SodbSummary, Error> {
    let path = path.as_ref();
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

    let bytecode_by_key = if catalog.present.contains(&TableId::ShaderBytecode) {
        read_shader_bytecode(&conn, &catalog, &mut budget, limits)?
    } else {
        BTreeMap::new()
    };

    let pipeline_states = if catalog.present.contains(&TableId::PipelineStates) {
        read_pipeline_states(&conn, &catalog, &mut budget, &bytecode_by_key, limits)?
    } else {
        Vec::new()
    };

    let state_objects = if catalog.present.contains(&TableId::StateObjects) {
        emit_state_objects(&catalog, &bytecode_by_key)
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
        wal_ignored: header.wal_ignored,
        app_id,
        missing_tables,
        pipeline_states,
        state_objects,
        shader_bytecode,
    })
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

fn inspect_path(path: &Path, max_file_bytes: u64) -> Result<FileHeader, Error> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(Error::NotARegularFile);
    }
    let file = File::open(path)?;
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
                if self.bytes[id as usize].as_ref() == bytes {
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

    fn find(&self, bytes: &[u8]) -> Option<KeyId> {
        let h = hash_key_bytes(bytes);
        let ids = self.index.get(&h)?;
        ids.iter()
            .copied()
            .find(|&id| self.bytes[id as usize].as_ref() == bytes)
    }

    fn get(&self, id: KeyId) -> &[u8] {
        self.bytes.get(id as usize).map_or(&[], |b| b.as_ref())
    }

    fn app_key(&self, id: KeyId) -> AppKey {
        AppKey::new(self.get(id).to_vec())
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
        catalog.sort_so_dxil();
        catalog.finalize_subhashes()?;
        Ok(catalog)
    }

    fn cols(&self, table: TableId) -> &[String] {
        self.columns.get(&table).map_or(&[], Vec::as_slice)
    }

    fn subhash(&self, table: TableId, key: &AppKey) -> Sha256Hash {
        if let Some(id) = self.intern.find(key.as_bytes()) {
            if let Some(hash) = self.subhashes.get(&table).and_then(|m| m.get(&id)) {
                return *hash;
            }
        }
        miss_hash(table, key.as_bytes())
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

    fn sort_so_dxil(&mut self) {
        let intern = &self.intern;
        for keys in self.so_dxil.values_mut() {
            keys.sort_by(|a, b| intern.get(*a).cmp(intern.get(*b)));
        }
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

fn lookup_hash(catalog: &Catalog, cache: &SubHashCache, table: TableId, key: KeyId) -> Sha256Hash {
    cache_get(cache, table, key).unwrap_or_else(|| miss_hash(table, catalog.intern.get(key)))
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

fn finish_hash(catalog: &Catalog, cache: &SubHashCache, follow: Follow) -> Sha256Hash {
    match follow {
        Follow::Keyed { table, key } => {
            let Some(node) = catalog.keyed.get(&table).and_then(|m| m.get(&key)) else {
                return miss_hash(table, catalog.intern.get(key));
            };
            let mut hasher = CanonicalHasher::new();
            hasher.tag(b"keyed-sub");
            hasher.bytes(node.own.as_bytes());
            for child in &node.follows {
                hasher.tag(b"follow");
                let (t, k) = child.id();
                hasher.bytes(lookup_hash(catalog, cache, t, k).as_bytes());
            }
            hasher.finish()
        }
        Follow::Assoc { table, owner } => {
            let mut hasher = CanonicalHasher::new();
            hasher.tag(b"assoc-sub");
            hasher.tag(table.sql_name().as_bytes());
            hasher.bytes(catalog.intern.get(owner));
            match catalog.assoc.get(&table).and_then(|m| m.get(&owner)) {
                None => hasher.tag(b"assoc-miss"),
                Some(rows) => {
                    for row in rows {
                        hasher.tag(b"assoc-row");
                        hasher.bytes(row.own.as_bytes());
                        for child in &row.follows {
                            hasher.tag(b"follow");
                            let (t, k) = child.id();
                            hasher.bytes(lookup_hash(catalog, cache, t, k).as_bytes());
                        }
                    }
                }
            }
            hasher.finish()
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
                            _ => finish_hash(catalog, cache, follow),
                        }
                    }
                    Follow::Assoc { .. } => finish_hash(catalog, cache, follow),
                };
                gray.remove(&(table, key));
                cache_insert(cache, table, key, hash);
            }
        }
    }
    Ok(lookup_hash(catalog, cache, start_table, start_key))
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
                let child_id = catalog.intern.intern(bytes)?;
                if let Some(fk_table) = keyed_fk_table(table, &cell.name) {
                    follows.push(Follow::Keyed {
                        table: fk_table,
                        key: child_id,
                    });
                }
                if cell.name.eq_ignore_ascii_case("InputLayout") {
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
    let bytecode = actual_column(&columns, COL_BYTECODE).map(|name| select.push(name, max));
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
        let Some(bc) = bytecode.as_ref() else {
            continue;
        };
        let Some(bytes) = read_opt_blob(row, bc, table, limits)? else {
            continue;
        };
        let bytecode_hash = sha256(&bytes);
        drop(bytes);
        out.insert(key, bytecode_hash);
    }
    Ok(out)
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

fn feed_cached(hasher: &mut CanonicalHasher, catalog: &Catalog, table: TableId, key: &AppKey) {
    hasher.tag(b"follow");
    hasher.tag(table.sql_name().as_bytes());
    hasher.bytes(catalog.subhash(table, key).as_bytes());
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
                    feed_cached(&mut hasher, catalog, fk.table, &fk_key);
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
                );
            }
        }

        let version = catalog
            .intern
            .find(key.as_bytes())
            .and_then(|id| catalog.pso_version.get(&id).copied())
            .unwrap_or(ObjectVersion::new(0));
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

fn so_bytecode_keys(catalog: &Catalog, so_key: KeyId) -> Vec<Option<AppKey>> {
    catalog
        .so_dxil
        .get(&so_key)
        .map(|keys| {
            keys.iter()
                .map(|id| Some(catalog.intern.app_key(*id)))
                .collect()
        })
        .unwrap_or_default()
}

fn emit_state_objects(
    catalog: &Catalog,
    bytecode_by_key: &BTreeMap<AppKey, Sha256Hash>,
) -> Vec<HashedObject> {
    let mut out = Vec::new();
    if let Some(map) = catalog.subhashes.get(&TableId::StateObjects) {
        out.reserve(map.len());
        for (key, desc_hash) in map {
            let bytecode_keys = so_bytecode_keys(catalog, *key);
            let version = catalog
                .so_version
                .get(key)
                .copied()
                .unwrap_or(ObjectVersion::new(0));
            out.push(HashedObject {
                kind: ObjectKind::StateObject,
                key: catalog.intern.app_key(*key),
                version,
                desc_hash: *desc_hash,
                bytecode_hashes: hashes_for_keys(&bytecode_keys, bytecode_by_key),
            });
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

#[cfg(test)]
pub(crate) fn sqlite_has_octet_length(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row("SELECT octet_length(x'616263')", [], |row| row.get(0))?;
    Ok(n == 3)
}
