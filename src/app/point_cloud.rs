//! Native LAS/LAZ attachment and classification workflow.
//!
//! A tab owns a dataset of attached sources. Each source keeps a bounded
//! display sample and sparse edits; the dataset carries the shared display
//! configuration (color mode, class table, filters) for the merged view. The
//! original files remain authoritative until the user explicitly exports a
//! new file.

use super::{Message, OpenCADStudio};
use crate::scene::{
    PointChunk, PointCloudModel, PointCloudPoint, PointStyle, COLOR_MODE_CLASSIFICATION,
    COLOR_MODE_ELEVATION, COLOR_MODE_INTENSITY, COLOR_MODE_LABEL, COLOR_MODE_RETURN,
    COLOR_MODE_RGB, COLOR_MODE_SOURCE,
};
use iced::Task;
use ocs_pointcloud::{
    classification_statistics, parse_ptc, select_brush, select_nearest, select_polygon,
    sidecar_path_for_drawing, write_ptc, AttachmentState, ClassTable, ColorMode, Density,
    DisplaySettings, EditStore, ExportStats, PointFilter, PointPatch, PointSample, SampleOptions,
    SelectionSet, SidecarStore, TileCacheManifest, TileCacheOptions,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

impl OpenCADStudio {
    pub(super) fn create_spatial_project(&mut self, tab_index: usize, path: PathBuf) {
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Spatial Project")
            .to_string();
        self.tabs[tab_index].spatial_project =
            Some((path.clone(), ocs_pointcloud::SpatialProject::new(name)));
        self.save_spatial_project(tab_index, Some(path));
    }

    pub(super) fn open_spatial_project(
        &mut self,
        tab_index: usize,
        path: PathBuf,
    ) -> Task<Message> {
        let mut project = match ocs_pointcloud::SpatialProject::open(&path) {
            Ok(project) => project,
            Err(error) => {
                self.command_line
                    .push_error(format!("SPATIALPROJECTOPEN: {error}").as_str());
                return Task::none();
            }
        };
        let mut queue = ocs_pointcloud::JobQueue {
            max_running: 1,
            jobs: std::mem::take(&mut project.jobs),
        };
        let recovered = queue.recover_interrupted();
        project.jobs = queue.jobs;

        if let Some(section) = project.sections.first() {
            let nx = section.normal[0];
            let ny = section.normal[1];
            let normal_len = (nx * nx + ny * ny).sqrt().max(f64::EPSILON);
            let direction = [ny / normal_len, -nx / normal_len];
            let half = section.axis_length * 0.5;
            self.tabs[tab_index].point_cloud.section =
                Some(crate::scene::model::point_cloud_model::Section {
                    p0: [
                        section.origin[0] - direction[0] * half,
                        section.origin[1] - direction[1] * half,
                    ],
                    p1: [
                        section.origin[0] + direction[0] * half,
                        section.origin[1] + direction[1] * half,
                    ],
                    width_world: section.total_width,
                    mode: crate::scene::model::point_cloud_model::SectionMode::Discard,
                });
        }
        let sources: Vec<PathBuf> = project
            .sources
            .iter()
            .filter(|source| {
                matches!(
                    source.kind,
                    ocs_pointcloud::SourceKind::LasLaz
                        | ocs_pointcloud::SourceKind::Copc
                        | ocs_pointcloud::SourceKind::Derived
                )
            })
            .filter_map(|source| source.resolve(&path))
            .filter(|source| {
                !self.tabs[tab_index]
                    .point_cloud
                    .contains_source_path(source)
            })
            .collect();
        let feature_sources: Vec<PathBuf> = project
            .sources
            .iter()
            .filter(|source| matches!(source.kind, ocs_pointcloud::SourceKind::Feature))
            .filter_map(|source| source.resolve(&path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let project_name = project.name.clone();
        self.tabs[tab_index].spatial_project = Some((path.clone(), project));
        for source in &feature_sources {
            self.import_gis_source(tab_index, source.clone());
        }
        self.command_line.push_output(
            format!(
                "SPATIALPROJECTOPEN: opened \"{project_name}\" with {} point source(s) and {} feature source(s); recovered {recovered} interrupted job(s).",
                sources.len(),
                feature_sources.len()
            )
            .as_str(),
        );
        Task::batch(
            sources
                .into_iter()
                .map(|source| Task::done(Message::PointCloudPathPicked(Some(source)))),
        )
    }

    pub(super) fn save_spatial_project(
        &mut self,
        tab_index: usize,
        path_override: Option<PathBuf>,
    ) {
        let (existing_path, mut project) = self.tabs[tab_index]
            .spatial_project
            .take()
            .unwrap_or_else(|| {
                (
                    PathBuf::new(),
                    ocs_pointcloud::SpatialProject::new("Spatial Project"),
                )
            });
        let path = path_override.unwrap_or(existing_path);
        if path.as_os_str().is_empty() {
            self.command_line
                .push_error("SPATIALPROJECTSAVE: choose a .ocsproj path first.");
            self.tabs[tab_index].spatial_project = Some((path, project));
            return;
        }

        project.sources.retain(|source| {
            !matches!(
                source.kind,
                ocs_pointcloud::SourceKind::LasLaz
                    | ocs_pointcloud::SourceKind::Copc
                    | ocs_pointcloud::SourceKind::Derived
            )
        });
        for source in &self.tabs[tab_index].point_cloud.sources {
            let lower = source.source_path.to_string_lossy().to_ascii_lowercase();
            let kind = if lower.ends_with(".copc.laz") {
                ocs_pointcloud::SourceKind::Copc
            } else {
                ocs_pointcloud::SourceKind::LasLaz
            };
            match ocs_pointcloud::ProjectSource::local(&source.id, &path, &source.source_path, kind)
            {
                Ok(mut catalog) => {
                    catalog.crs = source.source_crs.clone();
                    catalog.point_count = Some(source.sample.metadata.point_count);
                    catalog.bounds_min = Some(source.drawing_bounds.0);
                    catalog.bounds_max = Some(source.drawing_bounds.1);
                    catalog.cache_relative = source.cache_path.as_ref().and_then(|cache| {
                        path.parent()
                            .and_then(|base| cache.strip_prefix(base).ok())
                            .map(PathBuf::from)
                    });
                    project.sources.push(catalog);
                }
                Err(error) => self.command_line.push_error(
                    format!(
                        "SPATIALPROJECTSAVE: cannot catalog {}: {error}",
                        source.source_path.display()
                    )
                    .as_str(),
                ),
            }
        }

        if let Some(section) = self.tabs[tab_index].point_cloud.section {
            let dx = section.p1[0] - section.p0[0];
            let dy = section.p1[1] - section.p0[1];
            let length = (dx * dx + dy * dy).sqrt().max(f64::EPSILON);
            let named = ocs_pointcloud::NamedSection {
                id: "active-section".to_string(),
                name: "Active section".to_string(),
                kind: ocs_pointcloud::SectionKind::CrossSection,
                origin: [
                    (section.p0[0] + section.p1[0]) * 0.5,
                    (section.p0[1] + section.p1[1]) * 0.5,
                    0.0,
                ],
                normal: [-dy / length, dx / length, 0.0],
                axis_length: length,
                total_width: section.width_world,
                vertical_limits: None,
                crs: self.tabs[tab_index]
                    .spatial
                    .drawing_crs
                    .as_ref()
                    .map(crate::app::spatial::DrawingCrs::as_crs_info)
                    .unwrap_or_default(),
                locked: false,
            };
            if let Err(error) = project.upsert_section(named) {
                self.command_line
                    .push_error(format!("SPATIALPROJECTSAVE: {error}").as_str());
            }
        }

        let mut named: BTreeMap<String, BTreeMap<String, SelectionSet>> = BTreeMap::new();
        for source in &self.tabs[tab_index].point_cloud.sources {
            for selection in &source.selection_sets {
                named
                    .entry(selection.name.clone())
                    .or_default()
                    .insert(source.id.clone(), selection.clone());
            }
        }
        project.selections = named
            .into_iter()
            .enumerate()
            .map(
                |(index, (name, point_ranges))| ocs_pointcloud::NamedSelection {
                    id: format!("selection-{}", index + 1),
                    name,
                    objects: BTreeSet::new(),
                    point_ranges,
                },
            )
            .collect();
        if let Some(crs) = self.tabs[tab_index].spatial.drawing_crs.as_ref() {
            project.spatial_reference.horizontal = crs.as_crs_info();
            project.spatial_reference.working_unit =
                format!("{:?}", crs.working_unit()).to_ascii_lowercase();
        }

        match project.save_atomic(&path) {
            Ok(()) => self.command_line.push_output(
                format!(
                    "SPATIALPROJECTSAVE: saved {} source(s), {} section(s), and {} named selection(s) to {}.",
                    project.sources.len(),
                    project.sections.len(),
                    project.selections.len(),
                    path.display()
                )
                .as_str(),
            ),
            Err(error) => self.command_line
                .push_error(format!("SPATIALPROJECTSAVE: {error}").as_str()),
        }
        self.tabs[tab_index].spatial_project = Some((path, project));
    }

    pub(super) fn save_named_point_cloud_section(&mut self, tab_index: usize, name: String) {
        let Some(section) = self.tabs[tab_index].point_cloud.section else {
            self.command_line
                .push_error("POINTCLOUDSECTIONSAVE: no section is active.");
            return;
        };
        let crs = self.tabs[tab_index]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info)
            .unwrap_or_default();
        let Some((project_path, project)) = self.tabs[tab_index].spatial_project.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDSECTIONSAVE: create or open a spatial project first.");
            return;
        };
        let dx = section.p1[0] - section.p0[0];
        let dy = section.p1[1] - section.p0[1];
        let length = (dx * dx + dy * dy).sqrt().max(f64::EPSILON);
        let id = project
            .sections
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&name))
            .map(|item| item.id.clone())
            .unwrap_or_else(|| format!("section-{}", project.sections.len() + 1));
        let named = ocs_pointcloud::NamedSection {
            id,
            name: name.clone(),
            kind: ocs_pointcloud::SectionKind::CrossSection,
            origin: [
                (section.p0[0] + section.p1[0]) * 0.5,
                (section.p0[1] + section.p1[1]) * 0.5,
                0.0,
            ],
            normal: [-dy / length, dx / length, 0.0],
            axis_length: length,
            total_width: section.width_world,
            vertical_limits: None,
            crs,
            locked: false,
        };
        match project
            .upsert_section(named)
            .and_then(|_| project.save_atomic(project_path.clone()))
        {
            Ok(()) => self
                .command_line
                .push_output(format!("POINTCLOUDSECTIONSAVE: saved \"{name}\".").as_str()),
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDSECTIONSAVE: {error}").as_str()),
        }
    }

    pub(super) fn list_named_point_cloud_sections(&mut self, tab_index: usize) {
        let Some((_, project)) = self.tabs[tab_index].spatial_project.as_ref() else {
            self.command_line
                .push_info("POINTCLOUDSECTIONS: no spatial project is open.");
            return;
        };
        if project.sections.is_empty() {
            self.command_line
                .push_info("POINTCLOUDSECTIONS: no named sections.");
            return;
        }
        for section in &project.sections {
            self.command_line.push_output(
                format!(
                    "{} — {} ({:.3} x {:.3} map units{})",
                    section.id,
                    section.name,
                    section.axis_length,
                    section.total_width,
                    if section.locked { ", locked" } else { "" }
                )
                .as_str(),
            );
        }
    }

    pub(super) fn activate_named_point_cloud_section(&mut self, tab_index: usize, id: &str) {
        let section = self.tabs[tab_index]
            .spatial_project
            .as_ref()
            .and_then(|(_, project)| {
                project.sections.iter().find(|section| {
                    section.id.eq_ignore_ascii_case(id) || section.name.eq_ignore_ascii_case(id)
                })
            })
            .cloned();
        let Some(section) = section else {
            self.command_line.push_error(
                format!("POINTCLOUDSECTIONACTIVATE: section \"{id}\" was not found.").as_str(),
            );
            return;
        };
        let normal_length = section.normal[0].hypot(section.normal[1]).max(f64::EPSILON);
        let direction = [
            section.normal[1] / normal_length,
            -section.normal[0] / normal_length,
        ];
        let half = section.axis_length * 0.5;
        self.set_point_cloud_section(
            tab_index,
            [
                section.origin[0] - direction[0] * half,
                section.origin[1] - direction[1] * half,
            ],
            [
                section.origin[0] + direction[0] * half,
                section.origin[1] + direction[1] * half,
            ],
            section.total_width,
            crate::scene::model::point_cloud_model::SectionMode::Discard,
        );
        self.command_line.push_output(
            format!("POINTCLOUDSECTIONACTIVATE: \"{}\" is active.", section.name).as_str(),
        );
    }

    pub(super) fn mutate_named_point_cloud_section(
        &mut self,
        tab_index: usize,
        id: &str,
        action: &str,
        argument: Option<&str>,
    ) {
        let Some((project_path, project)) = self.tabs[tab_index].spatial_project.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDSECTION: no spatial project is open.");
            return;
        };
        let Some(index) = project.sections.iter().position(|section| {
            section.id.eq_ignore_ascii_case(id) || section.name.eq_ignore_ascii_case(id)
        }) else {
            self.command_line
                .push_error(format!("POINTCLOUDSECTION: section \"{id}\" was not found.").as_str());
            return;
        };
        match action {
            "DUPLICATE" => {
                let name = argument.unwrap_or("Section copy");
                let copy = project.sections[index]
                    .duplicate(format!("section-{}", project.sections.len() + 1), name);
                project.sections.push(copy);
            }
            "FLIP" => {
                if project.sections[index].locked {
                    self.command_line
                        .push_error("POINTCLOUDSECTIONFLIP: section is locked.");
                    return;
                }
                project.sections[index].flip();
            }
            "LOCK" => {
                project.sections[index].locked = argument.is_none_or(|value| {
                    !matches!(value.to_ascii_uppercase().as_str(), "OFF" | "NO" | "0")
                });
            }
            "DELETE" => {
                if project.sections[index].locked {
                    self.command_line
                        .push_error("POINTCLOUDSECTIONDELETE: section is locked.");
                    return;
                }
                project.sections.remove(index);
            }
            _ => return,
        }
        match project.save_atomic(project_path.clone()) {
            Ok(()) => self.command_line.push_output(
                format!("POINTCLOUDSECTION{action}: project section state saved.").as_str(),
            ),
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDSECTION{action}: {error}").as_str()),
        }
    }
}

const DISPLAY_POINT_LIMIT: usize = 1_000_000;
const DISPLAY_READ_CHUNK: usize = 65_536;
const MAX_COMMAND_EDIT_POINTS: usize = 5_000_000;
/// GPU cost of one point instance (two position vec4s + attribute vec4 +
/// color/flag vec4); drives the display point budget.
const GPU_POINT_BYTES: usize = crate::scene::pipeline::point_gpu::POINT_INSTANCE_BYTES;

