//! Local-first spatial project manifest and source catalog.
//!
//! An `.ocsproj` stores references, fingerprints, named spatial objects, job
//! history, and provenance. Large source files remain external and read-only.

use crate::{CrsInfo, SelectionSet, SourceFingerprint};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    error, fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Schema 2 adds the v2 platform state. Schema-1 projects remain readable;
/// the serde default produces an empty platform state and the next atomic
/// save upgrades the manifest.
pub const PROJECT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug)]
pub enum ProjectError {
    Io(io::Error),
    Json(serde_json::Error),
    UnsupportedSchema(u32),
    Invalid(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "project I/O error: {error}"),
            Self::Json(error) => write!(f, "project data error: {error}"),
            Self::UnsupportedSchema(version) => write!(
                f,
                "project schema {version} is newer than supported schema {PROJECT_SCHEMA_VERSION}"
            ),
            Self::Invalid(message) => write!(f, "invalid spatial project: {message}"),
        }
    }
}

impl error::Error for ProjectError {}

impl From<io::Error> for ProjectError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ProjectError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type ProjectResult<T> = std::result::Result<T, ProjectError>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSpatialReference {
    pub horizontal: CrsInfo,
    pub vertical_wkt: Option<String>,
    pub coordinate_epoch: Option<f64>,
    pub working_unit: String,
    pub transformation_policy: TransformationPolicy,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformationPolicy {
    #[default]
    RequireExplicit,
    PreferHighestAccuracy,
    AllowApproximate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Cad,
    LasLaz,
    Copc,
    E57,
    Feature,
    Raster,
    Terrain,
    Mesh,
    Service,
    Derived,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    #[default]
    Unknown,
    Online,
    Missing,
    Changed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSource {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub uri: String,
    pub relative_path: Option<PathBuf>,
    pub absolute_path_hint: Option<PathBuf>,
    pub fingerprint: Option<SourceFingerprint>,
    pub crs: CrsInfo,
    pub group: Option<String>,
    pub read_only: bool,
    pub point_count: Option<u64>,
    pub bounds_min: Option<[f64; 3]>,
    pub bounds_max: Option<[f64; 3]>,
    pub cache_relative: Option<PathBuf>,
    pub derived_from: Vec<String>,
    pub tags: BTreeSet<String>,
    pub metadata: BTreeMap<String, Value>,
}

impl Default for ProjectSource {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: SourceKind::LasLaz,
            uri: String::new(),
            relative_path: None,
            absolute_path_hint: None,
            fingerprint: None,
            crs: CrsInfo::default(),
            group: None,
            read_only: true,
            point_count: None,
            bounds_min: None,
            bounds_max: None,
            cache_relative: None,
            derived_from: Vec::new(),
            tags: BTreeSet::new(),
            metadata: BTreeMap::new(),
        }
    }
}

impl ProjectSource {
    pub fn local(
        id: impl Into<String>,
        project_path: impl AsRef<Path>,
        source_path: impl AsRef<Path>,
        kind: SourceKind,
    ) -> io::Result<Self> {
        let project_dir = project_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let absolute = fs::canonicalize(source_path.as_ref())?;
        let project_dir = fs::canonicalize(project_dir)?;
        let relative_path = relative_path(&project_dir, &absolute);
        Ok(Self {
            id: id.into(),
            name: absolute
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("source")
                .to_string(),
            kind,
            uri: absolute.to_string_lossy().into_owned(),
            relative_path,
            absolute_path_hint: Some(absolute.clone()),
            fingerprint: SourceFingerprint::from_path(&absolute).ok(),
            ..Default::default()
        })
    }

    pub fn resolve(&self, project_path: impl AsRef<Path>) -> Option<PathBuf> {
        if self.uri.starts_with("http://") || self.uri.starts_with("https://") {
            return None;
        }
        let project_dir = project_path.as_ref().parent()?;
        let matches = |path: &Path| {
            path.exists()
                && self
                    .fingerprint
                    .as_ref()
                    .is_none_or(|fingerprint| fingerprint.matches_path(path))
        };
        self.relative_path
            .as_ref()
            .map(|relative| project_dir.join(relative))
            .filter(|path| matches(path))
            .or_else(|| self.absolute_path_hint.clone().filter(|path| matches(path)))
            .or_else(|| {
                let path = PathBuf::from(&self.uri);
                matches(&path).then_some(path)
            })
    }

