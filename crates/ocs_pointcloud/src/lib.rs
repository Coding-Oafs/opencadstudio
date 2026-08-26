//! Scalable LAS/LAZ metadata, sampling, classification edits, and export.
//!
//! The crate deliberately keeps the source cloud outside the CAD document.  A
//! viewer can retain only a bounded display sample plus a sparse set of edits,
//! then stream the original file when it is time to export a revised LAS/LAZ.

mod classify;
mod copc;
mod crs;
mod display;
mod dtm;
mod e57_import;
mod edit;
mod jobs;
mod measurement;
mod processing;
mod production_classify;
mod project;
mod ptc;
mod selection;
mod sidecar;
mod surface;
mod tile_cache;
mod tools;

pub use classify::{
    classify_by_rules, classify_ground, detect_noise, ClassifyResult, ClassifyRule, GroundOptions,
    RuleField, RuleOp,
};
pub use copc::{
    inspect_copc, inspect_copc_http, query_copc, query_copc_http, CopcLod, CopcMetadata, CopcQuery,
    HttpRangeReader,
};
pub use crs::{
    assess_survey_readiness, crs_equivalent, crs_horizontal_unit, epsg_area_of_use,
    epsg_horizontal_unit, inspect_crs, reproject_between_crs, reproject_bounds_between_crs,
    reproject_from_crs, reproject_from_proj4, reproject_points_between_crs, reproject_to_crs,
    reproject_with_patches_progress, reproject_xy, CrsInfo, ReprojectionStats, SurveyReadiness,
};
pub use display::{
    classification_statistics, ClassDefinition, ClassStatistics, ClassTable, ColorMode, Density,
    DisplaySettings,
};
pub use dtm::{generate_contours, Contour, Tin};
pub use e57_import::{import_e57, E57ImportProgress, E57ImportStage, E57ImportStats};
pub use edit::{EditStore, EditTransaction, PointPatch};
pub use jobs::{JobCheckpoint, JobQueue, JobRecord, JobStatus, ProtectedOutput};
pub use measurement::{
    cloud_to_cloud, point_to_plane, point_to_point, point_to_surface, CloudDistanceStatistics,
    PlaneMeasurement, SurfaceMeasurement, SurfaceSampler, VectorMeasurement,
};
pub use processing::{
    select_full_density, visit_full_density, FullDensityProgress, ProcessingExtent,
};
pub use production_classify::{
    classify_buildings, classify_roads, classify_vegetation, BuildingClassifier,
    ClassificationPipeline, ClassifierStage, GroundStage, NoiseStage, PipelineResult,
    RoadCenterline, RoadClassifier, StageStatistics, VegetationClassifier,
};
pub use project::{
    ModelObjectId, NamedSection, NamedSelection, ProcessingHistoryEntry, ProjectError,
    ProjectResult, ProjectSource, ProjectSpatialReference, SectionKind, SourceKind, SourceStatus,
    SpatialProject, TransformationPolicy, PROJECT_SCHEMA_VERSION,
};
pub use ptc::{parse_ptc, write_ptc, PtcError};
pub use selection::{
    select_brush, select_nearest, select_polygon, IndexRange, PointFilter, SelectionSet,
};
pub use sidecar::{
    sidecar_path_for_drawing, AttachmentState, AuditEntry, CollectionState, SidecarError,
    SidecarResult, SidecarStore, SourceFingerprint,
};
pub use surface::{
    rasterize_full_density, validate_breaklines, Breakline, BreaklineIssue, BreaklineIssueKind,
    GridStatistic, RasterSurface,
};
pub use tile_cache::{
    build_tiled_cache, estimate_cache_bytes, read_tile, read_tiles_parallel, IndexProgress,
    TileCacheError, TileCacheManifest, TileCacheOptions, TileCacheResult, TileEntry, TileKey,
    MAX_TILE_READ_WORKERS,
};
pub use tools::{
    production_lidar_tools, DensityRequirement, InvocationSource, ToolDescriptor, ToolInvocation,
    ToolRegistry, ToolRequirements, UndoBehavior,
};

