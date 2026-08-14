//! Native LAS/LAZ attachment and classification workflow.
//!
//! A tab owns a bounded display sample and sparse edits. The original source
//! remains authoritative until the user explicitly exports a new file.

use super::{Message, OpenCADStudio};
use crate::scene::{PointCloudModel, PointCloudPoint};
use iced::Task;
use ocs_pointcloud::{
    classification_statistics, parse_ptc, select_brush, select_nearest, select_polygon,
    sidecar_path_for_drawing, write_ptc, AttachmentState, ClassTable, ColorMode, DisplaySettings,
    EditStore, ExportStats, PointFilter, PointPatch, PointSample, SampleOptions, SelectionSet,
    SidecarStore, TileCacheManifest, TileCacheOptions,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

const DISPLAY_POINT_LIMIT: usize = 1_000_000;
const DISPLAY_READ_CHUNK: usize = 65_536;
const MAX_COMMAND_EDIT_POINTS: usize = 5_000_000;

/// Interactive bridge used by imported TerraScan-style `classify using brush
/// <class>` function keys. The current production bridge gathers a world-space
/// spherical brush; direct screen-space brush strokes are a later viewport tool.
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
            "Classify using brush {}  Enter centerX centerY centerZ radius:",
            self.classification
        )
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn on_text_input(&mut self, text: &str) -> Option<crate::command::CmdResult> {
        let values = text.trim();
        (!values.is_empty()).then(|| {
            crate::command::CmdResult::Dispatch(format!(
                "POINTCLOUDBRUSHCLASSIFY {} {values}",
                self.classification
            ))
        })
    }

    fn on_point(&mut self, _point: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

#[derive(Debug)]
struct PointCloudJobProgress {
    completed: AtomicU64,
    total: u64,
    cancel: AtomicBool,
}

impl PointCloudJobProgress {
    fn new(total: u64) -> Self {
        Self {
            completed: AtomicU64::new(0),
            total,
            cancel: AtomicBool::new(false),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PointCloudAttachment {
    pub(super) source_path: PathBuf,
    pub(super) sample: PointSample,
    pub(super) edits: EditStore,
    pub(super) display: DisplaySettings,
    pub(super) classes: ClassTable,
    pub(super) selection_sets: Vec<SelectionSet>,
    pub(super) selection_filter: PointFilter,
    pub(super) cache_path: Option<PathBuf>,
    pub(super) cache_manifest: Option<TileCacheManifest>,
    index_cancel: Option<Arc<AtomicBool>>,
    export_job: Option<Arc<PointCloudJobProgress>>,
    display_generation: u64,
}

impl PointCloudAttachment {
    pub(super) fn new(source_path: PathBuf, sample: PointSample) -> Self {
        Self {
            source_path,
            sample,
            edits: EditStore::default(),
            display: DisplaySettings::default(),
            classes: ClassTable::default(),
            selection_sets: Vec::new(),
            selection_filter: PointFilter::default(),
            cache_path: None,
            cache_manifest: None,
            index_cancel: None,
            export_job: None,
            display_generation: 1,
        }
    }

    pub(super) fn display_model(&self) -> PointCloudModel {
        let active_selection = self
            .selection_sets
            .iter()
            .find(|selection| selection.name == "active");
        let intensity_range = self.display.intensity_range.unwrap_or_else(|| {
            self.sample
                .points
                .iter()
                .fold([u16::MAX, 0], |range, point| {
                    [range[0].min(point.intensity), range[1].max(point.intensity)]
                })
        });
        let elevation_range = self.display.elevation_range.unwrap_or([
            self.sample.metadata.bounds_min[2],
            self.sample.metadata.bounds_max[2],
        ]);
        let points = self
            .sample
            .points
            .iter()
            .filter_map(|source| {
                let point = self
                    .edits
                    .patch_for(source.source_index)
                    .map_or_else(|| source.clone(), |patch| source.clone().with_patch(patch));
                let class_visible = self
                    .classes
                    .classes
                    .get(&point.classification)
                    .is_none_or(|definition| definition.visible);
                if !class_visible || self.display.hidden_classes.contains(&point.classification) {
                    return None;
                }
                let mut color = point_color(
                    &point,
                    self.display.color_mode,
                    &self.classes,
                    intensity_range,
                    elevation_range,
                );
                if active_selection
                    .is_some_and(|selection| selection.contains(point.source_index))
                {
                    color = [1.0, 0.82, 0.05, 1.0];
                }
                Some(PointCloudPoint {
                    position: point.position,
                    color,
                })
            })
            .collect();
        PointCloudModel {
            points: Arc::new(points),
            point_size_px: self.display.point_size_px,
            generation: self.display_generation,
        }
    }

    fn mark_display_changed(&mut self) {
        self.display_generation = self.display_generation.wrapping_add(1).max(1);
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

    fn manager_data(&self) -> crate::ui::window::point_cloud_manager::PointCloudManagerData {
        let active_selection = self
            .selection_sets
            .iter()
            .find(|selection| selection.name == "active")
            .map_or(0, SelectionSet::len);
        let color_mode = match self.display.color_mode {
            ColorMode::Classification => "Classification",
            ColorMode::Rgb => "RGB",
            ColorMode::Intensity => "Intensity",
            ColorMode::Elevation => "Elevation",
            ColorMode::ReturnNumber => "Return number",
            ColorMode::PointSource => "Point source",
        };
        let export_progress = self.export_job.as_ref().map(|job| {
            (
                job.completed.load(Ordering::Relaxed),
                job.total,
            )
        });
        crate::ui::window::point_cloud_manager::PointCloudManagerData {
            attached: true,
            source: self.source_path.display().to_string(),
            source_points: self.sample.metadata.point_count,
            displayed_points: self.sample.points.len(),
            sample_label: match self.sample.stride {
                0 => "tiled LOD".to_string(),
                1 => "full cloud".to_string(),
                stride => format!("1-in-{stride} sample"),
            },
            pending_edits: self.edits.len(),
            transactions: self.edits.transaction_count(),
            active_selection,
            selection_sets: self.selection_sets.len(),
            class_count: self.classes.classes.len(),
            color_mode: color_mode.to_string(),
            point_size_px: self.display.point_size_px,
            crs_declared: self.sample.metadata.has_crs,
            indexed: self.cache_manifest.is_some(),
            index_running: self.index_cancel.is_some(),
            cache: self
                .cache_path
                .as_ref()
                .map_or_else(|| "not available".to_string(), |path| path.display().to_string()),
            export_progress,
            sidecar_available: false,
            selection_filter: describe_filter(&self.selection_filter),
        }
    }
}

impl OpenCADStudio {
    pub(super) fn point_cloud_manager_data(
        &self,
        tab_index: usize,
    ) -> crate::ui::window::point_cloud_manager::PointCloudManagerData {
        let mut data = self.tabs
            .get(tab_index)
            .and_then(|tab| tab.point_cloud.as_ref())
            .map_or_else(Default::default, PointCloudAttachment::manager_data);
        data.sidecar_available = self
            .tabs
            .get(tab_index)
            .and_then(|tab| tab.current_path.as_ref())
            .is_some_and(|drawing| sidecar_path_for_drawing(drawing).exists());
        data
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
        let source = SidecarStore::open(&sidecar_path)
            .and_then(|store| store.load_attachment("primary"))
            .map_err(|error| error.to_string())
            .and_then(|state| {
                state
                    .and_then(|state| state.resolve_source(&drawing_path))
                    .ok_or_else(|| {
                        "sidecar has no source whose path and fingerprint can be validated"
                            .to_string()
                    })
            });
        match source {
            Ok(source) => {
                self.command_line.push_output(
                    format!(
                        "POINTCLOUDRESTORE: repaired and validated source path \"{}\".",
                        source.display()
                    )
                    .as_str(),
                );
                self.start_point_cloud_load(source)
            }
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDRESTORE: {error}.").as_str());
                Task::none()
            }
        }
    }

    pub(super) fn start_point_cloud_load(&mut self, path: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        self.command_line.push_info(
            format!(
                "POINTCLOUDATTACH: reading bounded display sample from \"{}\"...",
                path.display()
            )
            .as_str(),
        );
        let worker_path = path.clone();
        background_task(
            move || {
                ocs_pointcloud::sample(
                    &worker_path,
                    SampleOptions {
                        max_points: DISPLAY_POINT_LIMIT,
                        chunk_size: DISPLAY_READ_CHUNK,
                    },
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudLoaded(tab_id, path, result),
        )
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
            return Task::none();
        };
        let sample = match result {
            Ok(sample) => sample,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDATTACH: {error}").as_str());
                return Task::none();
            }
        };

        let mut attachment = PointCloudAttachment::new(path.clone(), sample);
        let mut restored_sidecar = false;
        if let Some(drawing_path) = self.tabs[tab_index].current_path.as_ref() {
            let sidecar_path = sidecar_path_for_drawing(drawing_path);
            if sidecar_path.exists() {
                match SidecarStore::open(&sidecar_path)
                    .and_then(|store| store.load_attachment("primary"))
                {
                    Ok(Some(mut state)) if state.source_fingerprint.matches_path(&path) => {
                        state.edits.normalize_after_load();
                        attachment.edits = state.edits;
                        attachment.display = state.display;
                        attachment.classes = state.classes;
                        attachment.selection_sets = state.selection_sets;
                        attachment.selection_filter = state.selection_filter;
                        attachment.cache_path = state
                            .cache_relative
                            .and_then(|relative| drawing_path.parent().map(|parent| parent.join(relative)))
                            .filter(|candidate| candidate.exists());
                        attachment.mark_display_changed();
                        restored_sidecar = true;
                    }
                    Ok(_) => {}
                    Err(error) => self.command_line.push_error(
                        format!("POINTCLOUDATTACH: could not read sidecar: {error}").as_str(),
                    ),
                }
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
        let model = attachment.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        self.tabs[tab_index].point_cloud = Some(attachment);
        self.tabs[tab_index]
            .scene
            .fit_external_bounds(bounds_min, bounds_max);

        self.command_line.push_output(
            format!(
                "POINTCLOUDATTACH: {} points ({compressed}, LAS {version}, format {format}); displaying {sampled} sampled points. Bounds [{:.3}, {:.3}, {:.3}] to [{:.3}, {:.3}, {:.3}].",
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
        self.persist_point_cloud(tab_index, "attach", "attached point cloud");
        Task::none()
    }

    pub(super) fn point_cloud_info(&mut self, tab_index: usize) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_info("POINTCLOUDINFO: no LAS/LAZ cloud is attached.");
            return;
        };
        let metadata = &cloud.sample.metadata;
        self.command_line.push_output(
            format!(
                "POINTCLOUDINFO: \"{}\"; {} source points; {} displayed (stride {}); {} pending classification edits; CRS metadata: {}; VLRs: {}, EVLRs: {}.",
                cloud.source_path.display(),
                metadata.point_count,
                cloud.sample.points.len(),
                cloud.sample.stride,
                cloud.edits.len(),
                if metadata.has_crs { "present" } else { "not declared" },
                metadata.vlr_count,
                metadata.evlr_count,
            )
            .as_str(),
        );
    }

    pub(super) fn reclassify_point_cloud(
        &mut self,
        tab_index: usize,
        classification: u8,
        index_spec: &str,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDCLASSIFY: attach a LAS/LAZ cloud first.");
            return;
        };
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
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        self.command_line.push_output(
            format!(
                "POINTCLOUDCLASSIFY: queued {changed} point(s) as class {classification}; export to create a revised LAS/LAZ."
            )
            .as_str(),
        );
        self.persist_point_cloud(
            tab_index,
            "classification",
            &format!("assigned class {classification} to {changed} points"),
        );
    }

    pub(super) fn undo_point_cloud_edit(&mut self, tab_index: usize) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_info("POINTCLOUDUNDO: no LAS/LAZ cloud is attached.");
            return;
        };
        if cloud.edits.undo().is_some() {
            cloud.mark_display_changed();
            let model = cloud.display_model();
            self.tabs[tab_index].scene.set_point_cloud(model);
            self.command_line
                .push_output("POINTCLOUDUNDO: restored the previous classification edit state.");
            self.persist_point_cloud(tab_index, "undo", "undid point-cloud transaction");
        } else {
            self.command_line
                .push_info("POINTCLOUDUNDO: no point-cloud edit to undo.");
        }
    }

    pub(super) fn detach_point_cloud(&mut self, tab_index: usize) {
        if self.tabs[tab_index].point_cloud.take().is_some() {
            self.tabs[tab_index]
                .scene
                .set_point_cloud(PointCloudModel::default());
            self.command_line.push_output(
                "POINTCLOUDDETACH: detached the session cloud; the source file was unchanged.",
            );
        } else {
            self.command_line
                .push_info("POINTCLOUDDETACH: no LAS/LAZ cloud is attached.");
        }
    }

    pub(super) fn start_point_cloud_index(&mut self, tab_index: usize) -> Task<Message> {
        let tab_id = self.tabs[tab_index].id;
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDINDEX: attach a LAS/LAZ cloud first.");
            return Task::none();
        };
        if cloud.index_cancel.is_some() {
            self.command_line
                .push_info("POINTCLOUDINDEX: an index build is already running.");
            return Task::none();
        }
        let source = cloud.source_path.clone();
        let cache_path = cache_path_for_source(&source);
        if cache_path.exists() {
            let result = TileCacheManifest::open(&cache_path)
                .and_then(|manifest| {
                    manifest.validate_source(&source)?;
                    Ok(manifest)
                })
                .map_err(|error| error.to_string());
            return Task::done(Message::PointCloudIndexed(tab_id, cache_path, result));
        }

        let cancel = Arc::new(AtomicBool::new(false));
        cloud.index_cancel = Some(Arc::clone(&cancel));
        self.command_line.push_info(
            format!(
                "POINTCLOUDINDEX: building disk-backed LOD tiles at \"{}\"; use POINTCLOUDINDEXCANCEL to cancel.",
                cache_path.display()
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
                    |_| !cancel.load(Ordering::Relaxed),
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudIndexed(tab_id, cache_path, result),
        )
    }

    pub(super) fn cancel_point_cloud_index(&mut self, tab_index: usize) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_info("POINTCLOUDINDEXCANCEL: no point cloud is attached.");
            return;
        };
        if let Some(cancel) = cloud.index_cancel.as_ref() {
            cancel.store(true, Ordering::Relaxed);
            self.command_line
                .push_output("POINTCLOUDINDEXCANCEL: cancellation requested.");
        } else {
            self.command_line
                .push_info("POINTCLOUDINDEXCANCEL: no index build is running.");
        }
    }

    pub(super) fn finish_point_cloud_index(
        &mut self,
        tab_id: u64,
        cache_path: PathBuf,
        result: Result<TileCacheManifest, String>,
    ) {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            return;
        };
        cloud.index_cancel = None;
        let manifest = match result {
            Ok(manifest) => manifest,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDINDEX: {error}").as_str());
                return;
            }
        };
        let selected = manifest.select_tiles(
            manifest.source_metadata.bounds_min,
            manifest.source_metadata.bounds_max,
            cloud.display.point_budget as u64,
        );
        let mut points = Vec::new();
        for tile in &selected {
            match ocs_pointcloud::read_tile(&cache_path, tile) {
                Ok(mut tile_points) => points.append(&mut tile_points),
                Err(error) => {
                    self.command_line
                        .push_error(format!("POINTCLOUDINDEX: {error}").as_str());
                    return;
                }
            }
        }
        let displayed = points.len();
        cloud.sample.points = points;
        cloud.sample.stride = 0;
        cloud.cache_path = Some(cache_path.clone());
        cloud.cache_manifest = Some(manifest.clone());
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index]
            .scene
            .set_point_cloud(model);
        self.command_line.push_output(
            format!(
                "POINTCLOUDINDEX: {} tiles indexed through level {}; loaded {} budgeted LOD point(s) into the GPU view.",
                manifest.tiles.len(), manifest.leaf_level, displayed
            )
            .as_str(),
        );
        self.persist_point_cloud(tab_index, "index", "built or opened tiled LOD cache");
    }

    pub(super) fn set_point_cloud_color_mode(&mut self, tab_index: usize, mode: ColorMode) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDCOLOR: attach a LAS/LAZ cloud first.");
            return;
        };
        cloud.display.color_mode = mode;
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        self.command_line
            .push_output(format!("POINTCLOUDCOLOR: mode set to {mode:?}.").as_str());
        self.persist_point_cloud(tab_index, "display", &format!("color mode {mode:?}"));
    }

    pub(super) fn set_point_cloud_point_size(&mut self, tab_index: usize, size: f32) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDPOINTSIZE: attach a LAS/LAZ cloud first.");
            return;
        };
        if !size.is_finite() || !(1.0..=32.0).contains(&size) {
            self.command_line
                .push_error("POINTCLOUDPOINTSIZE: size must be between 1 and 32 pixels.");
            return;
        }
        cloud.display.point_size_px = size;
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index]
            .scene
            .set_point_cloud(model);
        self.command_line.push_output(
            format!("POINTCLOUDPOINTSIZE: fixed screen size set to {size:.1} px.").as_str(),
        );
        self.persist_point_cloud(tab_index, "display", &format!("point size {size:.1} px"));
    }

    pub(super) fn set_point_cloud_class_visible(
        &mut self,
        tab_index: usize,
        classification: u8,
        visible: bool,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDCLASSVISIBLE: attach a LAS/LAZ cloud first.");
            return;
        };
        if visible {
            cloud.display.hidden_classes.remove(&classification);
        } else {
            cloud.display.hidden_classes.insert(classification);
        }
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index]
            .scene
            .set_point_cloud(model);
        self.command_line.push_output(
            format!(
                "POINTCLOUDCLASSVISIBLE: class {classification} {}.",
                if visible { "shown" } else { "hidden" }
            )
            .as_str(),
        );
        self.persist_point_cloud(tab_index, "display", "changed class visibility");
    }

    pub(super) fn point_cloud_statistics(&mut self, tab_index: usize) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDSTATS: attach a LAS/LAZ cloud first.");
            return;
        };
        let points = cloud.sample.points.iter().cloned().map(|point| {
            cloud
                .edits
                .patch_for(point.source_index)
                .map_or(point.clone(), |patch| point.with_patch(patch))
        });
        let stats = classification_statistics(points);
        let summary = stats
            .iter()
            .map(|(class, stats)| format!("{class}:{}", stats.total))
            .collect::<Vec<_>>()
            .join(", ");
        let qualifier = if cloud.sample.stride == 1 {
            "full cloud"
        } else {
            "display sample"
        };
        self.command_line.push_output(
            format!("POINTCLOUDSTATS ({qualifier}): {summary}.").as_str(),
        );
    }

    pub(super) fn set_point_cloud_selection(
        &mut self,
        tab_index: usize,
        selection: SelectionSet,
    ) {
        let count = selection.len();
        let name = selection.name.clone();
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDSELECT: attach a LAS/LAZ cloud first.");
            return;
        };
        if let Some(existing) = cloud
            .selection_sets
            .iter_mut()
            .find(|candidate| candidate.name == name)
        {
            *existing = selection;
        } else {
            cloud.selection_sets.push(selection);
        }
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index].scene.set_point_cloud(model);
        self.command_line.push_output(
            format!("POINTCLOUDSELECT: selection set \"{name}\" contains {count} point(s).").as_str(),
        );
        self.persist_point_cloud(tab_index, "selection", &format!("updated {name}: {count} points"));
    }

    pub(super) fn point_cloud_select_box(
        &mut self,
        tab_index: usize,
        min: [f64; 3],
        max: [f64; 3],
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDSELECTBOX: attach a LAS/LAZ cloud first.");
            return;
        };
        let polygon = [
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ];
        let selection = select_polygon(
            &cloud.sample.points,
            &polygon,
            Some([min[2], max[2]]),
            &cloud.selection_filter,
        );
        let selection = SelectionSet::from_indices("active", selection.iter());
        self.set_point_cloud_selection(tab_index, selection);
    }

    pub(super) fn point_cloud_select_brush(
        &mut self,
        tab_index: usize,
        center: [f64; 3],
        radius: f64,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDSELECTBRUSH: attach a LAS/LAZ cloud first.");
            return;
        };
        let selection = select_brush(
            &cloud.sample.points,
            center,
            radius,
            &cloud.selection_filter,
        );
        let selection = SelectionSet::from_indices("active", selection.iter());
        self.set_point_cloud_selection(tab_index, selection);
    }

    pub(super) fn point_cloud_select_nearest(
        &mut self,
        tab_index: usize,
        position: [f64; 3],
        radius: f64,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDSELECTPOINT: attach a LAS/LAZ cloud first.");
            return;
        };
        let selection = select_nearest(
            &cloud.sample.points,
            position,
            radius,
            &cloud.selection_filter,
        );
        let selection = SelectionSet::from_indices("active", selection.iter());
        self.set_point_cloud_selection(tab_index, selection);
    }

    pub(super) fn point_cloud_select_elevation_slice(
        &mut self,
        tab_index: usize,
        low: f64,
        high: f64,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDSELECTSLICE: attach a LAS/LAZ cloud first.");
            return;
        };
        let bounds = [low.min(high), low.max(high)];
        let selection = SelectionSet::from_indices(
            "active",
            cloud.sample.points.iter().filter_map(|point| {
                (point.position[2] >= bounds[0]
                    && point.position[2] <= bounds[1]
                    && cloud.selection_filter.matches(point))
                .then_some(point.source_index)
            }),
        );
        self.set_point_cloud_selection(tab_index, selection);
    }

    pub(super) fn set_point_cloud_selection_filter(
        &mut self,
        tab_index: usize,
        filter: PointFilter,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDSELECTFILTER: attach a LAS/LAZ cloud first.");
            return;
        };
        cloud.selection_filter = filter;
        let description = describe_filter(&cloud.selection_filter);
        self.command_line
            .push_output(format!("POINTCLOUDSELECTFILTER: {description}.").as_str());
        self.persist_point_cloud(tab_index, "selection_filter", &description);
    }

    pub(super) fn patch_point_cloud_selection(
        &mut self,
        tab_index: usize,
        label: &str,
        patch: PointPatch,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDEDITSELECTION: attach a LAS/LAZ cloud first.");
            return;
        };
        let Some(selection) = cloud
            .selection_sets
            .iter()
            .find(|selection| selection.name == "active")
            .cloned()
        else {
            self.command_line
                .push_error("POINTCLOUDEDITSELECTION: create an active selection first.");
            return;
        };
        let changed = cloud.edits.apply(label, selection.iter(), patch);
        cloud.mark_display_changed();
        let model = cloud.display_model();
        self.tabs[tab_index]
            .scene
            .set_point_cloud(model);
        self.command_line.push_output(
            format!("POINTCLOUDEDITSELECTION: {label}; {changed} point(s) queued.").as_str(),
        );
        self.persist_point_cloud(tab_index, "edit", &format!("{label}: {changed} points"));
    }

    pub(super) fn import_point_cloud_ptc(&mut self, tab_index: usize, path: PathBuf) {
        let result = std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse_ptc(&text).map_err(|error| error.to_string()));
        match result {
            Ok(classes) => {
                let count = classes.classes.len();
                let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
                    self.command_line
                        .push_error("POINTCLOUDPTCIMPORT: attach a LAS/LAZ cloud first.");
                    return;
                };
                cloud.classes = classes;
                cloud.mark_display_changed();
                let model = cloud.display_model();
                self.tabs[tab_index]
                    .scene
                    .set_point_cloud(model);
                self.command_line.push_output(
                    format!("POINTCLOUDPTCIMPORT: loaded {count} class definitions from \"{}\".", path.display()).as_str(),
                );
                self.persist_point_cloud(tab_index, "classes", "imported PTC class table");
            }
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDPTCIMPORT: {error}").as_str()),
        }
    }

    pub(super) fn export_point_cloud_ptc(&mut self, tab_index: usize, path: PathBuf) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDPTCEXPORT: attach a LAS/LAZ cloud first.");
            return;
        };
        match std::fs::write(&path, write_ptc(&cloud.classes)) {
            Ok(()) => self.command_line.push_output(
                format!("POINTCLOUDPTCEXPORT: wrote \"{}\".", path.display()).as_str(),
            ),
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDPTCEXPORT: {error}").as_str()),
        }
    }

    fn persist_point_cloud(&mut self, tab_index: usize, action: &str, detail: &str) {
        let Some(drawing_path) = self.tabs[tab_index].current_path.clone() else {
            return;
        };
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            return;
        };
        let state = AttachmentState::new("primary", &drawing_path, &cloud.source_path).map(
            |mut state| {
                state.display = cloud.display.clone();
                state.classes = cloud.classes.clone();
                state.edits = cloud.edits.clone();
                state.selection_sets = cloud.selection_sets.clone();
                state.selection_filter = cloud.selection_filter.clone();
                state.cache_relative = cloud.cache_path.as_ref().and_then(|cache_path| {
                    drawing_path
                        .parent()
                        .and_then(|parent| cache_path.strip_prefix(parent).ok())
                        .map(std::path::Path::to_path_buf)
                });
                state
            },
        );
        let result = state
            .map_err(|error| error.to_string())
            .and_then(|state| {
                let mut store = SidecarStore::open(sidecar_path_for_drawing(&drawing_path))
                    .map_err(|error| error.to_string())?;
                store
                    .save_attachment(&state)
                    .and_then(|_| store.append_audit("primary", action, detail))
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.command_line
                .push_error(format!("POINTCLOUDSIDECAR: {error}").as_str());
        }
    }

    pub(super) fn start_point_cloud_export(&mut self, output: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        let Some(cloud) = self.tabs[self.active_tab].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDEXPORT: attach a LAS/LAZ cloud first.");
            return Task::none();
        };
        if cloud.export_job.is_some() {
            self.command_line
                .push_error("POINTCLOUDEXPORT: an export is already running.");
            return Task::none();
        }
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

    pub(super) fn finish_point_cloud_export(
        &mut self,
        tab_id: u64,
        output: PathBuf,
        result: Result<ExportStats, String>,
    ) {
        let tab_index = self.tabs.iter().position(|tab| tab.id == tab_id);
        if let Some(tab_index) = tab_index {
            if let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() {
                cloud.export_job = None;
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
                    self.persist_point_cloud(tab_index, "export", &detail);
                }
            }
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDEXPORT: {error}").as_str());
                if let Some(tab_index) = tab_index {
                    self.persist_point_cloud(tab_index, "export_failed", &error);
                }
            }
        }
    }

    pub(super) fn point_cloud_export_status(&mut self, tab_index: usize) {
        let job = self.tabs[tab_index]
            .point_cloud
            .as_ref()
            .and_then(|cloud| cloud.export_job.as_ref());
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
            .as_ref()
            .and_then(|cloud| cloud.export_job.as_ref());
        if let Some(job) = job {
            job.cancel.store(true, Ordering::Relaxed);
            self.command_line
                .push_output("POINTCLOUDEXPORTCANCEL: cancellation requested.");
        } else {
            self.command_line
                .push_info("POINTCLOUDEXPORTCANCEL: no export is running.");
        }
    }
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