    pub fn refresh_status(&mut self, project_path: impl AsRef<Path>) -> SourceStatus {
        let status = if self.uri.starts_with("http://") || self.uri.starts_with("https://") {
            SourceStatus::Unknown
        } else if self.resolve(project_path).is_some() {
            SourceStatus::Online
        } else {
            let hinted = self
                .absolute_path_hint
                .as_ref()
                .is_some_and(|path| path.exists());
            if hinted {
                SourceStatus::Changed
            } else {
                SourceStatus::Missing
            }
        };
        self.metadata.insert(
            "source_status".to_string(),
            Value::String(format!("{status:?}").to_ascii_lowercase()),
        );
        status
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    #[default]
    Plan,
    Profile,
    CrossSection,
    ArbitraryPlane,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedSection {
    pub id: String,
    pub name: String,
    pub kind: SectionKind,
    pub origin: [f64; 3],
    pub normal: [f64; 3],
    /// Length of the section baseline in world units. Older schema-v1 files
    /// omitted this field and load with a practical 100-unit default.
    #[serde(default = "default_section_axis_length")]
    pub axis_length: f64,
    pub total_width: f64,
    pub vertical_limits: Option<[f64; 2]>,
    pub crs: CrsInfo,
    pub locked: bool,
}

impl NamedSection {
    pub fn validate(&self) -> ProjectResult<()> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err(ProjectError::Invalid(
                "section id and name must not be empty".to_string(),
            ));
        }
        if !self.total_width.is_finite() || self.total_width <= 0.0 {
            return Err(ProjectError::Invalid(format!(
                "section '{}' width must be positive",
                self.name
            )));
        }
        if !self.axis_length.is_finite() || self.axis_length <= 0.0 {
            return Err(ProjectError::Invalid(format!(
                "section '{}' baseline length must be positive",
                self.name
            )));
        }
        if !self.origin.into_iter().all(f64::is_finite)
            || !self.normal.into_iter().all(f64::is_finite)
            || self.normal.iter().map(|value| value * value).sum::<f64>() <= f64::EPSILON
        {
            return Err(ProjectError::Invalid(format!(
                "section '{}' has an invalid plane",
                self.name
            )));
        }
        if let Some([low, high]) = self.vertical_limits {
            if !low.is_finite() || !high.is_finite() || low > high {
                return Err(ProjectError::Invalid(format!(
                    "section '{}' has invalid vertical limits",
                    self.name
                )));
            }
        }
        Ok(())
    }

    pub fn duplicate(&self, id: impl Into<String>, name: impl Into<String>) -> Self {
        let mut copy = self.clone();
        copy.id = id.into();
        copy.name = name.into();
        copy.locked = false;
        copy
    }

    pub fn flip(&mut self) {
        for value in &mut self.normal {
            *value = -*value;
        }
    }
}