use las::{point::Classification, Header, Point, Reader, Writer};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error, fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Errors returned by point-cloud operations.
#[derive(Debug)]
pub enum Error {
    Las(las::Error),
    Io(io::Error),
    InvalidLimit(&'static str),
    UnsupportedExtension(PathBuf),
    OutputExists(PathBuf),
    SameInputAndOutput(PathBuf),
    Cancelled(&'static str),
    Crs(String),
    E57(String),
    /// The sources of a merged export disagree on LAS version, point format,
    /// or declared horizontal CRS.
    MergeIncompatible(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Las(error) => write!(f, "LAS/LAZ error: {error}"),
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidLimit(name) => write!(f, "{name} must be greater than zero"),
            Self::UnsupportedExtension(path) => write!(
                f,
                "point-cloud output must have a .las or .laz extension: {}",
                path.display()
            ),
            Self::OutputExists(path) => {
                write!(
                    f,
                    "refusing to overwrite existing output: {}",
                    path.display()
                )
            }
            Self::SameInputAndOutput(path) => write!(
                f,
                "input and output point-cloud paths must differ: {}",
                path.display()
            ),
            Self::Cancelled(operation) => write!(f, "{operation} cancelled"),
            Self::Crs(message) => write!(f, "coordinate-reference-system error: {message}"),
            Self::E57(message) => write!(f, "E57 import error: {message}"),
            Self::MergeIncompatible(message) => {
                write!(f, "cannot merge these sources: {message}")
            }
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Las(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<las::Error> for Error {
    fn from(value: las::Error) -> Self {
        Self::Las(value)
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// File-level information that can be read without loading point records.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudMetadata {
    pub point_count: u64,
    pub version_major: u8,
    pub version_minor: u8,
    pub point_format: u8,
    pub compressed: bool,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub scales: [f64; 3],
    pub offsets: [f64; 3],
    pub system_identifier: String,
    pub generating_software: String,
    pub creation_date: Option<String>,
    pub file_source_id: u16,
    pub has_crs: bool,
    #[serde(default)]
    pub crs: CrsInfo,
    pub vlr_count: usize,
    pub evlr_count: usize,
}

impl CloudMetadata {
    fn from_header(header: &Header) -> Result<Self> {
        let version = header.version();
        let format = header.point_format();
        let bounds = header.bounds();
        let transforms = header.transforms();

        Ok(Self {
            point_count: header.number_of_points(),
            version_major: version.major,
            version_minor: version.minor,
            point_format: format.to_u8()?,
            compressed: format.is_compressed,
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z],
            scales: [transforms.x.scale, transforms.y.scale, transforms.z.scale],
            offsets: [
                transforms.x.offset,
                transforms.y.offset,
                transforms.z.offset,
            ],
            system_identifier: header.system_identifier().to_owned(),
            generating_software: header.generating_software().to_owned(),
            creation_date: header.date().map(|date| date.to_string()),
            file_source_id: header.file_source_id(),
            has_crs: header.has_crs_vlrs(),
            crs: CrsInfo::from_header(header),
            vlr_count: header.vlrs().len(),
            evlr_count: header.evlrs().len(),
        })
    }
}

/// A display-oriented point retaining the attributes needed for inspection and
/// classification workflows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SamplePoint {
    /// Zero-based position of the point in the source file.
    pub source_index: u64,
    pub position: [f64; 3],
    pub intensity: u16,
    pub classification: u8,
    pub return_number: u8,
    pub number_of_returns: u8,
    pub scan_angle: f32,
    pub user_data: u8,
    pub point_source_id: u16,
    pub gps_time: Option<f64>,
    pub color: Option<[u16; 3]>,
    pub nir: Option<u16>,
    pub is_synthetic: bool,
    pub is_key_point: bool,
    pub is_withheld: bool,
    pub is_overlap: bool,
}

impl SamplePoint {
    fn from_point(source_index: u64, point: Point) -> Self {
        Self {
            source_index,
            position: [point.x, point.y, point.z],
            intensity: point.intensity,
            classification: u8::from(point.classification),
            return_number: point.return_number,
            number_of_returns: point.number_of_returns,
            scan_angle: point.scan_angle,
            user_data: point.user_data,
            point_source_id: point.point_source_id,
            gps_time: point.gps_time,
            color: point
                .color
                .map(|color| [color.red, color.green, color.blue]),
            nir: point.nir,
            is_synthetic: point.is_synthetic,
            is_key_point: point.is_key_point,
            is_withheld: point.is_withheld,
            is_overlap: point.is_overlap,
        }
    }

    /// Returns a display copy with a sparse edit overlay applied.
    pub fn with_patch(mut self, patch: PointPatch) -> Self {
        if let Some(classification) = patch.classification {
            self.classification = classification;
        }
        if let Some(value) = patch.synthetic {
            self.is_synthetic = value;
        }
        if let Some(value) = patch.key_point {
            self.is_key_point = value;
        }
        if let Some(value) = patch.withheld {
            self.is_withheld = value;
        }
        if let Some(value) = patch.overlap {
            self.is_overlap = value;
        }
        if let Some(value) = patch.elevation {
            self.position[2] = value;
        }
        self
    }
}

/// Memory limits for building a display sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleOptions {
    /// Maximum number of points retained in memory for display.
    pub max_points: usize,
    /// Maximum number of source records decoded in one batch.
    pub chunk_size: usize,
    /// Explicit decimation: keep every `Some(n)`th source point. When `None`,
    /// the stride is derived from `max_points` (an approximate 1-in-N sample).
    pub stride: Option<u64>,
}

impl Default for SampleOptions {
    fn default() -> Self {
        Self {
            max_points: 1_000_000,
            chunk_size: 65_536,
            stride: None,
        }
    }
}

/// A bounded, approximately uniform sample of a source cloud.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointSample {
    pub metadata: CloudMetadata,
    pub points: Vec<SamplePoint>,
    /// Every `stride`th source point was selected.
    pub stride: u64,
    /// Number of source records decoded to produce the sample.
    pub scanned_points: u64,
}

/// Reads only the LAS/LAZ header and metadata.
pub fn inspect(path: impl AsRef<Path>) -> Result<CloudMetadata> {
    let reader = Reader::from_path(path)?;
    CloudMetadata::from_header(reader.header())
}

/// Streams a LAS/LAZ file and retains at most `options.max_points` records.
///
/// The source index is retained so a selected display point can be recorded as
/// a sparse classification edit and applied to the correct full-resolution
/// record during export.
pub fn sample(path: impl AsRef<Path>, options: SampleOptions) -> Result<PointSample> {
    if options.max_points == 0 {
        return Err(Error::InvalidLimit("max_points"));
    }
    if options.chunk_size == 0 {
        return Err(Error::InvalidLimit("chunk_size"));
    }

    let mut reader = Reader::from_path(path)?;
    let metadata = CloudMetadata::from_header(reader.header())?;
    let max_points = u64::try_from(options.max_points).unwrap_or(u64::MAX);
    let stride = match options.stride {
        Some(n) if n >= 1 => n,
        _ => metadata.point_count.max(1).div_ceil(max_points).max(1),
    };
    // Reserve based on the stride-derived retained count, not the full source
    // count, so an explicit 1-in-N read doesn't over-allocate for a huge cloud.
    let retained = metadata
        .point_count
        .div_ceil(stride)
        .min(max_points)
        .min(u64::try_from(usize::MAX).unwrap_or(u64::MAX));
    let mut points = Vec::with_capacity(retained as usize);
    let mut source_index = 0_u64;

    while source_index < metadata.point_count && points.len() < options.max_points {
        let remaining = metadata.point_count - source_index;
        let request = remaining.min(options.chunk_size as u64);
        let point_data = reader.read_points(request)?;
        if point_data.is_empty() {
            break;
        }

        for point in point_data.points() {
            let point = point?;
            if source_index % stride == 0 && points.len() < options.max_points {
                points.push(SamplePoint::from_point(source_index, point));
            }
            source_index += 1;
        }
    }

    Ok(PointSample {
        metadata,
        points,
        stride,
        scanned_points: source_index,
    })
}

/// Sparse, transactional point-classification changes.
///
/// Only source indices that were modified consume memory. Undo restores the
/// previous sparse state; source files are never modified in place.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationEdits {
    changes: BTreeMap<u64, u8>,
    #[serde(skip)]
    history: Vec<Vec<(u64, Option<u8>)>>,
}

impl ClassificationEdits {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, u8)> + '_ {
        self.changes
            .iter()
            .map(|(&source_index, &classification)| (source_index, classification))
    }

    pub fn classification_for(&self, source_index: u64) -> Option<u8> {
        self.changes.get(&source_index).copied()
    }

    /// Applies one classification to a set of source indices as one undoable
    /// transaction. Duplicate indices are collapsed.
    pub fn reclassify(
        &mut self,
        source_indices: impl IntoIterator<Item = u64>,
        classification: u8,
    ) -> usize {
        let unique: BTreeSet<_> = source_indices.into_iter().collect();
        if unique.is_empty() {
            return 0;
        }

        let mut previous = Vec::with_capacity(unique.len());
        for source_index in unique {
            let old = self.changes.insert(source_index, classification);
            previous.push((source_index, old));
        }
        let changed = previous.len();
        self.history.push(previous);
        changed
    }

    /// Undoes the most recent reclassification transaction.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        for (source_index, classification) in previous {
            match classification {
                Some(classification) => {
                    self.changes.insert(source_index, classification);
                }
                None => {
                    self.changes.remove(&source_index);
                }
            }
        }
        true
    }

    pub fn clear(&mut self) {
        if self.changes.is_empty() {
            return;
        }
        let previous = self
            .changes
            .iter()
            .map(|(&source_index, &classification)| (source_index, Some(classification)))
            .collect();
        self.changes.clear();
        self.history.push(previous);
    }
}

