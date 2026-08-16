//! Versioned SQLite sidecar for attachment metadata, sparse edits and audit.

use crate::{ClassTable, DisplaySettings, EditStore, PointFilter, SelectionSet};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    error, fmt,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCHEMA_VERSION: i64 = 3;
const FINGERPRINT_SAMPLE_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum SidecarError {
    Sql(rusqlite::Error),
    Io(io::Error),
    Json(serde_json::Error),
    UnsupportedSchema(i64),
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(error) => write!(f, "point-cloud sidecar database error: {error}"),
            Self::Io(error) => write!(f, "point-cloud sidecar I/O error: {error}"),
            Self::Json(error) => write!(f, "point-cloud sidecar data error: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "point-cloud sidecar schema {version} is newer than supported schema {SCHEMA_VERSION}")
            }
        }
    }
}

impl error::Error for SidecarError {}

impl From<rusqlite::Error> for SidecarError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

impl From<io::Error> for SidecarError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for SidecarError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type SidecarResult<T> = std::result::Result<T, SidecarError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub byte_length: u64,
    pub modified_unix_ms: Option<u64>,
    /// SHA-256 of file length plus the first and last 64 KiB.
    pub sampled_sha256: String,
}

impl SourceFingerprint {
    pub fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let metadata = fs::metadata(path)?;
        let byte_length = metadata.len();
        let modified_unix_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .and_then(|value| value.as_millis().try_into().ok());
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        hash.update(byte_length.to_le_bytes());
        let mut buffer = vec![0_u8; FINGERPRINT_SAMPLE_BYTES];
        let head = file.read(&mut buffer)?;
        hash.update(&buffer[..head]);
        if byte_length > FINGERPRINT_SAMPLE_BYTES as u64 {
            let tail_start = byte_length.saturating_sub(FINGERPRINT_SAMPLE_BYTES as u64);
            file.seek(SeekFrom::Start(tail_start))?;
            let tail = file.read(&mut buffer)?;
            hash.update(&buffer[..tail]);
        }
        Ok(Self {
            byte_length,
            modified_unix_ms,
            sampled_sha256: format!("{:x}", hash.finalize()),
        })
    }

    pub fn matches_path(&self, path: impl AsRef<Path>) -> bool {
        Self::from_path(path).is_ok_and(|candidate| {
            candidate.byte_length == self.byte_length
                && candidate.sampled_sha256 == self.sampled_sha256
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttachmentState {
    pub id: String,
    pub source_relative: Option<PathBuf>,
    pub source_absolute: PathBuf,
    pub source_fingerprint: SourceFingerprint,
    pub cache_relative: Option<PathBuf>,
    pub display: DisplaySettings,
    pub classes: ClassTable,
    pub edits: EditStore,
    pub selection_sets: Vec<SelectionSet>,
    pub selection_filter: PointFilter,
}

impl AttachmentState {
    pub fn new(
        id: impl Into<String>,
        drawing_path: impl AsRef<Path>,
        source_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let drawing_dir = drawing_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let source_path = source_path.as_ref();
        let source_absolute = absolute(source_path)?;
        let source_relative = relative_path(&absolute(drawing_dir)?, &source_absolute);
        Ok(Self {
            id: id.into(),
            source_relative,
            source_absolute: source_absolute.clone(),
            source_fingerprint: SourceFingerprint::from_path(source_absolute)?,
            cache_relative: None,
            display: DisplaySettings::default(),
            classes: ClassTable::default(),
            edits: EditStore::default(),
            selection_sets: Vec::new(),
            selection_filter: PointFilter::default(),
        })
    }

    /// Resolves moved projects by preferring the drawing-relative path, then
    /// accepting the original absolute path only when its fingerprint matches.
    pub fn resolve_source(&self, drawing_path: impl AsRef<Path>) -> Option<PathBuf> {
        let drawing_dir = absolute(drawing_path.as_ref().parent()?).ok()?;
        self.source_relative
            .as_ref()
            .map(|relative| normalize_path(&drawing_dir.join(relative)))
            .filter(|candidate| self.source_fingerprint.matches_path(candidate))
            .or_else(|| {
                self.source_fingerprint
                    .matches_path(&self.source_absolute)
                    .then(|| self.source_absolute.clone())
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub created_unix_ms: u64,
    pub action: String,
    pub detail: String,
}

/// A saved folder attachment set. Schema v3 keeps collections alongside
/// attachment rows so a dataset of sources can carry its origin folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionState {
    pub id: String,
    pub display_name: String,
    pub source_folder: Option<String>,
    /// `None` stamps the insertion time on first save.
    pub created_unix_ms: Option<u64>,
}

pub struct SidecarStore {
    connection: Connection,
    path: PathBuf,
}

impl SidecarStore {
    pub fn open(path: impl AsRef<Path>) -> SidecarResult<Self> {
        let path = path.as_ref().to_owned();
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = DELETE;")?;
        let mut store = Self { connection, path };
        store.migrate()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save_attachment(&mut self, attachment: &AttachmentState) -> SidecarResult<()> {
        let tx = self.connection.transaction()?;
        // Order index 0 keeps single-attachment sidecars stable while leaving
        // an existing position untouched when the row is updated.
        let existing: Option<i64> = tx
            .query_row(
                "SELECT order_index FROM attachments WHERE id = ?1",
                params![attachment.id],
                |row| row.get(0),
            )
            .optional()?;
        let order = existing.unwrap_or(0);
        Self::upsert_attachment(&tx, attachment, order, None)?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_attachment(&self, id: &str) -> SidecarResult<Option<AttachmentState>> {
        type Row = (
            Option<String>,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
            String,
        );
        let row: Option<Row> = self
            .connection
            .query_row(
                "SELECT source_relative, source_absolute, fingerprint_json,
                        cache_relative, display_json, classes_json, edits_json,
                        selection_filter_json
                 FROM attachments WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            relative,
            absolute,
            fingerprint,
            cache,
            display,
            classes,
            edits,
            selection_filter,
        )) = row
        else {
            return Ok(None);
        };
        let selection_sets = self.selection_sets_for(id)?;
        Ok(Some(Self::state_from_columns(
            id,
            relative,
            absolute,
            fingerprint,
            cache,
            display,
            classes,
            edits,
            selection_filter,
            selection_sets,
        )?))
    }

    /// Loads every persisted attachment in dataset order (`order_index`, id).
    pub fn load_attachments(&self) -> SidecarResult<Vec<AttachmentState>> {
        type Row = (
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
            String,
        );
        let mut statement = self.connection.prepare(
            "SELECT id, source_relative, source_absolute, fingerprint_json,
                    cache_relative, display_json, classes_json, edits_json,
                    selection_filter_json
             FROM attachments ORDER BY order_index, id",
        )?;
        let rows = statement
            .query_map([], |row| -> rusqlite::Result<Row> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(id, relative, absolute, fingerprint, cache, display, classes, edits, filter)| {
                    let selection_sets = self.selection_sets_for(&id)?;
                    Ok(Self::state_from_columns(
                        &id,
                        relative,
                        absolute,
                        fingerprint,
                        cache,
                        display,
                        classes,
                        edits,
                        filter,
                        selection_sets,
                    )?)
                },
            )
            .collect()
    }

    /// Returns persisted attachment ids in dataset order.
    pub fn attachment_ids(&self) -> SidecarResult<Vec<String>> {
        let mut statement =
            self.connection.prepare("SELECT id FROM attachments ORDER BY order_index, id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Permanently removes an attachment row; cascading deletes drop its
    /// selection sets and audit history. Returns whether a row was removed.
    pub fn remove_attachment(&mut self, id: &str) -> SidecarResult<bool> {
        let removed = self
            .connection
            .execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    /// Saves the whole dataset in one transaction: each attachment is upserted
    /// with its position as `order_index` and optionally linked to a
    /// collection. Rows that are no longer listed keep their history so a
    /// detached source can still be restored; use [`SidecarStore::remove_attachment`]
    /// for permanent removal.
    pub fn save_dataset(
        &mut self,
        attachments: &[AttachmentState],
        collection: Option<&CollectionState>,
    ) -> SidecarResult<()> {
        let tx = self.connection.transaction()?;
        if let Some(collection) = collection {
            tx.execute(
                "INSERT INTO collections (id, display_name, source_folder, created_unix_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                  display_name=excluded.display_name,
                  source_folder=excluded.source_folder",
                params![
                    collection.id,
                    collection.display_name,
                    collection.source_folder,
                    collection.created_unix_ms.unwrap_or_else(unix_ms),
                ],
            )?;
        }
        for (order, attachment) in attachments.iter().enumerate() {
            Self::upsert_attachment(&tx, attachment, order as i64, collection)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_collection(&self, id: &str) -> SidecarResult<Option<CollectionState>> {
        let row = self
            .connection
            .query_row(
                "SELECT display_name, source_folder, created_unix_ms
                 FROM collections WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(display_name, source_folder, created)| CollectionState {
            id: id.to_owned(),
            display_name,
            source_folder,
            created_unix_ms: Some(created.unsigned_abs()),
        }))
    }

    fn selection_sets_for(&self, id: &str) -> SidecarResult<Vec<SelectionSet>> {
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM selection_sets
             WHERE attachment_id = ?1 ORDER BY name",
        )?;
        let selection_sets = statement
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .map(|value| -> SidecarResult<SelectionSet> { Ok(serde_json::from_str(&value?)?) })
            .collect::<SidecarResult<Vec<_>>>()?;
        Ok(selection_sets)
    }

    #[allow(clippy::too_many_arguments)]
    fn state_from_columns(
        id: &str,
        relative: Option<String>,
        absolute: String,
        fingerprint: String,
        cache: Option<String>,
        display: String,
        classes: String,
        edits: String,
        selection_filter: String,
        selection_sets: Vec<SelectionSet>,
    ) -> SidecarResult<AttachmentState> {
        let mut edits: EditStore = serde_json::from_str(&edits)?;
        edits.normalize_after_load();
        Ok(AttachmentState {
            id: id.to_owned(),
            source_relative: relative.map(PathBuf::from),
            source_absolute: PathBuf::from(absolute),
            source_fingerprint: serde_json::from_str(&fingerprint)?,
            cache_relative: cache.map(PathBuf::from),
            display: serde_json::from_str::<DisplaySettings>(&display)?.normalized(),
            classes: serde_json::from_str(&classes)?,
            edits,
            selection_sets,
            selection_filter: serde_json::from_str(&selection_filter)?,
        })
    }

    fn upsert_attachment(
        tx: &rusqlite::Transaction<'_>,
        attachment: &AttachmentState,
        order_index: i64,
        collection: Option<&CollectionState>,
    ) -> SidecarResult<()> {
        tx.execute(
            "INSERT INTO attachments
             (id, source_relative, source_absolute, fingerprint_json, cache_relative,
              display_json, classes_json, edits_json, selection_filter_json,
              collection_id, order_index)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
              source_relative=excluded.source_relative,
              source_absolute=excluded.source_absolute,
              fingerprint_json=excluded.fingerprint_json,
              cache_relative=excluded.cache_relative,
              display_json=excluded.display_json,
              classes_json=excluded.classes_json,
              edits_json=excluded.edits_json,
              selection_filter_json=excluded.selection_filter_json,
              collection_id=excluded.collection_id,
              order_index=excluded.order_index",
            params![
                attachment.id,
                path_text(attachment.source_relative.as_deref()),
                attachment.source_absolute.to_string_lossy(),
                serde_json::to_string(&attachment.source_fingerprint)?,
                path_text(attachment.cache_relative.as_deref()),
                serde_json::to_string(&attachment.display)?,
                serde_json::to_string(&attachment.classes)?,
                serde_json::to_string(&attachment.edits)?,
                serde_json::to_string(&attachment.selection_filter)?,
                collection.map(|collection| collection.id.as_str()),
                order_index,
            ],
        )?;
        tx.execute(
            "DELETE FROM selection_sets WHERE attachment_id = ?1",
            params![attachment.id],
        )?;
        for selection in &attachment.selection_sets {
            tx.execute(
                "INSERT INTO selection_sets (attachment_id, name, payload_json)
                 VALUES (?1, ?2, ?3)",
                params![
                    attachment.id,
                    selection.name,
                    serde_json::to_string(selection)?
                ],
            )?;
        }
        Ok(())
    }

    pub fn append_audit(
        &self,
        attachment_id: &str,
        action: &str,
        detail: &str,
    ) -> SidecarResult<()> {
        self.connection.execute(
            "INSERT INTO audit_log (attachment_id, created_unix_ms, action, detail)
             VALUES (?1, ?2, ?3, ?4)",
            params![attachment_id, unix_ms(), action, detail],
        )?;
        Ok(())
    }

    pub fn audit_log(&self, attachment_id: &str) -> SidecarResult<Vec<AuditEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT created_unix_ms, action, detail FROM audit_log
             WHERE attachment_id = ?1 ORDER BY sequence",
        )?;
        let entries = statement
            .query_map(params![attachment_id], |row| {
                Ok(AuditEntry {
                    created_unix_ms: row.get(0)?,
                    action: row.get(1)?,
                    detail: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn migrate(&mut self) -> SidecarResult<()> {
        let version = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
        if version > SCHEMA_VERSION {
            return Err(SidecarError::UnsupportedSchema(version));
        }
        if version == 0 {
            self.connection.execute_batch(
                "BEGIN;
                 CREATE TABLE attachments (
                    id TEXT PRIMARY KEY,
                    source_relative TEXT,
                    source_absolute TEXT NOT NULL,
                    fingerprint_json TEXT NOT NULL,
                    cache_relative TEXT,
                    display_json TEXT NOT NULL,
                    classes_json TEXT NOT NULL,
                    edits_json TEXT NOT NULL,
                    selection_filter_json TEXT NOT NULL,
                    collection_id TEXT REFERENCES collections(id) ON DELETE SET NULL,
                    order_index INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE collections (
                    id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    source_folder TEXT,
                    created_unix_ms INTEGER NOT NULL
                 );
                 CREATE TABLE selection_sets (
                    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (attachment_id, name)
                 );
                 CREATE TABLE audit_log (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                    created_unix_ms INTEGER NOT NULL,
                    action TEXT NOT NULL,
                    detail TEXT NOT NULL
                 );
                 CREATE TABLE export_jobs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                    output_path TEXT NOT NULL,
                    status TEXT NOT NULL,
                    points_written INTEGER NOT NULL DEFAULT 0,
                    total_points INTEGER NOT NULL DEFAULT 0,
                    error TEXT
                 );
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?;
        }
        if version == 1 {
            self.connection.execute_batch(
                "BEGIN;
                 ALTER TABLE attachments ADD COLUMN selection_filter_json TEXT NOT NULL DEFAULT '{}';
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }
        // Existing v1/v2 databases gain the v3 multi-source columns. A fresh
        // database (version 0 above) already created them.
        if version == 1 || version == 2 {
            self.connection.execute_batch(
                "BEGIN;
                 CREATE TABLE collections (
                    id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    source_folder TEXT,
                    created_unix_ms INTEGER NOT NULL
                 );
                 ALTER TABLE attachments ADD COLUMN collection_id TEXT REFERENCES collections(id) ON DELETE SET NULL;
                 ALTER TABLE attachments ADD COLUMN order_index INTEGER NOT NULL DEFAULT 0;
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?;
        }
        Ok(())
    }
}

pub fn sidecar_path_for_drawing(drawing_path: impl AsRef<Path>) -> PathBuf {
    let drawing_path = drawing_path.as_ref();
    let file_name = drawing_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("drawing");
    drawing_path.with_file_name(format!("{file_name}.ocspc"))
}

fn path_text(path: Option<&Path>) -> Option<String> {
    path.map(|path| path.to_string_lossy().into_owned())
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(normalize_path(path))
    } else {
        Ok(normalize_path(&std::env::current_dir()?.join(path)))
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Produces a lexical relative path for files on the same volume. Unlike
/// `strip_prefix`, this also handles a drawing and cloud in sibling folders.
fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let mut common = 0;
    while common < base.len() && common < target.len() && component_eq(base[common], target[common])
    {
        common += 1;
    }
    if common == 0 {
        return None;
    }

    let mut relative = PathBuf::new();
    for component in &base[common..] {
        match component {
            std::path::Component::Normal(_) | std::path::Component::ParentDir => {
                relative.push("..")
            }
            std::path::Component::CurDir => {}
            std::path::Component::Prefix(_) | std::path::Component::RootDir => return None,
        }
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn component_eq(left: std::path::Component<'_>, right: std::path::Component<'_>) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a scratch source file that only needs to be fingerprintable.
    fn scratch_source(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ocs-sidecar-test-{tag}-{}.bin",
            unix_ms() ^ u64::from(std::process::id())
        ));
        std::fs::write(&path, format!("point data for {tag}")).expect("scratch source");
        path
    }

    fn scratch_sidecar(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ocs-sidecar-test-{tag}-{}.ocspc",
            unix_ms() ^ u64::from(std::process::id())
        ))
    }

    fn state(id: &str, tag: &str, drawing: &Path) -> AttachmentState {
        AttachmentState::new(id, drawing, scratch_source(tag)).expect("attachment state")
    }

    #[test]
    fn save_dataset_round_trips_multiple_attachments_in_order() {
        let sidecar = scratch_sidecar("roundtrip");
        let _ = fs::remove_file(&sidecar);
        let drawing = std::env::temp_dir().join("drawing.dwg");
        let first = state("a", "a", &drawing);
        let second = state("b", "b", &drawing);
        let third = state("c", "c", &drawing);
        {
            let mut store = SidecarStore::open(&sidecar).expect("open");
            store
                .save_dataset(&[first.clone(), second.clone(), third.clone()], None)
                .expect("save dataset");
        }
        let store = SidecarStore::open(&sidecar).expect("reopen");
        assert_eq!(
            store.attachment_ids().expect("ids"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        let loaded = store.load_attachments().expect("load all");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(loaded[0].source_fingerprint, first.source_fingerprint);
        assert_eq!(loaded[2].id, "c");
        assert_eq!(loaded[2].source_absolute, third.source_absolute);
        // Re-saving a shorter dataset keeps the removed row restorable.
        let mut store = SidecarStore::open(&sidecar).expect("reopen");
        store.save_dataset(&[second.clone()], None).expect("resave");
        assert_eq!(store.attachment_ids().expect("ids after").len(), 3);
        let _ = fs::remove_file(&sidecar);
    }

    #[test]
    fn collections_persist_and_link() {
        let sidecar = scratch_sidecar("collections");
        let _ = fs::remove_file(&sidecar);
        let drawing = std::env::temp_dir().join("drawing.dwg");
        let attachment = state("tile-1", "tile-1", &drawing);
        let collection = CollectionState {
            id: "folder-1".to_string(),
            display_name: "Project LAS folder".to_string(),
            source_folder: Some("Z:\\survey\\las".to_string()),
            created_unix_ms: None,
        };
        {
            let mut store = SidecarStore::open(&sidecar).expect("open");
            store
                .save_dataset(&[attachment], Some(&collection))
                .expect("save with collection");
        }
        let store = SidecarStore::open(&sidecar).expect("reopen");
        let loaded = store.load_collection("folder-1").expect("collection");
        let loaded = loaded.expect("collection row");
        assert_eq!(loaded.display_name, "Project LAS folder");
        assert_eq!(loaded.source_folder.as_deref(), Some("Z:\\survey\\las"));
        assert!(loaded.created_unix_ms.is_some());
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
        let linked: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM attachments WHERE collection_id = 'folder-1'",
                [],
                |row| row.get(0),
            )
            .expect("link");
        assert_eq!(linked, 1);
        let _ = fs::remove_file(&sidecar);
    }

    #[test]
    fn remove_attachment_cascades_audit_history() {
        let sidecar = scratch_sidecar("remove");
        let _ = fs::remove_file(&sidecar);
        let drawing = std::env::temp_dir().join("drawing.dwg");
        let attachment = state("doomed", "doomed", &drawing);
        let mut store = SidecarStore::open(&sidecar).expect("open");
        store.save_dataset(&[attachment], None).expect("save");
        store
            .append_audit("doomed", "classification", "class 2 -> 6")
            .expect("audit");
        assert!(store.remove_attachment("doomed").expect("remove"));
        assert!(!store.remove_attachment("doomed").expect("remove again"));
        assert!(store.attachment_ids().expect("ids").is_empty());
        assert!(store.audit_log("doomed").expect("audit").is_empty());
        let _ = fs::remove_file(&sidecar);
    }

    /// Builds a hand-rolled schema-v2 database, the way v0.9.6 wrote it, and
    /// verifies the upgrade path to the v3 multi-source schema.
    #[test]
    fn migrates_v2_sidecar_to_v3() {
        let sidecar = scratch_sidecar("migrate-v2");
        let _ = fs::remove_file(&sidecar);
        let drawing = std::env::temp_dir().join("drawing.dwg");
        let legacy = state("primary", "legacy", &drawing);
        {
            let raw = Connection::open(&sidecar).expect("raw open");
            raw.execute_batch(
                "PRAGMA user_version = 2;
                 CREATE TABLE attachments (
                    id TEXT PRIMARY KEY,
                    source_relative TEXT,
                    source_absolute TEXT NOT NULL,
                    fingerprint_json TEXT NOT NULL,
                    cache_relative TEXT,
                    display_json TEXT NOT NULL,
                    classes_json TEXT NOT NULL,
                    edits_json TEXT NOT NULL,
                    selection_filter_json TEXT NOT NULL
                 );
                 CREATE TABLE selection_sets (
                    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (attachment_id, name)
                 );
                 CREATE TABLE audit_log (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                    created_unix_ms INTEGER NOT NULL,
                    action TEXT NOT NULL,
                    detail TEXT NOT NULL
                 );
                 CREATE TABLE export_jobs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                    output_path TEXT NOT NULL,
                    status TEXT NOT NULL,
                    points_written INTEGER NOT NULL DEFAULT 0,
                    total_points INTEGER NOT NULL DEFAULT 0,
                    error TEXT
                 );",
            )
            .expect("v2 schema");
            raw.execute(
                "INSERT INTO attachments (id, source_relative, source_absolute, fingerprint_json,
                                          cache_relative, display_json, classes_json, edits_json,
                                          selection_filter_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    legacy.id,
                    path_text(legacy.source_relative.as_deref()),
                    legacy.source_absolute.to_string_lossy(),
                    serde_json::to_string(&legacy.source_fingerprint).unwrap(),
                    path_text(legacy.cache_relative.as_deref()),
                    serde_json::to_string(&legacy.display).unwrap(),
                    serde_json::to_string(&legacy.classes).unwrap(),
                    serde_json::to_string(&legacy.edits).unwrap(),
                    serde_json::to_string(&legacy.selection_filter).unwrap(),
                ],
            )
            .expect("legacy row");
            raw.execute(
                "INSERT INTO audit_log (attachment_id, created_unix_ms, action, detail)
                 VALUES ('primary', 1, 'classification', 'legacy entry')",
                [],
            )
            .expect("legacy audit");
        }
        // Opening performs the v3 migration and preserves the legacy row.
        let mut store = SidecarStore::open(&sidecar).expect("migrated open");
        assert_eq!(
            store.attachment_ids().expect("ids"),
            vec!["primary".to_string()]
        );
        let loaded = store.load_attachments().expect("loaded");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].source_fingerprint, legacy.source_fingerprint);
        assert_eq!(
            store.audit_log("primary").expect("audit").len(),
            1,
            "legacy audit history must survive the migration"
        );
        // The migrated store accepts new multi-source writes.
        let extra = state("added", "added", &drawing);
        store
            .save_dataset(&[legacy, extra], None)
            .expect("post-migration save");
        assert_eq!(store.attachment_ids().expect("ids").len(), 2);
        let _ = fs::remove_file(&sidecar);
    }
}