#[derive(Clone, Debug)]
pub struct TileLoadBatch {
    pub source_id: String,
    pub request_id: u64,
    pub camera_generation: u64,
    pub selected: Vec<ocs_pointcloud::TileKey>,
    pub loaded: Vec<(ocs_pointcloud::TileKey, Vec<ocs_pointcloud::SamplePoint>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointCloudSurfaceProduct {
    Dtm,
    Dsm,
    Hillshade,
}

impl PointCloudSurfaceProduct {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dtm => "DTM",
            Self::Dsm => "DSM",
            Self::Hillshade => "Hillshade",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceJobSummary {
    pub rows: usize,
    pub columns: usize,
    pub selected_points: u64,
}

impl OpenCADStudio {
    pub(super) fn start_point_cloud_surface(
        &mut self,
        tab_index: usize,
        product: PointCloudSurfaceProduct,
        cell_size: f64,
        output: PathBuf,
    ) -> Task<Message> {
        let Some(source) = self.tabs[tab_index].point_cloud.active() else {
            self.command_line
                .push_error("POINTCLOUDSURFACE: attach a LAS/LAZ cloud first.");
            return Task::none();
        };
        let source_path = source.source_path.clone();
        let filter = self.tabs[tab_index].point_cloud.selection_filter.clone();
        let extent = self.tabs[tab_index]
            .point_cloud
            .section
            .map(|section| {
                let dx = section.p1[0] - section.p0[0];
                let dy = section.p1[1] - section.p0[1];
                let length = (dx * dx + dy * dy).sqrt().max(f64::EPSILON);
                ocs_pointcloud::ProcessingExtent::Slab {
                    origin: [
                        (section.p0[0] + section.p1[0]) * 0.5,
                        (section.p0[1] + section.p1[1]) * 0.5,
                        0.0,
                    ],
                    normal: [-dy / length, dx / length, 0.0],
                    total_width: section.width_world,
                    vertical_limits: None,
                }
            })
            .unwrap_or(ocs_pointcloud::ProcessingExtent::All);
        let tab_id = self.tabs[tab_index].id;
        let job_id = format!(
            "surface-{}-{}",
            product.label().to_ascii_lowercase(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        if let Some((project_path, project)) = self.tabs[tab_index].spatial_project.as_mut() {
            let mut job = ocs_pointcloud::JobRecord::new(
                format!("lidar.surface.{}", product.label().to_ascii_lowercase()),
                format!("Create {} surface", product.label()),
            );
            job.id = job_id.clone();
            job.inputs = vec![source_path.to_string_lossy().into_owned()];
            job.outputs = vec![output.clone()];
            job.parameters = serde_json::json!({ "cell_size": cell_size, "extent": extent });
            job.start().ok();
            project.jobs.push(job);
            let _ = project.save_atomic(project_path.clone());
        }
        self.command_line.push_info(
            format!(
                "POINTCLOUD{}: full-density processing started (cell size {cell_size}).",
                product.label().to_ascii_uppercase()
            )
            .as_str(),
        );
        Task::perform(
            async move {
                let classification = (product == PointCloudSurfaceProduct::Dtm).then_some(2);
                let statistic = if product == PointCloudSurfaceProduct::Dtm {
                    ocs_pointcloud::GridStatistic::Minimum
                } else {
                    ocs_pointcloud::GridStatistic::Maximum
                };
                let result = ocs_pointcloud::rasterize_full_density(
                    &source_path,
                    cell_size,
                    classification,
                    statistic,
                    &extent,
                    &filter,
                    None,
                    |_| {},
                )
                .and_then(|(surface, progress)| {
                    if product == PointCloudSurfaceProduct::Hillshade {
                        surface.write_hillshade_pgm(&output, 315.0, 45.0, false)?;
                    } else {
                        surface.write_ascii_grid(&output, false)?;
                    }
                    Ok(SurfaceJobSummary {
                        rows: surface.rows,
                        columns: surface.columns,
                        selected_points: progress.selected,
                    })
                })
                .map_err(|error| error.to_string());
                (tab_id, job_id, product, output, result)
            },
            |(tab_id, job_id, product, output, result)| {
                Message::PointCloudSurfaceGenerated(tab_id, job_id, product, output, result)
            },
        )
    }

    pub(super) fn finish_point_cloud_surface(
        &mut self,
        tab_id: u64,
        job_id: String,
        product: PointCloudSurfaceProduct,
        output: PathBuf,
        result: Result<SurfaceJobSummary, String>,
    ) {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let result_text = match &result {
            Ok(summary) => format!(
                "{}x{} grid from {} full-density points",
                summary.columns, summary.rows, summary.selected_points
            ),
            Err(error) => error.clone(),
        };
        let history_inputs = self.tabs[tab_index]
            .point_cloud
            .active()
            .map(|source| vec![source.source_path.to_string_lossy().into_owned()])
            .unwrap_or_default();
        if let Some((project_path, project)) = self.tabs[tab_index].spatial_project.as_mut() {
            if let Some(job) = project.jobs.iter_mut().find(|job| job.id == job_id) {
                match &result {
                    Ok(_) => job.complete(),
                    Err(error) => job.fail(error),
                }
            }
            project
                .history
                .push(ocs_pointcloud::ProcessingHistoryEntry {
                    id: format!("history-{job_id}"),
                    created_unix_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                    tool_id: format!("lidar.surface.{}", product.label().to_ascii_lowercase()),
                    inputs: history_inputs,
                    outputs: vec![output.to_string_lossy().into_owned()],
                    parameters: serde_json::Value::Null,
                    software_version: env!("CARGO_PKG_VERSION").to_string(),
                    crs_transformations: Vec::new(),
                    status: if result.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    }
                    .to_string(),
                    detail: result_text.clone(),
                });
            let _ = project.save_atomic(project_path.clone());
        }
        match result {
            Ok(_) => self.command_line.push_output(
                format!(
                    "POINTCLOUD{}: wrote {result_text} to {}.",
                    product.label().to_ascii_uppercase(),
                    output.display()
                )
                .as_str(),
            ),
            Err(error) => self.command_line.push_error(
                format!(
                    "POINTCLOUD{}: {error}",
                    product.label().to_ascii_uppercase()
                )
                .as_str(),
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UrbanClassificationResult {
    pub outputs: Vec<PathBuf>,
    pub folder_scope: bool,
}

#[derive(Clone, Debug)]
struct ResidentTile {
    points: Arc<Vec<ocs_pointcloud::SamplePoint>>,
    last_used: u64,
}

#[derive(Clone, Copy, Debug)]
struct ProjectedPoint {
    screen: [f32; 2],
    depth: f64,
    sample_index: usize,
}

#[derive(Clone, Debug)]
struct ScreenSpatialIndex {
    camera_generation: u64,
    viewport_size: [u32; 2],
    cell_size: f32,
    cells_x: usize,
    cells_y: usize,
    points: Vec<ProjectedPoint>,
    cells: Vec<Vec<usize>>,
    /// Snapshot of the active points this index was built over; `sample_index`
    /// refers into this, not the (possibly released) `sample.points` buffer.
    snapshot: Arc<Vec<ocs_pointcloud::SamplePoint>>,
}

/// Repeating fixed-pixel brush used by imported TerraScan-style function keys.
pub(super) struct PointCloudBrushClassifyCommand {
    classification: u8,
}

impl PointCloudBrushClassifyCommand {
    pub(super) fn new(classification: u8) -> Self {
        Self { classification }
    }
}

impl crate::command::CadCommand for PointCloudBrushClassifyCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDBRUSHCLASSIFY"
    }

    fn prompt(&self) -> String {
        format!(
            "Classify using brush {}  Click in the viewport; Enter finishes:",
            self.classification
        )
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::Dispatch(format!(
            "POINTCLOUDSCREENBRUSH CLASS {} {:.17} {:.17} {:.17} 32",
            self.classification, point.x, point.y, point.z
        ))
    }

    fn brush_classification(&self) -> Option<u8> {
        Some(self.classification)
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

pub(super) struct PointCloudScreenPointCommand;

impl crate::command::CadCommand for PointCloudScreenPointCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDSELECTPOINT"
    }

    fn prompt(&self) -> String {
        "Select LiDAR point  Click a displayed point:".to_string()
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::Dispatch(format!(
            "POINTCLOUDSCREENPOINT {:.17} {:.17} {:.17} 10",
            point.x, point.y, point.z
        ))
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

/// Pick two displayed LiDAR points and report their true 3D separation. The
/// viewport clicks are only screen anchors; the dispatch handler snaps each to
/// the nearest resident point so Z comes from the cloud, not the drawing plane.
pub(super) struct PointCloudMeasureCommand {
    first: Option<glam::DVec3>,
}

impl PointCloudMeasureCommand {
    pub(super) fn new() -> Self {
        Self { first: None }
    }
}

impl crate::command::CadCommand for PointCloudMeasureCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDMEASURE"
    }

    fn prompt(&self) -> String {
        if self.first.is_some() {
            "LiDAR distance  Click the second displayed point:".to_string()
        } else {
            "LiDAR distance  Click the first displayed point:".to_string()
        }
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        if let Some(first) = self.first.take() {
            crate::command::CmdResult::Dispatch(format!(
                "POINTCLOUDSCREENMEASURE {:.17} {:.17} {:.17} {:.17} {:.17} {:.17} 10",
                first.x, first.y, first.z, point.x, point.y, point.z
            ))
        } else {
            self.first = Some(point);
            crate::command::CmdResult::NeedPoint
        }
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

pub(super) struct PointCloudScreenRectangleCommand {
    first: Option<glam::DVec3>,
}

impl PointCloudScreenRectangleCommand {
    pub(super) fn new() -> Self {
        Self { first: None }
    }
}

impl crate::command::CadCommand for PointCloudScreenRectangleCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDSELECTBOX"
    }

    fn prompt(&self) -> String {
        if self.first.is_some() {
            "LiDAR screen fence  Click opposite corner:".to_string()
        } else {
            "LiDAR screen fence  Click first corner:".to_string()
        }
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        if let Some(first) = self.first.take() {
            crate::command::CmdResult::Dispatch(format!(
                "POINTCLOUDSCREENRECT {:.17} {:.17} {:.17} {:.17} {:.17} {:.17}",
                first.x, first.y, first.z, point.x, point.y, point.z
            ))
        } else {
            self.first = Some(point);
            crate::command::CmdResult::NeedPoint
        }
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

pub(super) struct PointCloudScreenFenceCommand {
    points: Vec<glam::DVec3>,
}

impl PointCloudScreenFenceCommand {
    pub(super) fn new() -> Self {
        Self { points: Vec::new() }
    }
}

impl crate::command::CadCommand for PointCloudScreenFenceCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDSELECTFENCE"
    }

    fn prompt(&self) -> String {
        if self.points.len() < 3 {
            "LiDAR polygon fence  Click at least three vertices:".to_string()
        } else {
            "LiDAR polygon fence  Click next vertex or Enter to close:".to_string()
        }
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        self.points.push(point);
        crate::command::CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        if self.points.len() < 3 {
            return crate::command::CmdResult::Cancel;
        }
        let values = self
            .points
            .iter()
            .map(|point| format!("{:.17} {:.17} {:.17}", point.x, point.y, point.z))
            .collect::<Vec<_>>()
            .join(" ");
        crate::command::CmdResult::Dispatch(format!("POINTCLOUDSCREENFENCE {values}"))
    }
}

pub(super) struct PointCloudSectionCommand {
    first: Option<glam::DVec3>,
}

impl PointCloudSectionCommand {
    pub(super) fn new() -> Self {
        Self { first: None }
    }
}

impl crate::command::CadCommand for PointCloudSectionCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDSECTION"
    }

    fn prompt(&self) -> String {
        if self.first.is_some() {
            "LiDAR section  Click the section end point:".to_string()
        } else {
            "LiDAR section  Click the section start point:".to_string()
        }
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        if let Some(first) = self.first.take() {
            // Use XY only; the section is a vertical plane through two points.
            crate::command::CmdResult::Dispatch(format!(
                "POINTCLOUDSECTION {:.17} {:.17} {:.17} {:.17}",
                first.x, first.y, point.x, point.y
            ))
        } else {
            self.first = Some(point);
            crate::command::CmdResult::NeedPoint
        }
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

pub(super) struct PointCloudScreenBrushCommand;

impl crate::command::CadCommand for PointCloudScreenBrushCommand {
    fn name(&self) -> &'static str {
        "POINTCLOUDSELECTBRUSH"
    }

    fn prompt(&self) -> String {
        "LiDAR selection brush (32 px)  Click repeatedly; Enter finishes:".to_string()
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::Dispatch(format!(
            "POINTCLOUDSCREENBRUSH SELECT {:.17} {:.17} {:.17} 32",
            point.x, point.y, point.z
        ))
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

/// Live state of a native urban classification job, polled by the manager
/// UI on redraw ticks. The background worker mutates the atomics; cancel is
/// a cooperative flag checked between point chunks.
#[derive(Debug)]
pub(super) struct UrbanJobState {
    pub cancel: AtomicBool,
    /// `UrbanStage` discriminant: 0 loading, 1 classifying, 2 validating,
    /// 3 completed.
    pub stage: AtomicU64,
    pub points_done: AtomicU64,
    pub points_total: AtomicU64,
    pub tile_index: AtomicU64,
    pub tile_total: AtomicU64,
    pub building_features: AtomicU64,
    pub road_features: AtomicU64,
    pub tree_features: AtomicU64,
    pub output_path: std::sync::Mutex<PathBuf>,
    pub started_at: std::time::Instant,
}

impl UrbanJobState {
    fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
            stage: AtomicU64::new(0),
            points_done: AtomicU64::new(0),
            points_total: AtomicU64::new(0),
            tile_index: AtomicU64::new(0),
            tile_total: AtomicU64::new(1),
            building_features: AtomicU64::new(0),
            road_features: AtomicU64::new(0),
            tree_features: AtomicU64::new(0),
            output_path: std::sync::Mutex::new(PathBuf::new()),
            started_at: std::time::Instant::now(),
        }
    }

    pub fn snapshot(&self) -> UrbanJobSnapshot {
        UrbanJobSnapshot {
            stage: self.stage.load(Ordering::Relaxed),
            points_done: self.points_done.load(Ordering::Relaxed),
            points_total: self.points_total.load(Ordering::Relaxed),
            tile_index: self.tile_index.load(Ordering::Relaxed),
            tile_total: self.tile_total.load(Ordering::Relaxed),
            building_features: self.building_features.load(Ordering::Relaxed),
            road_features: self.road_features.load(Ordering::Relaxed),
            tree_features: self.tree_features.load(Ordering::Relaxed),
            elapsed_ms: self.started_at.elapsed().as_millis(),
        }
    }
}

/// Redraw-time copy of [`UrbanJobState`] for the manager window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UrbanJobSnapshot {
    pub stage: u64,
    pub points_done: u64,
    pub points_total: u64,
    pub tile_index: u64,
    pub tile_total: u64,
    pub building_features: u64,
    pub road_features: u64,
    pub tree_features: u64,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
struct PointCloudJobProgress {
    completed: AtomicU64,
    total: u64,
    cancel: AtomicBool,
    /// Auxiliary count for index builds (tiles written so far).
    tiles_created: AtomicU64,
}

impl PointCloudJobProgress {
    fn new(total: u64) -> Self {
        Self {
            completed: AtomicU64::new(0),
            total,
            cancel: AtomicBool::new(false),
            tiles_created: AtomicU64::new(0),
        }
    }
}

/// One attached LAS/LAZ source: its sample, sparse edits, selections and
/// streaming state. Display configuration lives on the dataset so the merged
/// view stays visually consistent across sources.
#[derive(Clone, Debug)]
pub(super) struct PointCloudAttachment {
    pub(super) id: String,
    pub(super) source_path: PathBuf,
    pub(super) sample: PointSample,
    /// Effective source coordinate reference. This is LAS metadata when
    /// declared, or the drawing CRS explicitly assumed for an unreferenced
    /// source.
    source_crs: ocs_pointcloud::CrsInfo,
    crs_assumed_from_drawing: bool,
    /// Source metadata bounds transformed into the drawing coordinate space.
    drawing_bounds: ([f64; 3], [f64; 3]),
    pub(super) edits: EditStore,
    pub(super) selection_sets: Vec<SelectionSet>,
    pub(super) cache_path: Option<PathBuf>,
    pub(super) cache_manifest: Option<TileCacheManifest>,
    index_cancel: Option<Arc<AtomicBool>>,
    index_job: Option<Arc<PointCloudJobProgress>>,
    index_error: Option<String>,
    export_job: Option<Arc<PointCloudJobProgress>>,
    resident_tiles: BTreeMap<ocs_pointcloud::TileKey, ResidentTile>,
    active_tiles: Vec<ocs_pointcloud::TileKey>,
    stream_request_id: u64,
    stream_camera_generation: u64,
    stream_in_flight: bool,
    cancelled_tile_requests: u64,
    stale_tile_results: u64,
    lru_clock: u64,
    screen_index: Option<ScreenSpatialIndex>,
}

impl PointCloudAttachment {
    pub(super) fn new(id: String, source_path: PathBuf, sample: PointSample) -> Self {
        let source_crs = sample.metadata.crs.clone();
        let drawing_bounds = (sample.metadata.bounds_min, sample.metadata.bounds_max);
        Self {
            id,
            source_path,
            sample,
            source_crs,
            crs_assumed_from_drawing: false,
            drawing_bounds,
            edits: EditStore::default(),
            selection_sets: Vec::new(),
            cache_path: None,
            cache_manifest: None,
            index_cancel: None,
            index_job: None,
            index_error: None,
            export_job: None,
            resident_tiles: BTreeMap::new(),
            active_tiles: Vec::new(),
            stream_request_id: 0,
            stream_camera_generation: u64::MAX,
            stream_in_flight: false,
            cancelled_tile_requests: 0,
            stale_tile_results: 0,
            lru_clock: 0,
            screen_index: None,
        }
    }

    pub(super) fn suggested_export_name(&self) -> String {
        let stem = self
            .source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("point-cloud");
        let extension = self
            .source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| {
                extension.eq_ignore_ascii_case("las") || extension.eq_ignore_ascii_case("laz")
            })
            .unwrap_or("laz");
        format!("{stem}_classified.{extension}")
    }

    fn align_sample_to_drawing(
        &mut self,
        drawing_crs: &ocs_pointcloud::CrsInfo,
    ) -> Result<(), String> {
        if !self.source_crs.is_resolvable() {
            if self.sample.metadata.has_crs {
                return Err(format!(
                    "source declares {}, but its horizontal projection is not supported; repair or reproject the LAS/LAZ CRS before attaching",
                    self.source_crs.label()
                ));
            }
            self.source_crs = drawing_crs.clone();
            self.crs_assumed_from_drawing = true;
        }
        ocs_pointcloud::reproject_points_between_crs(
            &self.source_crs,
            drawing_crs,
            &mut self.sample.points,
        )
        .map_err(|error| error.to_string())?;
        self.drawing_bounds = ocs_pointcloud::reproject_bounds_between_crs(
            self.sample.metadata.bounds_min,
            self.sample.metadata.bounds_max,
            &self.source_crs,
            drawing_crs,
        )
        .ok_or_else(|| {
            format!(
                "cannot transform source bounds from {} to {}",
                self.source_crs.horizontal_label(),
                drawing_crs.horizontal_label()
            )
        })?;
        Ok(())
    }

    fn activate_cache(
        &mut self,
        cache_path: PathBuf,
        manifest: TileCacheManifest,
        drawing_crs: &ocs_pointcloud::CrsInfo,
    ) -> Result<(), String> {
        let manifest = manifest_in_drawing_crs(manifest, &self.source_crs, drawing_crs)?;
        self.cache_path = Some(cache_path);
        self.cache_manifest = Some(manifest);
        self.stream_camera_generation = u64::MAX;
        Ok(())
    }

    pub(super) fn suggested_reprojected_name(&self, target_epsg: u16) -> String {
        let stem = self
            .source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("point-cloud");
        let extension = self
            .source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| {
                extension.eq_ignore_ascii_case("las") || extension.eq_ignore_ascii_case("laz")
            })
            .unwrap_or("laz");
        format!("{stem}_epsg{target_epsg}.{extension}")
    }

    /// Number of points currently displayed for this source: the streamed
    /// active tiles when the LOD index is active, otherwise the bounded sample.
    pub(super) fn displayed_len(&self) -> usize {
        if self.sample.stride == 0 && !self.active_tiles.is_empty() {
            self.active_tiles
                .iter()
                .filter_map(|key| self.resident_tiles.get(key))
                .map(|tile| tile.points.len())
                .sum()
        } else {
            self.sample.points.len()
        }
    }

    /// A contiguous snapshot of the active working set. Once the LOD index is
    /// built this flattens the active tiles on demand (and is dropped by the
    /// caller), so the streamed working set is never duplicated permanently.
    /// Selection and classification tools take a slice, so they use this
    /// rather than reaching into the resident tile map directly.
    pub(super) fn active_points(&self) -> Vec<ocs_pointcloud::SamplePoint> {
        if self.sample.stride == 0 && !self.active_tiles.is_empty() {
            let capacity = self
                .active_tiles
                .iter()
                .filter_map(|key| self.resident_tiles.get(key))
                .map(|tile| tile.points.len())
                .sum();
            let mut points = Vec::with_capacity(capacity);
            for key in &self.active_tiles {
                if let Some(tile) = self.resident_tiles.get(key) {
                    points.extend(tile.points.iter().cloned());
                }
            }
            points
        } else {
            self.sample.points.clone()
        }
    }
}

/// The tab's point-cloud session: every attached source plus the shared
/// display configuration for the merged view.
#[derive(Clone, Debug, Default)]
pub(super) struct PointCloudDataset {
    pub(super) sources: Vec<PointCloudAttachment>,
    pub(super) display: DisplaySettings,
    pub(super) classes: ClassTable,
    pub(super) selection_filter: PointFilter,
    /// Folder this dataset was attached from, persisted as the sidecar
    /// collection so a restored dataset knows its origin.
    pub(super) collection: Option<ocs_pointcloud::CollectionState>,
    /// Dataset-wide merge-export job (POINTCLOUDEXPORTALL).
    pub(super) export_all_job: Option<Arc<PointCloudJobProgress>>,
    /// `POINTCLOUDINDEX` walks every source sequentially while this is set.
    index_batch_active: bool,
    /// Active vertical cross-section; `None` shows the whole cloud.
    pub(super) section: Option<crate::scene::model::point_cloud_model::Section>,
    /// Native urban classification job; presence means a job is running.
    pub(super) urban_job: Option<Arc<UrbanJobState>>,
    pub(super) urban_status: String,
    /// UPCP label class table, present when an attached source carries a
    /// `label` extra dimension. Label mode colorizes through it instead of
    /// the ASPRS table so the two schemes never overwrite each other.
    pub(super) label_classes: Option<ClassTable>,
    display_generation: u64,
    /// Bumps on style-only changes (color mode, class visibility, class
    /// colors, point size): the GPU rewrites its style uniform, not the
    /// instance buffer.
    style_generation: u64,
    /// Cached sample intensity range for style updates that skip a full
    /// display rebuild.
    resolved_intensity_range: Option<[u16; 2]>,
    /// Ids of the sources touched by the most recent edit action; undo steps
    /// exactly those sources so one cross-source action is undone as one.
    last_edit_sources: Option<Vec<String>>,
}

impl PointCloudDataset {
    pub(super) fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.sources.len()
    }

    /// The active (first) source: the target for single-file commands such as
    /// export, reprojection and index building.
    pub(super) fn active(&self) -> Option<&PointCloudAttachment> {
        self.sources.first()
    }

    pub(super) fn active_mut(&mut self) -> Option<&mut PointCloudAttachment> {
        self.sources.first_mut()
    }

    pub(super) fn source(&self, id: &str) -> Option<&PointCloudAttachment> {
        self.sources.iter().find(|source| source.id == id)
    }

    pub(super) fn source_mut(&mut self, id: &str) -> Option<&mut PointCloudAttachment> {
        self.sources.iter_mut().find(|source| source.id == id)
    }

    pub(super) fn contains_source_path(&self, path: &std::path::Path) -> bool {
        self.sources
            .iter()
            .any(|source| path_matches(&source.source_path, path))
    }

    /// Generates a stable, collision-free sidecar id for a new source.
    pub(super) fn next_source_id(&self) -> String {
        let taken: std::collections::BTreeSet<&str> = self
            .sources
            .iter()
            .map(|source| source.id.as_str())
            .collect();
        let mut counter = self.sources.len() + 1;
        loop {
            let candidate = format!("source-{counter}");
            if !taken.contains(candidate.as_str()) {
                return candidate;
            }
            counter += 1;
        }
    }

    pub(super) fn mark_display_changed(&mut self) {
        self.display_generation = self.display_generation.wrapping_add(1).max(1);
        for source in &mut self.sources {
            source.screen_index = None;
        }
    }

    pub(super) fn mark_style_changed(&mut self) {
        self.style_generation = self.style_generation.wrapping_add(1).max(1);
    }

    pub(super) fn note_edit_sources(&mut self, ids: Vec<String>) {
        self.last_edit_sources = Some(ids);
    }

    pub(super) fn push_selection(&mut self, id: &str, selection: SelectionSet) {
        let name = selection.name.clone();
        if let Some(source) = self.source_mut(id) {
            if let Some(existing) = source
                .selection_sets
                .iter_mut()
                .find(|candidate| candidate.name == name)
            {
                *existing = selection;
            } else {
                source.selection_sets.push(selection);
            }
        }
        self.mark_display_changed();
    }

    fn clear_selections_named(&mut self, name: &str) {
        for source in &mut self.sources {
            source
                .selection_sets
                .retain(|selection| selection.name != name);
        }
        self.mark_display_changed();
    }

    /// Union of every source's metadata bounds for view fitting.
    pub(super) fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut bounds: Option<([f64; 3], [f64; 3])> = None;
        for source in &self.sources {
            let (min, max) = source.drawing_bounds;
            bounds = Some(match bounds {
                None => (min, max),
                Some((mut union_min, mut union_max)) => {
                    for axis in 0..3 {
                        union_min[axis] = union_min[axis].min(min[axis]);
                        union_max[axis] = union_max[axis].max(max[axis]);
                    }
                    (union_min, union_max)
                }
            });
        }
        bounds
    }

    pub(super) fn display_model(&mut self) -> PointCloudModel {
        let mut points = Vec::new();
        let mut chunks = Vec::new();
        let mut chunk_offset: u32 = 0;
        let mut intensity_range = self.display.intensity_range.unwrap_or([u16::MAX, 0]);
        for source in &self.sources {
            let active_selection = source
                .selection_sets
                .iter()
                .find(|selection| selection.name == "active");
            let generation = source_chunk_generation(source);
            let tiled = source.sample.stride == 0 && !source.active_tiles.is_empty();
            // A unified view of the source's active points: the streamed
            // resident tiles when tiled, otherwise the bounded sample. Only
            // references are collected — the points stay owned by
            // `resident_tiles` / `sample`, never duplicated here.
            let active: Vec<&ocs_pointcloud::SamplePoint> = if tiled {
                source
                    .active_tiles
                    .iter()
                    .filter_map(|key| source.resident_tiles.get(key))
                    .flat_map(|tile| tile.points.iter())
                    .collect()
            } else {
                source.sample.points.iter().collect()
            };
            for sampled in active {
                let point = source.edits.patch_for(sampled.source_index).map_or_else(
                    || sampled.clone(),
                    |patch| sampled.clone().with_patch(patch),
                );
                if self.display.intensity_range.is_none() {
                    intensity_range[0] = intensity_range[0].min(point.intensity);
                    intensity_range[1] = intensity_range[1].max(point.intensity);
                }
                // Class visibility is a shader-side mask, not a filter: hiding
                // a class must not rebuild the instance buffer.
                points.push(PointCloudPoint {
                    position: point.position,
                    classification: point.classification,
                    intensity: point.intensity,
                    return_number: point.return_number,
                    point_source_id: point.point_source_id,
                    color: point.color,
                    label: point.label.unwrap_or(0),
                    selected: active_selection
                        .is_some_and(|selection| selection.contains(point.source_index)),
                });
            }
            // Chunk the stream by upload identity: one chunk per streamed
            // tile, or one per source for a bounded sample. The point order
            // built above matches active-tile order, so chunk ranges align.
            if tiled {
                for key in &source.active_tiles {
                    let len = source
                        .resident_tiles
                        .get(key)
                        .map_or(0, |tile| tile.points.len()) as u32;
                    chunks.push(PointChunk {
                        key: tile_chunk_key(&source.id, key),
                        generation,
                        offset: chunk_offset,
                        len,
                    });
                    chunk_offset += len;
                }
            } else {
                let len = source.sample.points.len() as u32;
                chunks.push(PointChunk {
                    key: tile_chunk_key(&source.id, &SAMPLE_CHUNK_TILE),
                    generation,
                    offset: chunk_offset,
                    len,
                });
                chunk_offset += len;
            }
        }
        debug_assert_eq!(chunk_offset as usize, points.len());
        self.resolved_intensity_range = Some(intensity_range);
        PointCloudModel {
            points: Arc::new(points),
            point_size_px: self.display.point_size_px,
            style: self.point_style(),
            chunks,
            geometry_generation: self.display_generation,
            style_generation: self.style_generation,
        }
    }

    /// The colorization state uploaded to the GPU as one uniform write.
    pub(super) fn point_style(&self) -> PointStyle {
        // Label mode drives the same tables from the UPCP class table so
        // visibility and colors describe the urban scheme, not ASPRS.
        let scheme_classes = if matches!(self.display.color_mode, ColorMode::Label) {
            self.label_classes
                .as_ref()
                .cloned()
                .unwrap_or_else(ocs_pointcloud::upcp_class_table)
        } else {
            self.classes.clone()
        };
        let mut class_visible = [0_u32; 8];
        for class in 0..u8::MAX as u32 + 1 {
            let visible = scheme_classes
                .classes
                .get(&(class as u8))
                .map_or(true, |definition| definition.visible)
                && !self.display.hidden_classes.contains(&(class as u8));
            if visible {
                class_visible[(class / 32) as usize] |= 1 << (class % 32);
            }
        }
        let mut class_colors = [[0.92, 0.92, 0.92, 1.0]; 256];
        for class in 0..u8::MAX as u32 + 1 {
            let [red, green, blue] = scheme_classes.color(class as u8);
            class_colors[class as usize] = [
                red as f32 / 255.0,
                green as f32 / 255.0,
                blue as f32 / 255.0,
                1.0,
            ];
        }
        let intensity_range = self
            .resolved_intensity_range
            .unwrap_or(self.display.intensity_range.unwrap_or([0, u16::MAX]));
        let elevation_range = self.display.elevation_range.unwrap_or_else(|| {
            self.bounds()
                .map_or([0.0, 0.0], |(min, max)| [min[2], max[2]])
        });
        PointStyle {
            color_mode: match self.display.color_mode {
                ColorMode::Classification => COLOR_MODE_CLASSIFICATION,
                ColorMode::Rgb => COLOR_MODE_RGB,
                ColorMode::Intensity => COLOR_MODE_INTENSITY,
                ColorMode::Elevation => COLOR_MODE_ELEVATION,
                ColorMode::ReturnNumber => COLOR_MODE_RETURN,
                ColorMode::PointSource => COLOR_MODE_SOURCE,
                ColorMode::Label => COLOR_MODE_LABEL,
            },
            point_size_px: self.display.point_size_px,
            class_visible,
            class_colors,
            intensity_range: [intensity_range[0] as f32, intensity_range[1] as f32],
            elevation_range: [elevation_range[0] as f32, elevation_range[1] as f32],
            section: self.section,
        }
    }
}

impl OpenCADStudio {
    pub(super) fn point_cloud_manager_data(
        &self,
        tab_index: usize,
    ) -> crate::ui::window::point_cloud_manager::PointCloudManagerData {
        let mut data = self
            .tabs
            .get(tab_index)
            .map(|tab| tab.point_cloud.manager_data())
            .unwrap_or_default();
        data.sidecar_available = self
            .tabs
            .get(tab_index)
            .and_then(|tab| tab.current_path.as_ref())
            .is_some_and(|drawing| sidecar_path_for_drawing(drawing).exists());
        if let (Some(drawing), Some(active_id)) = (
            self.tabs
                .get(tab_index)
                .and_then(|tab| tab.current_path.as_ref()),
            self.tabs
                .get(tab_index)
                .and_then(|tab| tab.point_cloud.active().map(|source| source.id.clone())),
        ) {
            let sidecar = sidecar_path_for_drawing(drawing);
            if let Ok(store) = SidecarStore::open(&sidecar) {
                if let Ok(entries) = store.audit_log(&active_id) {
                    data.audit_rows = entries
                        .into_iter()
                        .map(
                            |entry| crate::ui::window::point_cloud_manager::PointCloudAuditRow {
                                created_unix_ms: entry.created_unix_ms,
                                action: entry.action,
                                detail: entry.detail,
                            },
                        )
                        .collect();
                }
            }
        }
        data
    }

    pub(super) fn point_cloud_stream_needed(&self, tab_index: usize) -> bool {
        self.tabs.get(tab_index).is_some_and(|tab| {
            !tab.point_cloud
                .sources
                .iter()
                .any(|cloud| cloud.stream_in_flight)
                && tab.point_cloud.sources.iter().any(|cloud| {
                    cloud.cache_manifest.is_some()
                        && !cloud.stream_in_flight
                        && cloud.stream_camera_generation != tab.scene.camera_generation
                })
        })
    }

    pub(super) fn start_point_cloud_restore(&mut self, tab_index: usize) -> Task<Message> {
        let Some(drawing_path) = self.tabs[tab_index].current_path.clone() else {
            self.command_line
                .push_error("POINTCLOUDRESTORE: save the drawing before restoring its sidecar.");
            return Task::none();
        };
        let sidecar_path = sidecar_path_for_drawing(&drawing_path);
        if !sidecar_path.exists() {
            self.command_line.push_error(
                format!(
                    "POINTCLOUDRESTORE: no sidecar exists at \"{}\".",
                    sidecar_path.display()
                )
                .as_str(),
            );
            return Task::none();
        }
        let states = SidecarStore::open(&sidecar_path).and_then(|store| store.load_attachments());
        let states = match states {
            Ok(states) if states.is_empty() => {
                self.command_line
                    .push_error("POINTCLOUDRESTORE: the sidecar has no attachments.");
                return Task::none();
            }
            Ok(states) => states,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDRESTORE: {error}").as_str());
                return Task::none();
            }
        };
        let mut resolved = Vec::new();
        let mut failed = Vec::new();
        for state in &states {
            match state.resolve_source(&drawing_path) {
                Some(source) => resolved.push(source),
                None => failed.push(state.source_absolute.display().to_string()),
            }
        }
        for path in &failed {
            self.command_line.push_error(
                format!(
                    "POINTCLOUDRESTORE: could not validate \"{}\" by path or fingerprint.",
                    path
                )
                .as_str(),
            );
        }
        if resolved.is_empty() {
            return Task::none();
        }
        self.command_line.push_output(
            format!(
                "POINTCLOUDRESTORE: repaired and validated {} source path(s).",
                resolved.len()
            )
            .as_str(),
        );
        let mut task = Task::none();
        for source in resolved {
            task = self.start_point_cloud_load(source);
        }
        task
    }