/// Result of a full-resolution export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportStats {
    pub points_read: u64,
    pub points_written: u64,
    pub points_reclassified: u64,
    pub point_flags_changed: u64,
    pub elevations_changed: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportProgress {
    pub points_read: u64,
    pub total_points: u64,
}

/// Streams the source cloud to a new LAS/LAZ and applies sparse edits.
///
/// The source header is cloned, including CRS VLRs and extra-byte definitions.
/// The output is written to an adjacent temporary file and renamed only after
/// the writer closes successfully. Existing outputs and in-place replacement
/// are refused.
pub fn export_with_edits(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    edits: &ClassificationEdits,
) -> Result<ExportStats> {
    export_internal(
        input.as_ref(),
        output.as_ref(),
        |source_index, point, stats| {
            if let Some(classification) = edits.classification_for(source_index) {
                apply_classification(point, classification)?;
                stats.points_reclassified += 1;
            }
            Ok(())
        },
        |_| true,
    )
}

/// Streams the source cloud while applying generalized classification, point
/// flag and elevation transactions.
pub fn export_with_patches(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    edits: &EditStore,
) -> Result<ExportStats> {
    export_internal(
        input.as_ref(),
        output.as_ref(),
        |source_index, point, stats| {
            if let Some(patch) = edits.patch_for(source_index) {
                apply_point_patch(point, patch, stats)?;
            }
            Ok(())
        },
        |_| true,
    )
}

pub fn export_with_patches_progress(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    edits: &EditStore,
    continue_export: impl FnMut(ExportProgress) -> bool,
) -> Result<ExportStats> {
    export_internal(
        input.as_ref(),
        output.as_ref(),
        |source_index, point, stats| {
            if let Some(patch) = edits.patch_for(source_index) {
                apply_point_patch(point, patch, stats)?;
            }
            Ok(())
        },
        continue_export,
    )
}

/// One input of a merged multi-file export.
#[derive(Clone, Debug, Default)]
pub struct MergeSource {
    pub path: PathBuf,
    pub edits: EditStore,
    /// Effective source coordinate space. This is normally read from LAS/LAZ
    /// metadata; callers set it when an unreferenced source has explicitly
    /// adopted the drawing CRS.
    pub source_crs: Option<CrsInfo>,
}

/// Streams every source into one merged LAS/LAZ in the given order.
///
/// The first source's header (LAS version, point format, scales, CRS VLRs,
/// extra bytes) becomes the output template; every source must match it.
/// Each point's `point_source_id` is set to its file's ordinal (1..=N) so
/// tile identity survives the merge, and each source's sparse edits are
/// applied against its own record indices. Like the single-file export, the
/// output is written to an adjacent temporary file and renamed only after a
/// successful close.
pub fn export_merged_progress(
    sources: &[MergeSource],
    output: &Path,
    continue_export: impl FnMut(ExportProgress) -> bool,
) -> Result<ExportStats> {
    export_merged_internal(sources, output, None, continue_export)
}

/// Streams multiple LAS/LAZ sources into one output coordinate space.
/// Horizontal XY values and output bounds are transformed into `target_crs`;
/// Z is preserved because no vertical-datum operation is implied.
pub fn export_merged_reprojected_progress(
    sources: &[MergeSource],
    output: &Path,
    target_crs: &CrsInfo,
    continue_export: impl FnMut(ExportProgress) -> bool,
) -> Result<ExportStats> {
    export_merged_internal(sources, output, Some(target_crs), continue_export)
}