fn point_color(
    point: &ocs_pointcloud::SamplePoint,
    mode: ColorMode,
    classes: &ClassTable,
    intensity: [u16; 2],
    elevation: [f64; 2],
) -> [f32; 4] {
    match mode {
        ColorMode::Classification => rgb8(classes.color(point.classification)),
        ColorMode::Rgb => point.color.map_or_else(
            || rgb8(classes.color(point.classification)),
            |color| {
                [
                    color[0] as f32 / 65_535.0,
                    color[1] as f32 / 65_535.0,
                    color[2] as f32 / 65_535.0,
                    1.0,
                ]
            },
        ),
        ColorMode::Intensity => {
            let value = normalize(
                point.intensity as f64,
                intensity[0] as f64,
                intensity[1] as f64,
            );
            [value, value, value, 1.0]
        }
        ColorMode::Elevation => gradient(normalize(point.position[2], elevation[0], elevation[1])),
        ColorMode::ReturnNumber => categorical(point.return_number as u32),
        ColorMode::PointSource => categorical(point.point_source_id as u32),
    }
}

fn rgb8(color: [u8; 3]) -> [f32; 4] {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        1.0,
    ]
}

fn normalize(value: f64, low: f64, high: f64) -> f32 {
    if !value.is_finite() || high <= low {
        0.5
    } else {
        ((value - low) / (high - low)).clamp(0.0, 1.0) as f32
    }
}

fn gradient(value: f32) -> [f32; 4] {
    let red = (value * 1.5).clamp(0.0, 1.0);
    let blue = ((1.0 - value) * 1.5).clamp(0.0, 1.0);
    let green = (1.0 - (value - 0.5).abs() * 2.0).clamp(0.0, 1.0);
    [red, green, blue, 1.0]
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

fn parse_source_indices(spec: &str, point_count: u64) -> Result<Vec<u64>, String> {
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
}