    /// Translate a user-chosen [`Density`] into the sampling options used to
    /// build the attach-time display sample.
    fn sample_options_for(density: Density) -> SampleOptions {
        match density {
            Density::Auto => SampleOptions {
                max_points: DISPLAY_POINT_LIMIT,
                chunk_size: DISPLAY_READ_CHUNK,
                stride: None,
            },
            Density::EveryNth(n) => SampleOptions {
                max_points: usize::MAX,
                chunk_size: DISPLAY_READ_CHUNK,
                stride: Some(n.max(1)),
            },
            Density::Full => SampleOptions {
                max_points: usize::MAX,
                chunk_size: DISPLAY_READ_CHUNK,
                stride: Some(1),
            },
        }
    }

    pub(super) fn start_point_cloud_load(&mut self, path: PathBuf) -> Task<Message> {
        if self.tabs[self.active_tab]
            .point_cloud
            .contains_source_path(&path)
        {
            self.command_line.push_info(
                format!(
                    "POINTCLOUDATTACH: \"{}\" is already attached; skipped duplicate.",
                    path.display()
                )
                .as_str(),
            );
            return Task::none();
        }
        let tab_id = self.tabs[self.active_tab].id;
        let mut density = self.tabs[self.active_tab].point_cloud.display.density;
        let budget = self.tabs[self.active_tab]
            .point_cloud
            .display
            .cpu_budget_bytes;
        let options = if density == Density::Full {
            let point_count = ocs_pointcloud::inspect(&path)
                .map(|metadata| metadata.point_count)
                .unwrap_or(u64::MAX);
            if full_density_over_budget(point_count, budget) {
                if find_valid_tile_cache(&path, None).is_some() {
                    self.command_line.push_info(
                        format!(
                            "POINTCLOUDATTACH: \"{}\" is too large to hold at full density in memory; streaming full-resolution tiles from its LOD cache instead.",
                            path.display()
                        )
                        .as_str(),
                    );
                    // A bounded transient sample is shown only until the first
                    // stream tick; the cache is auto-activated in
                    // `install_point_cloud`.
                    density = Density::Auto;
                    Self::sample_options_for(Density::Auto)
                } else {
                    self.tabs[self.active_tab].point_cloud.display.density = Density::Auto;
                    density = Density::Auto;
                    self.command_line.push_error(
                        format!(
                            "POINTCLOUDATTACH: \"{}\" is too large to read at full density within the {} MB CPU budget. Falling back to Auto; build the LOD index (POINTCLOUDINDEX) to view it at full density.",
                            path.display(),
                            budget / (1024 * 1024)
                        )
                        .as_str(),
                    );
                    Self::sample_options_for(Density::Auto)
                }
            } else {
                Self::sample_options_for(Density::Full)
            }
        } else {
            Self::sample_options_for(density)
        };
        let density_desc = match density {
            Density::Auto => "bounded display sample".to_string(),
            Density::EveryNth(n) => format!("1-in-{n} display sample"),
            Density::Full => "full-density display sample".to_string(),
        };
        self.command_line.push_info(
            format!(
                "POINTCLOUDATTACH: reading {density_desc} from \"{}\"...",
                path.display()
            )
            .as_str(),
        );
        let worker_path = path.clone();
        background_task(
            move || {
                let mut sample = ocs_pointcloud::sample(&worker_path, options)
                    .map_err(|error| error.to_string())?;
                // Urban-classified sources carry their UPCP label in an extra
                // byte the LAS sampler cannot see; one sequential pass fills
                // it for the sampled indices. Sources without a label
                // dimension return false and stay untouched.
                let _ = ocs_pointcloud::attach_sample_labels(&worker_path, &mut sample.points);
                Ok(sample)
            },
            move |result| Message::PointCloudLoaded(tab_id, path, result),
        )
    }