fn export_merged_internal(
    sources: &[MergeSource],
    output: &Path,
    target_crs: Option<&CrsInfo>,
    mut continue_export: impl FnMut(ExportProgress) -> bool,
) -> Result<ExportStats> {
    const CHUNK_SIZE: u64 = 65_536;

    if sources.is_empty() {
        return Err(Error::InvalidLimit(
            "a merged export needs at least one source",
        ));
    }
    for source in sources {
        validate_output_path(&source.path, output)?;
    }

    // Compatibility pass: one output point format can only represent sources
    // that agree on version and point format. The legacy entry point also
    // requires one horizontal CRS; the reprojected entry point deliberately
    // accepts mixed source projections.
    let template = Reader::from_path(&sources[0].path)?.header().clone();
    let template_crs = CrsInfo::from_header(&template);
    let template_effective_crs = sources[0]
        .source_crs
        .clone()
        .unwrap_or_else(|| template_crs.clone());
    let mut total_points = template.number_of_points();
    let mut source_crs = vec![template_effective_crs];
    let mut source_bounds = vec![template.bounds()];
    let mut z_scale = template.transforms().z.scale;
    for source in &sources[1..] {
        let header = Reader::from_path(&source.path)?.header().clone();
        if header.version() != template.version()
            || header.point_format() != template.point_format()
        {
            return Err(Error::MergeIncompatible(format!(
                "\"{}\" uses LAS {} point format {:?}, but \"{}\" uses LAS {} point format {:?}",
                sources[0].path.display(),
                template.version(),
                template.point_format(),
                source.path.display(),
                header.version(),
                header.point_format(),
            )));
        }
        let crs = CrsInfo::from_header(&header);
        let effective_crs = source.source_crs.clone().unwrap_or_else(|| crs.clone());
        if target_crs.is_none()
            && !crs_equivalent(&effective_crs, &source_crs[0])
            && !(effective_crs.horizontal_epsg.is_none()
                && effective_crs.proj4.is_none()
                && source_crs[0].horizontal_epsg.is_none()
                && source_crs[0].proj4.is_none())
        {
            return Err(Error::MergeIncompatible(format!(
                "\"{}\" declares {} but \"{}\" declares {}",
                sources[0].path.display(),
                source_crs[0].horizontal_label(),
                source.path.display(),
                effective_crs.horizontal_label(),
            )));
        }
        total_points += header.number_of_points();
        source_crs.push(effective_crs);
        source_bounds.push(header.bounds());
        z_scale = z_scale.min(header.transforms().z.scale);
    }

    let output_header = if let Some(target_crs) = target_crs {
        let target_projection = crs::projection_from_crs(target_crs).ok_or_else(|| {
            Error::Crs(format!(
                "target horizontal CRS is unresolved: {}",
                target_crs.label()
            ))
        })?;
        let mut union_min = [f64::INFINITY; 3];
        let mut union_max = [f64::NEG_INFINITY; 3];
        for ((bounds, source_crs), source) in source_bounds
            .iter()
            .zip(source_crs.iter())
            .zip(sources.iter())
        {
            if !source_crs.is_resolvable() {
                return Err(Error::Crs(format!(
                    "source horizontal CRS is unresolved for \"{}\"",
                    source.path.display()
                )));
            }
            let (min, max) = reproject_bounds_between_crs(
                [bounds.min.x, bounds.min.y, bounds.min.z],
                [bounds.max.x, bounds.max.y, bounds.max.z],
                source_crs,
                target_crs,
            )
            .ok_or_else(|| {
                Error::Crs(format!(
                    "cannot transform bounds for \"{}\" from {} to {}",
                    source.path.display(),
                    source_crs.horizontal_label(),
                    target_crs.horizontal_label()
                ))
            })?;
            for axis in 0..3 {
                union_min[axis] = union_min[axis].min(min[axis]);
                union_max[axis] = union_max[axis].max(max[axis]);
            }
        }

        let mut builder = las::Builder::from(template.clone());
        if builder.version.major < 1 || builder.version.minor < 4 {
            builder.version = las::Version::new(1, 4);
        }
        let horizontal_scale = if target_projection.is_latlong() {
            1.0e-8
        } else {
            0.001
        };
        builder.transforms.x = crs::output_transform(union_min[0], union_max[0], horizontal_scale);
        builder.transforms.y = crs::output_transform(union_min[1], union_max[1], horizontal_scale);
        builder.transforms.z =
            crs::output_transform(union_min[2], union_max[2], z_scale.max(f64::EPSILON));
        let mut header = builder.into_header()?;
        let wkt = target_crs.wkt.clone().or_else(|| {
            target_crs.horizontal_epsg.and_then(|epsg| {
                crs_definitions::from_code(epsg).map(|definition| definition.wkt.to_string())
            })
        });
        let wkt = wkt.ok_or_else(|| {
            Error::Crs(format!(
                "{} has no WKT definition for the merged output",
                target_crs.horizontal_label()
            ))
        })?;
        header
            .set_wkt_crs(wkt.into_bytes())
            .map_err(|error| Error::Crs(format!("cannot write target WKT: {error}")))?;
        header
    } else {
        template
    };

    let temporary = temporary_output_path(output);
    let mut temporary_guard = TemporaryOutput::new(temporary.clone());
    let mut writer = Writer::from_path(&temporary, output_header)?;
    let mut stats = ExportStats::default();

    for (ordinal, source) in sources.iter().enumerate() {
        let source_id = u16::try_from(ordinal + 1).unwrap_or(u16::MAX);
        let mut reader = Reader::from_path(&source.path)?;
        let point_count = reader.header().number_of_points();
        let transform = target_crs
            .filter(|target| !crs_equivalent(&source_crs[ordinal], target))
            .map(|target| {
                let source_projection =
                    crs::projection_from_crs(&source_crs[ordinal]).ok_or_else(|| {
                        Error::Crs(format!(
                            "source horizontal CRS is unresolved: {}",
                            source_crs[ordinal].label()
                        ))
                    })?;
                let target_projection = crs::projection_from_crs(target).ok_or_else(|| {
                    Error::Crs(format!(
                        "target horizontal CRS is unresolved: {}",
                        target.label()
                    ))
                })?;
                Ok::<_, Error>((source_projection, target_projection))
            })
            .transpose()?;
        let mut source_index = 0_u64;
        while source_index < point_count {
            let point_data = reader.read_points((point_count - source_index).min(CHUNK_SIZE))?;
            if point_data.is_empty() {
                break;
            }
            for point in point_data.points() {
                let mut point = point?;
                if let Some(patch) = source.edits.patch_for(source_index) {
                    apply_point_patch(&mut point, patch, &mut stats)?;
                }
                if let Some((source_projection, target_projection)) = transform.as_ref() {
                    let original_z = point.z;
                    let coordinate = crs::transform_coordinate(
                        source_projection,
                        target_projection,
                        (point.x, point.y, point.z),
                    )
                    .map_err(|error| {
                        Error::Crs(format!(
                            "point {source_index} in \"{}\" cannot be transformed: {error}",
                            source.path.display()
                        ))
                    })?;
                    if !coordinate.0.is_finite() || !coordinate.1.is_finite() {
                        return Err(Error::Crs(format!(
                            "point {source_index} in \"{}\" transformed to a non-finite coordinate",
                            source.path.display()
                        )));
                    }
                    point.x = coordinate.0;
                    point.y = coordinate.1;
                    point.z = original_z;
                }
                point.point_source_id = source_id;
                writer.write_point(point)?;
                source_index += 1;
                stats.points_read += 1;
                stats.points_written += 1;
            }
            if !continue_export(ExportProgress {
                points_read: stats.points_read,
                total_points,
            }) {
                return Err(Error::Cancelled("point-cloud export"));
            }
        }
        // Every source record must be represented; a short read silently
        // dropping points would corrupt the merged count.
        if source_index != point_count {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "\"{}\" ended after {source_index} of {point_count} points",
                    source.path.display()
                ),
            )));
        }
    }

    writer.close()?;
    drop(writer);
    fs::rename(&temporary, output)?;
    temporary_guard.commit();
    Ok(stats)
}