fn default_section_axis_length() -> f64 {
    100.0
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelObjectId {
    CadEntity { document: String, entity: u64 },
    Feature { layer: String, feature: String },
    Point { source: String, record: u64 },
    Raster { source: String },
    Surface { source: String },
    Mesh { source: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedSelection {
    pub id: String,
    pub name: String,
    pub objects: BTreeSet<ModelObjectId>,
    pub point_ranges: BTreeMap<String, SelectionSet>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessingHistoryEntry {
    pub id: String,
    pub created_unix_ms: u64,
    pub tool_id: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub parameters: Value,
    pub software_version: String,
    pub crs_transformations: Vec<String>,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpatialProject {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub created_unix_ms: u64,
    pub updated_unix_ms: u64,
    pub spatial_reference: ProjectSpatialReference,
    pub sources: Vec<ProjectSource>,
    pub sections: Vec<NamedSection>,
    pub selections: Vec<NamedSelection>,
    pub jobs: Vec<crate::JobRecord>,
    pub history: Vec<ProcessingHistoryEntry>,
    pub named_views: BTreeMap<String, Value>,
    pub workspace: Option<Value>,
    pub python_environment: Option<PathBuf>,
    pub scripts: Vec<PathBuf>,
    pub legacy_sidecar: Option<PathBuf>,
    pub metadata: BTreeMap<String, Value>,
    /// Cross-domain transactions, workflows, standards, trust, and complete
    /// provenance. These records are small; source geometry remains external.
    pub platform: ocs_platform::PlatformState,
}

impl Default for SpatialProject {
    fn default() -> Self {
        let now = unix_ms();
        Self {
            schema_version: PROJECT_SCHEMA_VERSION,
            id: format!("project-{now}"),
            name: "Untitled Spatial Project".to_string(),
            created_unix_ms: now,
            updated_unix_ms: now,
            spatial_reference: ProjectSpatialReference::default(),
            sources: Vec::new(),
            sections: Vec::new(),
            selections: Vec::new(),
            jobs: Vec::new(),
            history: Vec::new(),
            named_views: BTreeMap::new(),
            workspace: None,
            python_environment: None,
            scripts: Vec::new(),
            legacy_sidecar: None,
            metadata: BTreeMap::new(),
            platform: ocs_platform::PlatformState::default(),
        }
    }
}

impl SpatialProject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn open(path: impl AsRef<Path>) -> ProjectResult<Self> {
        let data = fs::read(path)?;
        let project: Self = serde_json::from_slice(&data)?;
        project.validate()?;
        Ok(project)
    }

    pub fn save_atomic(&mut self, path: impl AsRef<Path>) -> ProjectResult<()> {
        let path = path.as_ref();
        if path.extension().and_then(|value| value.to_str()) != Some("ocsproj") {
            return Err(ProjectError::Invalid(
                "project path must use the .ocsproj extension".to_string(),
            ));
        }
        self.schema_version = PROJECT_SCHEMA_VERSION;
        self.updated_unix_ms = unix_ms();
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("ocsproj.partial");
        let backup = path.with_extension("ocsproj.backup");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        if path.exists() {
            if backup.exists() {
                fs::remove_file(&backup)?;
            }
            fs::rename(path, &backup)?;
        }
        if let Err(error) = fs::rename(&temporary, path) {
            if backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            return Err(error.into());
        }
        if backup.exists() {
            fs::remove_file(backup)?;
        }
        Ok(())
    }

    pub fn add_source(&mut self, source: ProjectSource) -> ProjectResult<()> {
        if source.id.trim().is_empty() {
            return Err(ProjectError::Invalid(
                "source id must not be empty".to_string(),
            ));
        }
        if self.sources.iter().any(|existing| existing.id == source.id) {
            return Err(ProjectError::Invalid(format!(
                "duplicate source id '{}'",
                source.id
            )));
        }
        self.sources.push(source);
        Ok(())
    }

    pub fn upsert_section(&mut self, section: NamedSection) -> ProjectResult<()> {
        section.validate()?;
        if let Some(existing) = self.sections.iter_mut().find(|item| item.id == section.id) {
            *existing = section;
        } else {
            self.sections.push(section);
        }
        Ok(())
    }

    pub fn validate(&self) -> ProjectResult<()> {
        if self.schema_version > PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchema(self.schema_version));
        }
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err(ProjectError::Invalid(
                "project id and name must not be empty".to_string(),
            ));
        }
        unique_ids(
            self.sources.iter().map(|source| source.id.as_str()),
            "source",
        )?;
        unique_ids(
            self.sections.iter().map(|section| section.id.as_str()),
            "section",
        )?;
        unique_ids(self.jobs.iter().map(|job| job.id.as_str()), "job")?;
        for section in &self.sections {
            section.validate()?;
        }
        self.platform.validate().map_err(ProjectError::Invalid)?;
        Ok(())
    }
}

fn unique_ids<'a>(ids: impl IntoIterator<Item = &'a str>, kind: &str) -> ProjectResult<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() || !seen.insert(id) {
            return Err(ProjectError::Invalid(format!(
                "{kind} identifiers must be non-empty and unique"
            )));
        }
    }
    Ok(())
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let common = base_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    (common > 0).then(|| {
        let mut path = PathBuf::new();
        for _ in common..base_components.len() {
            path.push("..");
        }
        for component in &target_components[common..] {
            path.push(component.as_os_str());
        }
        path
    })
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

    fn test_section() -> NamedSection {
        NamedSection {
            id: "section-1".to_string(),
            name: "Station 10+00".to_string(),
            kind: SectionKind::CrossSection,
            origin: [10.0, 20.0, 30.0],
            normal: [0.0, 1.0, 0.0],
            axis_length: 250.0,
            total_width: 12.0,
            vertical_limits: Some([0.0, 100.0]),
            crs: CrsInfo::default(),
            locked: true,
        }
    }

    #[test]
    fn project_round_trips_and_keeps_external_sources() {
        let root = std::env::temp_dir().join(format!("ocs-project-{}", unix_ms()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("tile.laz");
        fs::write(&source, b"fixture").unwrap();
        let path = root.join("survey.ocsproj");
        let mut project = SpatialProject::new("Survey");
        project
            .add_source(ProjectSource::local("tile-1", &path, &source, SourceKind::LasLaz).unwrap())
            .unwrap();
        project.upsert_section(test_section()).unwrap();
        project.save_atomic(&path).unwrap();

        let loaded = SpatialProject::open(&path).unwrap();
        assert_eq!("Survey", loaded.name);
        assert_eq!(Some(source), loaded.sources[0].resolve(&path));
        assert_eq!(12.0, loaded.sections[0].total_width);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn section_flip_and_duplicate_preserve_world_slab() {
        let section = test_section();
        let mut copy = section.duplicate("section-2", "Station 10+00 reverse");
        copy.flip();
        assert_eq!([0.0, -1.0, 0.0], copy.normal);
        assert_eq!(section.origin, copy.origin);
        assert_eq!(section.total_width, copy.total_width);
        assert!(!copy.locked);
    }

    #[test]
    fn schema_one_projects_upgrade_with_empty_platform_state() {
        let json = serde_json::json!({
            "schema_version": 1,
            "id": "legacy-project",
            "name": "Legacy",
            "created_unix_ms": 1,
            "updated_unix_ms": 1
        });
        let project: SpatialProject = serde_json::from_value(json).unwrap();
        project.validate().unwrap();
        assert!(project.platform.transactions.is_empty());
        assert_eq!(project.schema_version, 1);
    }
}