    /// Change the dataset load density and re-read every attached source at the
    /// new density. Only the display sample is replaced; sparse edits and
    /// selections survive.
    pub(super) fn set_point_cloud_density(&mut self, i: usize, density: Density) -> Task<Message> {
        self.tabs[i].point_cloud.display.density = density;
        let tab_id = self.tabs[i].id;
        let budget = self.tabs[i].point_cloud.display.cpu_budget_bytes;
        let drawing_crs = self.tabs[i]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info);
        // Snapshot per-source state so the worker closures below don't borrow
        // the dataset, and so full-density requests can keep already-streamed
        // sources streaming instead of materializing the whole file again.
        let sources: Vec<(String, PathBuf, u64, bool)> = self.tabs[i]
            .point_cloud
            .sources
            .iter()
            .map(|source| {
                (
                    source.id.clone(),
                    source.source_path.clone(),
                    source.sample.metadata.point_count,
                    source.cache_manifest.is_some(),
                )
            })
            .collect();
        let count = sources.len();
        let desc = match density {
            Density::Auto => "Auto".to_string(),
            Density::EveryNth(n) => format!("1-in-{n}"),
            Density::Full => "Full".to_string(),
        };
        self.command_line.push_info(
            format!("POINTCLOUDDENSITY: density set to {desc}; re-reading {count} source(s)...")
                .as_str(),
        );
        if sources.is_empty() {
            return Task::none();
        }
        let requested_options = Self::sample_options_for(density);
        let auto_options = Self::sample_options_for(Density::Auto);
        let mut tasks: Vec<Task<Message>> = Vec::new();
        let mut skipped_streaming = 0usize;
        let mut fell_back = false;
        for (source_id, path, point_count, cache_active) in sources {
            if density == Density::Full && full_density_over_budget(point_count, budget) {
                // Full density would exceed the CPU budget. Keep streaming from
                // the LOD cache when it is (or can be) active, otherwise fall
                // back to the bounded Auto sample rather than OOMing.
                if cache_active {
                    skipped_streaming += 1;
                    continue;
                }
                if let Some((cache_path, manifest)) = find_valid_tile_cache(&path, None) {
                    if let Some(source) = self.tabs[i].point_cloud.source_mut(&source_id) {
                        let target = drawing_crs
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| source.source_crs.clone());
                        if let Err(error) = source.activate_cache(cache_path, manifest, &target) {
                            self.command_line.push_error(
                                format!("POINTCLOUDDENSITY: LOD cache ignored: {error}").as_str(),
                            );
                            continue;
                        }
                    }
                    tasks.push(deferred_message(Message::PointCloudStreamTick(i)));
                    skipped_streaming += 1;
                    continue;
                }
                fell_back = true;
                let worker_path = path.clone();
                tasks.push(background_task(
                    move || {
                        ocs_pointcloud::sample(&worker_path, auto_options)
                            .map_err(|e| e.to_string())
                    },
                    move |result| Message::PointCloudResampled(tab_id, source_id, result),
                ));
                continue;
            }
            let worker_path = path.clone();
            tasks.push(background_task(
                move || {
                    ocs_pointcloud::sample(&worker_path, requested_options)
                        .map_err(|e| e.to_string())
                },
                move |result| Message::PointCloudResampled(tab_id, source_id, result),
            ));
        }
        if skipped_streaming > 0 {
            self.command_line.push_info(
                format!(
                    "POINTCLOUDDENSITY: {skipped_streaming} source(s) stream full resolution from their LOD cache; left streaming."
                )
                .as_str(),
            );
        }
        if fell_back {
            self.tabs[i].point_cloud.display.density = Density::Auto;
            self.command_line.push_error(
                format!(
                    "POINTCLOUDDENSITY: one or more sources are too large to read at full density within the {} MB CPU budget; those sources fell back to Auto. Build the LOD index (POINTCLOUDINDEX) to view them at full density.",
                    budget / (1024 * 1024)
                )
                .as_str(),
            );
        }
        Task::batch(tasks)
    }

    /// Swap a freshly re-sampled display sample into an existing source and
    /// rebuild the merged view.
    pub(super) fn install_point_cloud_resample(
        &mut self,
        tab_id: u64,
        source_id: String,
        result: Result<PointSample, String>,
    ) -> Task<Message> {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Task::none();
        };
        let mut sample = match result {
            Ok(sample) => sample,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDDENSITY: {error}").as_str());
                return Task::none();
            }
        };
        let drawing_crs = self.tabs[tab_index]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info);
        if let (Some(target), Some(source)) = (
            drawing_crs.as_ref(),
            self.tabs[tab_index].point_cloud.source(&source_id),
        ) {
            if !source.crs_assumed_from_drawing {
                if let Err(error) = ocs_pointcloud::reproject_points_between_crs(
                    &source.source_crs,
                    target,
                    &mut sample.points,
                ) {
                    self.command_line
                        .push_error(format!("POINTCLOUDDENSITY: {error}").as_str());
                    return Task::none();
                }
            }
        }
        let model = {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            if let Some(source) = dataset.source_mut(&source_id) {
                source.sample = sample;
                // The display now returns to the sample path (stride != 0), so
                // release any streamed LOD tiles and streaming state instead of
                // holding the point set twice.
                source.resident_tiles.clear();
                source.active_tiles.clear();
                source.cache_manifest = None;
                source.cache_path = None;
                source.stream_in_flight = false;
                source.stream_request_id = 0;
                source.stream_camera_generation = u64::MAX;
                source.screen_index = None;
            }
            dataset.mark_display_changed();
            dataset.display_model()
        };
        self.tabs[tab_index].scene.set_point_cloud(model);
        Task::none()
    }

    /// Attaches every LAS/LAZ file under `folder` (recursively). Loads are
    /// queued and run one at a time so a large folder cannot exhaust memory
    /// with dozens of concurrent bounded samples.
    pub(super) fn start_point_cloud_folder_load(&mut self, folder: PathBuf) -> Task<Message> {
        if !folder.is_dir() {
            self.command_line.push_error(
                format!(
                    "POINTCLOUDATTACHFOLDER: \"{}\" is not a folder.",
                    folder.display()
                )
                .as_str(),
            );
            return Task::none();
        }
        let files = scan_lidar_folder(&folder);
        if files.is_empty() {
            self.command_line.push_error(
                format!(
                    "POINTCLOUDATTACHFOLDER: no .las/.laz files under \"{}\".",
                    folder.display()
                )
                .as_str(),
            );
            return Task::none();
        }
        let tab_id = self.tabs[self.active_tab].id;
        let already_attached: Vec<PathBuf> = self.tabs[self.active_tab]
            .point_cloud
            .sources
            .iter()
            .map(|source| source.source_path.clone())
            .collect();
        let queued_already: usize = self
            .point_cloud_load_queue
            .iter()
            .filter(|(id, _)| *id == tab_id)
            .count();
        let mut fresh = Vec::new();
        for file in files {
            if already_attached
                .iter()
                .any(|attached| path_matches(attached, &file))
            {
                continue;
            }
            if self
                .point_cloud_load_queue
                .iter()
                .any(|(id, path)| *id == tab_id && path_matches(path, &file))
            {
                continue;
            }
            fresh.push(file);
        }
        let skipped = queued_already;
        if fresh.is_empty() {
            self.command_line.push_info(
                format!(
                    "POINTCLOUDATTACHFOLDER: every LAS/LAZ under \"{}\" is already attached or queued.",
                    folder.display()
                )
                .as_str(),
            );
            return Task::none();
        }
        let count = fresh.len();
        // A full-density read of a large folder can exceed the CPU budget by
        // an order of magnitude. Estimate the cost up front (header reads are
        // cheap) and fall back to Auto with a hint, rather than OOMing mid-load.
        let density = self.tabs[self.active_tab].point_cloud.display.density;
        if density == Density::Full {
            let mut total_points: u64 = 0;
            for file in &fresh {
                if let Ok(meta) = ocs_pointcloud::inspect(file) {
                    total_points = total_points.saturating_add(meta.point_count);
                }
            }
            let per_point = std::mem::size_of::<ocs_pointcloud::SamplePoint>() as u64;
            let est_bytes = total_points.saturating_mul(per_point);
            let budget = self.tabs[self.active_tab]
                .point_cloud
                .display
                .cpu_budget_bytes as u64;
            if est_bytes > budget {
                let suggested = (est_bytes as f64 / budget as f64).ceil().max(2.0) as u64;
                self.tabs[self.active_tab].point_cloud.display.density = Density::Auto;
                self.command_line.push_error(
                    format!(
                        "POINTCLOUDATTACHFOLDER: full density needs ~{} MB for {} points, over the {} MB budget. Falling back to Auto; use POINTCLOUDDENSITY {} to adjust.",
                        est_bytes / (1024 * 1024),
                        total_points,
                        budget / (1024 * 1024),
                        suggested
                    )
                    .as_str(),
                );
            }
        }
        for file in &fresh {
            self.point_cloud_load_queue.push((tab_id, file.clone()));
        }
        if self.tabs[self.active_tab].point_cloud.collection.is_none() {
            let display_name = folder
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("folder")
                .to_string();
            self.tabs[self.active_tab].point_cloud.collection =
                Some(ocs_pointcloud::CollectionState {
                    id: format!("folder-{tab_id}"),
                    display_name,
                    source_folder: Some(folder.to_string_lossy().into_owned()),
                    created_unix_ms: None,
                });
        }
        self.command_line.push_info(
            format!(
                "POINTCLOUDATTACHFOLDER: queued {count} LAS/LAZ file(s) from \"{}\"; {skipped} already queued; attaching one at a time...",
                folder.display()
            )
            .as_str(),
        );
        self.start_next_queued_point_cloud(tab_id)
    }

    /// Starts the next queued folder load for `tab_id`, dropping stale entries
    /// for closed tabs and skipping files that disappeared.
    pub(super) fn start_next_queued_point_cloud(&mut self, tab_id: u64) -> Task<Message> {
        let live_tab_ids: std::collections::HashSet<u64> =
            self.tabs.iter().map(|tab| tab.id).collect();
        self.point_cloud_load_queue
            .retain(|(id, _)| live_tab_ids.contains(id));
        while let Some((queued_id, path)) = self.point_cloud_load_queue.first().cloned() {
            self.point_cloud_load_queue.remove(0);
            if queued_id != tab_id {
                // Another tab's entry surfaced; requeue it at the back.
                self.point_cloud_load_queue.push((queued_id, path));
                if self
                    .point_cloud_load_queue
                    .iter()
                    .all(|(id, _)| *id != tab_id)
                {
                    return Task::none();
                }
                continue;
            }
            if !path.is_file() {
                self.command_line.push_error(
                    format!(
                        "POINTCLOUDATTACHFOLDER: skipped \"{}\"; the file is no longer reachable.",
                        path.display()
                    )
                    .as_str(),
                );
                continue;
            }
            let tab_index = self
                .tabs
                .iter()
                .position(|tab| tab.id == tab_id)
                .expect("live tab id");
            let prior_active = self.active_tab;
            self.active_tab = tab_index;
            let task = self.start_point_cloud_load(path);
            self.active_tab = prior_active;
            return task;
        }
        Task::none()
    }

    pub(super) fn install_point_cloud(
        &mut self,
        tab_id: u64,
        path: PathBuf,
        result: Result<PointSample, String>,
    ) -> Task<Message> {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            self.command_line
                .push_info("POINTCLOUDATTACH: target drawing was closed.");
            self.point_cloud_load_queue.retain(|(id, _)| *id != tab_id);
            return Task::none();
        };
        let sample = match result {
            Ok(sample) => sample,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDATTACH: {error}").as_str());
                return self.start_next_queued_point_cloud(tab_id);
            }
        };

        // A second picker/queue completion can race the first background read.
        // Recheck at installation time so one physical source never becomes
        // two live attachments even when both reads were already in flight.
        if self.tabs[tab_index].point_cloud.contains_source_path(&path) {
            self.command_line.push_info(
                format!(
                    "POINTCLOUDATTACH: \"{}\" is already attached; skipped duplicate.",
                    path.display()
                )
                .as_str(),
            );
            return deferred_message(Message::PointCloudQueuePump(tab_id));
        }

        let id = self.tabs[tab_index].point_cloud.next_source_id();
        let mut attachment = PointCloudAttachment::new(id, path.clone(), sample);
        // Auto scheme: switch to UPCP label coloring when the attached
        // source actually carries urban labels (the sampler only fills them
        // when a typed label dimension and provenance exist).
        if attachment
            .sample
            .points
            .iter()
            .any(|point| point.label.is_some())
        {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            if dataset.label_classes.is_none() {
                dataset.label_classes = Some(ocs_pointcloud::upcp_class_table());
            }
            if matches!(dataset.display.color_mode, ColorMode::Classification) {
                dataset.display.color_mode = ColorMode::Label;
                self.command_line.push_info(
                    "POINTCLOUDATTACH: urban labels detected; coloring by UPCP label (POINTCLOUDCOLOR CLASS switches back).",
                );
            }
        }
        if attachment.sample.metadata.has_crs && !attachment.source_crs.is_resolvable() {
            self.command_line.push_error(
                format!(
                    "POINTCLOUDATTACH: source declares {}, but its horizontal projection is not supported; repair or reproject the LAS/LAZ CRS before attaching.",
                    attachment.source_crs.label()
                )
                .as_str(),
            );
            return self.start_next_queued_point_cloud(tab_id);
        }
        let mut inferred_drawing_crs = None;
        if self.tabs[tab_index].spatial.drawing_crs.is_none()
            && attachment.source_crs.is_resolvable()
        {
            match crate::app::spatial::DrawingCrs::from_crs_info(&attachment.source_crs) {
                Ok(crs) => {
                    self.tabs[tab_index].spatial.working_unit = crs.working_unit();
                    self.tabs[tab_index].spatial.drawing_crs = Some(crs.clone());
                    self.basemap.projection = crate::scene::basemap::BasemapProjection::FromDrawing;
                    inferred_drawing_crs = Some(crs.label());
                }
                Err(error) => {
                    self.command_line.push_error(
                        format!("POINTCLOUDATTACH: cannot adopt the source CRS: {error}").as_str(),
                    );
                    return self.start_next_queued_point_cloud(tab_id);
                }
            }
        }
        if let Some(drawing_crs) = self.tabs[tab_index]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info)
        {
            if let Err(error) = attachment.align_sample_to_drawing(&drawing_crs) {
                self.command_line
                    .push_error(format!("POINTCLOUDATTACH: {error}").as_str());
                return self.start_next_queued_point_cloud(tab_id);
            }
        }
        let mut restored_sidecar = false;
        if let Some(drawing_path) = self.tabs[tab_index].current_path.as_ref() {
            let sidecar_path = sidecar_path_for_drawing(drawing_path);
            if sidecar_path.exists() {
                match SidecarStore::open(&sidecar_path).and_then(|store| store.load_attachments()) {
                    Ok(states) => {
                        if let Some(mut state) = states.into_iter().find(|state| {
                            path_matches(&state.source_absolute, &path)
                                || state.source_fingerprint.matches_path(&path)
                        }) {
                            state.edits.normalize_after_load();
                            attachment.edits = state.edits;
                            attachment.selection_sets = state.selection_sets;
                            attachment.cache_path = state
                                .cache_relative
                                .and_then(|relative| {
                                    drawing_path.parent().map(|parent| parent.join(relative))
                                })
                                .filter(|candidate| candidate.exists());
                            let dataset = &mut self.tabs[tab_index].point_cloud;
                            if dataset.sources.is_empty() {
                                // The first restored source also restores the
                                // dataset-wide display configuration.
                                dataset.display = state.display;
                                dataset.classes = state.classes;
                                dataset.selection_filter = state.selection_filter;
                            }
                            restored_sidecar = true;
                        }
                    }
                    Err(error) => self.command_line.push_error(
                        format!("POINTCLOUDATTACH: could not read sidecar: {error}").as_str(),
                    ),
                }
            }
        }
        if let Some((cache_path, manifest)) =
            find_valid_tile_cache(&path, attachment.cache_path.as_deref())
        {
            let drawing_crs = self.tabs[tab_index]
                .spatial
                .drawing_crs
                .as_ref()
                .map(crate::app::spatial::DrawingCrs::as_crs_info)
                .unwrap_or_else(|| attachment.source_crs.clone());
            if let Err(error) = attachment.activate_cache(cache_path, manifest, &drawing_crs) {
                self.command_line
                    .push_error(format!("POINTCLOUDATTACH: LOD cache ignored: {error}").as_str());
            }
        }
        let metadata = &attachment.sample.metadata;
        let point_count = metadata.point_count;
        let sampled = attachment.sample.points.len();
        let format = metadata.point_format;
        let version = format!("{}.{}", metadata.version_major, metadata.version_minor);
        let compressed = if metadata.compressed { "LAZ" } else { "LAS" };
        let bounds_min = metadata.bounds_min;
        let bounds_max = metadata.bounds_max;
        let source_id = attachment.id.clone();
        let dataset_had_sources = !self.tabs[tab_index].point_cloud.is_empty();
        let model = {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            dataset.sources.push(attachment);
            dataset.mark_display_changed();
            dataset.display_model()
        };
        self.tabs[tab_index].scene.set_point_cloud(model);
        if let Some((union_min, union_max)) = self.tabs[tab_index].point_cloud.bounds() {
            self.tabs[tab_index]
                .scene
                .fit_external_bounds(union_min, union_max);
        }

        let source_label = if dataset_had_sources {
            let count = self.tabs[tab_index].point_cloud.len();
            format!(" (source {count} in dataset)")
        } else {
            String::new()
        };
        self.command_line.push_output(
            format!(
                "POINTCLOUDATTACH: {} points ({compressed}, LAS {version}, format {format}); displaying {sampled} sampled points{source_label}. Bounds [{:.3}, {:.3}, {:.3}] to [{:.3}, {:.3}, {:.3}].",
                point_count,
                bounds_min[0],
                bounds_min[1],
                bounds_min[2],
                bounds_max[0],
                bounds_max[1],
                bounds_max[2],
            )
            .as_str(),
        );
        if restored_sidecar {
            self.command_line.push_output(
                "POINTCLOUDATTACH: restored display settings, selections and sparse edits from the drawing sidecar.",
            );
        }
        if let Some(label) = inferred_drawing_crs {
            self.sync_basemap_dropdown();
            self.persist_spatial_settings(tab_index);
            self.command_line.push_output(
                format!(
                    "CRS: drawing CRS inferred automatically from LAS/LAZ metadata as {label}."
                )
                .as_str(),
            );
        } else if self.tabs[tab_index]
            .point_cloud
            .source(&source_id)
            .is_some_and(|source| source.crs_assumed_from_drawing)
        {
            let label = self.tabs[tab_index]
                .spatial
                .drawing_crs
                .as_ref()
                .map(crate::app::spatial::DrawingCrs::label)
                .unwrap_or_else(|| "the drawing CRS".to_string());
            self.command_line.push_info(
                format!(
                    "CRS: this source has no CRS metadata; its coordinates are being interpreted as {label}."
                )
                .as_str(),
            );
        }
        self.persist_point_cloud(
            tab_index,
            "attach",
            "attached point cloud",
            &[source_id.clone()],
        );
        let stream_task = if self.tabs[tab_index]
            .point_cloud
            .source(&source_id)
            .is_some_and(|source| source.cache_manifest.is_some())
        {
            deferred_message(Message::PointCloudStreamTick(tab_index))
        } else {
            Task::none()
        };
        let more_folder_sources = self
            .point_cloud_load_queue
            .iter()
            .any(|(queued_tab, _)| *queued_tab == tab_id);
        let basemap_task = if !more_folder_sources
            && self.basemap.provider != crate::scene::basemap::BasemapProvider::Off
        {
            self.refresh_basemap(tab_id)
        } else {
            Task::none()
        };
        // Both continuations are deferred: completing them inline would nest
        // event-loop dispatch frames (see deferred_message).
        Task::batch([
            stream_task,
            basemap_task,
            deferred_message(Message::PointCloudQueuePump(tab_id)),
        ])
    }

    pub(super) fn point_cloud_info(&mut self, tab_index: usize) {
        let dataset = &self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            self.command_line
                .push_info("POINTCLOUDINFO: no LAS/LAZ cloud is attached.");
            return;
        }
        for source in &dataset.sources {
            let metadata = &source.sample.metadata;
            self.command_line.push_output(
                format!(
                    "POINTCLOUDINFO [{}]: \"{}\"; {} source points; {} displayed (stride {}); {} pending classification edits; CRS metadata: {}; VLRs: {}, EVLRs: {}.",
                    source.id,
                    source.source_path.display(),
                    metadata.point_count,
                    source.displayed_len(),
                    source.sample.stride,
                    source.edits.len(),
                    if metadata.has_crs { metadata.crs.label() } else { "not declared".to_string() },
                    metadata.vlr_count,
                    metadata.evlr_count,
                )
                .as_str(),
            );
        }
        if dataset.len() > 1 {
            let total: u64 = dataset
                .sources
                .iter()
                .map(|source| source.sample.metadata.point_count)
                .sum();
            self.command_line.push_output(
                format!(
                    "POINTCLOUDINFO: {} source(s), {} total source points.",
                    dataset.len(),
                    total
                )
                .as_str(),
            );
        }
    }

    /// Re-express every attached source in a new drawing CRS. Source files and
    /// cache records remain untouched; bounded samples and cache tile bounds
    /// are transformed for the session, and resident tiles are reloaded.
    pub(super) fn reproject_point_cloud_to_drawing_crs(
        &mut self,
        tab_index: usize,
        old_drawing_crs: Option<&ocs_pointcloud::CrsInfo>,
        new_drawing_crs: &ocs_pointcloud::CrsInfo,
    ) -> Result<(), String> {
        if self.tabs[tab_index].point_cloud.is_empty() {
            return Ok(());
        }
        if !new_drawing_crs.is_resolvable() {
            return Err("the requested drawing CRS is not resolvable".to_string());
        }

        // Validate every transform before mutating any source so a bad CRS
        // never leaves a partially transformed dataset.
        for source in &self.tabs[tab_index].point_cloud.sources {
            if source.crs_assumed_from_drawing
                || (!source.sample.metadata.has_crs && !source.source_crs.is_resolvable())
            {
                continue;
            }
            let current = old_drawing_crs.unwrap_or(&source.source_crs);
            let center = [
                (source.drawing_bounds.0[0] + source.drawing_bounds.1[0]) * 0.5,
                (source.drawing_bounds.0[1] + source.drawing_bounds.1[1]) * 0.5,
            ];
            ocs_pointcloud::reproject_between_crs(current, new_drawing_crs, center[0], center[1])
                .ok_or_else(|| {
                format!(
                    "cannot transform {} from {} to {}",
                    source.source_path.display(),
                    current.horizontal_label(),
                    new_drawing_crs.horizontal_label()
                )
            })?;
        }

        for source in &mut self.tabs[tab_index].point_cloud.sources {
            if source.crs_assumed_from_drawing
                || (!source.sample.metadata.has_crs && !source.source_crs.is_resolvable())
            {
                // An unreferenced source is assigned, not reprojected: its
                // numeric coordinates define the newly selected drawing CRS.
                source.source_crs = new_drawing_crs.clone();
                source.crs_assumed_from_drawing = true;
            } else {
                let current = old_drawing_crs.unwrap_or(&source.source_crs);
                ocs_pointcloud::reproject_points_between_crs(
                    current,
                    new_drawing_crs,
                    &mut source.sample.points,
                )
                .map_err(|error| error.to_string())?;
            }
            source.drawing_bounds = ocs_pointcloud::reproject_bounds_between_crs(
                source.sample.metadata.bounds_min,
                source.sample.metadata.bounds_max,
                &source.source_crs,
                new_drawing_crs,
            )
            .ok_or_else(|| {
                format!(
                    "cannot transform bounds for {}",
                    source.source_path.display()
                )
            })?;
            source.resident_tiles.clear();
            source.active_tiles.clear();
            source.stream_in_flight = false;
            source.stream_request_id = source.stream_request_id.wrapping_add(1).max(1);
            source.stream_camera_generation = u64::MAX;
            source.screen_index = None;

            source.cache_manifest = source.cache_path.as_ref().and_then(|cache_path| {
                TileCacheManifest::open(cache_path)
                    .ok()
                    .and_then(|manifest| {
                        manifest_in_drawing_crs(manifest, &source.source_crs, new_drawing_crs).ok()
                    })
            });
        }
        self.tabs[tab_index].point_cloud.mark_display_changed();
        let model = self.tabs[tab_index].point_cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        if let Some((min, max)) = self.tabs[tab_index].point_cloud.bounds() {
            self.tabs[tab_index].scene.fit_external_bounds(min, max);
        }
        Ok(())
    }

    pub(super) fn point_cloud_crs_info(&mut self, tab_index: usize) {
        let dataset = &self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            self.command_line
                .push_error("POINTCLOUDCRS: attach a LAS/LAZ cloud first.");
            return;
        }
        for source in &dataset.sources {
            let metadata = &source.sample.metadata;
            let crs = &metadata.crs;
            let readiness = ocs_pointcloud::assess_survey_readiness(metadata);
            self.command_line.push_output(
                format!(
                    "POINTCLOUDCRS [{}]: {}; source {}; horizontal {}; vertical {}; survey safeguard {}.",
                    source.id,
                    crs.name.as_deref().unwrap_or("unnamed CRS"),
                    crs.source.as_deref().unwrap_or("none"),
                    crs.horizontal_label(),
                    crs.vertical_epsg
                        .map(|code| format!("EPSG:{code}"))
                        .unwrap_or_else(|| "unresolved (Z units/datum must be verified)".to_string()),
                    readiness.summary(),
                )
                .as_str(),
            );
            if let Some(warning) = &crs.parse_warning {
                self.command_line
                    .push_info(format!("POINTCLOUDCRS: {warning}.").as_str());
            }
        }
    }

    pub(super) fn reclassify_point_cloud(
        &mut self,
        tab_index: usize,
        classification: u8,
        index_spec: &str,
    ) {
        let dataset_len = self.tabs[tab_index].point_cloud.len();
        let dataset = &mut self.tabs[tab_index].point_cloud;
        let Some(cloud) = dataset.active_mut() else {
            self.command_line
                .push_error("POINTCLOUDCLASSIFY: attach a LAS/LAZ cloud first.");
            return;
        };
        let source_id = cloud.id.clone();
        let indices = match parse_source_indices(index_spec, cloud.sample.metadata.point_count) {
            Ok(indices) => indices,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDCLASSIFY: {error}").as_str());
                return;
            }
        };
        let changed = cloud.edits.apply(
            format!("Assign class {classification}"),
            indices,
            PointPatch::classification(classification),
        );
        let multi_note = if dataset_len > 1 {
            format!(" to source {source_id}")
        } else {
            String::new()
        };
        drop(cloud);
        self.tabs[tab_index].point_cloud.mark_display_changed();
        let model = self.tabs[tab_index].point_cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        self.command_line.push_output(
            format!(
                "POINTCLOUDCLASSIFY: queued {changed} point(s){multi_note} as class {classification}; export to create a revised LAS/LAZ."
            )
            .as_str(),
        );
        self.tabs[tab_index]
            .point_cloud
            .note_edit_sources(vec![source_id.clone()]);
        self.persist_point_cloud(
            tab_index,
            "classification",
            &format!("assigned class {classification} to {changed} points"),
            &[source_id],
        );
    }

    pub(super) fn undo_point_cloud_edit(&mut self, tab_index: usize) {
        let last_edit = self.tabs[tab_index].point_cloud.last_edit_sources.clone();
        let Some(ids) = last_edit else {
            self.command_line
                .push_info("POINTCLOUDUNDO: no point-cloud edit to undo.");
            return;
        };
        let mut undone = 0_usize;
        let mut restored_ids = Vec::new();
        for id in ids {
            let Some(cloud) = self.tabs[tab_index].point_cloud.source_mut(&id) else {
                continue;
            };
            if cloud.edits.undo().is_some() {
                undone += 1;
                restored_ids.push(id);
            }
        }
        if undone == 0 {
            self.command_line
                .push_info("POINTCLOUDUNDO: no point-cloud edit to undo.");
            return;
        }
        self.tabs[tab_index].point_cloud.last_edit_sources = None;
        self.tabs[tab_index].point_cloud.mark_display_changed();
        let model = self.tabs[tab_index].point_cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        let detail = if undone == 1 {
            "restored the previous classification edit state.".to_string()
        } else {
            format!("restored the previous edit state across {undone} sources.")
        };
        self.command_line
            .push_output(format!("POINTCLOUDUNDO: {detail}").as_str());
        self.persist_point_cloud(
            tab_index,
            "undo",
            "undid point-cloud transaction",
            &restored_ids,
        );
    }

    pub(super) fn detach_point_cloud(&mut self, tab_index: usize) {
        let count = self.tabs[tab_index].point_cloud.len();
        if count == 0 {
            self.command_line
                .push_info("POINTCLOUDDETACH: no LAS/LAZ cloud is attached.");
            return;
        }
        self.tabs[tab_index].point_cloud = PointCloudDataset::default();
        self.tabs[tab_index]
            .scene
            .set_point_cloud(PointCloudModel::default());
        let detail = if count == 1 {
            "detached the session cloud; the source file was unchanged.".to_string()
        } else {
            format!("detached {count} session sources; the source files were unchanged.")
        };
        self.command_line
            .push_output(format!("POINTCLOUDDETACH: {detail}").as_str());
    }

    pub(super) fn start_point_cloud_index(&mut self, tab_index: usize) -> Task<Message> {
        let tab_id = self.tabs[tab_index].id;
        let dataset = &mut self.tabs[tab_index].point_cloud;
        if dataset
            .sources
            .iter()
            .any(|source| source.index_cancel.is_some())
        {
            self.command_line
                .push_info("POINTCLOUDINDEX: an index build is already running.");
            return Task::none();
        }
        if dataset.sources.is_empty() {
            self.command_line
                .push_error("POINTCLOUDINDEX: attach a LAS/LAZ cloud first.");
            return Task::none();
        }
        if !dataset.index_batch_active {
            dataset.index_batch_active = true;
            for source in &mut dataset.sources {
                source.index_error = None;
            }
            self.command_line.push_info(
                format!(
                    "POINTCLOUDINDEX: preparing LOD caches for {} source(s), one at a time.",
                    dataset.sources.len()
                )
                .as_str(),
            );
        }
        let Some(source_index) = next_index_source_index(&dataset.sources) else {
            dataset.index_batch_active = false;
            let failed = dataset
                .sources
                .iter()
                .filter(|source| source.index_error.is_some())
                .count();
            let ready = dataset
                .sources
                .iter()
                .filter(|source| source.cache_manifest.is_some())
                .count();
            if failed == 0 {
                self.command_line.push_output(
                    format!("POINTCLOUDINDEX: all {ready} source LOD caches are ready.").as_str(),
                );
            } else {
                self.command_line.push_error(
                    format!(
                        "POINTCLOUDINDEX: {ready} source cache(s) ready; {failed} source(s) failed. Run POINTCLOUDINDEXSTATUS for details, then retry after correcting the reported cache/source issue."
                    )
                    .as_str(),
                );
            }
            return self.start_point_cloud_stream(tab_index);
        };
        let cloud = &mut dataset.sources[source_index];
        let source_id = cloud.id.clone();
        let source = cloud.source_path.clone();
        let cache_path = cache_path_for_source(&source);
        if cache_path.exists() {
            let result = TileCacheManifest::open(&cache_path)
                .and_then(|manifest| {
                    manifest.validate_source(&source)?;
                    Ok(manifest)
                })
                .map_err(|error| error.to_string());
            return deferred_message(Message::PointCloudIndexed(
                tab_id, source_id, cache_path, result,
            ));
        }

        let cancel = Arc::new(AtomicBool::new(false));
        cloud.index_cancel = Some(Arc::clone(&cancel));
        let progress = Arc::new(PointCloudJobProgress::new(
            cloud.sample.metadata.point_count,
        ));
        cloud.index_job = Some(Arc::clone(&progress));
        let estimate_bytes =
            ocs_pointcloud::estimate_cache_bytes(cloud.sample.metadata.point_count, 65_536, 12);
        if estimate_bytes >= 1024 * 1024 * 1024 {
            self.command_line.push_info(
                format!(
                    "POINTCLOUDINDEX: this will write ~{:.1} GB of LOD tiles to disk; the build can take several minutes (use POINTCLOUDINDEXSTATUS to monitor).",
                    estimate_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
                )
                .as_str(),
            );
        }
        self.command_line.push_info(
            format!(
                "POINTCLOUDINDEX [{}]: building disk-backed LOD tiles at \"{}\"; use POINTCLOUDINDEXSTATUS for progress and POINTCLOUDINDEXCANCEL to cancel the batch.",
                cloud.id,
                cache_path.display(),
            )
            .as_str(),
        );
        let worker_cache = cache_path.clone();
        background_task(
            move || {
                ocs_pointcloud::build_tiled_cache(
                    source,
                    &worker_cache,
                    TileCacheOptions::default(),
                    |state| {
                        progress
                            .completed
                            .store(state.points_read, Ordering::Relaxed);
                        progress
                            .tiles_created
                            .store(state.tiles_created as u64, Ordering::Relaxed);
                        !cancel.load(Ordering::Relaxed)
                    },
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudIndexed(tab_id, source_id, cache_path, result),
        )
    }

    pub(super) fn cancel_point_cloud_index(&mut self, tab_index: usize) {
        self.tabs[tab_index].point_cloud.index_batch_active = false;
        let cancel = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter_mut()
            .find_map(|source| source.index_cancel.as_ref().map(Arc::clone));
        if let Some(cancel) = cancel {
            cancel.store(true, Ordering::Relaxed);
            self.command_line
                .push_output("POINTCLOUDINDEXCANCEL: batch cancellation requested.");
        } else {
            self.command_line
                .push_info("POINTCLOUDINDEXCANCEL: no index build is running.");
        }
    }

    pub(super) fn point_cloud_index_status(&mut self, tab_index: usize) {
        let job = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .filter_map(|source| source.index_job.as_ref())
            .next()
            .cloned();
        let Some(job) = job else {
            let failures: Vec<_> = self.tabs[tab_index]
                .point_cloud
                .sources
                .iter()
                .filter_map(|source| {
                    source
                        .index_error
                        .as_ref()
                        .map(|error| format!("{}: {error}", source.source_path.display()))
                })
                .collect();
            if failures.is_empty() {
                self.command_line
                    .push_info("POINTCLOUDINDEXSTATUS: no index build is running.");
            } else {
                for failure in failures {
                    self.command_line
                        .push_error(format!("POINTCLOUDINDEXSTATUS: {failure}").as_str());
                }
            }
            return;
        };
        let completed = job.completed.load(Ordering::Relaxed);
        let tiles = job.tiles_created.load(Ordering::Relaxed);
        let percent = if job.total == 0 {
            100.0
        } else {
            completed as f64 / job.total as f64 * 100.0
        };
        self.command_line.push_output(
            format!(
                "POINTCLOUDINDEXSTATUS: {completed}/{} points ({percent:.1}%), {} tile(s) written.",
                job.total, tiles
            )
            .as_str(),
        );
    }

    pub(super) fn finish_point_cloud_index(
        &mut self,
        tab_id: u64,
        source_id: String,
        cache_path: PathBuf,
        result: Result<TileCacheManifest, String>,
    ) -> Task<Message> {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Task::none();
        };
        let drawing_crs = self.tabs[tab_index]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info);
        let batch_active = self.tabs[tab_index].point_cloud.index_batch_active;
        let manifest = match result {
            Ok(manifest) => manifest,
            Err(error) => {
                if let Some(cloud) = self.tabs[tab_index].point_cloud.source_mut(&source_id) {
                    cloud.index_cancel = None;
                    cloud.index_job = None;
                    cloud.index_error = Some(error.clone());
                }
                self.command_line
                    .push_error(format!("POINTCLOUDINDEX [{source_id}]: {error}").as_str());
                return if batch_active {
                    self.start_point_cloud_index(tab_index)
                } else {
                    Task::none()
                };
            }
        };
        let cache_bytes = manifest
            .tiles
            .iter()
            .map(|tile| tile.point_count)
            .sum::<u64>()
            .saturating_mul(manifest.record_size as u64);
        let (tile_count, leaf_level) = (manifest.tiles.len(), manifest.leaf_level);
        let Some(cloud) = self.tabs[tab_index].point_cloud.source_mut(&source_id) else {
            return Task::none();
        };
        cloud.index_cancel = None;
        cloud.index_job = None;
        cloud.index_error = None;
        let target = drawing_crs.unwrap_or_else(|| cloud.source_crs.clone());
        if let Err(error) = cloud.activate_cache(cache_path.clone(), manifest, &target) {
            cloud.index_error = Some(error.clone());
            self.command_line
                .push_error(format!("POINTCLOUDINDEX [{source_id}]: {error}").as_str());
            return if batch_active {
                self.start_point_cloud_index(tab_index)
            } else {
                Task::none()
            };
        }
        self.command_line.push_output(
            format!(
                "POINTCLOUDINDEX [{source_id}]: {} tiles indexed through level {} (~{} MB).",
                tile_count,
                leaf_level,
                cache_bytes / (1024 * 1024)
            )
            .as_str(),
        );
        self.persist_point_cloud(
            tab_index,
            "index",
            "built or opened tiled LOD cache",
            &[source_id],
        );
        if batch_active {
            self.start_point_cloud_index(tab_index)
        } else {
            self.start_point_cloud_stream(tab_index)
        }
    }

    pub(super) fn start_point_cloud_stream(&mut self, tab_index: usize) -> Task<Message> {
        if self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .any(|source| source.stream_in_flight)
        {
            return Task::none();
        }
        let (camera, viewport, camera_generation, tab_id) = {
            let tab = &self.tabs[tab_index];
            let canvas = tab.scene.selection.borrow().vp_size;
            let (camera, viewport) = tab.scene.viewport_edit_frame(canvas).unwrap_or_else(|| {
                (
                    tab.scene.camera.borrow().clone(),
                    tab.scene.active_model_tile_bounds(canvas.0, canvas.1),
                )
            });
            (camera, viewport, tab.scene.camera_generation, tab.id)
        };
        if viewport.width <= 1.0 || viewport.height <= 1.0 {
            return Task::none();
        }
        // Budgets are cloned before the mutable find so the display settings
        // stay readable while a source is borrowed for scheduling.
        let display = self.tabs[tab_index].point_cloud.display.clone();
        let drawing_crs = self.tabs[tab_index]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info);
        // The active cross-section band, captured before the mutable find so
        // the section can steer tile selection while a source is borrowed.
        // Its width is already in world/map units and must not depend on the
        // current camera: zooming or rotating keeps the same geographic area.
        let section_band = self.tabs[tab_index].point_cloud.section.map(|section| {
            let half = (0.5 * section.width_world).max(0.0);
            let min_x = section.p0[0].min(section.p1[0]) - half;
            let max_x = section.p0[0].max(section.p1[0]) + half;
            let min_y = section.p0[1].min(section.p1[1]) - half;
            let max_y = section.p0[1].max(section.p1[1]) + half;
            (
                [min_x, min_y, f64::NEG_INFINITY],
                [max_x, max_y, f64::INFINITY],
            )
        });
        // One source streams per tick; the stream-needed check keeps calling
        // back until every source has caught up with the camera.
        let source_count = self.tabs[tab_index].point_cloud.len().max(1);
        let Some(cloud) = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter_mut()
            .find(|cloud| {
                cloud.cache_manifest.is_some()
                    && !cloud.stream_in_flight
                    && cloud.stream_camera_generation != camera_generation
            })
        else {
            return Task::none();
        };
        let (Some(manifest), Some(cache_path)) =
            (cloud.cache_manifest.as_ref(), cloud.cache_path.as_ref())
        else {
            return Task::none();
        };
        let memory_point_budget = display.cpu_budget_bytes
            / source_count
            / std::mem::size_of::<ocs_pointcloud::SamplePoint>().max(1);
        let gpu_point_budget = display.gpu_budget_bytes / source_count / GPU_POINT_BYTES;
        let point_budget = display
            .point_budget
            .min(memory_point_budget)
            .min(gpu_point_budget)
            .max(1) as u64;
        // Stream only tiles that intersect the live camera frame. An active
        // section adds its fixed world-space corridor as a second filter; it
        // must never pull full-density leaves for off-screen parts of the cut.
        // Walking finest-to-coarsest makes a close view reach leaf/full density
        // naturally, while a wider view selects a lower-density level that
        // fits the same CPU/GPU point budget.
        let selected = select_visible_lod_tiles(
            &manifest.tiles,
            manifest.leaf_level,
            point_budget,
            section_band,
            |tile| camera.aabb_visible(tile.bounds_min, tile.bounds_max, viewport),
        );
        let selected_keys: Vec<_> = selected.iter().map(|tile| tile.key).collect();
        cloud.stream_camera_generation = camera_generation;
        cloud.lru_clock = cloud.lru_clock.wrapping_add(1).max(1);
        for key in &selected_keys {
            if let Some(resident) = cloud.resident_tiles.get_mut(key) {
                resident.last_used = cloud.lru_clock;
            }
        }
        let missing: Vec<_> = selected
            .into_iter()
            .filter(|tile| !cloud.resident_tiles.contains_key(&tile.key))
            .collect();
        let source_id = cloud.id.clone();
        let source_crs = cloud.source_crs.clone();
        let drawing_crs = drawing_crs.unwrap_or_else(|| source_crs.clone());
        if missing.is_empty() {
            cloud.active_tiles = selected_keys;
            rebuild_resident_display(cloud);
            let model = self.tabs[tab_index].point_cloud.display_model();
            self.tabs[tab_index].scene.set_point_cloud(model);
            return Task::none();
        }

        cloud.stream_request_id = cloud.stream_request_id.wrapping_add(1).max(1);
        let request_id = cloud.stream_request_id;
        cloud.stream_in_flight = true;
        let cache_path = cache_path.clone();
        let tile_workers = tile_read_workers();
        background_task(
            move || {
                let mut loaded =
                    ocs_pointcloud::read_tiles_parallel(&cache_path, &missing, tile_workers)
                        .map_err(|error| error.to_string())?;
                for (_, points) in &mut loaded {
                    ocs_pointcloud::reproject_points_between_crs(&source_crs, &drawing_crs, points)
                        .map_err(|error| error.to_string())?;
                }
                Ok(TileLoadBatch {
                    source_id,
                    request_id,
                    camera_generation,
                    selected: selected_keys,
                    loaded,
                })
            },
            move |result| Message::PointCloudTilesLoaded(tab_id, result),
        )
    }

    pub(super) fn finish_point_cloud_stream(
        &mut self,
        tab_id: u64,
        result: Result<TileLoadBatch, String>,
    ) {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let (active_tile_count, resident_tile_count, camera_generation) = {
            let batch_source = match &result {
                Ok(batch) => batch.source_id.clone(),
                Err(_) => {
                    // Error batches belong to whichever source is in flight.
                    match self.tabs[tab_index]
                        .point_cloud
                        .sources
                        .iter()
                        .position(|source| source.stream_in_flight)
                    {
                        Some(index) => self.tabs[tab_index].point_cloud.sources[index].id.clone(),
                        None => return,
                    }
                }
            };
            let cpu_budget = self.tabs[tab_index].point_cloud.display.cpu_budget_bytes
                / self.tabs[tab_index].point_cloud.len().max(1);
            let Some(cloud) = self.tabs[tab_index].point_cloud.source_mut(&batch_source) else {
                return;
            };
            cloud.stream_in_flight = false;
            let batch = match result {
                Ok(batch) if batch.request_id == cloud.stream_request_id => batch,
                Ok(_) => {
                    cloud.stale_tile_results = cloud.stale_tile_results.saturating_add(1);
                    return;
                }
                Err(error) => {
                    cloud.cancelled_tile_requests = cloud.cancelled_tile_requests.saturating_add(1);
                    self.command_line
                        .push_error(format!("POINTCLOUDLOD: {error}").as_str());
                    return;
                }
            };
            let camera_generation = batch.camera_generation;
            cloud.lru_clock = cloud.lru_clock.wrapping_add(1).max(1);
            for (key, points) in batch.loaded {
                cloud.resident_tiles.insert(
                    key,
                    ResidentTile {
                        points: Arc::new(points),
                        last_used: cloud.lru_clock,
                    },
                );
            }
            cloud.active_tiles = batch.selected;
            let active = cloud.active_tiles.clone();
            for key in &active {
                if let Some(tile) = cloud.resident_tiles.get_mut(key) {
                    tile.last_used = cloud.lru_clock;
                }
            }
            evict_resident_tiles(cloud, cpu_budget);
            rebuild_resident_display(cloud);
            (
                cloud.active_tiles.len(),
                cloud.resident_tiles.len(),
                camera_generation,
            )
        };
        let model = self.tabs[tab_index].point_cloud.display_model();
        let points = model.points.len();
        self.tabs[tab_index].scene.set_point_cloud(model);
        if camera_generation == self.tabs[tab_index].scene.camera_generation {
            self.command_line.push_output(
                format!(
                    "POINTCLOUDLOD: {} visible tile(s), {points} GPU point(s), {} resident tile(s).",
                    active_tile_count, resident_tile_count
                )
                .as_str(),
            );
        }
    }

    /// Applies a style-only change (color mode, class visibility, class
    /// colors, point size): shares the resident point data and rewrites just
    /// the GPU style uniform — no instance-buffer rebuild, no CPU point pass.
    pub(super) fn restyle_point_cloud(&mut self, tab_index: usize) {
        let style = self.tabs[tab_index].point_cloud.point_style();
        let point_size = self.tabs[tab_index].point_cloud.display.point_size_px;
        self.tabs[tab_index].point_cloud.mark_style_changed();
        let current = self.tabs[tab_index].scene.point_cloud.clone();
        let model = PointCloudModel {
            points: Arc::clone(&current.points),
            point_size_px: point_size,
            style,
            chunks: current.chunks.clone(),
            geometry_generation: current.geometry_generation,
            style_generation: self.tabs[tab_index].point_cloud.style_generation,
        };
        self.tabs[tab_index].scene.set_point_cloud(model);
    }

    /// Set (or replace) the active vertical cross-section. Style-only: the
    /// shader re-applies the band, so no instance buffer is rebuilt.
    pub(super) fn set_point_cloud_section(
        &mut self,
        tab_index: usize,
        p0: [f64; 2],
        p1: [f64; 2],
        width_world: f64,
        mode: crate::scene::model::point_cloud_model::SectionMode,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSECTION: attach a LAS/LAZ cloud first.");
            return;
        }
        if !width_world.is_finite() || !(1.0..=1024.0).contains(&width_world) {
            self.command_line
                .push_error("POINTCLOUDSECTION: width must be between 1 and 1024 map units.");
            return;
        }
        self.tabs[tab_index].point_cloud.section =
            Some(crate::scene::model::point_cloud_model::Section {
                p0,
                p1,
                width_world,
                mode,
            });
        // A section change invalidates every source's stream so the next tick
        // re-selects visible tiles at the finest LOD that fits the point budget.
        for source in &mut self.tabs[tab_index].point_cloud.sources {
            source.stream_camera_generation = u64::MAX;
        }
        self.restyle_point_cloud(tab_index);
    }

    /// Move the active section by `delta` along its normal (perpendicular to
    /// the cut), keeping its length and width. Walks a corridor TerraScan-style.
    pub(super) fn move_point_cloud_section(&mut self, tab_index: usize, delta: f64) {
        let Some(section) = self.tabs[tab_index].point_cloud.section else {
            self.command_line
                .push_error("POINTCLOUDSECTIONMOVE: no section is active.");
            return;
        };
        let seg = [section.p1[0] - section.p0[0], section.p1[1] - section.p0[1]];
        let len = (seg[0] * seg[0] + seg[1] * seg[1]).sqrt();
        if len <= f64::EPSILON {
            self.command_line
                .push_error("POINTCLOUDSECTIONMOVE: the section line is degenerate.");
            return;
        }
        // Normal is the CCW perpendicular of the cut direction.
        let nx = -seg[1] / len;
        let ny = seg[0] / len;
        let shift = [nx * delta, ny * delta];
        let moved = crate::scene::model::point_cloud_model::Section {
            p0: [section.p0[0] + shift[0], section.p0[1] + shift[1]],
            p1: [section.p1[0] + shift[0], section.p1[1] + shift[1]],
            ..section
        };
        self.tabs[tab_index].point_cloud.section = Some(moved);
        for source in &mut self.tabs[tab_index].point_cloud.sources {
            source.stream_camera_generation = u64::MAX;
        }
        self.restyle_point_cloud(tab_index);
    }

    /// Change the active section's total band width in drawing/map units.
    pub(super) fn set_point_cloud_section_width(&mut self, tab_index: usize, width_world: f64) {
        let Some(mut section) = self.tabs[tab_index].point_cloud.section else {
            self.command_line
                .push_error("POINTCLOUDSECTIONWIDTH: no section is active.");
            return;
        };
        if !width_world.is_finite() || !(1.0..=1024.0).contains(&width_world) {
            self.command_line
                .push_error("POINTCLOUDSECTIONWIDTH: width must be between 1 and 1024 map units.");
            return;
        }
        section.width_world = width_world;
        self.tabs[tab_index].point_cloud.section = Some(section);
        for source in &mut self.tabs[tab_index].point_cloud.sources {
            source.stream_camera_generation = u64::MAX;
        }
        self.restyle_point_cloud(tab_index);
    }

    /// Remove the active section and show the whole cloud again.
    pub(super) fn clear_point_cloud_section(&mut self, tab_index: usize) {
        if self.tabs[tab_index].point_cloud.section.take().is_none() {
            self.command_line
                .push_info("POINTCLOUDSECTIONCLEAR: no section was active.");
            return;
        }
        for source in &mut self.tabs[tab_index].point_cloud.sources {
            source.stream_camera_generation = u64::MAX;
        }
        self.restyle_point_cloud(tab_index);
    }

    /// Snap the active pane's camera to look along the active section line
    /// (side-on vertical view): gaze runs down the cut, up stays world +Z.
    pub(super) fn point_cloud_section_view(&mut self, tab_index: usize) {
        let Some(section) = self.tabs[tab_index].point_cloud.section else {
            self.command_line
                .push_error("POINTCLOUDSECTIONVIEW: no section is active.");
            return;
        };
        let seg = [section.p1[0] - section.p0[0], section.p1[1] - section.p0[1]];
        let len = (seg[0] * seg[0] + seg[1] * seg[1]).sqrt();
        if len <= f64::EPSILON {
            self.command_line
                .push_error("POINTCLOUDSECTIONVIEW: the section line is degenerate.");
            return;
        }
        // Gaze along the cut direction (perpendicular to the vertical plane).
        let eye_dir = glam::Vec3::new(seg[0] as f32, seg[1] as f32, 0.0);
        let eye_dir = eye_dir.normalize_or(glam::Vec3::X);
        // Snapping a floating viewport writes its stored view_direction; the
        // plain model pane snaps the live camera directly. Mirror the ViewCube
        // snap, which handles both.
        if self.tabs[tab_index].scene.active_viewport.is_some() {
            self.tabs[tab_index]
                .scene
                .snap_active_viewport_to_direction(eye_dir, glam::Mat4::IDENTITY);
        } else {
            let mut cam = self.tabs[tab_index].scene.camera.borrow_mut();
            cam.snap_to_direction(eye_dir, glam::Mat4::IDENTITY);
        }
        self.tabs[tab_index].scene.camera_generation += 1;
    }

    pub(super) fn set_point_cloud_color_mode(&mut self, tab_index: usize, mode: ColorMode) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDCOLOR: attach a LAS/LAZ cloud first.");
            return;
        }
        self.tabs[tab_index].point_cloud.display.color_mode = mode;
        self.restyle_point_cloud(tab_index);
        self.command_line
            .push_output(format!("POINTCLOUDCOLOR: mode set to {mode:?}.").as_str());
        self.persist_point_cloud(tab_index, "display", &format!("color mode {mode:?}"), &[]);
    }

    pub(super) fn set_point_cloud_point_size(&mut self, tab_index: usize, size: f32) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDPOINTSIZE: attach a LAS/LAZ cloud first.");
            return;
        }
        if !size.is_finite() || !(1.0..=32.0).contains(&size) {
            self.command_line
                .push_error("POINTCLOUDPOINTSIZE: size must be between 1 and 32 pixels.");
            return;
        }
        self.tabs[tab_index].point_cloud.display.point_size_px = size;
        self.restyle_point_cloud(tab_index);
        self.command_line.push_output(
            format!("POINTCLOUDPOINTSIZE: fixed screen size set to {size:.1} px.").as_str(),
        );
        self.persist_point_cloud(
            tab_index,
            "display",
            &format!("point size {size:.1} px"),
            &[],
        );
    }

    pub(super) fn set_point_cloud_class_visible(
        &mut self,
        tab_index: usize,
        classification: u8,
        visible: bool,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDCLASSVISIBLE: attach a LAS/LAZ cloud first.");
            return;
        }
        if visible {
            self.tabs[tab_index]
                .point_cloud
                .display
                .hidden_classes
                .remove(&classification);
        } else {
            self.tabs[tab_index]
                .point_cloud
                .display
                .hidden_classes
                .insert(classification);
        }
        self.restyle_point_cloud(tab_index);
        self.command_line.push_output(
            format!(
                "POINTCLOUDCLASSVISIBLE: class {classification} {}.",
                if visible { "shown" } else { "hidden" }
            )
            .as_str(),
        );
        self.persist_point_cloud(tab_index, "display", "changed class visibility", &[]);
    }

    pub(super) fn update_point_cloud_class(
        &mut self,
        tab_index: usize,
        code: u8,
        name: Option<String>,
        visible: Option<bool>,
        locked: Option<bool>,
    ) {
        let dataset = &mut self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            return;
        }
        let Some(class) = dataset.classes.classes.get_mut(&code) else {
            return;
        };
        if let Some(name) = name {
            class.name = name.chars().take(128).collect();
        }
        if let Some(visible) = visible {
            class.visible = visible;
            if visible {
                dataset.display.hidden_classes.remove(&code);
            } else {
                dataset.display.hidden_classes.insert(code);
            }
        }
        if let Some(locked) = locked {
            class.locked = locked;
        }
        drop(class);
        drop(dataset);
        self.restyle_point_cloud(tab_index);
        self.persist_point_cloud(tab_index, "classes", &format!("edited class {code}"), &[]);
    }

    pub(super) fn update_point_cloud_class_color(
        &mut self,
        tab_index: usize,
        code: u8,
        channel: usize,
        value: u8,
    ) {
        let dataset = &mut self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            return;
        }
        let Some(class) = dataset.classes.classes.get_mut(&code) else {
            return;
        };
        let Some(component) = class.color.get_mut(channel) else {
            return;
        };
        *component = value;
        drop(component);
        drop(class);
        drop(dataset);
        self.restyle_point_cloud(tab_index);
        self.persist_point_cloud(
            tab_index,
            "classes",
            &format!("changed class {code} color"),
            &[],
        );
    }

    pub(super) fn add_point_cloud_class(&mut self, tab_index: usize) {
        let dataset = &mut self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            return;
        }
        let Some(code) = (0_u8..=u8::MAX).find(|code| !dataset.classes.classes.contains_key(code))
        else {
            self.command_line
                .push_error("POINTCLOUDCLASSADD: all class codes are already defined.");
            return;
        };
        dataset.classes.upsert(ocs_pointcloud::ClassDefinition {
            code,
            name: format!("Class {code}"),
            color: categorical(code as u32)
                .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)[..3]
                .try_into()
                .unwrap_or([220, 220, 220]),
            visible: true,
            locked: false,
        });
        drop(dataset);
        self.restyle_point_cloud(tab_index);
        self.persist_point_cloud(tab_index, "classes", &format!("added class {code}"), &[]);
    }

    pub(super) fn remove_point_cloud_class(&mut self, tab_index: usize, code: u8) {
        let dataset = &mut self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            return;
        }
        if dataset
            .classes
            .classes
            .get(&code)
            .is_some_and(|class| class.locked)
        {
            return;
        }
        if dataset.classes.remove(code).is_none() {
            return;
        }
        dataset.display.hidden_classes.remove(&code);
        drop(dataset);
        self.restyle_point_cloud(tab_index);
        self.persist_point_cloud(tab_index, "classes", &format!("removed class {code}"), &[]);
    }

    pub(super) fn point_cloud_statistics(&mut self, tab_index: usize) {
        let dataset = &self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSTATS: attach a LAS/LAZ cloud first.");
            return;
        }
        let points = dataset.sources.iter().flat_map(|source| {
            source.active_points().into_iter().map(|point| {
                source
                    .edits
                    .patch_for(point.source_index)
                    .map_or(point.clone(), |patch| point.with_patch(patch))
            })
        });
        let stats = classification_statistics(points);
        let summary = stats
            .iter()
            .map(|(class, stats)| format!("{class}:{}", stats.total))
            .collect::<Vec<_>>()
            .join(", ");
        let strides: std::collections::BTreeSet<u64> = dataset
            .sources
            .iter()
            .map(|source| source.sample.stride)
            .collect();
        let qualifier = if strides.iter().all(|&stride| stride == 1) {
            "full cloud"
        } else if dataset.len() == 1 && strides.contains(&0) {
            "tiled LOD"
        } else {
            "display sample"
        };
        self.command_line
            .push_output(format!("POINTCLOUDSTATS ({qualifier}): {summary}.").as_str());
    }

    /// Replaces the named selection on every source and reports the merged
    /// count. Each source keeps its own index space.
    pub(super) fn set_point_cloud_selection(
        &mut self,
        tab_index: usize,
        selections: Vec<(String, SelectionSet)>,
    ) {
        let name = selections
            .first()
            .map(|(_, selection)| selection.name.clone())
            .unwrap_or_else(|| "active".to_string());
        let count = selections
            .iter()
            .map(|(_, selection)| selection.len())
            .sum::<u64>();
        let ids: Vec<String> = selections.iter().map(|(id, _)| id.clone()).collect();
        if selections.is_empty() {
            self.tabs[tab_index]
                .point_cloud
                .clear_selections_named(&name);
            return;
        }
        for (id, selection) in selections {
            self.tabs[tab_index]
                .point_cloud
                .push_selection(&id, selection);
        }
        let model = self.tabs[tab_index].point_cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        self.command_line.push_output(
            format!("POINTCLOUDSELECT: selection set \"{name}\" contains {count} point(s).")
                .as_str(),
        );
        self.persist_point_cloud(
            tab_index,
            "selection",
            &format!("updated {name}: {count} points"),
            &ids,
        );
    }

    pub(super) fn clear_point_cloud_selections(&mut self, tab_index: usize) {
        self.tabs[tab_index]
            .point_cloud
            .clear_selections_named("active");
        let model = self.tabs[tab_index].point_cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
    }

    pub(super) fn point_cloud_select_box(
        &mut self,
        tab_index: usize,
        min: [f64; 3],
        max: [f64; 3],
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSELECTBOX: attach a LAS/LAZ cloud first.");
            return;
        }
        let polygon = [
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ];
        let selections = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .map(|cloud| {
                let indices = select_polygon(
                    &cloud.active_points(),
                    &polygon,
                    Some([min[2], max[2]]),
                    &self.tabs[tab_index].point_cloud.selection_filter,
                );
                (
                    cloud.id.clone(),
                    SelectionSet::from_indices("active", indices.iter()),
                )
            })
            .collect();
        self.set_point_cloud_selection(tab_index, selections);
    }

    pub(super) fn point_cloud_select_brush(
        &mut self,
        tab_index: usize,
        center: [f64; 3],
        radius: f64,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSELECTBRUSH: attach a LAS/LAZ cloud first.");
            return;
        }
        let selections = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .map(|cloud| {
                let indices = select_brush(
                    &cloud.active_points(),
                    center,
                    radius,
                    &self.tabs[tab_index].point_cloud.selection_filter,
                );
                (
                    cloud.id.clone(),
                    SelectionSet::from_indices("active", indices.iter()),
                )
            })
            .collect();
        self.set_point_cloud_selection(tab_index, selections);
    }

    pub(super) fn point_cloud_select_nearest(
        &mut self,
        tab_index: usize,
        position: [f64; 3],
        radius: f64,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSELECTPOINT: attach a LAS/LAZ cloud first.");
            return;
        }
        let selections = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .filter_map(|cloud| {
                let indices = select_nearest(
                    &cloud.active_points(),
                    position,
                    radius,
                    &self.tabs[tab_index].point_cloud.selection_filter,
                );
                (!indices.is_empty()).then(|| {
                    (
                        cloud.id.clone(),
                        SelectionSet::from_indices("active", indices.iter()),
                    )
                })
            })
            .collect();
        self.set_point_cloud_selection(tab_index, selections);
    }

    fn point_cloud_view_frame(
        &self,
        tab_index: usize,
    ) -> Option<(crate::scene::view::camera::Camera, iced::Rectangle)> {
        let tab = self.tabs.get(tab_index)?;
        let canvas = tab.scene.selection.borrow().vp_size;
        if canvas.0 <= 1.0 || canvas.1 <= 1.0 {
            return None;
        }
        Some(tab.scene.viewport_edit_frame(canvas).unwrap_or_else(|| {
            (
                tab.scene.camera.borrow().clone(),
                tab.scene.active_model_tile_bounds(canvas.0, canvas.1),
            )
        }))
    }

    pub(super) fn point_cloud_select_screen_point(
        &mut self,
        tab_index: usize,
        anchor: glam::DVec3,
        radius_px: f32,
    ) {
        let nearest = match self.point_cloud_nearest_screen_point(tab_index, anchor, radius_px) {
            Ok(nearest) => nearest,
            Err(error) => {
                self.command_line.push_error(error);
                return;
            }
        };
        let selections = nearest.map_or_else(Vec::new, |(id, point)| {
            vec![(
                id,
                SelectionSet::from_indices("active", [point.source_index].into_iter()),
            )]
        });
        self.set_point_cloud_selection(tab_index, selections);
    }

    fn point_cloud_nearest_screen_point(
        &mut self,
        tab_index: usize,
        anchor: glam::DVec3,
        radius_px: f32,
    ) -> Result<Option<(String, ocs_pointcloud::SamplePoint)>, &'static str> {
        let Some((camera, viewport)) = self.point_cloud_view_frame(tab_index) else {
            return Err("POINTCLOUDMEASURE: viewport size is unavailable.");
        };
        let Some(center) = camera.project(anchor, viewport) else {
            return Ok(None);
        };
        let camera_generation = self.tabs[tab_index].scene.camera_generation;
        if self.tabs[tab_index].point_cloud.is_empty() {
            return Err("POINTCLOUDMEASURE: attach a LAS/LAZ cloud first.");
        }
        let radius_sq = radius_px.max(1.0).powi(2);
        let filter = self.tabs[tab_index].point_cloud.selection_filter.clone();
        let mut nearest: Option<(f32, f64, String, ocs_pointcloud::SamplePoint)> = None;
        for cloud in &mut self.tabs[tab_index].point_cloud.sources {
            ensure_screen_spatial_index(cloud, &camera, viewport, camera_generation);
            let index = cloud.screen_index.as_ref().expect("screen index");
            let snapshot = Arc::clone(&index.snapshot);
            let candidates = screen_candidates(
                index,
                [center.x - radius_px, center.y - radius_px],
                [center.x + radius_px, center.y + radius_px],
            );
            for projected in candidates {
                let Some(source) = snapshot.get(projected.sample_index) else {
                    continue;
                };
                let point = cloud
                    .edits
                    .patch_for(source.source_index)
                    .map_or_else(|| source.clone(), |patch| source.clone().with_patch(patch));
                if !filter.matches(&point) {
                    continue;
                }
                let dx = projected.screen[0] - center.x;
                let dy = projected.screen[1] - center.y;
                let distance_sq = dx * dx + dy * dy;
                if distance_sq > radius_sq {
                    continue;
                }
                let closer = nearest.as_ref().is_none_or(|(best_sq, best_depth, _, _)| {
                    distance_sq < *best_sq
                        || (distance_sq == *best_sq && projected.depth < *best_depth)
                });
                if closer {
                    nearest = Some((distance_sq, projected.depth, cloud.id.clone(), point));
                }
            }
        }
        Ok(nearest.map(|(_, _, id, point)| (id, point)))
    }

    pub(super) fn point_cloud_measure_screen(
        &mut self,
        tab_index: usize,
        first: glam::DVec3,
        second: glam::DVec3,
        radius_px: f32,
    ) {
        let first = match self.point_cloud_nearest_screen_point(tab_index, first, radius_px) {
            Ok(Some(point)) => point,
            Ok(None) => {
                self.command_line
                    .push_error("POINTCLOUDMEASURE: no displayed point near the first pick.");
                return;
            }
            Err(error) => {
                self.command_line.push_error(error);
                return;
            }
        };
        let second = match self.point_cloud_nearest_screen_point(tab_index, second, radius_px) {
            Ok(Some(point)) => point,
            Ok(None) => {
                self.command_line
                    .push_error("POINTCLOUDMEASURE: no displayed point near the second pick.");
                return;
            }
            Err(error) => {
                self.command_line.push_error(error);
                return;
            }
        };
        let a = glam::DVec3::from_array(first.1.position);
        let b = glam::DVec3::from_array(second.1.position);
        let delta = b - a;
        let horizontal = delta.x.hypot(delta.y);
        let distance = delta.length();
        let unit = self.tabs[tab_index].spatial.working_unit.short();

        let mut by_source: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        by_source
            .entry(first.0)
            .or_default()
            .push(first.1.source_index);
        by_source
            .entry(second.0)
            .or_default()
            .push(second.1.source_index);
        let selections = by_source
            .into_iter()
            .map(|(id, indices)| {
                (
                    id,
                    SelectionSet::from_indices("active", indices.into_iter()),
                )
            })
            .collect();
        self.set_point_cloud_selection(tab_index, selections);
        self.command_line.push_output(
            format!(
                "LiDAR distance = {distance:.4} {unit}; horizontal = {horizontal:.4} {unit}; ΔX = {:.4}, ΔY = {:.4}, ΔZ = {:.4} {unit}.",
                delta.x, delta.y, delta.z
            )
            .as_str(),
        );
    }

    pub(super) fn point_cloud_select_screen_rectangle(
        &mut self,
        tab_index: usize,
        first: glam::DVec3,
        second: glam::DVec3,
    ) {
        let Some((camera, viewport)) = self.point_cloud_view_frame(tab_index) else {
            return;
        };
        let (Some(a), Some(b)) = (
            camera.project(first, viewport),
            camera.project(second, viewport),
        ) else {
            return;
        };
        let polygon = [
            [a.x.min(b.x), a.y.min(b.y)],
            [a.x.max(b.x), a.y.min(b.y)],
            [a.x.max(b.x), a.y.max(b.y)],
            [a.x.min(b.x), a.y.max(b.y)],
        ];
        self.point_cloud_select_screen_polygon(tab_index, &camera, viewport, &polygon);
    }

    pub(super) fn point_cloud_select_screen_fence(
        &mut self,
        tab_index: usize,
        vertices: &[glam::DVec3],
    ) {
        let Some((camera, viewport)) = self.point_cloud_view_frame(tab_index) else {
            return;
        };
        let polygon: Vec<_> = vertices
            .iter()
            .filter_map(|point| camera.project(*point, viewport))
            .map(|point| [point.x, point.y])
            .collect();
        self.point_cloud_select_screen_polygon(tab_index, &camera, viewport, &polygon);
    }

    fn point_cloud_select_screen_polygon(
        &mut self,
        tab_index: usize,
        camera: &crate::scene::view::camera::Camera,
        viewport: iced::Rectangle,
        polygon: &[[f32; 2]],
    ) {
        if polygon.len() < 3 {
            self.command_line
                .push_error("POINTCLOUDSELECTFENCE: at least three visible vertices are required.");
            return;
        }
        let camera_generation = self.tabs[tab_index].scene.camera_generation;
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSELECTFENCE: attach a LAS/LAZ cloud first.");
            return;
        }
        let bounds = screen_polygon_bounds(polygon);
        let filter = self.tabs[tab_index].point_cloud.selection_filter.clone();
        let mut selections = Vec::new();
        for cloud in &mut self.tabs[tab_index].point_cloud.sources {
            ensure_screen_spatial_index(cloud, camera, viewport, camera_generation);
            let index = cloud.screen_index.as_ref().expect("screen index");
            let snapshot = Arc::clone(&index.snapshot);
            let candidates = screen_candidates(index, bounds.0, bounds.1);
            let indices = candidates.into_iter().filter_map(|projected| {
                let source = snapshot.get(projected.sample_index)?;
                let point = cloud
                    .edits
                    .patch_for(source.source_index)
                    .map_or_else(|| source.clone(), |patch| source.clone().with_patch(patch));
                if !filter.matches(&point) {
                    return None;
                }
                point_in_screen_polygon(projected.screen, polygon).then_some(point.source_index)
            });
            selections.push((
                cloud.id.clone(),
                SelectionSet::from_indices("active", indices),
            ));
        }
        self.set_point_cloud_selection(tab_index, selections);
    }

    pub(super) fn point_cloud_select_screen_brush(
        &mut self,
        tab_index: usize,
        center_world: glam::DVec3,
        radius_px: f32,
        classification: Option<u8>,
    ) {
        let Some((camera, viewport)) = self.point_cloud_view_frame(tab_index) else {
            return;
        };
        let Some(center) = camera.project(center_world, viewport) else {
            return;
        };
        let camera_generation = self.tabs[tab_index].scene.camera_generation;
        if self.tabs[tab_index].point_cloud.is_empty() {
            return;
        }
        let radius_sq = radius_px.clamp(2.0, 256.0).powi(2);
        let filter = self.tabs[tab_index].point_cloud.selection_filter.clone();
        let mut selections = Vec::new();
        for cloud in &mut self.tabs[tab_index].point_cloud.sources {
            ensure_screen_spatial_index(cloud, &camera, viewport, camera_generation);
            let index = cloud.screen_index.as_ref().expect("screen index");
            let snapshot = Arc::clone(&index.snapshot);
            let candidates = screen_candidates(
                index,
                [center.x - radius_px, center.y - radius_px],
                [center.x + radius_px, center.y + radius_px],
            );
            let stroke: Vec<u64> = candidates
                .into_iter()
                .filter_map(|projected| {
                    let source = snapshot.get(projected.sample_index)?;
                    let point = cloud
                        .edits
                        .patch_for(source.source_index)
                        .map_or_else(|| source.clone(), |patch| source.clone().with_patch(patch));
                    if !filter.matches(&point) {
                        return None;
                    }
                    let dx = projected.screen[0] - center.x;
                    let dy = projected.screen[1] - center.y;
                    (dx * dx + dy * dy <= radius_sq).then_some(point.source_index)
                })
                .collect();
            let stroke_set = SelectionSet::from_indices("stroke", stroke.iter().copied());
            let unioned = cloud
                .selection_sets
                .iter()
                .find(|selection| selection.name == "active")
                .map_or_else(
                    || SelectionSet::from_indices("active", stroke.iter().copied()),
                    |active| active.union("active", &stroke_set),
                );
            selections.push((cloud.id.clone(), unioned));
        }
        self.set_point_cloud_selection(tab_index, selections);
        if let Some(classification) = classification {
            self.patch_point_cloud_selection(
                tab_index,
                &format!("Screen brush assign class {classification}"),
                PointPatch::classification(classification),
            );
        }
    }

    pub(super) fn point_cloud_select_elevation_slice(
        &mut self,
        tab_index: usize,
        low: f64,
        high: f64,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSELECTSLICE: attach a LAS/LAZ cloud first.");
            return;
        }
        let bounds = [low.min(high), low.max(high)];
        let filter = self.tabs[tab_index].point_cloud.selection_filter.clone();
        let selections = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .map(|cloud| {
                let indices = cloud.active_points().into_iter().filter_map(|point| {
                    (point.position[2] >= bounds[0]
                        && point.position[2] <= bounds[1]
                        && filter.matches(&point))
                    .then_some(point.source_index)
                });
                (
                    cloud.id.clone(),
                    SelectionSet::from_indices("active", indices),
                )
            })
            .collect();
        self.set_point_cloud_selection(tab_index, selections);
    }

    pub(super) fn set_point_cloud_selection_filter(
        &mut self,
        tab_index: usize,
        filter: PointFilter,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDSELECTFILTER: attach a LAS/LAZ cloud first.");
            return;
        }
        self.tabs[tab_index].point_cloud.selection_filter = filter;
        let description = describe_filter(&self.tabs[tab_index].point_cloud.selection_filter);
        self.command_line
            .push_output(format!("POINTCLOUDSELECTFILTER: {description}.").as_str());
        self.persist_point_cloud(tab_index, "selection_filter", &description, &[]);
    }

    /// Runs an automated classifier over every source's display working set
    /// and commits the sparse patches as audited, undoable transactions.
    fn apply_classifier(
        &mut self,
        tab_index: usize,
        label: &str,
        classify: impl Fn(&[ocs_pointcloud::SamplePoint]) -> ocs_pointcloud::ClassifyResult,
    ) {
        let dataset = &self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            self.command_line
                .push_error(format!("{label}: attach a LAS/LAZ cloud first.").as_str());
            return;
        }
        let sources: Vec<(
            String,
            Vec<ocs_pointcloud::SamplePoint>,
            ocs_pointcloud::EditStore,
        )> = dataset
            .sources
            .iter()
            .map(|source| {
                let points: Vec<_> = source
                    .active_points()
                    .into_iter()
                    .map(|point| {
                        source
                            .edits
                            .patch_for(point.source_index)
                            .map_or_else(|| point.clone(), |patch| point.clone().with_patch(patch))
                    })
                    .collect();
                (source.id.clone(), points, source.edits.clone())
            })
            .collect();
        let mut touched = Vec::new();
        let mut total = 0_usize;
        for (id, points, mut edits) in sources {
            let result = classify(&points);
            if result.is_empty() {
                continue;
            }
            let changed = result.apply_grouped(&mut edits, label);
            if changed == 0 {
                continue;
            }
            total += changed;
            touched.push((id, edits));
        }
        if touched.is_empty() {
            self.command_line
                .push_info(format!("{label}: no points matched.").as_str());
            return;
        }
        let ids: Vec<String> = touched.iter().map(|(id, _)| id.clone()).collect();
        {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            for (id, edits) in touched {
                if let Some(source) = dataset.source_mut(&id) {
                    source.edits = edits;
                }
            }
            dataset.note_edit_sources(ids.clone());
            dataset.mark_display_changed();
            let model = dataset.display_model();
            self.tabs[tab_index].scene.set_point_cloud(model);
        }
        self.command_line.push_output(
            format!(
                "{label}: classified {total} point(s) across {} source(s); export to publish.",
                ids.len()
            )
            .as_str(),
        );
        self.persist_point_cloud(
            tab_index,
            "classify",
            &format!("{label}: {total} points"),
            &ids,
        );
    }

    pub(super) fn classify_point_cloud_noise(
        &mut self,
        tab_index: usize,
        radius: f64,
        min_neighbors: usize,
        noise_class: u8,
    ) {
        self.apply_classifier(tab_index, "Auto noise", move |points| {
            ocs_pointcloud::detect_noise(points, radius, min_neighbors, noise_class)
        });
    }

    pub(super) fn classify_point_cloud_ground(
        &mut self,
        tab_index: usize,
        options: ocs_pointcloud::GroundOptions,
    ) {
        self.apply_classifier(tab_index, "Auto ground", move |points| {
            ocs_pointcloud::classify_ground(points, &options)
        });
    }

    pub(super) fn classify_point_cloud_rule(
        &mut self,
        tab_index: usize,
        rule: ocs_pointcloud::ClassifyRule,
    ) {
        self.apply_classifier(tab_index, "Rule classify", move |points| {
            ocs_pointcloud::classify_by_rules(points, std::slice::from_ref(&rule))
        });
    }

    /// Builds a ground TIN over the dataset's class-2 points and writes
    /// chained contour polylines as CAD entities. When no ground class
    /// exists yet, it offers to use every point rather than failing.
    pub(super) fn generate_point_cloud_contours(&mut self, tab_index: usize, interval: f64) {
        const GROUND_CLASS: u8 = 2;
        let dataset = &self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            self.command_line
                .push_error("POINTCLOUDCONTOUR: attach a LAS/LAZ cloud first.");
            return;
        }
        if !interval.is_finite() || interval <= 0.0 {
            self.command_line
                .push_error("POINTCLOUDCONTOUR: interval must be positive.");
            return;
        }
        let patched = |source: &PointCloudAttachment| {
            source
                .active_points()
                .into_iter()
                .map(|point| {
                    source
                        .edits
                        .patch_for(point.source_index)
                        .map_or_else(|| point.clone(), |patch| point.clone().with_patch(patch))
                })
                .collect::<Vec<_>>()
        };
        let mut ground_points: Vec<ocs_pointcloud::SamplePoint> = Vec::new();
        let mut all_points: Vec<ocs_pointcloud::SamplePoint> = Vec::new();
        for source in &dataset.sources {
            let points = patched(source);
            ground_points.extend(
                points
                    .iter()
                    .filter(|p| p.classification == GROUND_CLASS)
                    .cloned(),
            );
            all_points.extend(points);
        }
        let (surface_points, label) = if ground_points.len() >= 3 {
            (ground_points, "ground class")
        } else {
            self.command_line.push_info(
                "POINTCLOUDCONTOUR: fewer than three class-2 points; contouring every point — run POINTCLOUDGROUND first for true bare-earth contours.",
            );
            (all_points, "all points")
        };
        let Some(tin) = ocs_pointcloud::Tin::from_points(&surface_points, None) else {
            self.command_line
                .push_error("POINTCLOUDCONTOUR: not enough points to triangulate.");
            return;
        };
        let triangles = tin.triangle_count();
        let contours = ocs_pointcloud::generate_contours(&tin, interval, 0.0);
        if contours.is_empty() {
            self.command_line.push_output(
                format!(
                    "POINTCLOUDCONTOUR: {triangles} triangles over {label}; the elevation range has no full {interval} interval.",
                )
                .as_str(),
            );
            return;
        }
        let layer = "LIDAR-CONTOURS";
        let mut created = 0_usize;
        for contour in &contours {
            let mut polyline = acadrust::entities::Polyline2D::new();
            polyline.elevation = contour.elevation;
            polyline.common.layer = layer.to_string();
            // Contours are open polylines; the default flags already leave
            // CLOSED unset.
            for point in &contour.points {
                polyline.add_vertex(acadrust::entities::Vertex2D::new(
                    acadrust::types::Vector3::new(point[0], point[1], point[2]),
                ));
            }
            self.commit_entity(acadrust::EntityType::Polyline2D(polyline));
            created += 1;
        }
        self.command_line.push_output(
            format!(
                "POINTCLOUDCONTOUR: {created} contour polylines at {interval} intervals from {triangles} triangles over {label}, on layer \"{layer}\".",
            )
            .as_str(),
        );
    }

    pub(super) fn patch_point_cloud_selection(
        &mut self,
        tab_index: usize,
        label: &str,
        patch: PointPatch,
    ) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDEDITSELECTION: attach a LAS/LAZ cloud first.");
            return;
        }
        let mut touched = Vec::new();
        let mut changed = 0_usize;
        for cloud in &mut self.tabs[tab_index].point_cloud.sources {
            let Some(selection) = cloud
                .selection_sets
                .iter()
                .find(|selection| selection.name == "active")
                .cloned()
            else {
                continue;
            };
            let count = cloud.edits.apply(label, selection.iter(), patch);
            if count > 0 {
                changed += count;
                touched.push(cloud.id.clone());
            }
        }
        if touched.is_empty() {
            self.command_line
                .push_error("POINTCLOUDEDITSELECTION: create an active selection first.");
            return;
        }
        self.tabs[tab_index]
            .point_cloud
            .note_edit_sources(touched.clone());
        self.tabs[tab_index].point_cloud.mark_display_changed();
        let model = self.tabs[tab_index].point_cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        let source_note = if touched.len() > 1 {
            format!(" across {} sources", touched.len())
        } else {
            String::new()
        };
        self.command_line.push_output(
            format!("POINTCLOUDEDITSELECTION: {label}; {changed} point(s) queued{source_note}.")
                .as_str(),
        );
        self.persist_point_cloud(
            tab_index,
            "edit",
            &format!("{label}: {changed} points"),
            &touched,
        );
    }

    pub(super) fn import_point_cloud_ptc(&mut self, tab_index: usize, path: PathBuf) {
        let result = std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse_ptc(&text).map_err(|error| error.to_string()));
        match result {
            Ok(classes) => {
                let count = classes.classes.len();
                if self.tabs[tab_index].point_cloud.is_empty() {
                    self.command_line
                        .push_error("POINTCLOUDPTCIMPORT: attach a LAS/LAZ cloud first.");
                    return;
                }
                self.tabs[tab_index].point_cloud.classes = classes;
                self.restyle_point_cloud(tab_index);
                self.command_line.push_output(
                    format!(
                        "POINTCLOUDPTCIMPORT: loaded {count} class definitions from \"{}\".",
                        path.display()
                    )
                    .as_str(),
                );
                self.persist_point_cloud(tab_index, "classes", "imported PTC class table", &[]);
            }
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDPTCIMPORT: {error}").as_str()),
        }
    }

    pub(super) fn export_point_cloud_ptc(&mut self, tab_index: usize, path: PathBuf) {
        if self.tabs[tab_index].point_cloud.is_empty() {
            self.command_line
                .push_error("POINTCLOUDPTCEXPORT: attach a LAS/LAZ cloud first.");
            return;
        }
        let classes = self.tabs[tab_index].point_cloud.classes.clone();
        match std::fs::write(&path, write_ptc(&classes)) {
            Ok(()) => self.command_line.push_output(
                format!("POINTCLOUDPTCEXPORT: wrote \"{}\".", path.display()).as_str(),
            ),
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDPTCEXPORT: {error}").as_str()),
        }
    }

    fn persist_point_cloud(
        &mut self,
        tab_index: usize,
        action: &str,
        detail: &str,
        audit_sources: &[String],
    ) {
        let Some(drawing_path) = self.tabs[tab_index].current_path.clone() else {
            return;
        };
        let dataset = &self.tabs[tab_index].point_cloud;
        if dataset.is_empty() {
            return;
        }
        let states: Vec<AttachmentState> = match dataset
            .sources
            .iter()
            .map(|cloud| {
                AttachmentState::new(&cloud.id, &drawing_path, &cloud.source_path).map(|state| {
                    AttachmentState {
                        display: dataset.display.clone(),
                        classes: dataset.classes.clone(),
                        edits: cloud.edits.clone(),
                        selection_sets: cloud.selection_sets.clone(),
                        selection_filter: dataset.selection_filter.clone(),
                        cache_relative: cloud.cache_path.as_ref().and_then(|cache_path| {
                            drawing_path
                                .parent()
                                .and_then(|parent| cache_path.strip_prefix(parent).ok())
                                .map(std::path::Path::to_path_buf)
                        }),
                        ..state
                    }
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(states) => states,
            // A missing source must never prune its sidecar rows; skip the
            // save and report why.
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDSIDECAR: {error}").as_str());
                return;
            }
        };
        let result = (|| -> std::result::Result<(), String> {
            let mut store = SidecarStore::open(sidecar_path_for_drawing(&drawing_path))
                .map_err(|error| error.to_string())?;
            store
                .save_dataset(&states, dataset.collection.as_ref())
                .map_err(|error| error.to_string())?;
            for id in audit_sources {
                store
                    .append_audit(id, action, detail)
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.command_line
                .push_error(format!("POINTCLOUDSIDECAR: {error}").as_str());
        }
    }

    pub(super) fn start_point_cloud_export(&mut self, output: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        let dataset = &mut self.tabs[self.active_tab].point_cloud;
        if dataset
            .sources
            .iter()
            .any(|source| source.export_job.is_some())
        {
            self.command_line
                .push_error("POINTCLOUDEXPORT: an export is already running.");
            return Task::none();
        }
        let Some(cloud) = dataset.active_mut() else {
            self.command_line
                .push_error("POINTCLOUDEXPORT: attach a LAS/LAZ cloud first.");
            return Task::none();
        };
        let input = cloud.source_path.clone();
        let edits = cloud.edits.clone();
        let progress = Arc::new(PointCloudJobProgress::new(
            cloud.sample.metadata.point_count,
        ));
        cloud.export_job = Some(Arc::clone(&progress));
        let worker_output = output.clone();
        self.command_line.push_info(
            format!(
                "POINTCLOUDEXPORT: streaming {} source points to \"{}\"...",
                cloud.sample.metadata.point_count,
                output.display()
            )
            .as_str(),
        );
        background_task(
            move || {
                ocs_pointcloud::export_with_patches_progress(
                    input,
                    &worker_output,
                    &edits,
                    |state| {
                        progress
                            .completed
                            .store(state.points_read, Ordering::Relaxed);
                        !progress.cancel.load(Ordering::Relaxed)
                    },
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudExported(tab_id, output, result),
        )
    }

    pub(super) fn start_point_cloud_reprojection(
        &mut self,
        output: PathBuf,
        target_epsg: u16,
    ) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        let dataset = &mut self.tabs[self.active_tab].point_cloud;
        if dataset
            .sources
            .iter()
            .any(|source| source.export_job.is_some())
        {
            self.command_line
                .push_error("POINTCLOUDREPROJECT: an export/reprojection job is already running.");
            return Task::none();
        }
        let Some(cloud) = dataset.active_mut() else {
            return Task::none();
        };
        let source_crs = cloud.sample.metadata.crs.clone();
        if source_crs.horizontal_epsg.is_none() && source_crs.proj4.is_none() {
            self.command_line.push_error(
                "POINTCLOUDREPROJECT: source horizontal CRS is unresolved; assign/repair CRS metadata before transforming coordinates.",
            );
            return Task::none();
        }
        let source_label = source_crs.horizontal_label();
        let input = cloud.source_path.clone();
        let edits = cloud.edits.clone();
        let progress = Arc::new(PointCloudJobProgress::new(
            cloud.sample.metadata.point_count,
        ));
        cloud.export_job = Some(Arc::clone(&progress));
        let worker_output = output.clone();
        self.command_line.push_info(
            format!(
                "POINTCLOUDREPROJECT: streaming {source_label} to EPSG:{target_epsg}; XY will transform and Z will be preserved. Output: \"{}\".",
                output.display()
            )
            .as_str(),
        );
        background_task(
            move || {
                ocs_pointcloud::reproject_with_patches_progress(
                    input,
                    &worker_output,
                    &edits,
                    target_epsg,
                    |state| {
                        progress
                            .completed
                            .store(state.points_read, Ordering::Relaxed);
                        !progress.cancel.load(Ordering::Relaxed)
                    },
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudReprojected(tab_id, output, result),
        )
    }

    pub(super) fn finish_point_cloud_reprojection(
        &mut self,
        tab_id: u64,
        output: PathBuf,
        result: Result<ocs_pointcloud::ReprojectionStats, String>,
    ) {
        let tab_index = self.tabs.iter().position(|tab| tab.id == tab_id);
        let mut touched = Vec::new();
        if let Some(tab_index) = tab_index {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            if let Some(source) = dataset
                .sources
                .iter_mut()
                .find(|source| source.export_job.is_some())
            {
                source.export_job = None;
                touched.push(source.id.clone());
            }
        }
        match result {
            Ok(stats) => {
                let detail = format!(
                    "wrote {} points to EPSG:{} at \"{}\"; {} Z values preserved",
                    stats.points_written,
                    stats.target_horizontal_epsg,
                    output.display(),
                    stats.vertical_values_preserved,
                );
                self.command_line
                    .push_output(format!("POINTCLOUDREPROJECT: {detail}.").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "reproject", &detail, &touched);
                }
            }
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDREPROJECT: {error}").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "reproject_failed", &error, &touched);
                }
            }
        }
    }

    pub(super) fn finish_point_cloud_export(
        &mut self,
        tab_id: u64,
        output: PathBuf,
        result: Result<ExportStats, String>,
    ) {
        let tab_index = self.tabs.iter().position(|tab| tab.id == tab_id);
        let mut touched = Vec::new();
        if let Some(tab_index) = tab_index {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            if let Some(source) = dataset
                .sources
                .iter_mut()
                .find(|source| source.export_job.is_some())
            {
                source.export_job = None;
                touched.push(source.id.clone());
            }
        } else {
            self.command_line
                .push_info("POINTCLOUDEXPORT: target drawing was closed; export result follows.");
        }
        match result {
            Ok(stats) => {
                let detail = format!(
                    "wrote {} points to \"{}\"; {} classifications, {} flags and {} elevations changed",
                    stats.points_written,
                    output.display(),
                    stats.points_reclassified,
                    stats.point_flags_changed,
                    stats.elevations_changed,
                );
                self.command_line
                    .push_output(format!("POINTCLOUDEXPORT: {detail}.").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "export", &detail, &touched);
                }
            }
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDEXPORT: {error}").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "export_failed", &error, &touched);
                }
            }
        }
    }

    pub(super) fn start_point_cloud_export_all(&mut self, output: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        let dataset = &self.tabs[self.active_tab].point_cloud;
        if dataset
            .sources
            .iter()
            .any(|source| source.export_job.is_some())
            || dataset.export_all_job.is_some()
        {
            self.command_line
                .push_error("POINTCLOUDEXPORTALL: an export is already running.");
            return Task::none();
        }
        if dataset.len() < 2 {
            self.command_line.push_error(
                "POINTCLOUDEXPORTALL: attach at least two sources (use POINTCLOUDATTACHFOLDER) before merging.",
            );
            return Task::none();
        }
        let Some(target_crs) = self.tabs[self.active_tab]
            .spatial
            .drawing_crs
            .as_ref()
            .map(crate::app::spatial::DrawingCrs::as_crs_info)
        else {
            self.command_line.push_error(
                "POINTCLOUDEXPORTALL: set or infer the drawing CRS before merging sources.",
            );
            return Task::none();
        };
        let sources: Vec<ocs_pointcloud::MergeSource> = dataset
            .sources
            .iter()
            .map(|source| ocs_pointcloud::MergeSource {
                path: source.source_path.clone(),
                edits: source.edits.clone(),
                source_crs: Some(source.source_crs.clone()),
            })
            .collect();
        let total: u64 = dataset
            .sources
            .iter()
            .map(|source| source.sample.metadata.point_count)
            .sum();
        let progress = Arc::new(PointCloudJobProgress::new(total));
        self.tabs[self.active_tab].point_cloud.export_all_job = Some(Arc::clone(&progress));
        let worker_output = output.clone();
        self.command_line.push_info(
            format!(
                "POINTCLOUDEXPORTALL: streaming {} source(s), {} points total to \"{}\"; point_source_id records each source file...",
                sources.len(),
                total,
                output.display()
            )
            .as_str(),
        );
        background_task(
            move || {
                ocs_pointcloud::export_merged_reprojected_progress(
                    &sources,
                    &worker_output,
                    &target_crs,
                    |state| {
                        progress
                            .completed
                            .store(state.points_read, Ordering::Relaxed);
                        !progress.cancel.load(Ordering::Relaxed)
                    },
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudExportAllFinished(tab_id, output, result),
        )
    }

    pub(super) fn finish_point_cloud_export_all(
        &mut self,
        tab_id: u64,
        output: PathBuf,
        result: Result<ExportStats, String>,
    ) {
        let tab_index = self.tabs.iter().position(|tab| tab.id == tab_id);
        let mut touched = Vec::new();
        if let Some(tab_index) = tab_index {
            let dataset = &mut self.tabs[tab_index].point_cloud;
            dataset.export_all_job = None;
            touched = dataset
                .sources
                .iter()
                .map(|source| source.id.clone())
                .collect();
        }
        match result {
            Ok(stats) => {
                let detail = format!(
                    "wrote {} points from {} source(s) to \"{}\"; {} classifications, {} flags and {} elevations changed",
                    stats.points_written,
                    touched.len(),
                    output.display(),
                    stats.points_reclassified,
                    stats.point_flags_changed,
                    stats.elevations_changed,
                );
                self.command_line
                    .push_output(format!("POINTCLOUDEXPORTALL: {detail}.").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "export_all", &detail, &touched);
                }
            }
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDEXPORTALL: {error}").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "export_all_failed", &error, &touched);
                }
            }
        }
    }

    pub(super) fn start_point_cloud_3d_tiles_export(&mut self, output: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        let dataset = &self.tabs[self.active_tab].point_cloud;
        let Some(source) = dataset.active() else {
            self.command_line
                .push_error("POINTCLOUD3DTILES: attach a LAS/LAZ cloud first.");
            return Task::none();
        };
        if dataset
            .sources
            .iter()
            .any(|source| source.export_job.is_some())
            || dataset.export_all_job.is_some()
        {
            self.command_line
                .push_error("POINTCLOUD3DTILES: another export is already running.");
            return Task::none();
        }
        if !source.source_crs.is_resolvable() {
            self.command_line.push_error(
                "POINTCLOUD3DTILES: declare or assume the source CRS before exporting standards-compliant Earth-centered tiles.",
            );
            return Task::none();
        }
        let source_path = source.source_path.clone();
        let source_crs = source.source_crs.clone();
        let total = source.sample.metadata.point_count;
        let progress = Arc::new(PointCloudJobProgress::new(total));
        self.tabs[self.active_tab]
            .point_cloud
            .active_mut()
            .expect("active source was checked")
            .export_job = Some(Arc::clone(&progress));
        let worker_output = output.clone();
        self.command_line.push_info(
            format!(
                "POINTCLOUD3DTILES: streaming {total} full-density records into a disk-backed octree at '{}'...",
                output.display()
            )
            .as_str(),
        );
        background_task(
            move || {
                let mut writer = ocs_platform::PointOctreeWriter::create(
                    &worker_output,
                    0.0,
                    ocs_platform::OctreeOptions::default(),
                    false,
                )
                .map_err(|error| error.to_string())?;
                let wgs84 = ocs_pointcloud::CrsInfo {
                    horizontal_epsg: Some(4326),
                    name: Some("WGS 84".into()),
                    ..Default::default()
                };
                let height_scale = match ocs_pointcloud::crs_horizontal_unit(&source_crs) {
                    Some("us-ft") => 1200.0 / 3937.0,
                    Some("ft") => 0.3048,
                    _ => 1.0,
                };
                let mut chunk = Vec::with_capacity(65_536);
                let scan = ocs_pointcloud::visit_full_density(
                    &source_path,
                    &ocs_pointcloud::ProcessingExtent::All,
                    &ocs_pointcloud::PointFilter::default(),
                    Some(&progress.cancel),
                    |state| {
                        progress.completed.store(state.scanned, Ordering::Relaxed);
                    },
                    |point| {
                        chunk.push(point.clone());
                        if chunk.len() == 65_536 {
                            write_ecef_tile_chunk(
                                &source_crs,
                                &wgs84,
                                height_scale,
                                &mut chunk,
                                &mut writer,
                            )?;
                        }
                        Ok(())
                    },
                )
                .map_err(|error| error.to_string())?;
                write_ecef_tile_chunk(
                    &source_crs,
                    &wgs84,
                    height_scale,
                    &mut chunk,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?;
                progress.completed.store(scan.scanned, Ordering::Relaxed);
                writer.finish().map_err(|error| error.to_string())
            },
            move |result| Message::PointCloud3DTilesFinished(tab_id, output, result),
        )
    }

    pub(super) fn finish_point_cloud_3d_tiles_export(
        &mut self,
        tab_id: u64,
        output: PathBuf,
        result: Result<ocs_platform::OctreeTilesetExport, String>,
    ) {
        let tab_index = self.tabs.iter().position(|tab| tab.id == tab_id);
        let mut touched = Vec::new();
        if let Some(tab_index) = tab_index {
            if let Some(source) = self.tabs[tab_index].point_cloud.active_mut() {
                source.export_job = None;
                touched.push(source.id.clone());
            }
        }
        match result {
            Ok(export) => {
                let detail = format!(
                    "streamed {} points into {} PNTS tiles (depth {}, {:.1} MiB) at '{}'",
                    export.point_count,
                    export.tile_count,
                    export.max_depth,
                    export.byte_length as f64 / (1024.0 * 1024.0),
                    output.display()
                );
                self.command_line
                    .push_output(format!("POINTCLOUD3DTILES: {detail}.").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "3d_tiles_export", &detail, &touched);
                }
            }
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUD3DTILES: {error}").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "3d_tiles_export_failed", &error, &touched);
                }
            }
        }
    }

    pub(super) fn suggested_merged_export_name(&self, tab_index: usize) -> String {
        let dataset = &self.tabs[tab_index].point_cloud;
        let stem = dataset
            .collection
            .as_ref()
            .map(|collection| collection.display_name.clone())
            .unwrap_or_else(|| "point-cloud-dataset".to_string());
        format!("{stem}_merged.laz")
    }

    /// Live export/reprojection progress for the dataset, if a job runs.
    pub(super) fn point_cloud_export_progress(&self, tab_index: usize) -> Option<(u64, u64)> {
        self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .filter_map(|source| source.export_job.as_ref())
            .next()
            .or(self.tabs[tab_index].point_cloud.export_all_job.as_ref())
            .map(|job| (job.completed.load(Ordering::Relaxed), job.total))
    }

    pub(super) fn point_cloud_export_status(&mut self, tab_index: usize) {
        let job = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .filter_map(|source| source.export_job.as_ref())
            .next()
            .cloned()
            .or_else(|| self.tabs[tab_index].point_cloud.export_all_job.clone());
        let Some(job) = job else {
            self.command_line
                .push_info("POINTCLOUDEXPORTSTATUS: no export is running.");
            return;
        };
        let completed = job.completed.load(Ordering::Relaxed);
        let percent = if job.total == 0 {
            100.0
        } else {
            completed as f64 / job.total as f64 * 100.0
        };
        self.command_line.push_output(
            format!(
                "POINTCLOUDEXPORTSTATUS: {completed}/{} points ({percent:.1}%).",
                job.total
            )
            .as_str(),
        );
    }

    pub(super) fn cancel_point_cloud_export(&mut self, tab_index: usize) {
        let job = self.tabs[tab_index]
            .point_cloud
            .sources
            .iter()
            .filter_map(|source| source.export_job.as_ref())
            .next()
            .cloned()
            .or_else(|| self.tabs[tab_index].point_cloud.export_all_job.clone());
        if let Some(job) = job {
            job.cancel.store(true, Ordering::Relaxed);
            self.command_line
                .push_output("POINTCLOUDEXPORTCANCEL: cancellation requested.");
        } else {
            self.command_line
                .push_info("POINTCLOUDEXPORTCANCEL: no export is running.");
        }
    }

    /// Runs the native UPCP/Boston fusion engine against either the active
    /// tile or its source folder. The source `classification` byte is never
    /// modified; results carry a uint8 `label` extra dimension plus
    /// provenance and publish only after a validated atomic write.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn start_point_cloud_urban_classification(
        &mut self,
        tab_index: usize,
        folder_scope: bool,
    ) -> Task<Message> {
        self.start_point_cloud_urban_classification_with_scope(tab_index, folder_scope, None, None)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn start_point_cloud_urban_classification_with_scope(
        &mut self,
        tab_index: usize,
        folder_scope: bool,
        input_override: Option<PathBuf>,
        output_override: Option<PathBuf>,
    ) -> Task<Message> {
        let settings = ocs_pointcloud::UrbanClassificationSettings {
            scope: if folder_scope {
                ocs_pointcloud::UrbanScope::Folder
            } else {
                ocs_pointcloud::UrbanScope::CurrentTile
            },
            output_folder: output_override.clone(),
            ..Default::default()
        };
        self.start_point_cloud_urban_job(tab_index, settings, input_override)
    }

    /// Starts a native urban classification from a settings object, e.g. a
    /// script-supplied JSON preset. Returns an error message when the
    /// settings are unusable.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn start_point_cloud_urban_classification_from_settings(
        &mut self,
        tab_index: usize,
        settings: ocs_pointcloud::UrbanClassificationSettings,
    ) -> Result<Task<Message>, String> {
        if !matches!(settings.scope, ocs_pointcloud::UrbanScope::CurrentTile)
            && settings.output_folder.is_none()
        {
            // Folder scope without an explicit output still works: it derives
            // the sibling `classified` directory from the active source.
        }
        Ok(self.start_point_cloud_urban_job(tab_index, settings, None))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_point_cloud_urban_job(
        &mut self,
        tab_index: usize,
        settings: ocs_pointcloud::UrbanClassificationSettings,
        input_override: Option<PathBuf>,
    ) -> Task<Message> {
        let tab_id = self.tabs[tab_index].id;
        let dataset = &mut self.tabs[tab_index].point_cloud;
        if dataset.urban_job.is_some() {
            self.command_line
                .push_info("POINTCLOUDURBANCLASSIFY: a classification job is already running.");
            return Task::none();
        }
        let Some(active) = dataset.active() else {
            self.command_line
                .push_error("POINTCLOUDURBANCLASSIFY: attach a LAS/LAZ cloud first.");
            return Task::none();
        };

        let mut source = active.source_path.clone();
        let mut input_dir = source.parent().map(PathBuf::from).unwrap_or_default();
        // A completed classified output may be attached when the user reruns
        // the workflow. Resolve it back to the original sibling source.
        if input_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("classified"))
        {
            if let (Some(parent), Some(stem), Some(extension)) = (
                input_dir.parent(),
                source.file_stem().and_then(|value| value.to_str()),
                source.extension().and_then(|value| value.to_str()),
            ) {
                if let Some(original_stem) = stem.strip_suffix("_classified") {
                    let candidate = parent.join(format!("{original_stem}.{extension}"));
                    if candidate.is_file() {
                        source = candidate;
                        input_dir = parent.to_path_buf();
                    }
                }
            }
        }
        if let Some(input) = input_override {
            if input.is_dir() {
                input_dir = input;
            } else if input.is_file() {
                source = input.clone();
                input_dir = input.parent().map(PathBuf::from).unwrap_or_default();
            }
        }
        if !source.is_file() || !input_dir.is_dir() {
            self.command_line
                .push_error("POINTCLOUDURBANCLASSIFY: the original source tile is unavailable.");
            return Task::none();
        }

        let output_dir = settings
            .output_folder
            .clone()
            .filter(|path| path.is_dir() || path.parent().is_some())
            .unwrap_or_else(|| input_dir.join("classified"));
        let folder_scope = matches!(settings.scope, ocs_pointcloud::UrbanScope::Folder);
        let references_dir = output_dir.join("references");
        // Explicit profiles win; AutoDetect resolves against the cloud CRS
        // (the Boston layers are published in the EPSG:6492 survey-foot grid).
        let (provider, profile_label) = match &settings.profile {
            ocs_pointcloud::UrbanProfile::BostonArcGis => (
                Box::new(ocs_pointcloud::BostonArcGisProvider::new())
                    as Box<dyn ocs_pointcloud::UrbanReferenceProvider>,
                "Boston ArcGIS".to_string(),
            ),
            ocs_pointcloud::UrbanProfile::LocalDirectory { path } => (
                Box::new(ocs_pointcloud::LocalVectorProvider::new(path.clone()))
                    as Box<dyn ocs_pointcloud::UrbanReferenceProvider>,
                format!("local references ({})", path.display()),
            ),
            ocs_pointcloud::UrbanProfile::AutoDetect => {
                let crs = &active.source_crs;
                if crs.horizontal_epsg == Some(6492)
                    || crs
                        .name
                        .as_deref()
                        .is_some_and(|name| name.contains("6492"))
                    || crs.wkt.as_deref().is_some_and(|wkt| wkt.contains("6492"))
                {
                    (
                        Box::new(ocs_pointcloud::BostonArcGisProvider::new())
                            as Box<dyn ocs_pointcloud::UrbanReferenceProvider>,
                        "Boston ArcGIS (auto-detected)".to_string(),
                    )
                } else {
                    (
                        Box::new(ocs_pointcloud::LocalVectorProvider::new(
                            references_dir.clone(),
                        ))
                            as Box<dyn ocs_pointcloud::UrbanReferenceProvider>,
                        "local/cached references".to_string(),
                    )
                }
            }
        };
        let state = Arc::new(UrbanJobState::new());
        dataset.urban_job = Some(Arc::clone(&state));
        dataset.urban_status = if folder_scope {
            "Running full-density source-folder classification".to_string()
        } else {
            "Running full-density current-tile classification".to_string()
        };
        self.command_line.push_info(
            format!(
                "POINTCLOUDURBANCLASSIFY: {} via {} -> \"{}\"; buildings/roads/vegetation enabled (12 ft tree radius)...",
                if folder_scope { "source folder" } else { "current tile" },
                profile_label,
                output_dir.display()
            )
            .as_str(),
        );

        let run_source = source.clone();
        let run_input_dir = input_dir.clone();
        let run_output_dir = output_dir.clone();
        let progress_state = Arc::clone(&state);
        background_task(
            move || {
                let progress = &mut |tick: ocs_pointcloud::UrbanJobProgress| {
                    progress_state.stage.store(
                        match tick.stage {
                            ocs_pointcloud::UrbanStage::LoadingReferences => 0u64,
                            ocs_pointcloud::UrbanStage::Classifying => 1,
                            ocs_pointcloud::UrbanStage::Validating => 2,
                            ocs_pointcloud::UrbanStage::Completed => 3,
                        },
                        Ordering::Relaxed,
                    );
                    progress_state
                        .points_done
                        .store(tick.points_processed, Ordering::Relaxed);
                    progress_state
                        .points_total
                        .store(tick.points_total, Ordering::Relaxed);
                    progress_state
                        .tile_index
                        .store(tick.tile_index as u64, Ordering::Relaxed);
                    progress_state
                        .tile_total
                        .store(tick.tile_total as u64, Ordering::Relaxed);
                    progress_state
                        .building_features
                        .store(tick.building_features as u64, Ordering::Relaxed);
                    progress_state
                        .road_features
                        .store(tick.road_features as u64, Ordering::Relaxed);
                    progress_state
                        .tree_features
                        .store(tick.tree_features as u64, Ordering::Relaxed);
                    if let Ok(mut path) = progress_state.output_path.lock() {
                        *path = tick.output_path;
                    }
                };
                let cancel_flag = &state.cancel;
                let mut provider = provider;
                let result = if folder_scope {
                    ocs_pointcloud::classify_urban_folder(
                        &run_input_dir,
                        &run_output_dir,
                        &settings,
                        provider.as_mut(),
                        cancel_flag,
                        progress,
                    )
                    .map(|summary| UrbanClassificationResult {
                        outputs: summary.outputs,
                        folder_scope: true,
                    })
                } else {
                    ocs_pointcloud::classify_urban_tile(
                        &run_source,
                        &run_output_dir,
                        &settings,
                        provider.as_mut(),
                        cancel_flag,
                        progress,
                    )
                    .map(|stats| UrbanClassificationResult {
                        outputs: vec![stats.output],
                        folder_scope: false,
                    })
                };
                let _ = run_source;
                result.map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudUrbanClassified(tab_id, result),
        )
    }

    /// Request cancellation of the running urban classification. Partial
    /// outputs are removed; completed tiles stay published.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn cancel_point_cloud_urban_classification(&mut self) {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.point_cloud.urban_job.is_some());
        if let Some(tab) = tab {
            if let Some(job) = tab.point_cloud.urban_job.as_ref() {
                job.cancel.store(true, Ordering::Relaxed);
                tab.point_cloud.urban_status = "Cancelling after the current chunk...".to_string();
                self.command_line
                    .push_output("POINTCLOUDURBANCANCEL: cancellation requested.");
            }
        } else {
            self.command_line
                .push_info("POINTCLOUDURBANCANCEL: no urban classification is running.");
        }
    }

    /// Report the live urban classification state to the command line.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn point_cloud_urban_status(&mut self, tab_index: usize) {
        let dataset = &self.tabs[tab_index].point_cloud;
        match &dataset.urban_job {
            None => {
                let status = if dataset.urban_status.is_empty() {
                    "no job has run".to_string()
                } else {
                    dataset.urban_status.clone()
                };
                self.command_line
                    .push_output(format!("POINTCLOUDURBANSTATUS: {status}").as_str());
            }
            Some(job) => {
                let snapshot = job.snapshot();
                let stage = match snapshot.stage {
                    0 => "loading references",
                    1 => "classifying",
                    2 => "validating output",
                    _ => "completed",
                };
                self.command_line.push_output(
                    format!(
                        "POINTCLOUDURBANSTATUS: {stage}; tile {}/{}; {}/{} points; {} buildings · {} roads · {} trees; {:.1}s elapsed.",
                        snapshot.tile_index,
                        snapshot.tile_total,
                        snapshot.points_done,
                        if snapshot.points_total == 0 {
                            snapshot.points_done
                        } else {
                            snapshot.points_total
                        },
                        snapshot.building_features,
                        snapshot.road_features,
                        snapshot.tree_features,
                        snapshot.elapsed_ms as f64 / 1000.0,
                    )
                    .as_str(),
                );
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn finish_point_cloud_urban_classification(
        &mut self,
        tab_id: u64,
        result: Result<UrbanClassificationResult, String>,
    ) -> Task<Message> {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Task::none();
        };
        let cancelled = self.tabs[tab_index]
            .point_cloud
            .urban_job
            .as_ref()
            .is_some_and(|job| job.cancel.load(Ordering::Relaxed));
        self.tabs[tab_index].point_cloud.urban_job = None;
        match result {
            Err(error) => {
                self.tabs[tab_index].point_cloud.urban_status = if cancelled {
                    format!("Cancelled: {error}")
                } else {
                    format!("Failed: {error}")
                };
                self.command_line.push_error(
                    format!(
                        "POINTCLOUDURBANCLASSIFY: {}{error}",
                        if cancelled { "cancelled: " } else { "" }
                    )
                    .as_str(),
                );
                Task::none()
            }
            Ok(result) => {
                self.tabs[tab_index].point_cloud.urban_status =
                    format!("Completed {} classified tile(s)", result.outputs.len());
                self.command_line.push_output(
                    format!(
                        "POINTCLOUDURBANCLASSIFY: completed {} full-density tile(s); attaching classified output.",
                        result.outputs.len()
                    )
                    .as_str(),
                );
                let prior_active = self.active_tab;
                self.active_tab = tab_index;
                self.detach_point_cloud(tab_index);
                let task = if result.folder_scope {
                    result
                        .outputs
                        .first()
                        .and_then(|path| path.parent())
                        .map_or_else(Task::none, |folder| {
                            self.start_point_cloud_folder_load(folder.to_path_buf())
                        })
                } else {
                    result
                        .outputs
                        .first()
                        .cloned()
                        .map_or_else(Task::none, |path| self.start_point_cloud_load(path))
                };
                self.active_tab = prior_active;
                task
            }
        }
    }
}