fn export_internal(
    input: &Path,
    output: &Path,
    mut apply: impl FnMut(u64, &mut Point, &mut ExportStats) -> Result<()>,
    mut continue_export: impl FnMut(ExportProgress) -> bool,
) -> Result<ExportStats> {
    const CHUNK_SIZE: u64 = 65_536;

    validate_output_path(input, output)?;

    let mut reader = Reader::from_path(input)?;
    let header = reader.header().clone();
    let point_count = header.number_of_points();
    let temporary = temporary_output_path(output);
    let mut temporary_guard = TemporaryOutput::new(temporary.clone());
    let mut writer = Writer::from_path(&temporary, header)?;
    let mut stats = ExportStats::default();

    while stats.points_read < point_count {
        let point_data = reader.read_points((point_count - stats.points_read).min(CHUNK_SIZE))?;
        if point_data.is_empty() {
            break;
        }

        for point in point_data.points() {
            let mut point = point?;
            apply(stats.points_read, &mut point, &mut stats)?;
            writer.write_point(point)?;
            stats.points_read += 1;
            stats.points_written += 1;
        }
        if !continue_export(ExportProgress {
            points_read: stats.points_read,
            total_points: point_count,
        }) {
            return Err(Error::Cancelled("point-cloud export"));
        }
    }

    writer.close()?;
    drop(writer);
    fs::rename(&temporary, output)?;
    temporary_guard.commit();
    Ok(stats)
}

fn apply_point_patch(point: &mut Point, patch: PointPatch, stats: &mut ExportStats) -> Result<()> {
    if let Some(classification) = patch.classification {
        apply_classification(point, classification)?;
        stats.points_reclassified += 1;
    }
    let changes_flag = patch.synthetic.is_some()
        || patch.key_point.is_some()
        || patch.withheld.is_some()
        || patch.overlap.is_some();
    if let Some(value) = patch.synthetic {
        point.is_synthetic = value;
    }
    if let Some(value) = patch.key_point {
        point.is_key_point = value;
    }
    if let Some(value) = patch.withheld {
        point.is_withheld = value;
    }
    if let Some(value) = patch.overlap {
        point.is_overlap = value;
    }
    stats.point_flags_changed += u64::from(changes_flag);
    if let Some(value) = patch.elevation {
        point.z = value;
        stats.elevations_changed += 1;
    }
    Ok(())
}

fn apply_classification(point: &mut Point, classification: u8) -> Result<()> {
    if classification == 12 {
        // LAS 1.4 represents overlap as a flag. las-rs also maps legacy class
        // 12 to Unclassified + this flag when reading old point formats.
        point.classification = Classification::Unclassified;
        point.is_overlap = true;
    } else {
        point.classification = Classification::new(classification)?;
    }
    Ok(())
}

