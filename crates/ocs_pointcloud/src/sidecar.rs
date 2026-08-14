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

const SCHEMA_VERSION: i64 = 2;
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
        tx.execute(
            "INSERT INTO attachments
             (id, source_relative, source_absolute, fingerprint_json, cache_relative,
              display_json, classes_json, edits_json, selection_filter_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
              source_relative=excluded.source_relative,
              source_absolute=excluded.source_absolute,
              fingerprint_json=excluded.fingerprint_json,
              cache_relative=excluded.cache_relative,
              display_json=excluded.display_json,
              classes_json=excluded.classes_json,
              edits_json=excluded.edits_json,
              selection_filter_json=excluded.selection_filter_json",
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
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM selection_sets
             WHERE attachment_id = ?1 ORDER BY name",
        )?;
        let selection_sets = statement
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .map(|value| -> SidecarResult<SelectionSet> { Ok(serde_json::from_str(&value?)?) })
            .collect::<SidecarResult<Vec<_>>>()?;
        let mut edits: EditStore = serde_json::from_str(&edits)?;
        edits.normalize_after_load();
        Ok(Some(AttachmentState {
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
        }))
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
                 );
                 PRAGMA user_version = 2;
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