impl PointCloudDataset {
    fn manager_data(&self) -> crate::ui::window::point_cloud_manager::PointCloudManagerData {
        use crate::ui::window::point_cloud_manager::{PointCloudClassRow, PointCloudManagerData};
        if self.is_empty() {
            return PointCloudManagerData::default();
        }
        let source = self
            .active()
            .expect("non-empty dataset always has an active source");
        let active_selection: u64 = self
            .sources
            .iter()
            .flat_map(|source| source.selection_sets.iter())
            .filter(|selection| selection.name == "active")
            .map(SelectionSet::len)
            .sum();
        let color_mode = match self.display.color_mode {
            ColorMode::Classification => "Classification",
            ColorMode::Rgb => "RGB",
            ColorMode::Intensity => "Intensity",
            ColorMode::Elevation => "Elevation",
            ColorMode::ReturnNumber => "Return number",
            ColorMode::PointSource => "Point source",
            ColorMode::Label => "UPCP label",
        };
        let export_progress = self
            .sources
            .iter()
            .filter_map(|source| source.export_job.as_ref())
            .next()
            .or(self.export_all_job.as_ref())
            .map(|job| (job.completed.load(Ordering::Relaxed), job.total));
        let points = self.sources.iter().flat_map(|source| {
            source.active_points().into_iter().map(|point| {
                source
                    .edits
                    .patch_for(point.source_index)
                    .map_or(point.clone(), |patch| point.with_patch(patch))
            })
        });
        let statistics = classification_statistics(points);
        let class_rows = self
            .classes
            .classes
            .values()
            .map(|class| {
                let stats = statistics.get(&class.code).copied().unwrap_or_default();
                PointCloudClassRow {
                    code: class.code,
                    name: class.name.clone(),
                    color: class.color,
                    visible: class.visible && !self.display.hidden_classes.contains(&class.code),
                    locked: class.locked,
                    total: stats.total,
                    withheld: stats.withheld,
                    overlap: stats.overlap,
                    key_points: stats.key_points,
                }
            })
            .collect();
        let survey_readiness = ocs_pointcloud::assess_survey_readiness(&source.sample.metadata);
        let source_label = if self.len() > 1 {
            format!("{} (1 of {})", source.source_path.display(), self.len())
        } else {
            source.source_path.display().to_string()
        };
        let sample_label = if self.len() > 1 {
            "multi-source dataset".to_string()
        } else {
            match source.sample.stride {
                0 => "tiled LOD".to_string(),
                1 => "full cloud".to_string(),
                stride => format!("1-in-{stride} sample"),
            }
        };
        let index_running = self
            .sources
            .iter()
            .any(|source| source.index_cancel.is_some());
        let any_crs = self
            .sources
            .iter()
            .all(|source| source.sample.metadata.has_crs);
        let crs_label = if self.len() > 1 {
            if any_crs {
                format!("declared in {} source(s)", self.len())
            } else {
                "not declared in every source".to_string()
            }
        } else if source.sample.metadata.has_crs {
            source.sample.metadata.crs.label()
        } else {
            "not declared".to_string()
        };
        let resident_points: usize = self
            .sources
            .iter()
            .flat_map(|source| source.resident_tiles.values())
            .map(|tile| tile.points.len())
            .sum();
        let displayed_points: usize = self
            .sources
            .iter()
            .map(PointCloudAttachment::displayed_len)
            .sum();
        let mut lod_levels: Vec<u8> = self
            .sources
            .iter()
            .flat_map(|source| source.active_tiles.iter().map(|key| key.level))
            .collect();
        lod_levels.sort_unstable();
        let lod_label = lod_levels.first().zip(lod_levels.last()).map_or_else(
            || "sample path".to_string(),
            |(minimum, maximum)| {
                if minimum == maximum {
                    format!("level {minimum}")
                } else {
                    format!("levels {minimum}-{maximum}")
                }
            },
        );
        PointCloudManagerData {
            attached: true,
            source: source_label,
            source_points: self
                .sources
                .iter()
                .map(|source| source.sample.metadata.point_count)
                .sum(),
            displayed_points,
            sample_label,
            pending_edits: self.sources.iter().map(|source| source.edits.len()).sum(),
            transactions: self
                .sources
                .iter()
                .map(|source| source.edits.transaction_count())
                .sum(),
            active_selection,
            selection_sets: self
                .sources
                .iter()
                .map(|source| source.selection_sets.len())
                .sum(),
            class_count: self.classes.classes.len(),
            color_mode: color_mode.to_string(),
            point_size_px: self.display.point_size_px,
            section_width_map_units: self
                .section
                .map_or(32, |section| section.width_world.round() as i32)
                .clamp(1, 1024),
            crs_declared: any_crs,
            indexed: self
                .sources
                .iter()
                .all(|source| source.cache_manifest.is_some()),
            index_running,
            cache: source.cache_path.as_ref().map_or_else(
                || "not available".to_string(),
                |path| path.display().to_string(),
            ),
            export_progress,
            urban_job_running: self.urban_job.is_some(),
            urban_progress: self.urban_job.as_ref().map(|job| job.snapshot()),
            urban_output: self
                .urban_job
                .as_ref()
                .and_then(|job| job.output_path.lock().ok())
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            urban_status: self.urban_status.clone(),
            sidecar_available: false,
            selection_filter: describe_filter(&self.selection_filter),
            resident_tiles: self
                .sources
                .iter()
                .map(|source| source.resident_tiles.len())
                .sum(),
            resident_points,
            visible_tiles: self
                .sources
                .iter()
                .map(|source| source.active_tiles.len())
                .sum(),
            cpu_memory_bytes: resident_points
                .saturating_mul(std::mem::size_of::<ocs_pointcloud::SamplePoint>()),
            gpu_memory_bytes: displayed_points.saturating_mul(GPU_POINT_BYTES),
            lod_label,
            pending_tile_requests: self
                .sources
                .iter()
                .filter(|source| source.stream_in_flight)
                .count(),
            cancelled_tile_requests: self
                .sources
                .iter()
                .map(|source| source.cancelled_tile_requests)
                .sum(),
            stale_tile_results: self
                .sources
                .iter()
                .map(|source| source.stale_tile_results)
                .sum(),
            crs_label,
            survey_readiness: survey_readiness.summary(),
            class_rows,
            audit_rows: Vec::new(),
        }
    }
}