fn validate_output_path(input: &Path, output: &Path) -> Result<()> {
    let supported = output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("las") || extension.eq_ignore_ascii_case("laz")
        });
    if !supported {
        return Err(Error::UnsupportedExtension(output.to_owned()));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.to_owned()));
    }

    let input_absolute = absolute_path(input)?;
    let output_absolute = absolute_path(output)?;
    if input_absolute == output_absolute {
        return Err(Error::SameInputAndOutput(output.to_owned()));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn temporary_output_path(output: &Path) -> PathBuf {
    let extension = output
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("las");
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("point-cloud");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    output.with_file_name(format!(
        ".{stem}.ocs-part-{}-{nonce}.{extension}",
        std::process::id()
    ))
}

struct TemporaryOutput {
    path: PathBuf,
    committed: bool,
}

impl TemporaryOutput {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use las::{point::Format, Builder};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ocs-pointcloud-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: impl AsRef<Path>) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn create_cloud(path: &Path, count: u64) {
        let mut builder = Builder::default();
        builder.point_format = Format::new(3).unwrap();
        let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
        for index in 0..count {
            writer
                .write_point(Point {
                    x: 1000.0 + index as f64,
                    y: 2000.0 + index as f64 * 2.0,
                    z: 100.0 + index as f64 * 0.5,
                    intensity: (100 + index) as u16,
                    classification: Classification::new((index % 6) as u8).unwrap(),
                    return_number: 1,
                    number_of_returns: 1,
                    gps_time: Some(50_000.0 + index as f64),
                    color: Some(las::Color::new(index as u16, 20, 30)),
                    ..Default::default()
                })
                .unwrap();
        }
        writer.close().unwrap();
    }

    #[test]
    fn metadata_and_sample_are_bounded_for_las_and_laz() {
        let directory = TestDirectory::new();
        for name in ["sample.las", "sample.laz"] {
            let path = directory.join(name);
            create_cloud(&path, 101);

            let metadata = inspect(&path).unwrap();
            assert_eq!(101, metadata.point_count);
            assert_eq!(3, metadata.point_format);
            assert_eq!(name.ends_with(".laz"), metadata.compressed);
            assert_eq!([1000.0, 2000.0, 100.0], metadata.bounds_min);

            let sample = sample(
                &path,
                SampleOptions {
                    max_points: 10,
                    chunk_size: 7,
                    stride: None,
                },
            )
            .unwrap();
            assert!(sample.points.len() <= 10);
            assert_eq!(11, sample.stride);
            assert_eq!(0, sample.points[0].source_index);
            assert_eq!(11, sample.points[1].source_index);
            assert_eq!(Some(50_000.0), sample.points[0].gps_time);
        }
    }

    #[test]
    fn reprojection_streams_xy_preserves_z_and_rewrites_crs() {
        let directory = TestDirectory::new();
        let input = directory.join("geographic.las");
        let output = directory.join("web-mercator.las");
        let mut builder = Builder::default();
        builder.version = las::Version::new(1, 4);
        builder.point_format = Format::new(3).unwrap();
        let mut header = builder.into_header().unwrap();
        header
            .set_wkt_crs(
                crs_definitions::from_code(4326)
                    .unwrap()
                    .wkt
                    .as_bytes()
                    .to_vec(),
            )
            .unwrap();
        let mut writer = Writer::from_path(&input, header).unwrap();
        writer
            .write_point(Point {
                x: -88.0,
                y: 41.0,
                z: 182.75,
                gps_time: Some(1.0),
                color: Some(las::Color::new(1, 2, 3)),
                ..Point::default()
            })
            .unwrap();
        writer.close().unwrap();

        let stats =
            reproject_with_patches_progress(&input, &output, &EditStore::default(), 3857, |_| true)
                .unwrap();
        assert_eq!(1, stats.points_written);
        let metadata = inspect(&output).unwrap();
        assert_eq!(Some(3857), metadata.crs.horizontal_epsg);
        let mut reader = Reader::from_path(output).unwrap();
        let points: Vec<_> = reader
            .read_all()
            .unwrap()
            .points()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!((points[0].x + 9_796_113.0).abs() < 10.0);
        assert!((points[0].y - 5_012_342.0).abs() < 10.0);
        assert_eq!(182.75, points[0].z);
    }

    #[test]
    fn sparse_edits_are_transactional_and_undoable() {
        let mut edits = ClassificationEdits::default();
        assert_eq!(2, edits.reclassify([2, 4, 4], 2));
        assert_eq!(Some(2), edits.classification_for(4));
        assert_eq!(2, edits.reclassify([4, 8], 6));
        assert_eq!(Some(6), edits.classification_for(4));
        assert!(edits.undo());
        assert_eq!(Some(2), edits.classification_for(4));
        assert_eq!(None, edits.classification_for(8));
        edits.clear();
        assert!(edits.is_empty());
        assert!(edits.undo());
        assert_eq!(Some(2), edits.classification_for(2));
        assert_eq!(Some(2), edits.classification_for(4));
    }

    #[test]
    fn export_preserves_points_and_applies_edits_to_las_and_laz() {
        let directory = TestDirectory::new();
        for extension in ["las", "laz"] {
            let input = directory.join(format!("input.{extension}"));
            let output = directory.join(format!("output.{extension}"));
            create_cloud(&input, 20);
            let input_metadata = inspect(&input).unwrap();

            let mut edits = ClassificationEdits::default();
            edits.reclassify([1, 7], 2);
            edits.reclassify([9], 12);
            let stats = export_with_edits(&input, &output, &edits).unwrap();
            assert_eq!(20, stats.points_read);
            assert_eq!(20, stats.points_written);
            assert_eq!(3, stats.points_reclassified);

            let output_metadata = inspect(&output).unwrap();
            assert_eq!(input_metadata.point_count, output_metadata.point_count);
            assert_eq!(input_metadata.point_format, output_metadata.point_format);
            assert_eq!(input_metadata.bounds_min, output_metadata.bounds_min);
            assert_eq!(input_metadata.bounds_max, output_metadata.bounds_max);

            let mut reader = Reader::from_path(&output).unwrap();
            let points: Vec<_> = reader
                .read_all()
                .unwrap()
                .points()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert_eq!(2, u8::from(points[1].classification));
            assert_eq!(2, u8::from(points[7].classification));
            assert_eq!(1, u8::from(points[9].classification));
            assert!(points[9].is_overlap);
            assert_eq!(110, points[10].intensity);
            assert_eq!(Some(50_010.0), points[10].gps_time);
            assert_eq!(Some(las::Color::new(10, 20, 30)), points[10].color);
        }
    }

    #[test]
    fn export_refuses_overwrite_and_bad_extensions() {
        let directory = TestDirectory::new();
        let input = directory.join("input.las");
        create_cloud(&input, 1);
        assert!(matches!(
            export_with_edits(&input, &input, &ClassificationEdits::default()),
            Err(Error::OutputExists(_))
        ));
        assert!(matches!(
            export_with_edits(
                &input,
                directory.join("output.txt"),
                &ClassificationEdits::default()
            ),
            Err(Error::UnsupportedExtension(_))
        ));
    }

    #[test]
    fn generalized_edits_selection_and_export_are_source_indexed() {
        let directory = TestDirectory::new();
        let input = directory.join("patch-input.las");
        let output = directory.join("patch-output.las");
        create_cloud(&input, 12);

        let mut edits = EditStore::default();
        assert_eq!(
            2,
            edits.apply(
                "ground cleanup",
                [2, 5, 5],
                PointPatch {
                    classification: Some(2),
                    withheld: Some(true),
                    elevation: Some(321.25),
                    ..PointPatch::default()
                },
            )
        );
        assert_eq!(1, edits.transaction_count());
        assert_eq!(2, SelectionSet::from_indices("picked", [2, 5, 5]).len());
        assert!(edits.undo().is_some());
        assert!(edits.is_empty());
        assert!(edits.redo().is_some());

        let stats = export_with_patches(&input, &output, &edits).unwrap();
        assert_eq!(2, stats.points_reclassified);
        assert_eq!(2, stats.point_flags_changed);
        assert_eq!(2, stats.elevations_changed);
        let mut reader = Reader::from_path(output).unwrap();
        let points: Vec<_> = reader
            .read_all()
            .unwrap()
            .points()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(321.25, points[2].z);
        assert!(points[2].is_withheld);
    }

    #[test]
    fn tiled_cache_is_bounded_and_round_trips_attributes() {
        let directory = TestDirectory::new();
        let input = directory.join("tiled-input.las");
        let cache = directory.join("tiled-input.ocstiles");
        create_cloud(&input, 100);
        let manifest = build_tiled_cache(
            &input,
            &cache,
            TileCacheOptions {
                target_leaf_points: 8,
                read_chunk_size: 11,
                max_depth: 8,
            },
            |_| true,
        )
        .unwrap();
        manifest.validate_source(&input).unwrap();
        assert!(manifest.leaf_level > 0);
        let leaf_count: u64 = manifest
            .tiles
            .iter()
            .filter(|tile| tile.key.level == manifest.leaf_level)
            .map(|tile| tile.point_count)
            .sum();
        assert_eq!(100, leaf_count);

        let root = manifest.select_tiles(
            manifest.source_metadata.bounds_min,
            manifest.source_metadata.bounds_max,
            8,
        );
        assert!(root.iter().map(|tile| tile.point_count).sum::<u64>() <= 8);
        let points = read_tile(&cache, &root[0]).unwrap();
        assert_eq!(Some(50_000.0), points[0].gps_time);
        assert_eq!(Some([0, 20, 30]), points[0].color);
    }

    #[test]
    fn sidecar_persists_sparse_state_and_repairs_relative_paths() {
        let directory = TestDirectory::new();
        let drawing_dir = directory.join("drawings");
        let lidar_dir = directory.join("lidar");
        std::fs::create_dir_all(&drawing_dir).unwrap();
        std::fs::create_dir_all(&lidar_dir).unwrap();
        let drawing = drawing_dir.join("survey.dwg");
        let input = lidar_dir.join("survey.laz");
        create_cloud(&input, 10);
        let mut state = AttachmentState::new("primary", &drawing, &input).unwrap();
        state.edits.apply(
            "mark key point",
            [3],
            PointPatch {
                key_point: Some(true),
                ..PointPatch::default()
            },
        );
        state
            .selection_sets
            .push(SelectionSet::from_indices("review", [3, 4, 5]));
        state.selection_filter.classes = vec![2, 6];
        state.selection_filter.returns = vec![1];
        let sidecar = sidecar_path_for_drawing(&drawing);
        let mut store = SidecarStore::open(&sidecar).unwrap();
        store.save_attachment(&state).unwrap();
        store
            .append_audit("primary", "edit", "marked one key point")
            .unwrap();

        let loaded = store.load_attachment("primary").unwrap().unwrap();
        assert_eq!(Some(input), loaded.resolve_source(&drawing));
        assert_eq!(1, loaded.edits.len());
        assert_eq!(3, loaded.selection_sets[0].len());
        assert_eq!(vec![2, 6], loaded.selection_filter.classes);
        assert_eq!(vec![1], loaded.selection_filter.returns);
        assert_eq!(1, store.audit_log("primary").unwrap().len());
    }

    #[test]
    fn merged_export_writes_all_sources_with_distinct_point_source_ids() {
        let directory = TestDirectory::new();
        let first = directory.join("merge-a.las");
        let second = directory.join("merge-b.las");
        create_cloud(&first, 40);
        create_cloud(&second, 25);
        // Reclassify one point in each source to prove per-file edit routing.
        let mut first_edits = EditStore::default();
        first_edits.apply("class a", [7_u64], PointPatch::classification(6));
        let mut second_edits = EditStore::default();
        second_edits.apply("class b", [3_u64], PointPatch::classification(31));
        let output = directory.join("merged.las");
        let stats = export_merged_progress(
            &[
                MergeSource {
                    path: first,
                    edits: first_edits,
                    source_crs: None,
                },
                MergeSource {
                    path: second,
                    edits: second_edits,
                    source_crs: None,
                },
            ],
            &output,
            |_| true,
        )
        .unwrap();
        assert_eq!(65, stats.points_written);
        assert_eq!(2, stats.points_reclassified);
        let mut reader = Reader::from_path(&output).unwrap();
        let total = reader.header().number_of_points();
        let mut source_ids = std::collections::BTreeSet::new();
        let mut class_six = 0_u64;
        let mut class_thirty_one = 0_u64;
        let mut counts_by_source = std::collections::BTreeMap::new();
        let mut read = 0_u64;
        while read < total {
            let chunk = reader.read_points((total - read).min(65_536)).unwrap();
            if chunk.is_empty() {
                break;
            }
            for point in chunk.points() {
                let point = point.unwrap();
                source_ids.insert(point.point_source_id);
                *counts_by_source
                    .entry(point.point_source_id)
                    .or_insert(0_u64) += 1;
                if u8::from(point.classification) == 6 {
                    class_six += 1;
                }
                if u8::from(point.classification) == 31 {
                    class_thirty_one += 1;
                }
                read += 1;
            }
        }
        // Ordinals 1 and 2 mark which file each point came from; the base
        // clouds classify by index % 6, so classes 6 and 31 can only come
        // from the per-source edits.
        assert_eq!(source_ids, [1, 2].into_iter().collect());
        assert_eq!(counts_by_source[&1], 40);
        assert_eq!(counts_by_source[&2], 25);
        assert_eq!(1, class_six);
        assert_eq!(1, class_thirty_one);
    }

    #[test]
    fn merged_export_reprojects_mixed_sources_into_one_target_crs() {
        let directory = TestDirectory::new();
        let geographic = directory.join("merge-geographic.las");
        let mercator = directory.join("merge-mercator.las");
        let output = directory.join("merge-target.las");

        let mut builder = Builder::default();
        builder.version = las::Version::new(1, 4);
        builder.point_format = Format::new(3).unwrap();
        let mut header = builder.into_header().unwrap();
        header
            .set_wkt_crs(
                crs_definitions::from_code(4326)
                    .unwrap()
                    .wkt
                    .as_bytes()
                    .to_vec(),
            )
            .unwrap();
        let mut writer = Writer::from_path(&geographic, header).unwrap();
        writer
            .write_point(Point {
                x: -88.0,
                y: 41.0,
                z: 182.75,
                gps_time: Some(1.0),
                color: Some(las::Color::new(1, 2, 3)),
                ..Point::default()
            })
            .unwrap();
        writer.close().unwrap();
        reproject_with_patches_progress(
            &geographic,
            &mercator,
            &EditStore::default(),
            3857,
            |_| true,
        )
        .unwrap();

        let target = CrsInfo {
            horizontal_epsg: Some(3857),
            ..Default::default()
        };
        let stats = export_merged_reprojected_progress(
            &[
                MergeSource {
                    path: geographic,
                    edits: EditStore::default(),
                    source_crs: None,
                },
                MergeSource {
                    path: mercator,
                    edits: EditStore::default(),
                    source_crs: None,
                },
            ],
            &output,
            &target,
            |_| true,
        )
        .unwrap();
        assert_eq!(2, stats.points_written);
        assert_eq!(Some(3857), inspect(&output).unwrap().crs.horizontal_epsg);

        let mut reader = Reader::from_path(output).unwrap();
        let chunk = reader.read_points(2).unwrap();
        let points: Vec<_> = chunk.points().map(|point| point.unwrap()).collect();
        assert_eq!(2, points.len());
        assert!((points[0].x - points[1].x).abs() < 0.01);
        assert!((points[0].y - points[1].y).abs() < 0.01);
        assert_eq!(182.75, points[0].z);
        assert_eq!(182.75, points[1].z);
    }

    #[test]
    fn merged_export_refuses_to_overwrite() {
        let directory = TestDirectory::new();
        let input = directory.join("merge-single.las");
        create_cloud(&input, 10);
        let output = directory.join("exists.las");
        std::fs::write(&output, b"taken").unwrap();
        let result = export_merged_progress(
            &[MergeSource {
                path: input,
                edits: EditStore::default(),
                source_crs: None,
            }],
            &output,
            |_| true,
        );
        assert!(matches!(result, Err(Error::OutputExists(_))));
    }

    #[test]
    fn parallel_tile_reads_match_sequential_results() {
        let directory = TestDirectory::new();
        let input = directory.join("parallel-tiles.las");
        let cache = directory.join("parallel-tiles.ocstiles");
        create_cloud(&input, 2_000);
        let manifest = build_tiled_cache(
            &input,
            &cache,
            TileCacheOptions {
                target_leaf_points: 8,
                read_chunk_size: 256,
                max_depth: 8,
            },
            |_| true,
        )
        .unwrap();
        assert!(
            manifest.tiles.len() > MAX_TILE_READ_WORKERS,
            "fixture must produce more tiles than workers, got {}",
            manifest.tiles.len()
        );

        let sequential: Vec<_> = manifest
            .tiles
            .iter()
            .map(|tile| (tile.key, read_tile(&cache, tile).unwrap()))
            .collect();
        let parallel = read_tiles_parallel(&cache, &manifest.tiles, MAX_TILE_READ_WORKERS).unwrap();
        let mut parallel = parallel;
        parallel.sort_by_key(|(key, _)| *key);
        let mut sequential_sorted = sequential;
        sequential_sorted.sort_by_key(|(key, _)| *key);
        assert_eq!(sequential_sorted.len(), parallel.len());
        for ((seq_key, seq_points), (par_key, par_points)) in
            sequential_sorted.iter().zip(parallel.iter())
        {
            assert_eq!(seq_key, par_key);
            assert_eq!(seq_points.len(), par_points.len());
            assert!(seq_points
                .iter()
                .zip(par_points.iter())
                .all(|(left, right)| left.source_index == right.source_index));
        }
    }

    #[test]
    fn parallel_tile_reads_clamp_workers_and_handle_empty_and_errors() {
        let directory = TestDirectory::new();
        let input = directory.join("clamp-tiles.las");
        let cache = directory.join("clamp-tiles.ocstiles");
        create_cloud(&input, 40);
        let manifest = build_tiled_cache(
            &input,
            &cache,
            TileCacheOptions {
                target_leaf_points: 8,
                read_chunk_size: 16,
                max_depth: 8,
            },
            |_| true,
        )
        .unwrap();
        // Empty batches and degenerate worker counts are safe.
        assert!(read_tiles_parallel(&cache, &[], 0).unwrap().is_empty());
        let single = read_tiles_parallel(&cache, &manifest.tiles, 99).unwrap();
        assert_eq!(manifest.tiles.len(), single.len());
        // A corrupt entry fails the whole batch, like the sequential reader.
        let mut broken = manifest.tiles[0].clone();
        broken.file_name = "missing.tile".to_string();
        let result = read_tiles_parallel(&cache, &[broken], 2);
        assert!(result.is_err());
    }

    #[test]
    fn sidecar_migrates_version_one_selection_filters() {
        let directory = TestDirectory::new();
        let sidecar = directory.join("legacy.ocspc");
        let connection = rusqlite::Connection::open(&sidecar).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE attachments (
                    id TEXT PRIMARY KEY,
                    source_relative TEXT,
                    source_absolute TEXT NOT NULL,
                    fingerprint_json TEXT NOT NULL,
                    cache_relative TEXT,
                    display_json TEXT NOT NULL,
                    classes_json TEXT NOT NULL,
                    edits_json TEXT NOT NULL
                 );
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        drop(connection);

        drop(SidecarStore::open(&sidecar).unwrap());
        let connection = rusqlite::Connection::open(&sidecar).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let filter_column: i64 = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info('attachments')
                 WHERE name = 'selection_filter_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let order_column: i64 = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info('attachments')
                 WHERE name = 'order_index'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            4, version,
            "v1 databases migrate through every schema revision"
        );
        assert_eq!(1, filter_column);
        assert_eq!(1, order_column);
    }

    #[test]
    fn ptc_text_round_trip_preserves_custom_classes() {
        let table = parse_ptc(
            "Code,Description,Red,Green,Blue\n100,Radial - <2 ft,255,0,0\n121,Undergrowth - 4 ft,128,128,220\n",
        )
        .unwrap();
        assert_eq!([255, 0, 0], table.color(100));
        assert_eq!("Undergrowth - 4 ft", table.classes[&121].name);
        let reparsed = parse_ptc(&write_ptc(&table)).unwrap();
        assert_eq!(table, reparsed);
    }

    #[test]
    fn cancellable_export_does_not_publish_partial_output() {
        let directory = TestDirectory::new();
        let input = directory.join("cancel-input.las");
        let output = directory.join("cancel-output.las");
        create_cloud(&input, 70_000);
        let result =
            export_with_patches_progress(&input, &output, &EditStore::default(), |_| false);
        assert!(matches!(result, Err(Error::Cancelled(_))));
        assert!(!output.exists());
    }
}