fn write_ecef_tile_chunk(
    source_crs: &ocs_pointcloud::CrsInfo,
    wgs84: &ocs_pointcloud::CrsInfo,
    height_scale: f64,
    points: &mut Vec<ocs_pointcloud::SamplePoint>,
    writer: &mut ocs_platform::PointOctreeWriter,
) -> ocs_pointcloud::Result<()> {
    if points.is_empty() {
        return Ok(());
    }
    ocs_pointcloud::reproject_points_between_crs(source_crs, wgs84, points)?;
    const SEMI_MAJOR: f64 = 6_378_137.0;
    const FLATTENING: f64 = 1.0 / 298.257_223_563;
    const ECCENTRICITY_SQUARED: f64 = FLATTENING * (2.0 - FLATTENING);
    for point in points.drain(..) {
        let longitude = point.position[0].to_radians();
        let latitude = point.position[1].to_radians();
        let height = point.position[2] * height_scale;
        let sin_latitude = latitude.sin();
        let cos_latitude = latitude.cos();
        let prime_vertical =
            SEMI_MAJOR / (1.0 - ECCENTRICITY_SQUARED * sin_latitude.powi(2)).sqrt();
        writer.write_point([
            (prime_vertical + height) * cos_latitude * longitude.cos(),
            (prime_vertical + height) * cos_latitude * longitude.sin(),
            (prime_vertical * (1.0 - ECCENTRICITY_SQUARED) + height) * sin_latitude,
        ])?;
    }
    Ok(())
}

fn background_task<T, F, M>(work: F, map: M) -> Task<Message>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    M: FnOnce(T) -> Message + Send + 'static,
{
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    Task::perform(
        async move { receiver.await.expect("point-cloud worker dropped") },
        map,
    )
}

/// Tile-read parallelism for one streaming batch. Sources stream one batch at
/// a time (round-robin per tick), so this bounds total reader threads.
fn tile_read_workers() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(ocs_pointcloud::MAX_TILE_READ_WORKERS)
        .max(1)
}

/// Returns the next source whose LOD cache still needs to be opened or built.
/// A recorded failure is skipped for the rest of the current batch so one bad
/// tile cannot trap the dispatcher in an immediate retry loop.
fn next_index_source_index(sources: &[PointCloudAttachment]) -> Option<usize> {
    sources
        .iter()
        .position(|source| source.cache_manifest.is_none() && source.index_error.is_none())
}

fn manifest_in_drawing_crs(
    mut manifest: TileCacheManifest,
    source_crs: &ocs_pointcloud::CrsInfo,
    drawing_crs: &ocs_pointcloud::CrsInfo,
) -> Result<TileCacheManifest, String> {
    if ocs_pointcloud::crs_equivalent(source_crs, drawing_crs) {
        return Ok(manifest);
    }
    for tile in &mut manifest.tiles {
        let (min, max) = ocs_pointcloud::reproject_bounds_between_crs(
            tile.bounds_min,
            tile.bounds_max,
            source_crs,
            drawing_crs,
        )
        .ok_or_else(|| {
            format!(
                "cannot transform LOD tile {:?} from {} to {}",
                tile.key,
                source_crs.horizontal_label(),
                drawing_crs.horizontal_label()
            )
        })?;
        tile.bounds_min = min;
        tile.bounds_max = max;
    }
    Ok(manifest)
}

/// Marks a source as streamed. From here the active working set lives in
/// `resident_tiles` (keyed by tile), so the bounded sample is released rather
/// than kept as a second in-memory copy of the same points.
fn rebuild_resident_display(cloud: &mut PointCloudAttachment) {
    cloud.sample.points = Vec::new();
    cloud.sample.stride = 0;
}

fn evict_resident_tiles(cloud: &mut PointCloudAttachment, cpu_budget_bytes: usize) {
    let point_size = std::mem::size_of::<ocs_pointcloud::SamplePoint>().max(1);
    let mut bytes = cloud
        .resident_tiles
        .values()
        .map(|tile| tile.points.len().saturating_mul(point_size))
        .sum::<usize>();
    if bytes <= cpu_budget_bytes {
        return;
    }
    let mut candidates: Vec<_> = cloud
        .resident_tiles
        .iter()
        .filter(|(key, _)| !cloud.active_tiles.contains(key))
        .map(|(key, tile)| (*key, tile.last_used))
        .collect();
    candidates.sort_by_key(|(_, last_used)| *last_used);
    for (key, _) in candidates {
        if bytes <= cpu_budget_bytes {
            break;
        }
        if let Some(tile) = cloud.resident_tiles.remove(&key) {
            bytes = bytes.saturating_sub(tile.points.len().saturating_mul(point_size));
        }
    }
}

/// Chooses the finest single LOD whose visible tiles fit the point budget.
/// `section_band` is an additional world-space query, never a substitute for
/// the camera test: this prevents a long slice from loading off-screen tiles.
fn select_visible_lod_tiles(
    tiles: &[ocs_pointcloud::TileEntry],
    leaf_level: u8,
    point_budget: u64,
    section_band: Option<([f64; 3], [f64; 3])>,
    mut is_visible: impl FnMut(&ocs_pointcloud::TileEntry) -> bool,
) -> Vec<ocs_pointcloud::TileEntry> {
    let budget = point_budget.max(1);
    for level in (0..=leaf_level).rev() {
        let candidates: Vec<_> = tiles
            .iter()
            .filter(|tile| {
                tile.key.level == level
                    && is_visible(tile)
                    && section_band
                        .is_none_or(|(band_min, band_max)| tile.intersects(band_min, band_max))
            })
            .cloned()
            .collect();
        let count = candidates.iter().map(|tile| tile.point_count).sum::<u64>();
        if count <= budget || level == 0 {
            return candidates;
        }
    }
    Vec::new()
}

/// Case-insensitive path equality on Windows, where LAS folders routinely
/// mix letter case between sessions.
fn path_matches(left: &std::path::Path, right: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Marker tile for a source's whole bounded sample (non-tiled display).
const SAMPLE_CHUNK_TILE: ocs_pointcloud::TileKey = ocs_pointcloud::TileKey {
    level: 0,
    x: 0,
    y: 0,
    z: 0,
};

/// Stable per-(source, tile) chunk identity for the GPU arena.
fn tile_chunk_key(source_id: &str, tile: &ocs_pointcloud::TileKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source_id.hash(&mut hasher);
    (tile.level, tile.x, tile.y, tile.z).hash(&mut hasher);
    hasher.finish()
}

/// Content revision of one source's rendered points: any edit, undo, or
/// selection change must produce a different value so its chunks re-upload.
fn source_chunk_generation(source: &PointCloudAttachment) -> u64 {
    let selection_len: u64 = source
        .selection_sets
        .iter()
        .find(|selection| selection.name == "active")
        .map_or(0, SelectionSet::len);
    (source.edits.transaction_count() as u64) << 44
        ^ (source.displayed_len() as u64) << 24
        ^ selection_len
}

fn categorical(value: u32) -> [f32; 4] {
    let hash = value.wrapping_mul(0x9e37_79b9).rotate_left(13);
    [
        0.25 + (hash & 0xff) as f32 / 510.0,
        0.25 + ((hash >> 8) & 0xff) as f32 / 510.0,
        0.25 + ((hash >> 16) & 0xff) as f32 / 510.0,
        1.0,
    ]
}

fn point_in_screen_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        let crosses = (current[1] > point[1]) != (previous[1] > point[1])
            && point[0]
                < (previous[0] - current[0]) * (point[1] - current[1]) / (previous[1] - current[1])
                    + current[0];
        if crosses {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn screen_polygon_bounds(polygon: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
    polygon.iter().fold(
        ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
        |(mut min, mut max), point| {
            min[0] = min[0].min(point[0]);
            min[1] = min[1].min(point[1]);
            max[0] = max[0].max(point[0]);
            max[1] = max[1].max(point[1]);
            (min, max)
        },
    )
}

fn ensure_screen_spatial_index(
    cloud: &mut PointCloudAttachment,
    camera: &crate::scene::view::camera::Camera,
    viewport: iced::Rectangle,
    camera_generation: u64,
) {
    let viewport_size = [
        viewport.width.max(1.0) as u32,
        viewport.height.max(1.0) as u32,
    ];
    // Display changes null the whole index (mark_display_changed), so only
    // camera generation and viewport size can make it stale.
    if cloud.screen_index.as_ref().is_some_and(|index| {
        index.camera_generation == camera_generation && index.viewport_size == viewport_size
    }) {
        return;
    }
    const CELL_SIZE: f32 = 32.0;
    let cells_x = (viewport.width / CELL_SIZE).ceil().max(1.0) as usize;
    let cells_y = (viewport.height / CELL_SIZE).ceil().max(1.0) as usize;
    // Snapshot the active working set once so the index does not depend on the
    // (possibly released) `sample.points` buffer for streamed sources.
    let snapshot: Arc<Vec<ocs_pointcloud::SamplePoint>> = Arc::new(cloud.active_points());
    let mut index = ScreenSpatialIndex {
        camera_generation,
        viewport_size,
        cell_size: CELL_SIZE,
        cells_x,
        cells_y,
        points: Vec::with_capacity(snapshot.len()),
        cells: vec![Vec::new(); cells_x.saturating_mul(cells_y)],
        snapshot,
    };
    let eye = camera.eye();
    let forward = (camera.rotation * glam::Vec3::NEG_Z).as_dvec3();
    for (sample_index, point) in index.snapshot.iter().enumerate() {
        let position = glam::DVec3::from_array(point.position);
        let depth = (position - eye).dot(forward);
        if depth <= 0.0 {
            continue;
        }
        let Some(screen) = camera.project(position, viewport) else {
            continue;
        };
        if screen.x < 0.0
            || screen.y < 0.0
            || screen.x > viewport.width
            || screen.y > viewport.height
        {
            continue;
        }
        let x = ((screen.x / CELL_SIZE) as usize).min(cells_x - 1);
        let y = ((screen.y / CELL_SIZE) as usize).min(cells_y - 1);
        let projected_index = index.points.len();
        index.points.push(ProjectedPoint {
            screen: [screen.x, screen.y],
            depth,
            sample_index,
        });
        index.cells[y * cells_x + x].push(projected_index);
    }
    cloud.screen_index = Some(index);
}

fn screen_candidates(
    index: &ScreenSpatialIndex,
    min: [f32; 2],
    max: [f32; 2],
) -> Vec<ProjectedPoint> {
    let cell = |value: f32, limit: usize| {
        ((value.max(0.0) / index.cell_size).floor() as usize).min(limit.saturating_sub(1))
    };
    let min_x = cell(min[0], index.cells_x);
    let min_y = cell(min[1], index.cells_y);
    let max_x = cell(max[0], index.cells_x);
    let max_y = cell(max[1], index.cells_y);
    let mut points = Vec::new();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            points.extend(
                index.cells[y * index.cells_x + x]
                    .iter()
                    .filter_map(|point| index.points.get(*point))
                    .copied(),
            );
        }
    }
    points
}

fn describe_filter(filter: &PointFilter) -> String {
    let mut parts = Vec::new();
    if !filter.classes.is_empty() {
        parts.push(format!(
            "classes={}",
            filter
                .classes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !filter.returns.is_empty() {
        parts.push(format!(
            "returns={}",
            filter
                .returns
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !filter.sources.is_empty() {
        parts.push(format!(
            "sources={}",
            filter
                .sources
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if let Some([low, high]) = filter.elevation {
        parts.push(format!("elevation={low:.3}..{high:.3}"));
    }
    for (name, value) in [
        ("synthetic", filter.synthetic),
        ("key", filter.key_point),
        ("withheld", filter.withheld),
        ("overlap", filter.overlap),
    ] {
        if let Some(value) = value {
            parts.push(format!("{name}={value}"));
        }
    }
    if parts.is_empty() {
        "no attribute filter".to_string()
    } else {
        parts.join("; ")
    }
}

fn cache_path_for_source(source: &std::path::Path) -> PathBuf {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("point-cloud");
    source.with_file_name(format!("{name}.ocstiles"))
}

/// The persisted sidecar path wins when valid, but a cache beside the source
/// is also discovered automatically. Older sidecars can therefore pick up an
/// already-built `<source>.ocstiles` directory without requiring a rebuild.
fn point_cloud_cache_candidates(
    source: &std::path::Path,
    persisted: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let default = cache_path_for_source(source);
    let mut candidates = Vec::with_capacity(2);
    if let Some(path) = persisted {
        candidates.push(path.to_path_buf());
    }
    if !candidates
        .iter()
        .any(|candidate| path_matches(candidate, &default))
    {
        candidates.push(default);
    }
    candidates
}

fn find_valid_tile_cache(
    source: &std::path::Path,
    persisted: Option<&std::path::Path>,
) -> Option<(PathBuf, TileCacheManifest)> {
    point_cloud_cache_candidates(source, persisted)
        .into_iter()
        .find_map(|cache_path| {
            TileCacheManifest::open(&cache_path)
                .and_then(|manifest| {
                    manifest.validate_source(source)?;
                    Ok((cache_path, manifest))
                })
                .ok()
        })
}

/// True when a full-density (`Density::Full`) read of `point_count` points would
/// exceed `cpu_budget_bytes`. Only `Full` is unbounded; `Auto` and explicit
/// 1-in-N samples are the user's own decimation choices and are left untouched.
fn full_density_over_budget(point_count: u64, cpu_budget_bytes: usize) -> bool {
    let point_size = std::mem::size_of::<ocs_pointcloud::SamplePoint>().max(1) as u64;
    point_count.saturating_mul(point_size) > cpu_budget_bytes as u64
}

/// Guard against pathological scans (junction loops, misdirected roots).
const MAX_FOLDER_SOURCES: usize = 2_000;

/// Recursively collects the LAS/LAZ files under `folder`, sorted for
/// deterministic attach order.
fn scan_lidar_folder(folder: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if found.len() + stack.len() < MAX_FOLDER_SOURCES * 4 {
                    stack.push(path);
                }
            } else {
                let is_lidar = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("las")
                            || extension.eq_ignore_ascii_case("laz")
                    });
                if is_lidar {
                    found.push(path);
                    if found.len() >= MAX_FOLDER_SOURCES {
                        found.sort();
                        return found;
                    }
                }
            }
        }
    }
    found.sort();
    found
}

pub(super) fn parse_source_indices(spec: &str, point_count: u64) -> Result<Vec<u64>, String> {
    let mut indices = Vec::new();
    for token in spec
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        if let Some((start, end)) = token.split_once('-') {
            let start = start
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid source index range: {token}"))?;
            let end = end
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid source index range: {token}"))?;
            if start > end {
                return Err(format!("range starts after it ends: {token}"));
            }
            let count = usize::try_from(end - start + 1).unwrap_or(usize::MAX);
            if indices.len().saturating_add(count) > MAX_COMMAND_EDIT_POINTS {
                return Err(format!(
                    "one command is limited to {MAX_COMMAND_EDIT_POINTS} point indices"
                ));
            }
            indices.extend(start..=end);
        } else {
            indices.push(
                token
                    .parse::<u64>()
                    .map_err(|_| format!("invalid source index: {token}"))?,
            );
        }
    }
    if indices.is_empty() {
        return Err("provide source indices such as 10,25-40".into());
    }
    if let Some(index) = indices.iter().copied().find(|&index| index >= point_count) {
        return Err(format!(
            "source index {index} is outside this cloud (0..{})",
            point_count.saturating_sub(1)
        ));
    }
    Ok(indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_individual_indices_and_inclusive_ranges() {
        assert_eq!(
            vec![1, 3, 4, 5, 8],
            parse_source_indices("1,3-5,8", 10).unwrap()
        );
    }

    #[test]
    fn rejects_reversed_and_out_of_bounds_ranges() {
        assert!(parse_source_indices("5-3", 10).is_err());
        assert!(parse_source_indices("9-10", 10).is_err());
    }

    fn sample_metadata(point_count: u64, min_z: f64, max_z: f64) -> ocs_pointcloud::CloudMetadata {
        ocs_pointcloud::CloudMetadata {
            point_count,
            version_major: 1,
            version_minor: 4,
            point_format: 6,
            compressed: false,
            bounds_min: [0.0, 0.0, min_z],
            bounds_max: [10.0, 10.0, max_z],
            scales: [0.001, 0.001, 0.001],
            offsets: [0.0; 3],
            system_identifier: String::new(),
            generating_software: String::new(),
            creation_date: None,
            file_source_id: 0,
            has_crs: false,
            crs: ocs_pointcloud::CrsInfo::default(),
            vlr_count: 0,
            evlr_count: 0,
        }
    }

    fn sample_point(source_index: u64, x: f64, classification: u8) -> ocs_pointcloud::SamplePoint {
        ocs_pointcloud::SamplePoint {
            source_index,
            position: [x, 0.0, 1.0],
            intensity: 100,
            classification,
            return_number: 1,
            number_of_returns: 1,
            scan_angle: 0.0,
            user_data: 0,
            point_source_id: 7,
            gps_time: None,
            color: None,
            nir: None,
            label: None,
            is_synthetic: false,
            is_key_point: false,
            is_withheld: false,
            is_overlap: false,
        }
    }

    fn attachment(id: &str, count: u64) -> PointCloudAttachment {
        PointCloudAttachment::new(
            id.to_string(),
            PathBuf::from(format!("C:\\clouds\\{id}.las")),
            PointSample {
                metadata: sample_metadata(count, 0.0, 10.0),
                points: (0..count)
                    .map(|index| sample_point(index, index as f64, 1))
                    .collect(),
                stride: 1,
                scanned_points: count,
            },
        )
    }

    #[test]
    fn dataset_generates_unique_source_ids() {
        let mut dataset = PointCloudDataset::default();
        dataset.sources.push(attachment("source-2", 1));
        dataset.sources.push(attachment("source-1", 1));
        assert_eq!("source-3", dataset.next_source_id());
        dataset.sources.remove(0);
        // Freed ids may be reused but never collide with live ones.
        let id = dataset.next_source_id();
        assert_ne!("source-1", id);
    }

    #[test]
    fn dataset_recognizes_an_already_attached_source_path() {
        let mut dataset = PointCloudDataset::default();
        dataset.sources.push(attachment("tile-a", 1));
        assert!(dataset.contains_source_path(&dataset.sources[0].source_path));
        assert!(!dataset.contains_source_path(std::path::Path::new("other.las")));
    }

    #[test]
    fn unreferenced_source_adopts_explicit_drawing_crs_without_moving_points() {
        let mut source = attachment("unreferenced", 2);
        let original: Vec<_> = source
            .sample
            .points
            .iter()
            .map(|point| point.position)
            .collect();
        let drawing_crs = ocs_pointcloud::CrsInfo {
            horizontal_epsg: Some(3857),
            ..Default::default()
        };

        source.align_sample_to_drawing(&drawing_crs).unwrap();

        assert!(source.crs_assumed_from_drawing);
        assert!(ocs_pointcloud::crs_equivalent(
            &source.source_crs,
            &drawing_crs
        ));
        assert_eq!(
            original,
            source
                .sample
                .points
                .iter()
                .map(|point| point.position)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn dataset_model_concatenates_every_source() {
        let mut dataset = PointCloudDataset::default();
        dataset.sources.push(attachment("a", 3));
        dataset.sources.push(attachment("b", 2));
        assert_eq!(5, dataset.display_model().points.len());
        assert_eq!(
            Some(([0.0, 0.0, 0.0], [10.0, 10.0, 10.0])),
            dataset.bounds()
        );
    }

    #[test]
    fn dataset_chunks_cover_the_point_stream_exactly() {
        let mut dataset = PointCloudDataset::default();
        dataset.sources.push(attachment("a", 3));
        dataset.sources.push(attachment("b", 2));
        // An edit bumps the first source's chunk generation only.
        dataset.sources[0]
            .edits
            .apply("class", [1_u64], PointPatch::classification(6));
        let model = dataset.display_model();
        // Non-tiled sources emit one whole-sample chunk each.
        assert_eq!(2, model.chunks.len());
        let covered: u32 = model.chunks.iter().map(|chunk| chunk.len).sum();
        assert_eq!(model.points.len() as u32, covered);
        assert_eq!(0, model.chunks[0].offset);
        assert_eq!(3, model.chunks[0].offset + model.chunks[0].len);
        assert_eq!(model.chunks[0].len, model.chunks[1].offset);
        assert_ne!(
            model.chunks[0].generation, model.chunks[1].generation,
            "the edited source's chunk generation must differ"
        );
        // Chunk keys stay stable across rebuilds while generations track edits.
        let first_keys: Vec<u64> = model.chunks.iter().map(|chunk| chunk.key).collect();
        let rebuilt = dataset.display_model();
        assert_eq!(
            first_keys,
            rebuilt.chunks.iter().map(|c| c.key).collect::<Vec<_>>()
        );
        dataset.sources[0].edits.undo();
        let bumped = dataset.display_model();
        assert_ne!(model.chunks[0].generation, bumped.chunks[0].generation);
        assert_eq!(model.chunks[1].generation, bumped.chunks[1].generation);
    }

    #[test]
    fn dataset_undo_tracks_only_the_last_edit_action() {
        let mut dataset = PointCloudDataset::default();
        dataset.sources.push(attachment("a", 4));
        dataset.sources.push(attachment("b", 4));
        // One edit action spanning both sources: class change on all points.
        let touched = vec!["a".to_string(), "b".to_string()];
        for id in &touched {
            let source = dataset.source_mut(id).expect("source");
            source
                .edits
                .apply("Assign class 2", 0..4, PointPatch::classification(2));
        }
        dataset.note_edit_sources(touched.clone());
        for id in &touched {
            assert_eq!(
                1,
                dataset
                    .source(id)
                    .expect("source")
                    .edits
                    .transaction_count()
            );
        }
        // Undo steps exactly the tracked sources.
        for id in &touched {
            assert!(dataset
                .source_mut(id)
                .expect("source")
                .edits
                .undo()
                .is_some());
        }
        for id in &touched {
            assert_eq!(0, dataset.source(id).expect("source").edits.len());
        }
    }

    #[test]
    fn folder_scan_finds_lidar_files_recursively_and_sorted() {
        let root = std::env::temp_dir().join(format!(
            "ocs-folder-scan-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let sub = root.join("tiles").join("north");
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(root.join("b.las"), b"las").expect("write");
        std::fs::write(root.join("ignore.txt"), b"no").expect("write");
        std::fs::write(root.join("a.LAZ"), b"laz").expect("write");
        std::fs::write(sub.join("c.las"), b"las").expect("write");
        let files = scan_lidar_folder(&root);
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            vec![
                "a.LAZ".to_string(),
                "b.las".to_string(),
                "c.las".to_string()
            ],
            names
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cache_candidates_fall_back_to_the_source_sidecar_directory() {
        let source = PathBuf::from("survey").join("tile.laz");
        let default = source.with_file_name("tile.laz.ocstiles");
        assert_eq!(
            vec![default.clone()],
            point_cloud_cache_candidates(&source, None)
        );
        assert_eq!(
            vec![default.clone()],
            point_cloud_cache_candidates(&source, Some(&default))
        );

        let persisted = PathBuf::from("drawing-cache").join("tile.ocstiles");
        assert_eq!(
            vec![persisted.clone(), default],
            point_cloud_cache_candidates(&source, Some(&persisted))
        );
    }

    #[test]
    fn index_batch_advances_past_failed_sources_without_retrying_them() {
        let mut sources = vec![attachment("a", 1), attachment("b", 1)];
        assert_eq!(Some(0), next_index_source_index(&sources));

        sources[0].index_error = Some("bad cache".to_string());
        assert_eq!(Some(1), next_index_source_index(&sources));

        sources[1].index_error = Some("bad source".to_string());
        assert_eq!(None, next_index_source_index(&sources));
    }

    fn lod_tile(level: u8, x: u32, point_count: u64) -> ocs_pointcloud::TileEntry {
        ocs_pointcloud::TileEntry {
            key: ocs_pointcloud::TileKey {
                level,
                x,
                y: 0,
                z: 0,
            },
            file_name: format!("l{level}-{x}.bin"),
            point_count,
            bounds_min: [x as f64 * 10.0, 0.0, 0.0],
            bounds_max: [x as f64 * 10.0 + 9.0, 9.0, 9.0],
        }
    }

    #[test]
    fn lod_never_loads_tiles_outside_the_view_frame() {
        let tiles = vec![
            lod_tile(0, 0, 20),
            lod_tile(1, 0, 60),
            lod_tile(1, 1, 60),
            lod_tile(1, 2, 60),
        ];
        let section = Some(([0.0, 0.0, f64::NEG_INFINITY], [40.0, 9.0, f64::INFINITY]));

        let selected = select_visible_lod_tiles(&tiles, 1, 100, section, |tile| tile.key.x == 1);

        assert_eq!(
            vec![ocs_pointcloud::TileKey {
                level: 1,
                x: 1,
                y: 0,
                z: 0,
            }],
            selected.iter().map(|tile| tile.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lod_uses_full_density_close_and_coarser_density_when_wide() {
        let tiles = vec![lod_tile(0, 0, 40), lod_tile(1, 0, 60), lod_tile(1, 1, 60)];
        let close = select_visible_lod_tiles(&tiles, 1, 100, None, |tile| tile.key.x == 0);
        assert_eq!(
            1, close[0].key.level,
            "close view should use leaf/full density"
        );

        let wide = select_visible_lod_tiles(&tiles, 1, 100, None, |_| true);
        assert_eq!(
            vec![0],
            wide.iter().map(|tile| tile.key.level).collect::<Vec<_>>(),
            "wide view should fall back to the coarser fitting LOD"
        );
    }
}

/// Delivers `message` on a fresh event-loop turn: a worker thread wakes the
/// executor ~1 ms later, so a follow-on action never re-enters the current
/// dispatch on the same stack. Returning chained `Task::done` continuations
/// from inside an update nests winit dispatch frames and, repeated across a
/// folder attach, overflows the main thread.
fn deferred_message(message: Message) -> Task<Message> {
    background_task(
        move || std::thread::sleep(std::time::Duration::from_millis(1)),
        move |_| message,
    )
}
