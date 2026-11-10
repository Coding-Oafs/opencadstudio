//! `TERRAIN` command: global bare-earth elevation from AWS Terrain Tiles.
//!
//! Fetches Mapzen "terrarium" PNG elevation tiles from the public AWS Open
//! Data bucket (no account or API key; SRTM/GMTED-derived, roughly 30 m
//! bare-earth, heights in meters relative to mean sea level) covering a
//! drawing area, resamples them into a regular elevation grid in the drawing
//! CRS, and keeps the result as a session [`TerrainDataset`] plus a committed
//! 3-D `Mesh` entity, so 3-D modelling, draping and elevation queries work
//! against real-world terrain.
//!
//! Tile payload: each 256×256 PNG encodes meters as
//! `elevation = R*256 + G + B/256 − 32768`. Data © Mapzen/AWS Open Data.

use super::point_cloud::{entity_drape_source, DrapeSourcePath};
use super::{Message, OpenCADStudio};
use crate::command::CadCommand;
use crate::scene::basemap::{self, Tile};
use iced::Task;
use ocs_pointcloud::SurfaceSampler;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

/// Side length of one terrarium tile image in pixels.
const TILE_PX: usize = 256;

/// Upper bound on a single fetch (same safety envelope as the basemap
/// underlay: a world envelope at zoom 15 would otherwise request millions of
/// tiles).
const MAX_TERRAIN_TILES: u64 = 256;

/// Default slippy zoom when neither a zoom nor a cell size was requested
/// (≈38 m/pixel at the equator, ≈27 m at 45° latitude — matching the source
/// data resolution without oversampling).
const DEFAULT_ZOOM: u32 = 12;

/// Highest terrarium zoom with global coverage worth fetching.
const MAX_ZOOM: u32 = 15;

/// Cap on the resampled grid so a huge request degrades by coarsening instead
/// of exhausting memory (16 M cells ≈ 128 MiB of f64 heights).
const MAX_GRID_CELLS: usize = 16_000_000;

/// Hard ceiling for mesh draping onto the terrain, matching the point-cloud
/// drape's per-mesh vertex limit.
const MAX_DRAPED_MESH_VERTICES: usize = 1_000_000;

/// Web-Mercator half-extent in meters (EPSG:3857), matching
/// `scene::basemap`'s private constant.
const MERCATOR_HALF: f64 = 20_037_508.342_789_244;

/// WGS-84 semimajor axis: the spherical mercator formula's linear radius
/// (`MERCATOR_HALF` = π × this).
const MERCATOR_EARTH_RADIUS: f64 = 6_378_137.0;

/// The public AWS Terrain Tiles terrarium endpoint.
const TERRARIUM_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium";

static NEXT_TERRAIN_JOB: AtomicU64 = AtomicU64::new(1);

/// One cancellable terrain fetch. Progress atomics drive `TERRAIN STATUS`
/// without sending a UI message per tile, mirroring `BasemapJob`.
#[derive(Clone)]
pub(crate) struct TerrainJob {
    pub id: u64,
    pub tab_id: u64,
    pub total: usize,
    pub completed: Arc<AtomicUsize>,
    pub failed: Arc<AtomicUsize>,
    pub cancel: Arc<AtomicBool>,
}

impl TerrainJob {
    fn new(tab_id: u64, total: usize) -> Self {
        Self {
            id: NEXT_TERRAIN_JOB.fetch_add(1, Ordering::Relaxed),
            tab_id,
            total,
            completed: Arc::new(AtomicUsize::new(0)),
            failed: Arc::new(AtomicUsize::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn snapshot(&self) -> (usize, usize, usize) {
        (
            self.completed.load(Ordering::Relaxed).min(self.total),
            self.total,
            self.failed.load(Ordering::Relaxed),
        )
    }
}

/// Persistent request settings for the next terrain fetch. Session-only: a
/// terrain model describes a specific site, unlike the app-wide basemap
/// underlay, so there is nothing sensible to restore into the next drawing.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct TerrainRequest {
    /// Drawing-coordinate envelope `[min_x, min_y, max_x, max_y]` for the next
    /// fetch. `None` falls back to the drawing's basemap bounds, then to the
    /// model-space extents.
    pub bounds: Option<[f64; 4]>,
    /// Explicit terrarium zoom (0-15) requested through `TERRAIN ZOOM`.
    pub zoom: Option<u32>,
    /// Approximate grid cell size in drawing units requested through
    /// `TERRAIN RES`; converted to a zoom before fetching.
    pub cell_size: Option<f64>,
    /// Vertex budget for the generated terrain mesh (`TERRAIN MESH`).
    pub mesh_max_vertices: usize,
}

impl Default for TerrainRequest {
    fn default() -> Self {
        Self {
            bounds: None,
            zoom: None,
            cell_size: None,
            mesh_max_vertices: 400_000,
        }
    }
}

/// The tab's resident fetched elevation model (session-only, like the
/// point-cloud dataset). Elevations and XY share the drawing's coordinate
/// units, so draping and mesh generation never rescale twice.
#[derive(Clone, Debug)]
pub struct TerrainDataset {
    pub surface: ocs_pointcloud::RasterSurface,
    pub zoom: u32,
    pub tiles: usize,
    pub failed_tiles: usize,
    /// Human label of the CRS the grid was resampled into.
    pub crs_label: String,
    /// Unit label for elevations (drawing units; meters for degree-based CRSs).
    pub elevation_unit: String,
    pub source: String,
    pub cache_key: Option<String>,
}

impl TerrainDataset {
    /// Lowest and highest valid elevation in drawing units.
    pub fn elevation_range(&self) -> Option<(f64, f64)> {
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut found = false;
        for value in &self.surface.values {
            if *value == self.surface.no_data || !value.is_finite() {
                continue;
            }
            found = true;
            min = min.min(*value);
            max = max.max(*value);
        }
        found.then_some((min, max))
    }
}

/// Completion of a background terrain fetch, delivered as
/// `Message::TerrainLoaded`.
#[derive(Debug, Clone)]
pub struct TerrainLoaded {
    pub job_id: u64,
    pub tab_id: u64,
    pub requested: usize,
    pub failed: usize,
    pub dataset: Option<TerrainDataset>,
    pub error: Option<String>,
}

/// Decode one terrarium pixel (its RGBA sample triple) into meters.
fn terrarium_elevation(red: u8, green: u8, blue: u8) -> f64 {
    f64::from(red) * 256.0 + f64::from(green) + f64::from(blue) / 256.0 - 32768.0
}

/// Decode a terrarium RGBA8 image into per-pixel elevations in meters,
/// row-major with row 0 at the tile's north edge.
fn terrarium_heights(pixels: &[u8], width: usize, height: usize) -> Vec<f32> {
    pixels
        .chunks_exact(4)
        .take(width.saturating_mul(height))
        .map(|px| terrarium_elevation(px[0], px[1], px[2]) as f32)
        .collect()
}

fn terrarium_tile_url(z: u32, x: u32, y: u32) -> String {
    format!("{TERRARIUM_URL}/{z}/{x}/{y}.png")
}

/// Latitude of a Web-Mercator Y coordinate (the inverse of the spherical
/// mercator formula behind the XYZ tile scheme).
fn mercator_latitude(merc_y: f64) -> f64 {
    (2.0 * (merc_y / MERCATOR_EARTH_RADIUS).exp().atan() - std::f64::consts::FRAC_PI_2)
        .to_degrees()
}

/// A decoded mosaic of terrarium tiles spanning a contiguous tile range, in
/// Web-Mercator pixel space. Missing tiles stay NaN and dissolve into
/// no-data during resampling instead of failing the whole fetch.
struct TileMosaic {
    /// Web-Mercator meters of the mosaic's west and south edges.
    min: [f64; 2],
    /// Size of one mosaic pixel in Web-Mercator meters.
    px: f64,
    width: usize,
    height: usize,
    /// Row-major from the mosaic's TOP row (image order, row 0 = north).
    values: Vec<f32>,
}

impl TileMosaic {
    /// Allocate the mosaic covering `tiles` (all at one zoom). Returns `None`
    /// for an empty list.
    fn new(tiles: &[Tile]) -> Option<Self> {
        let first = tiles.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
        for tile in tiles {
            x0 = x0.min(tile.x);
            y0 = y0.min(tile.y);
            x1 = x1.max(tile.x);
            y1 = y1.max(tile.y);
        }
        let tile_width = first.bounds[2] - first.bounds[0];
        let tile_height = first.bounds[3] - first.bounds[1];
        let width = (x1 - x0 + 1) as usize * TILE_PX;
        let height = (y1 - y0 + 1) as usize * TILE_PX;
        Some(Self {
            min: [
                first.bounds[0] - (first.x - x0) as f64 * tile_width,
                first.bounds[1] - (first.y - y0) as f64 * tile_height,
            ],
            px: tile_width / TILE_PX as f64,
            width,
            height,
            values: vec![f32::NAN; width * height],
        })
    }

    /// Paste one decoded tile into the mosaic. The offsets are recovered from
    /// the tile's mercator bounds, so fill order never matters.
    fn fill(&mut self, tile: &Tile, heights: &[f32]) {
        let start_col = ((tile.bounds[0] - self.min[0]) / self.px).round();
        let start_row_top =
            ((self.min[1] + self.height as f64 * self.px - tile.bounds[3]) / self.px).round();
        if start_col < 0.0 || start_row_top < 0.0 {
            return;
        }
        let (Ok(start_col), Ok(start_row_top)) = (
            usize::try_from(start_col as i64),
            usize::try_from(start_row_top as i64),
        ) else {
            return;
        };
        if start_col >= self.width || start_row_top >= self.height {
            return;
        }
        for row in 0..TILE_PX {
            let mosaic_row = start_row_top + row;
            if mosaic_row >= self.height {
                break;
            }
            let source = row * TILE_PX;
            let target = mosaic_row * self.width + start_col;
            let span = TILE_PX.min(self.width - start_col);
            self.values[target..target + span].copy_from_slice(&heights[source..source + span]);
        }
    }

    /// Bilinear elevation sample at a Web-Mercator position, or `None` when
    /// the position falls outside the mosaic or touches only missing pixels.
    /// Positions in the first/last half pixel blend toward the edge, with the
    /// outside corner simply absent from the weighted average.
    fn sample(&self, merc_x: f64, merc_y: f64) -> Option<f64> {
        let col = (merc_x - self.min[0]) / self.px - 0.5;
        let row = (self.min[1] + self.height as f64 * self.px - merc_y) / self.px - 0.5;
        if !col.is_finite() || !row.is_finite() {
            return None;
        }
        let col0 = col.floor();
        let row0 = row.floor();
        let fx = col - col0;
        let fy = row - row0;
        let at = |c: i64, r: i64| -> Option<f64> {
            if c < 0 || r < 0 || c >= self.width as i64 || r >= self.height as i64 {
                return None;
            }
            let value = self.values[r as usize * self.width + c as usize];
            (!value.is_nan()).then_some(f64::from(value))
        };
        let (c0, r0) = (col0 as i64, row0 as i64);
        let corners = [
            at(c0, r0),
            at(c0 + 1, r0),
            at(c0, r0 + 1),
            at(c0 + 1, r0 + 1),
        ];
        // Average only the valid corners: a NaN neighbour at the fringe of a
        // missing tile must not invalidate samples just inside the mosaic.
        let weights = [
            (1.0 - fx) * (1.0 - fy),
            fx * (1.0 - fy),
            (1.0 - fx) * fy,
            fx * fy,
        ];
        let mut sum = 0.0;
        let mut weight = 0.0;
        for (corner, weight_i) in corners.iter().zip(weights) {
            if let Some(value) = corner {
                sum += value * weight_i;
                weight += weight_i;
            }
        }
        (weight > 0.0).then_some(sum / weight)
    }
}

/// Geometry of the output grid: regular square cells in drawing coordinates.
struct GridPlan {
    origin: [f64; 2],
    cell_size: f64,
    columns: usize,
    rows: usize,
}

/// Choose the output grid so cells approximate the mosaic's ground resolution
/// while staying square in drawing units (what the CAD geometry and mesh care
/// about), bounded by the interactive column cap and the total-cell cap.
fn plan_grid(
    bounds_drawing: [f64; 4],
    world_bounds_merc: [f64; 4],
    mosaic_px_merc: f64,
) -> GridPlan {
    let drawing_width = (bounds_drawing[2] - bounds_drawing[0]).max(1e-9);
    // Mosaic pixels covering the drawing extent, expressed in drawing units.
    let target_columns = (world_bounds_merc[2] - world_bounds_merc[0]) / mosaic_px_merc.max(1e-9);
    let columns = (target_columns.round() as usize).clamp(8, 2048);
    let cell_size = drawing_width / columns as f64;
    let mut rows = (((bounds_drawing[3] - bounds_drawing[1]) / cell_size).round() as usize)
        .clamp(8, 2048);
    if columns.saturating_mul(rows) > MAX_GRID_CELLS {
        let shrink = (MAX_GRID_CELLS as f64 / (columns as f64 * rows as f64)).sqrt();
        rows = ((rows as f64 * shrink) as usize).max(8);
    }
    GridPlan {
        origin: [bounds_drawing[0], bounds_drawing[1]],
        cell_size,
        columns,
        rows,
    }
}

/// Build the grid's node positions in drawing coordinates, ready for the
/// batch reprojection into Web Mercator. Row-major from the bottom row,
/// matching `RasterSurface`'s value order.
fn grid_nodes(plan: &GridPlan) -> Vec<Option<[f64; 2]>> {
    let mut nodes = Vec::with_capacity(plan.columns * plan.rows);
    for row in 0..plan.rows {
        let y = plan.origin[1] + (row as f64 + 0.5) * plan.cell_size;
        for column in 0..plan.columns {
            let x = plan.origin[0] + (column as f64 + 0.5) * plan.cell_size;
            nodes.push(Some([x, y]));
        }
    }
    nodes
}

/// Resample the mosaic into grid values. `merc_nodes` holds one
/// pre-transformed Web-Mercator position per cell (see [`grid_nodes`]); a
/// failed or missing position becomes no-data. `elevation_scale` converts
/// source meters into drawing units.
fn resample_mosaic(
    mosaic: &TileMosaic,
    plan: &GridPlan,
    merc_nodes: &[Option<[f64; 2]>],
    elevation_scale: f64,
    no_data: f64,
) -> Vec<f64> {
    let mut values = vec![no_data; plan.columns * plan.rows];
    for (value, node) in values.iter_mut().zip(merc_nodes) {
        let Some([merc_x, merc_y]) = node else {
            continue;
        };
        if let Some(elevation) = mosaic.sample(*merc_x, *merc_y) {
            *value = elevation * elevation_scale;
        }
    }
    values
}

/// Bilinear [`ocs_pointcloud::SurfaceSampler`] over a fetched grid, with
/// nearest-valid-cell clamping so `CLAMP` draping works at coverage fringes.
pub(crate) struct TerrainSampler<'a> {
    surface: &'a ocs_pointcloud::RasterSurface,
}

impl<'a> TerrainSampler<'a> {
    pub fn new(surface: &'a ocs_pointcloud::RasterSurface) -> Self {
        Self { surface }
    }

    fn valid(&self, column: isize, row: isize) -> Option<f64> {
        if column < 0 || row < 0 {
            return None;
        }
        self.surface
            .value(column as usize, row as usize)
            .filter(|value| value.is_finite())
    }

    fn cell_of(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let column = (x - self.surface.origin[0]) / self.surface.cell_size - 0.5;
        let row = (y - self.surface.origin[1]) / self.surface.cell_size - 0.5;
        (column.is_finite() && row.is_finite()).then_some((column, row))
    }
}

impl ocs_pointcloud::SurfaceSampler for TerrainSampler<'_> {
    fn elevation_at(&self, x: f64, y: f64) -> Option<f64> {
        let (column, row) = self.cell_of(x, y)?;
        let (c0, r0) = (column.floor() as isize, row.floor() as isize);
        let (fx, fy) = (column - c0 as f64, row - r0 as f64);
        let corners = [
            self.valid(c0, r0),
            self.valid(c0 + 1, r0),
            self.valid(c0, r0 + 1),
            self.valid(c0 + 1, r0 + 1),
        ];
        let weights = [
            (1.0 - fx) * (1.0 - fy),
            fx * (1.0 - fy),
            (1.0 - fx) * fy,
            fx * fy,
        ];
        let mut sum = 0.0;
        let mut weight = 0.0;
        for (corner, weight_i) in corners.iter().zip(weights) {
            if let Some(value) = corner {
                sum += value * weight_i;
                weight += weight_i;
            }
        }
        (weight > 0.0).then_some(sum / weight)
    }

    /// Spiral outward from the requested cell to the nearest valid elevation,
    /// ring by ring, so a just-off-coverage point still clamps sensibly.
    fn clamped_elevation_at(&self, x: f64, y: f64) -> Option<f64> {
        if let Some(direct) = self.elevation_at(x, y) {
            return Some(direct);
        }
        let (column, row) = self.cell_of(x, y)?;
        let (center_c, center_r) = (column.round() as isize, row.round() as isize);
        let max_radius = self.surface.columns.max(self.surface.rows) as isize;
        for radius in 1..=max_radius {
            let mut best: Option<(f64, f64)> = None;
            let mut consider = |c: isize, r: isize| {
                if let Some(value) = self.valid(c, r) {
                    let distance = ((c - center_c) * (c - center_c)
                        + (r - center_r) * (r - center_r))
                        as f64;
                    if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                        best = Some((distance, value));
                    }
                }
            };
            for dc in -radius..=radius {
                consider(center_c + dc, center_r - radius);
                consider(center_c + dc, center_r + radius);
            }
            for dr in (-radius + 1)..radius {
                consider(center_c - radius, center_r + dr);
                consider(center_c + radius, center_r + dr);
            }
            if let Some((_, value)) = best {
                return Some(value);
            }
        }
        None
    }
}

/// Decimated quad mesh over the valid cells of a fetched grid.
pub(crate) struct TerrainMeshData {
    pub vertices: Vec<[f64; 3]>,
    pub quads: Vec<[usize; 4]>,
}

/// Build a decimated terrain mesh. `stride` is the node step between mesh
/// vertices; quads whose corners include no-data cells are skipped so holes
/// stay holes instead of plunging to a fake elevation. Quad winding is
/// counter-clockwise viewed from +Z so faces face upward.
fn build_mesh(surface: &ocs_pointcloud::RasterSurface, stride: usize) -> TerrainMeshData {
    let stride = stride.max(1);
    let out_columns = surface.columns.div_ceil(stride);
    let out_rows = surface.rows.div_ceil(stride);
    let mut vertices = Vec::with_capacity(out_columns * out_rows);
    let mut index_of = vec![usize::MAX; out_columns * out_rows];
    for out_row in 0..out_rows {
        let row = (out_row * stride).min(surface.rows - 1);
        for out_column in 0..out_columns {
            let column = (out_column * stride).min(surface.columns - 1);
            let x = surface.origin[0] + column as f64 * surface.cell_size;
            let y = surface.origin[1] + row as f64 * surface.cell_size;
            let z = surface
                .value(column, row)
                .filter(|value| value.is_finite())
                .unwrap_or(0.0);
            index_of[out_row * out_columns + out_column] = vertices.len();
            vertices.push([x, y, z]);
        }
    }
    let valid = |column: usize, row: usize| {
        surface
            .value(column, row)
            .is_some_and(|value| value.is_finite())
    };
    let mut quads = Vec::new();
    for out_row in 0..out_rows.saturating_sub(1) {
        for out_column in 0..out_columns.saturating_sub(1) {
            let c0 = out_column * stride;
            let r0 = out_row * stride;
            let c1 = (c0 + stride).min(surface.columns - 1);
            let r1 = (r0 + stride).min(surface.rows - 1);
            if !(valid(c0, r0) && valid(c1, r0) && valid(c1, r1) && valid(c0, r1)) {
                continue;
            }
            let base = out_row * out_columns + out_column;
            quads.push([
                index_of[base],
                index_of[base + 1],
                index_of[base + out_columns + 1],
                index_of[base + out_columns],
            ]);
        }
    }
    TerrainMeshData { vertices, quads }
}

/// Ground meters per drawing unit at `latitude` (degrees units scale with
/// latitude; this feeds only the zoom heuristic).
fn coordinate_unit_meters(unit: crate::app::spatial::CoordinateUnit, latitude: f64) -> f64 {
    match unit {
        crate::app::spatial::CoordinateUnit::Meters => 1.0,
        crate::app::spatial::CoordinateUnit::Feet => 0.3048,
        crate::app::spatial::CoordinateUnit::UsSurveyFeet => 1200.0 / 3937.0,
        crate::app::spatial::CoordinateUnit::Degrees => 111_320.0 * latitude.cos().abs().max(1e-6),
    }
}

/// Scale from source meters to drawing elevation units, plus a display label.
/// Degree-based CRSs keep elevations in meters: scaling meters into "degrees"
/// of height is meaningless, and point clouds pass Z through unchanged too.
fn elevation_scale_and_label(unit: crate::app::spatial::CoordinateUnit) -> (f64, &'static str) {
    match unit {
        crate::app::spatial::CoordinateUnit::Meters => (1.0, "meters"),
        crate::app::spatial::CoordinateUnit::Feet => (1.0 / 0.3048, "feet"),
        crate::app::spatial::CoordinateUnit::UsSurveyFeet => (3937.0 / 1200.0, "US survey feet"),
        crate::app::spatial::CoordinateUnit::Degrees => (1.0, "meters"),
    }
}

/// Approximate terrarium zoom whose pixel ground size matches a requested
/// cell size given in drawing units, evaluated at the drawing's center
/// latitude.
fn zoom_for_cell_size(
    cell_drawing_units: f64,
    merc_bounds: [f64; 4],
    unit: crate::app::spatial::CoordinateUnit,
) -> u32 {
    let latitude = mercator_latitude((merc_bounds[1] + merc_bounds[3]) / 2.0);
    let cell_meters = cell_drawing_units * coordinate_unit_meters(unit, latitude);
    if !cell_meters.is_finite() || cell_meters <= 0.0 {
        return DEFAULT_ZOOM;
    }
    let ground_m_per_px_zoom0 = 156_543.033_928_040_97 * latitude.cos().abs().max(1e-6);
    let zoom = (ground_m_per_px_zoom0 / cell_meters).log2().round();
    zoom.clamp(0.0, MAX_ZOOM as f64) as u32
}

struct TerrainBoundsCommand {
    first: Option<glam::DVec3>,
}

impl CadCommand for TerrainBoundsCommand {
    fn name(&self) -> &'static str {
        "TERRAIN BOUNDS"
    }

    fn prompt(&self) -> String {
        if self.first.is_some() {
            "TERRAIN BOUNDS  Specify opposite extent corner:".to_string()
        } else {
            "TERRAIN BOUNDS  Specify first extent corner in drawing coordinates:".to_string()
        }
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        match self.first {
            None => {
                self.first = Some(point);
                crate::command::CmdResult::NeedPoint
            }
            Some(first) => crate::command::CmdResult::Dispatch(format!(
                "TERRAIN BOUNDS {:.17} {:.17} {:.17} {:.17}",
                first.x.min(point.x),
                first.y.min(point.y),
                first.x.max(point.x),
                first.y.max(point.y)
            )),
        }
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }

    fn window_corner_pick(&self) -> bool {
        true
    }
}

struct TerrainInfoCommand;

impl CadCommand for TerrainInfoCommand {
    fn name(&self) -> &'static str {
        "TERRAIN INFO"
    }

    fn prompt(&self) -> String {
        "TERRAIN INFO  Pick a point to query the terrain elevation:".to_string()
    }

    fn on_point(&mut self, point: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::Dispatch(format!(
            "TERRAIN INFO {:.17} {:.17} {:.17}",
            point.x, point.y, point.z
        ))
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}

/// Offload a closure to a worker thread and map its result to a `Message`
/// (same shape as the basemap/point-cloud helpers).
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
        async move { receiver.await.expect("terrain worker dropped") },
        map,
    )
}

impl OpenCADStudio {
    /// Invalidate grids and late fetch results expressed in the previous CRS.
    pub(super) fn invalidate_terrain_coordinates(&mut self, tab_index: usize) {
        let tab_id = self.tabs[tab_index].id;
        if self.terrain_job.as_ref().is_some_and(|job| job.tab_id == tab_id) {
            if let Some(job) = self.terrain_job.take() { job.cancel.store(true, Ordering::Relaxed); }
        }
        let had_model = self.tabs[tab_index].terrain.take().is_some();
        self.tabs[tab_index].spatial.terrain = None;
        self.tabs[tab_index].terrain_request.bounds = None;
        if had_model {
            self.command_line.push_info("Coordinates changed: elevation data was unloaded. Download terrain again for the new CRS. Existing mesh geometry remains in the drawing.");
        }
    }
    /// `TERRAIN <subcommand> ...` parser. Bare `TERRAIN` reports status.
    pub(super) fn terrain_command(&mut self, cmd: &str) -> Task<Message> {
        let rest = cmd.trim_start_matches("TERRAIN").trim();
        let mut parts = rest.split_whitespace();
        let Some(sub) = parts.next() else {
            return self.terrain_status_task();
        };
        let i = self.active_tab;
        let tab_id = self.tabs[i].id;
        match sub.to_ascii_uppercase().as_str() {
            "STATUS" => self.terrain_status_task(),
            "AREA" | "CENTER" | "LOCATION" => {
                let values = parts.collect::<Vec<_>>();
                if values.is_empty() {
                    use crate::command::ValuePromptCommand;
                    let command = ValuePromptCommand::new(
                        "TERRAIN AREA",
                        "TERRAIN AREA  Enter longitude latitude [radius-km]:",
                    );
                    self.command_line.push_info(&command.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(command));
                    return Task::none();
                }
                let Ok(values) = values
                    .iter()
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>()
                else {
                    self.command_line.push_error(
                        "TERRAIN AREA <longitude> <latitude> [radius-km] (example: TERRAIN AREA -71.0589 42.3601 5).",
                    );
                    return Task::none();
                };
                if !(2..=3).contains(&values.len()) {
                    self.command_line.push_error(
                        "TERRAIN AREA <longitude> <latitude> [radius-km] (example: TERRAIN AREA -71.0589 42.3601 5).",
                    );
                    return Task::none();
                }
                let radius_km = values.get(2).copied().unwrap_or(5.0);
                let Some(area) =
                    basemap::wgs84_radius_area(values[0], values[1], radius_km)
                else {
                    self.command_line.push_error(
                        "TERRAIN AREA requires longitude -180..180, latitude -85..85, and a positive radius in km.",
                    );
                    return Task::none();
                };
                let Some(crs) = self.tabs[i].spatial.drawing_crs.as_ref() else {
                    self.command_line.push_error(
                        "TERRAIN AREA needs a drawing CRS first; use CRS <epsg>, then enter the site longitude/latitude.",
                    );
                    return Task::none();
                };
                let Some(bounds) =
                    basemap::source_bounds_from_wgs84_area(area, &crs.as_crs_info())
                else {
                    self.command_line.push_error(
                        "TERRAIN AREA could not transform that site into the drawing CRS.",
                    );
                    return Task::none();
                };
                self.tabs[i].terrain_request.bounds = Some(bounds);
                self.command_line.push_output(
                    format!(
                        "Terrain: site centered at {:.6}, {:.6} with {:.2} km radius; fetching elevation.",
                        values[0], values[1], radius_km
                    )
                    .as_str(),
                );
                self.terrain_begin_fetch(tab_id)
            }
            "BOUNDS" => {
                let values = parts.collect::<Vec<_>>();
                if values.is_empty() {
                    let command = TerrainBoundsCommand { first: None };
                    self.command_line.push_info(&command.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(command));
                    return Task::none();
                }
                if values.len() == 1
                    && matches!(values[0].to_ascii_uppercase().as_str(), "CLEAR" | "UNSET")
                {
                    self.tabs[i].terrain_request.bounds = None;
                    self.command_line.push_output(
                        "Terrain: stored terrain area cleared; the next fetch covers the drawing extent.",
                    );
                    return Task::none();
                }
                let Ok(values) = values
                    .iter()
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>()
                else {
                    self.command_line.push_error(
                        "TERRAIN BOUNDS [<min-x> <min-y> <max-x> <max-y>] (omit coordinates to pick two drawing corners).",
                    );
                    return Task::none();
                };
                if values.len() != 4
                    || values.iter().any(|value| !value.is_finite())
                    || values[0] >= values[2]
                    || values[1] >= values[3]
                {
                    self.command_line
                        .push_error("TERRAIN BOUNDS requires four finite values with min < max.");
                    return Task::none();
                }
                self.tabs[i].terrain_request.bounds = Some([values[0], values[1], values[2], values[3]]);
                self.command_line
                    .push_output("Terrain: drawing-coordinate area stored; fetching elevation.");
                self.terrain_begin_fetch(tab_id)
            }
            "ZOOM" => {
                let Some(zoom) = parts.next().and_then(|value| value.parse::<u32>().ok())
                else {
                    self.command_line.push_error(
                        "TERRAIN ZOOM <0-15> (0 = whole continents, 15 = ~5 m/pixel).",
                    );
                    return Task::none();
                };
                if zoom > MAX_ZOOM {
                    self.command_line.push_error(
                        format!("TERRAIN ZOOM accepts 0-{MAX_ZOOM}; the global data set carries no more detail.")
                            .as_str(),
                    );
                    return Task::none();
                }
                self.tabs[i].terrain_request.zoom = Some(zoom);
                self.tabs[i].terrain_request.cell_size = None;
                self.command_line
                    .push_output(format!("Terrain: source zoom set to {zoom}.").as_str());
                Task::none()
            }
            "RES" => {
                let Some(value) = parts.next().and_then(|value| value.parse::<f64>().ok())
                else {
                    self.command_line.push_error(
                        "TERRAIN RES <zoom 0-15 | cell-size in drawing units> (integers 0-15 are zoom levels; larger values are cell sizes).",
                    );
                    return Task::none();
                };
                if value.fract() == 0.0 && (0.0..=MAX_ZOOM as f64).contains(&value) {
                    self.tabs[i].terrain_request.zoom = Some(value as u32);
                    self.tabs[i].terrain_request.cell_size = None;
                    self.command_line.push_output(
                        format!("Terrain: source zoom set to {}.", value as u32).as_str(),
                    );
                } else if value.is_finite() && value > 0.0 {
                    self.tabs[i].terrain_request.cell_size = Some(value);
                    self.tabs[i].terrain_request.zoom = None;
                    self.command_line.push_output(
                        format!("Terrain: target cell size set to {value} drawing units.").as_str(),
                    );
                } else {
                    self.command_line
                        .push_error("TERRAIN RES accepts a zoom 0-15 or a positive cell size.");
                }
                Task::none()
            }
            "FETCH" | "DOWNLOAD" => self.terrain_begin_fetch(tab_id),
            "CANCEL" | "ABORT" => {
                if let Some(job) = self.terrain_job.take() {
                    job.cancel.store(true, Ordering::Relaxed);
                    self.command_line.push_output("Terrain: fetch cancelled.");
                } else {
                    self.command_line.push_output("Terrain: no fetch is running.");
                }
                Task::none()
            }
            "MESH" => {
                let max_vertices = parts
                    .next()
                    .map_or(Ok(self.tabs[i].terrain_request.mesh_max_vertices), |value| {
                        value.parse::<usize>()
                    });
                let Ok(max_vertices) = max_vertices else {
                    self.command_line
                        .push_error("TERRAIN MESH [max-vertices] (default 400000).");
                    return Task::none();
                };
                self.tabs[i].terrain_request.mesh_max_vertices = max_vertices.max(16);
                self.build_and_commit_terrain_mesh(i, max_vertices.max(16));
                Task::none()
            }
            "INFO" | "SURFACEAT" => {
                let values = parts.collect::<Vec<_>>();
                if values.is_empty() {
                    let command = TerrainInfoCommand;
                    self.command_line.push_info(&command.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(command));
                    return Task::none();
                }
                let Ok(values) = values
                    .iter()
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>()
                else {
                    self.command_line
                        .push_error("TERRAIN INFO [<x> <y> <z>] (omit coordinates to pick a point).");
                    return Task::none();
                };
                if values.len() != 3 {
                    self.command_line.push_error(
                        "TERRAIN INFO requires three coordinates, or none to pick a point.",
                    );
                    return Task::none();
                }
                self.inspect_terrain_surface(i, [values[0], values[1], values[2]]);
                Task::none()
            }
            "DRAPE" => {
                let fields = parts.collect::<Vec<_>>();
                if fields.len() > 3 {
                    self.command_line.push_error(
                        "Usage: TERRAIN DRAPE [positive-spacing] [vertical-offset] [ABORT|SKIP|CLAMP]",
                    );
                    return Task::none();
                }
                let spacing = fields.first().map_or(Ok(1.0), |value| value.parse::<f64>());
                let vertical_offset =
                    fields.get(1).map_or(Ok(0.0), |value| value.parse::<f64>());
                let outside_policy = fields.get(2).map_or(
                    Some(ocs_pointcloud::DrapeOutsidePolicy::Abort),
                    |value| match value.to_ascii_uppercase().as_str() {
                        "ABORT" => Some(ocs_pointcloud::DrapeOutsidePolicy::Abort),
                        "SKIP" => Some(ocs_pointcloud::DrapeOutsidePolicy::Skip),
                        "CLAMP" => Some(ocs_pointcloud::DrapeOutsidePolicy::Clamp),
                        _ => None,
                    },
                );
                match (spacing, vertical_offset, outside_policy) {
                    (Ok(spacing), Ok(vertical_offset), Some(outside_policy)) => {
                        self.drape_selected_to_terrain(
                            i,
                            spacing,
                            vertical_offset,
                            outside_policy,
                        );
                    }
                    _ => self.command_line.push_error(
                        "Usage: TERRAIN DRAPE [positive-spacing] [vertical-offset] [ABORT|SKIP|CLAMP]",
                    ),
                }
                Task::none()
            }
            "REMOVE" | "DETACH" | "OFF" => {
                self.invalidate_terrain_coordinates(i);
                self.tabs[i].terrain = None;
                self.persist_spatial_settings(i);
                self.command_line.push_output(
                    "Terrain: resident elevation model cleared (mesh entities remain; erase them separately).",
                );
                Task::none()
            }
            _ => {
                self.command_line.push_error(
                    "TERRAIN [AREA <lon lat [radius-km]>|BOUNDS [minx miny maxx maxy]|ZOOM <0-15>|RES <zoom|cell>|FETCH|MESH [max-vertices]|INFO [x y z]|DRAPE [spacing] [offset] [ABORT|SKIP|CLAMP]|STATUS|CANCEL|REMOVE].",
                );
                Task::none()
            }
        }
    }

    fn terrain_status_task(&mut self) -> Task<Message> {
        let i = self.active_tab;
        if let Some(job) = self.terrain_job.as_ref() {
            let (completed, total, failed) = job.snapshot();
            self.command_line.push_output(
                format!(
                    "Terrain: fetching {completed}/{total} tile(s) ({failed} failed); TERRAIN CANCEL stops it."
                )
                .as_str(),
            );
        }
        match self.tabs[i].terrain.as_ref() {
            Some(dataset) => {
                let surface = &dataset.surface;
                let range = dataset
                    .elevation_range()
                    .map(|(min, max)| format!("{min:.3} to {max:.3}"))
                    .unwrap_or_else(|| "empty".to_string());
                self.command_line.push_output(
                    format!(
                        "Terrain: {} grid {}x{} cells at {:.4} drawing units/cell, elevations {} {}; {} tile(s) at zoom {}{}, CRS {}.",
                        dataset.source,
                        surface.columns,
                        surface.rows,
                        surface.cell_size,
                        range,
                        dataset.elevation_unit,
                        dataset.tiles,
                        dataset.zoom,
                        if dataset.failed_tiles > 0 {
                            format!(" ({} failed)", dataset.failed_tiles)
                        } else {
                            String::new()
                        },
                        dataset.crs_label,
                    )
                    .as_str(),
                );
            }
            None => {
                self.command_line.push_output(
                    "Terrain: no resident elevation model; use TERRAIN AREA <lon> <lat> [radius-km] or TERRAIN FETCH.",
                );
            }
        }
        Task::none()
    }

    /// Resolve the area to cover, plan the tiles, and start the background
    /// fetch. All validation happens here so errors report synchronously.
    fn terrain_begin_fetch(&mut self, tab_id: u64) -> Task<Message> {
        let Some(i) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Task::none();
        };
        let Some(crs) = self.tabs[i].spatial.drawing_crs.as_ref() else {
            self.command_line.push_error(
                "Terrain: drawing CRS is unset; use CRS <epsg> (or attach a referenced LAS/LAZ source) before fetching terrain.",
            );
            return Task::none();
        };
        let crs_info = crs.as_crs_info();
        let bounds = self.tabs[i].terrain_request
            .bounds
            .or(self.tabs[i].spatial.basemap_bounds)
            .or_else(|| {
                self.tabs[i]
                    .scene
                    .model_space_extents()
                    .map(|(min, max)| {
                        [min.x as f64, min.y as f64, max.x as f64, max.y as f64]
                    })
            });
        let Some(bounds) = bounds else {
            self.command_line.push_error(
                "Terrain: no project extent is available; use TERRAIN AREA <lon> <lat> [radius-km], TERRAIN BOUNDS, BASEMAP BOUNDS, or draw geometry first.",
            );
            return Task::none();
        };
        let Some(world_bounds) = basemap::world_bounds_from_source(
            [bounds[0], bounds[1]],
            [bounds[2], bounds[3]],
            &crs_info,
        ) else {
            self.command_line
                .push_error("Terrain: cannot reproject the drawing bounds into Web Mercator.");
            return Task::none();
        };
        let requested_zoom = self.tabs[i].terrain_request
            .cell_size
            .map(|cell| zoom_for_cell_size(cell, world_bounds, crs.coordinate_unit))
            .or(self.tabs[i].terrain_request.zoom)
            .unwrap_or(DEFAULT_ZOOM);
        let Some((zoom, planned)) =
            basemap::zoom_for_tile_limit(world_bounds, requested_zoom, MAX_TERRAIN_TILES)
        else {
            self.command_line
                .push_error("Terrain: no terrain tiles cover the drawing bounds.");
            return Task::none();
        };
        let tiles =
            match basemap::tiles_covering_bounded(world_bounds, zoom, MAX_TERRAIN_TILES) {
                Ok(tiles) => tiles,
                Err(required) => {
                    self.command_line.push_error(
                        format!(
                            "Terrain: request needs {required} tiles, above the safe {MAX_TERRAIN_TILES}-tile limit; narrow the area with TERRAIN AREA or TERRAIN BOUNDS."
                        )
                        .as_str(),
                    );
                    return Task::none();
                }
            };
        if zoom < requested_zoom {
            self.command_line.push_info(
                format!(
                    "Terrain: area is large for zoom {requested_zoom}; using zoom {zoom} ({planned} tiles)."
                )
                .as_str(),
            );
        }
        if let Some(previous) = self.terrain_job.take() {
            previous.cancel.store(true, Ordering::Relaxed);
        }
        let job = TerrainJob::new(tab_id, tiles.len());
        let job_id = job.id;
        let completed = Arc::clone(&job.completed);
        let failed = Arc::clone(&job.failed);
        let failed_for_result = Arc::clone(&job.failed);
        let cancel = Arc::clone(&job.cancel);
        self.terrain_job = Some(job);
        self.command_line.push_info(
            format!(
                "Terrain: downloading {planned} tile(s) at zoom {zoom} (parallel workers + disk cache)..."
            )
            .as_str(),
        );
        let requested = tiles.len();
        let cache_root = crate::config::config_dir().map(|path| path.join("terrain_cache"));
        let (elevation_scale, elevation_unit) = elevation_scale_and_label(crs.coordinate_unit);
        let crs_label = crs.label();
        let worker_crs = crs_info;
        let worker_bounds = bounds;
        let worker_world_bounds = world_bounds;
        background_task(
            move || {
                let cancelled = || cancel.load(Ordering::Relaxed);
                // Fetch + decode every tile in parallel, tolerating failures.
                let fetched: Vec<(Tile, Option<Vec<f32>>)> = {
                    use rayon::prelude::*;
                    let worker_count = std::thread::available_parallelism()
                        .map(usize::from)
                        .unwrap_or(4)
                        .clamp(2, 8)
                        .min(tiles.len().max(1));
                    let fetch = |tile: &Tile| -> (Tile, Option<Vec<f32>>) {
                        if cancelled() {
                            return (*tile, None);
                        }
                        let url = terrarium_tile_url(tile.z, tile.x, tile.y);
                        let decoded = cache_root
                            .as_ref()
                            .and_then(|root| {
                                let path = root
                                    .join("terrarium")
                                    .join(tile.z.to_string())
                                    .join(tile.x.to_string())
                                    .join(format!("{}.img", tile.y));
                                crate::scene::resolve_remote_image_cached(&url, &path)
                            })
                            .or_else(|| {
                                crate::scene::model::image_model::resolve_image(&url)
                            });
                        let heights = decoded.and_then(|image| {
                            (image.width as usize == TILE_PX
                                && image.height as usize == TILE_PX)
                                .then(|| terrarium_heights(&image.pixels, TILE_PX, TILE_PX))
                        });
                        if heights.is_some() {
                            completed.fetch_add(1, Ordering::Relaxed);
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                            completed.fetch_add(1, Ordering::Relaxed);
                        }
                        (*tile, heights)
                    };
                    match rayon::ThreadPoolBuilder::new()
                        .num_threads(worker_count)
                        .thread_name(|index| format!("ocs-terrain-{index}"))
                        .build()
                    {
                        Ok(pool) => {
                            pool.install(|| tiles.par_iter().map(fetch).collect::<Vec<_>>())
                        }
                        Err(_) => tiles.iter().map(fetch).collect(),
                    }
                };
                let failed_tiles = fetched
                    .iter()
                    .filter(|(_, heights)| heights.is_none())
                    .count();
                if cancelled() {
                    return TerrainLoaded {
                        job_id,
                        tab_id,
                        requested,
                        failed: failed_tiles,
                        dataset: None,
                        error: Some("cancelled".to_string()),
                    };
                }
                let usable: Vec<Tile> = fetched
                    .iter()
                    .filter(|(_, heights)| heights.is_some())
                    .map(|(tile, _)| *tile)
                    .collect();
                if usable.is_empty() {
                    return TerrainLoaded {
                        job_id,
                        tab_id,
                        requested,
                        failed: failed_tiles,
                        dataset: None,
                        error: Some(format!(
                            "all {requested} tile request(s) failed; check the network connection and try again"
                        )),
                    };
                }
                let mut mosaic = TileMosaic::new(&usable).expect("non-empty usable tiles");
                for (tile, heights) in &fetched {
                    if let Some(heights) = heights {
                        mosaic.fill(tile, heights);
                    }
                }
                let mosaic_px = 2.0 * MERCATOR_HALF / ((1_u64 << zoom) * TILE_PX as u64) as f64;
                let plan = plan_grid(worker_bounds, worker_world_bounds, mosaic_px);
                let mut nodes = grid_nodes(&plan);
                ocs_pointcloud::reproject_xy_batch_between_crs(
                    &worker_crs,
                    &ocs_pointcloud::CrsInfo {
                        horizontal_epsg: Some(3857),
                        ..Default::default()
                    },
                    &mut nodes,
                );
                let values = resample_mosaic(
                    &mosaic,
                    &plan,
                    &nodes,
                    elevation_scale,
                    -9999.0,
                );
                let surface = ocs_pointcloud::RasterSurface {
                    origin: plan.origin,
                    cell_size: plan.cell_size,
                    columns: plan.columns,
                    rows: plan.rows,
                    no_data: -9999.0,
                    values,
                    counts: vec![1; plan.columns * plan.rows],
                };
                let mut dataset = TerrainDataset {
                    surface, zoom, tiles: usable.len(), failed_tiles, crs_label,
                    elevation_unit: elevation_unit.into(), source: "AWS Terrain Tiles".into(), cache_key: None,
                };
                match super::terrain_store::save(&dataset) {
                    Ok(key) => dataset.cache_key = Some(key),
                    Err(error) => eprintln!("Terrain snapshot could not be saved: {error}"),
                }
                TerrainLoaded {
                    job_id,
                    tab_id,
                    requested,
                    failed: failed_for_result.load(Ordering::Relaxed),
                    dataset: Some(dataset),
                    error: None,
                }
            },
            move |loaded| Message::TerrainLoaded(loaded),
        )
    }

    /// Install a finished fetch on its tab and generate the terrain mesh.
    pub(super) fn install_terrain(&mut self, loaded: TerrainLoaded) {
        if self
            .terrain_job
            .as_ref()
            .is_none_or(|job| job.id != loaded.job_id || job.tab_id != loaded.tab_id)
        {
            return;
        }
        self.terrain_job = None;
        let Some(i) = self.tabs.iter().position(|tab| tab.id == loaded.tab_id) else {
            return;
        };
        let Some(dataset) = loaded.dataset else {
            let reason = loaded
                .error
                .unwrap_or_else(|| "unknown failure".to_string());
            if reason != "cancelled" {
                self.command_line
                    .push_error(format!("Terrain: fetch failed: {reason}.").as_str());
            }
            return;
        };
        let surface = &dataset.surface;
        let range = dataset
            .elevation_range()
            .map(|(min, max)| format!("{min:.3} to {max:.3} {}", dataset.elevation_unit))
            .unwrap_or_else(|| "empty".to_string());
        self.command_line.push_output(
            format!(
                "Terrain: {}/{} tile(s) resampled to a {}x{} grid at {:.4} drawing units/cell, elevations {} (CRS {}; zoom {}).",
                dataset.tiles,
                loaded.requested,
                surface.columns,
                surface.rows,
                surface.cell_size,
                range,
                dataset.crs_label,
                dataset.zoom,
            )
            .as_str(),
        );
        self.tabs[i].spatial.terrain = dataset.cache_key.as_ref().and_then(|key| {
            self.tabs[i].spatial.drawing_crs.clone().map(|crs| super::terrain_store::SavedTerrain {
                key: key.clone(), request: self.tabs[i].terrain_request, crs,
            })
        });
        if dataset.cache_key.is_none() {
            self.command_line.push_error("Terrain is available for this session, but its snapshot could not be saved. Check free disk space before closing the drawing.");
        }
        self.tabs[i].terrain = Some(Arc::new(dataset));
        self.persist_spatial_settings(i);
        let max_vertices = self.tabs[i].terrain_request.mesh_max_vertices.max(16);
        self.build_and_commit_terrain_mesh(i, max_vertices);
    }

    /// Restore exact saved elevations in a worker. No network or new mesh is needed.
    pub(super) fn restore_saved_terrain(&mut self, i: usize) -> Task<Message> {
        let Some(saved) = self.tabs[i].spatial.terrain.clone() else { return Task::none(); };
        if self.tabs[i].spatial.drawing_crs.as_ref() != Some(&saved.crs) {
            self.command_line.push_error("Saved terrain CRS does not match the drawing; download terrain again.");
            return Task::none();
        }
        self.tabs[i].terrain_request = saved.request;
        let id = self.tabs[i].id;
        let key = saved.key;
        background_task(move || {
            let result = super::terrain_store::load(&key).map_err(|e| e.to_string());
            (id, key, result)
        }, |(id, key, result)| Message::TerrainRestored(id, key, result))
    }

    pub(super) fn install_restored_terrain(&mut self, id: u64, key: String, result: Result<TerrainDataset, String>) {
        let Some(i) = self.tabs.iter().position(|tab| tab.id == id) else { return; };
        let tab = &mut self.tabs[i];
        if !tab.spatial.terrain.as_ref().is_some_and(|s| s.key == key && Some(&s.crs) == tab.spatial.drawing_crs.as_ref()) ||
            self.terrain_job.as_ref().is_some_and(|j| j.tab_id == id) { return; }
        match result {
            Ok(dataset) => {
                tab.terrain = Some(Arc::new(dataset));
                self.command_line.push_output("Terrain: restored saved elevations; draping and elevation queries are ready. Existing meshes were preserved.");
            }
            Err(error) => self.command_line.push_error(format!("Terrain: saved snapshot unavailable ({error}); download the site again in Terrain workspace.").as_str()),
        }
    }

    /// Build a decimated 3-D mesh over the resident elevation grid and commit
    /// it as a regular Mesh entity through the standard undo flow.
    fn build_and_commit_terrain_mesh(&mut self, tab_index: usize, max_vertices: usize) {
        let Some(dataset) = self.tabs[tab_index].terrain.clone() else {
            self.command_line.push_error(
                "Terrain: no resident elevation model; fetch one with TERRAIN AREA first.",
            );
            return;
        };
        let cells = dataset.surface.columns.saturating_mul(dataset.surface.rows);
        let stride = (((cells as f64) / (max_vertices as f64))
            .sqrt()
            .ceil() as usize)
            .max(1);
        let data = build_mesh(&dataset.surface, stride);
        if data.quads.is_empty() {
            self.command_line.push_error(
                "Terrain: the fetched grid contains no valid elevation cells; nothing to mesh.",
            );
            return;
        }
        let vertices = data
            .vertices
            .iter()
            .map(|p| acadrust::types::Vector3::new(p[0], p[1], p[2]))
            .collect();
        let quads = data
            .quads
            .iter()
            .map(|q| (q[0], q[1], q[2], q[3]))
            .collect::<Vec<_>>();
        let mut mesh = acadrust::entities::mesh::Mesh::from_quads(vertices, &quads);
        mesh.common.handle = acadrust::Handle::NULL;
        mesh.common.layer = self.tabs[tab_index].active_layer.clone();
        let entity = acadrust::EntityType::Mesh(mesh);
        let delta_safe = self.delta_add_safe(tab_index, &entity);
        let pending = self.begin_undo(tab_index, "TERRAINMESH", 1, delta_safe);
        // A download may finish while another drawing is active. Terrain is
        // always model-space geometry owned by the job's original document.
        let handle = self.tabs[tab_index].scene.add_entity(entity);
        self.tabs[tab_index].dirty = true;
        let mut selection = rustc_hash::FxHashSet::default();
        selection.insert(handle);
        self.tabs[tab_index].scene.replace_selection(selection);
        if let Some(pending) = pending {
            self.commit_undo_delta(tab_index, pending);
        }
        self.command_line.push_output(
            format!(
                "Terrain: created a {}-vertex / {}-face terrain mesh (stride {stride}, {} source tiles at zoom {}).",
                data.vertices.len(),
                data.quads.len(),
                dataset.tiles,
                dataset.zoom,
            )
            .as_str(),
        );
    }

    /// Reports the terrain elevation under a point plus the signed vertical
    /// distance from the picked Z, mirroring POINTCLOUDSURFACEAT.
    fn inspect_terrain_surface(&mut self, tab_index: usize, point: [f64; 3]) {
        if !point.into_iter().all(f64::is_finite) {
            self.command_line
                .push_error("TERRAIN INFO: coordinates must be finite.");
            return;
        }
        let Some(dataset) = self.tabs[tab_index].terrain.clone() else {
            self.command_line.push_error(
                "TERRAIN INFO: no resident terrain model; fetch one with TERRAIN AREA first.",
            );
            return;
        };
        let sampler = TerrainSampler::new(&dataset.surface);
        let Some(measurement) = ocs_pointcloud::point_to_surface(point, &sampler) else {
            self.command_line
                .push_error("TERRAIN INFO: that point lies outside the fetched terrain area.");
            return;
        };
        self.command_line.push_output(
            format!(
                "TERRAIN INFO: ({:.6}, {:.6}) elevation {:.6} {} over the fetched terrain; input Z {:.6}, signed vertical distance {:.6}.",
                point[0],
                point[1],
                measurement.surface_elevation,
                dataset.elevation_unit,
                point[2],
                measurement.signed_vertical_distance
            )
            .as_str(),
        );
    }

    /// Drape the selected curves/polylines/meshes onto the resident fetched
    /// terrain grid. The point-cloud drape delegates here when no cloud is
    /// attached but a terrain model is resident.
    pub(super) fn drape_selected_to_terrain(
        &mut self,
        tab_index: usize,
        spacing: f64,
        vertical_offset: f64,
        outside_policy: ocs_pointcloud::DrapeOutsidePolicy,
    ) {
        if !spacing.is_finite() || spacing <= 0.0 {
            self.command_line
                .push_error("TERRAIN DRAPE: spacing must be finite and positive.");
            return;
        }
        if !vertical_offset.is_finite() {
            self.command_line
                .push_error("TERRAIN DRAPE: vertical offset must be finite.");
            return;
        }
        let Some(dataset) = self.tabs[tab_index].terrain.clone() else {
            self.command_line.push_error(
                "TERRAIN DRAPE: no resident terrain model; fetch one with TERRAIN AREA first.",
            );
            return;
        };
        let selected: Vec<acadrust::EntityType> = self.tabs[tab_index]
            .scene
            .selected_entities()
            .into_iter()
            .map(|(_, entity)| entity.clone())
            .collect();
        if selected.is_empty() {
            self.command_line.push_error(
                "TERRAIN DRAPE: select one or more curves, polylines, or meshes first.",
            );
            return;
        }
        let source_paths: Vec<DrapeSourcePath> =
            selected.iter().filter_map(entity_drape_source).collect();
        let source_meshes: Vec<acadrust::entities::mesh::Mesh> = selected
            .iter()
            .filter_map(|entity| match entity {
                acadrust::EntityType::Mesh(mesh) => Some(mesh.clone()),
                _ => None,
            })
            .collect();
        let supported = source_paths.len() + source_meshes.len();
        let unsupported = selected.len().saturating_sub(supported);
        if supported == 0 {
            self.command_line.push_error(
                "TERRAIN DRAPE: the selection contains no supported curves, polylines, or meshes.",
            );
            return;
        }
        let sampler = TerrainSampler::new(&dataset.surface);
        let mut outputs = Vec::with_capacity(supported);
        let mut output_points = 0_usize;
        let mut coverage_skipped = 0usize;
        let mut clamped_objects = 0usize;
        for (path_index, source) in source_paths.iter().enumerate() {
            let draped = match ocs_pointcloud::drape_path_with_policy(
                &source.points,
                source.closed,
                spacing,
                vertical_offset,
                &sampler,
                outside_policy,
            ) {
                Ok(path) => path,
                Err(error) => {
                    if outside_policy == ocs_pointcloud::DrapeOutsidePolicy::Skip
                        && matches!(error, ocs_pointcloud::DrapeError::OutsideSurface { .. })
                    {
                        coverage_skipped += 1;
                        self.command_line.push_info(
                            format!(
                                "TERRAIN DRAPE: skipped path {} because part of it lies outside the fetched terrain.",
                                path_index + 1
                            )
                            .as_str(),
                        );
                        continue;
                    }
                    self.command_line.push_error(
                        format!(
                            "TERRAIN DRAPE: path {} was not created: {error}. No draped entities were added.",
                            path_index + 1
                        )
                        .as_str(),
                    );
                    return;
                }
            };
            if outside_policy == ocs_pointcloud::DrapeOutsidePolicy::Clamp
                && source
                    .points
                    .iter()
                    .any(|point| sampler.elevation_at(point[0], point[1]).is_none())
            {
                clamped_objects += 1;
            }
            output_points = output_points.saturating_add(draped.points.len());
            let mut polyline = acadrust::entities::Polyline3D::from_points(
                draped
                    .points
                    .into_iter()
                    .map(|point| {
                        acadrust::types::Vector3::new(point[0], point[1], point[2])
                    })
                    .collect(),
            );
            if draped.closed {
                polyline.close();
            }
            outputs.push(acadrust::EntityType::Polyline3D(polyline));
        }
        for (mesh_index, mut mesh) in source_meshes.into_iter().enumerate() {
            if mesh.vertices.len() > MAX_DRAPED_MESH_VERTICES {
                self.command_line.push_error(
                    format!(
                        "TERRAIN DRAPE: mesh {} has {} vertices (interactive limit {MAX_DRAPED_MESH_VERTICES}); no draped entities were added.",
                        mesh_index + 1,
                        mesh.vertices.len(),
                    )
                    .as_str(),
                );
                return;
            }
            let mut mesh_was_clamped = false;
            let mut mesh_outside = false;
            for (vertex_index, vertex) in mesh.vertices.iter_mut().enumerate() {
                let elevation = sampler.elevation_at(vertex.x, vertex.y).or_else(|| {
                    (outside_policy == ocs_pointcloud::DrapeOutsidePolicy::Clamp)
                        .then(|| {
                            sampler.clamped_elevation_at(vertex.x, vertex.y).inspect(|_| {
                                mesh_was_clamped = true;
                            })
                        })
                        .flatten()
                });
                let Some(elevation) = elevation else {
                    if outside_policy == ocs_pointcloud::DrapeOutsidePolicy::Skip {
                        coverage_skipped += 1;
                        mesh_outside = true;
                        self.command_line.push_info(
                            format!(
                                "TERRAIN DRAPE: skipped mesh {} because vertex {} lies outside the fetched terrain.",
                                mesh_index + 1,
                                vertex_index
                            )
                            .as_str(),
                        );
                        break;
                    }
                    self.command_line.push_error(
                        format!(
                            "TERRAIN DRAPE: mesh {} vertex {} lies outside the fetched terrain; no draped entities were added.",
                            mesh_index + 1,
                            vertex_index
                        )
                        .as_str(),
                    );
                    return;
                };
                vertex.z = elevation + vertical_offset;
            }
            if mesh_outside {
                continue;
            }
            if mesh_was_clamped {
                clamped_objects += 1;
            }
            output_points = output_points.saturating_add(mesh.vertices.len());
            mesh.common.handle = acadrust::Handle::NULL;
            outputs.push(acadrust::EntityType::Mesh(mesh));
        }
        let delta_safe = outputs
            .iter()
            .all(|entity| self.delta_add_safe(tab_index, entity));
        let pending = self.begin_undo(tab_index, "TERRAINDRAPE", outputs.len(), delta_safe);
        let mut created_handles = rustc_hash::FxHashSet::default();
        for output in outputs {
            if let Some(handle) = self.commit_entity_handle(output) {
                created_handles.insert(handle);
            }
        }
        if created_handles.is_empty() {
            if let Some(pending) = pending {
                self.commit_undo_delta(tab_index, pending);
            }
            if coverage_skipped == supported
                && outside_policy == ocs_pointcloud::DrapeOutsidePolicy::Skip
            {
                self.command_line.push_output(
                    format!(
                        "TERRAIN DRAPE: skipped all {coverage_skipped} supported object(s) because they extend outside the fetched terrain; no entities were added."
                    )
                    .as_str(),
                );
            } else {
                self.command_line
                    .push_error("TERRAIN DRAPE: the draped paths could not be added.");
            }
            return;
        }
        self.tabs[tab_index].dirty = true;
        self.tabs[tab_index]
            .scene
            .replace_selection(created_handles.clone());
        if let Some(pending) = pending {
            self.commit_undo_delta(tab_index, pending);
        }
        let skipped = unsupported + coverage_skipped;
        let skipped_note = if skipped == 0 {
            String::new()
        } else {
            format!("; skipped {skipped} unsupported or out-of-coverage selected object(s)")
        };
        self.command_line.push_output(
            format!(
                "TERRAIN DRAPE: created {} terrain-draped object(s) with {output_points} sampled vertices from the fetched terrain grid ({} cells), spacing {spacing}, vertical offset {vertical_offset}, outside policy {:?}, clamped {clamped_objects} object(s){skipped_note}.",
                created_handles.len(),
                dataset.surface.columns * dataset.surface.rows,
                outside_policy,
            )
            .as_str(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dataset() -> TerrainDataset {
        TerrainDataset { surface: sample_surface(), zoom: 12, tiles: 1, failed_tiles: 0,
            crs_label: "EPSG:26986".into(), elevation_unit: "meters".into(), source: "fixture".into(), cache_key: None }
    }

    #[test]
    fn terrain_completion_targets_its_original_document_and_is_undoable() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let original = app.active_tab;
        let id = app.tabs[original].id;
        app.tabs.push(super::super::document::DocumentTab::new_drawing(502));
        app.active_tab = app.tabs.len() - 1;
        app.terrain_job = Some(TerrainJob::new(id, 1));
        let job_id = app.terrain_job.as_ref().unwrap().id;
        app.install_terrain(TerrainLoaded { job_id, tab_id: id, requested: 1, failed: 0,
            dataset: Some(fixture_dataset()), error: None });
        assert_eq!(app.tabs[original].scene.document.entities().count(), 1);
        assert_eq!(app.tabs[app.active_tab].scene.document.entities().count(), 0);
        app.active_tab = original;
        assert_eq!(app.automation_op(r#"{"op":"undo"}"#)["total"], 0);
    }

    #[test]
    fn saved_terrain_reopens_with_exact_grid_and_no_duplicate_mesh() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let _ = app.dispatch_command("CRS 26986");
        let mut dataset = fixture_dataset();
        dataset.cache_key = Some(super::super::terrain_store::save(&dataset).unwrap());
        let expected = dataset.surface.clone();
        let id = app.tabs[i].id;
        app.terrain_job = Some(TerrainJob::new(id, 1));
        let job_id = app.terrain_job.as_ref().unwrap().id;
        app.install_terrain(TerrainLoaded { job_id, tab_id: id, requested: 1, failed: 0,
            dataset: Some(dataset), error: None });
        let root = std::env::temp_dir().join(format!("ocs-terrain-reopen-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("terrain.dxf");
        let request = serde_json::json!({"op":"save", "path":path});
        assert_eq!(app.automation_op(&request.to_string())["ok"], true);
        app.automation_op(r#"{"op":"new"}"#);
        let opened = app.automation_op(&serde_json::json!({"op":"open", "path":path}).to_string());
        assert_eq!(opened["ok"], true);
        assert_eq!(opened["total"], 1);
        assert_eq!(app.tabs[app.active_tab].terrain.as_ref().unwrap().surface, expected);
        // A late restore must not resurrect a surface after its CRS changes.
        let key = app.tabs[app.active_tab].spatial.terrain.as_ref().unwrap().key.clone();
        let id = app.tabs[app.active_tab].id;
        let _ = app.dispatch_command("CRS 3857");
        app.install_restored_terrain(id, key, Ok(fixture_dataset()));
        assert!(app.tabs[app.active_tab].terrain.is_none());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terrarium_formula_matches_the_published_encoding() {
        // (128, 0, 0) is exactly sea level in the terrarium encoding.
        assert_eq!(terrarium_elevation(128, 0, 0), 0.0);
        assert_eq!(terrarium_elevation(0, 0, 0), -32768.0);
        // The all-max pixel is a quarter millimeter under 32768: the blue
        // channel carries 255/256.
        assert_eq!(terrarium_elevation(255, 255, 255), 32767.996_093_75);
        // A quarter-meter fraction arrives through the blue channel.
        assert_eq!(terrarium_elevation(128, 0, 64), 0.25);
    }

    #[test]
    fn terrarium_heights_decode_rgba_rows() {
        // Two pixels: sea level and 1 m. Alpha must be ignored.
        let pixels = [128, 0, 0, 255, 128, 1, 0, 255];
        let heights = terrarium_heights(&pixels, 2, 1);
        assert_eq!(heights, vec![0.0, 1.0]);
    }

    fn test_tile(z: u32, x: u32, y: u32) -> Tile {
        Tile {
            z,
            x,
            y,
            bounds: basemap::tile_bounds(z, x, y),
        }
    }

    #[test]
    fn mosaic_fill_and_sample_across_the_seam() {
        // Two adjacent z=1 tiles with constant elevations 100 and 200 m.
        let west = test_tile(1, 0, 1);
        let east = test_tile(1, 1, 1);
        let mosaic = TileMosaic::new(&[west, east]).expect("two tiles");
        assert_eq!(mosaic.width, 2 * TILE_PX);
        assert_eq!(mosaic.height, TILE_PX);
        let mut mosaic = mosaic;
        let west_heights = vec![100.0_f32; TILE_PX * TILE_PX];
        let east_heights = vec![200.0_f32; TILE_PX * TILE_PX];
        mosaic.fill(&west, &west_heights);
        mosaic.fill(&east, &east_heights);
        let (west_center, east_center) = (
            ((west.bounds[0] + west.bounds[2]) / 2.0, (west.bounds[1] + west.bounds[3]) / 2.0),
            ((east.bounds[0] + east.bounds[2]) / 2.0, (east.bounds[1] + east.bounds[3]) / 2.0),
        );
        let sampled_west = mosaic.sample(west_center.0, west_center.1).expect("west");
        let sampled_east = mosaic.sample(east_center.0, east_center.1).expect("east");
        assert!((sampled_west - 100.0).abs() < 1e-6, "west = {sampled_west}");
        assert!((sampled_east - 200.0).abs() < 1e-6, "east = {sampled_east}");
        // The seam blends bilinearly between the two constants: a quarter
        // pixel east of the seam leans three quarters toward the east tile.
        let seam_x = west.bounds[2];
        let seam = mosaic
            .sample(seam_x + mosaic.px / 4.0, west_center.1)
            .expect("seam");
        assert!(
            (seam - 175.0).abs() < 1e-6,
            "quarter-pixel-east sample = {seam}"
        );
        // Unfilled mosaic area is no-data.
        assert!(mosaic.sample(west_center.0, west.bounds[3] - mosaic.px * 4096.0).is_none());
    }

    #[test]
    fn mosaic_sample_rejects_outside_and_nan() {
        let tile = test_tile(1, 0, 0);
        let mut mosaic = TileMosaic::new(&[tile]).expect("one tile");
        assert!(mosaic.sample(0.0, 0.0).is_none(), "all-NaN mosaic");
        mosaic.fill(&tile, &vec![5.0_f32; TILE_PX * TILE_PX]);
        assert!(mosaic.sample(0.0, 0.0).is_some(), "filled center");
        // Outside the mosaic entirely (southern hemisphere).
        assert!(mosaic.sample(0.0, -1.5 * MERCATOR_HALF).is_none());
    }

    #[test]
    fn plan_grid_tracks_resolution_and_caps() {
        // A 10 km square whose mercator envelope matches one-to-one at ~38 m
        // per mosaic pixel gives roughly 263 columns.
        let square = [0.0, 0.0, 10_000.0, 10_000.0];
        let plan = plan_grid(square, square, 38.0);
        assert!(
            (250..=280).contains(&plan.columns),
            "columns = {}",
            plan.columns
        );
        assert!(
            (plan.rows as i64 - plan.columns as i64).abs() <= 1,
            "square area"
        );
        assert!(
            (plan.cell_size - 10_000.0 / plan.columns as f64).abs() < 1e-9,
            "square cells in drawing units"
        );
        // A sub-pixel mosaic saturates the interactive column cap instead of
        // allocating an unbounded grid.
        let clamped = plan_grid(square, square, 1e-6);
        assert_eq!(clamped.columns, 2048);
    }

    #[test]
    fn grid_nodes_are_bottom_row_major_and_offset_half_a_cell() {
        let plan = GridPlan {
            origin: [10.0, 20.0],
            cell_size: 2.0,
            columns: 3,
            rows: 2,
        };
        let nodes = grid_nodes(&plan);
        assert_eq!(nodes.len(), 6);
        assert_eq!(nodes[0], Some([11.0, 21.0]));
        assert_eq!(nodes[2], Some([15.0, 21.0]));
        assert_eq!(nodes[3], Some([11.0, 23.0]));
    }

    #[test]
    fn resample_maps_missing_nodes_and_pixels_to_no_data() {
        let tile = test_tile(1, 0, 0);
        let mut mosaic = TileMosaic::new(&[tile]).expect("one tile");
        mosaic.fill(&tile, &vec![50.0_f32; TILE_PX * TILE_PX]);
        let plan = GridPlan {
            origin: [0.0, 0.0],
            cell_size: 1.0,
            columns: 2,
            rows: 1,
        };
        // Sample at the mosaic tile's center, where bilinear weighting is
        // exact; the second node has no transformed position at all.
        let tile_center = [
            (tile.bounds[0] + tile.bounds[2]) / 2.0,
            (tile.bounds[1] + tile.bounds[3]) / 2.0,
        ];
        let values = resample_mosaic(
            &mosaic,
            &plan,
            &[Some(tile_center), None],
            2.0,
            -9999.0,
        );
        assert!((values[0] - 100.0).abs() < 1e-6, "50 m × scale 2");
        assert_eq!(values[1], -9999.0, "missing node");
    }

    fn sample_surface() -> ocs_pointcloud::RasterSurface {
        // 3x3 grid, origin (0,0), cell 1: value = column + 10*row.
        let values = (0..9)
            .map(|index| (index % 3) as f64 + 10.0 * (index / 3) as f64)
            .collect();
        ocs_pointcloud::RasterSurface {
            origin: [0.0, 0.0],
            cell_size: 1.0,
            columns: 3,
            rows: 3,
            no_data: -9999.0,
            values,
            counts: vec![1; 9],
        }
    }

    #[test]
    fn sampler_interpolates_bilinearly() {
        let surface = sample_surface();
        let sampler = TerrainSampler::new(&surface);
        // Cell centers sit at (col+0.5, row+0.5): the grid's middle value is
        // exactly 11 at (1.5, 1.5).
        let center = sampler.elevation_at(1.5, 1.5).expect("center");
        assert!((center - 11.0).abs() < 1e-9, "cell-center value");
        let edge = sampler.elevation_at(1.0, 1.5).expect("edge");
        assert!(
            (edge - 10.5).abs() < 1e-9,
            "between columns 0 and 1 of row 1: {edge}"
        );
        assert!(sampler.elevation_at(-5.0, 1.0).is_none(), "outside");
    }

    #[test]
    fn sampler_clamps_to_nearest_valid_cell() {
        let mut surface = sample_surface();
        // Knock out the right column so (2.5, 1.0) has only NaN neighbours.
        for row in 0..3 {
            surface.values[row * 3 + 2] = surface.no_data;
        }
        let sampler = TerrainSampler::new(&surface);
        assert!(sampler.elevation_at(2.5, 1.0).is_none());
        let clamped = sampler.clamped_elevation_at(2.9, 1.0).expect("clamp");
        assert!((clamped - 11.0).abs() < 1e-9, "nearest valid is column 1 row 1");
    }

    #[test]
    fn mesh_builder_skips_nodata_and_strides() {
        let surface = sample_surface();
        // A 3x3 grid yields a 2x2 quad mesh with every node valid.
        let full = build_mesh(&surface, 1);
        assert_eq!(full.vertices.len(), 9);
        assert_eq!(full.quads.len(), 4);
        // Knocking out one corner drops only the quad touching it.
        let mut corner_hole = surface.clone();
        corner_hole.values[0] = corner_hole.no_data;
        let corner = build_mesh(&corner_hole, 1);
        assert_eq!(corner.quads.len(), 3);
        // The center cell touches every quad, so the mesh loses them all.
        let mut center_hole = surface.clone();
        center_hole.values[4] = center_hole.no_data;
        assert_eq!(build_mesh(&center_hole, 1).quads.len(), 0);
        // Stride 2 decimates to the four corner nodes; the single quad spans
        // them with counter-clockwise winding viewed from +Z (first edge east).
        let coarse = build_mesh(&surface, 2);
        assert_eq!(coarse.vertices.len(), 4);
        assert_eq!(coarse.quads.len(), 1);
        let quad = coarse.quads[0];
        let (a, b) = (coarse.vertices[quad[0]], coarse.vertices[quad[1]]);
        assert!(b[0] > a[0], "first edge runs east, giving upward normals");
        assert!(coarse.vertices[quad[2]][1] > a[1], "second edge runs north");
    }

    #[test]
    fn elevation_scale_matches_coordinate_units() {
        use crate::app::spatial::CoordinateUnit;
        let (meters, label) = elevation_scale_and_label(CoordinateUnit::Meters);
        assert_eq!(meters, 1.0);
        assert_eq!(label, "meters");
        let (feet, label) = elevation_scale_and_label(CoordinateUnit::UsSurveyFeet);
        assert!((feet - 3937.0 / 1200.0).abs() < 1e-9);
        assert_eq!(label, "US survey feet");
        // Degree CRSs keep meters for elevation.
        let (degrees, label) = elevation_scale_and_label(CoordinateUnit::Degrees);
        assert_eq!(degrees, 1.0);
        assert_eq!(label, "meters");
    }

    #[test]
    fn zoom_from_cell_size_refines_with_smaller_cells() {
        let world = basemap::tile_bounds(0, 0, 0);
        use crate::app::spatial::CoordinateUnit;
        let coarse = zoom_for_cell_size(100.0, world, CoordinateUnit::Meters);
        let fine = zoom_for_cell_size(10.0, world, CoordinateUnit::Meters);
        assert!(fine > coarse, "10 m cells need a higher zoom than 100 m");
        assert!(fine <= MAX_ZOOM);
        // Absurd requests clamp into the valid range instead of panicking.
        let absurd = zoom_for_cell_size(1e-9, world, CoordinateUnit::Meters);
        assert_eq!(absurd, MAX_ZOOM);
    }

    #[test]
    fn terrain_url_matches_the_open_data_layout() {
        assert_eq!(
            terrarium_tile_url(12, 1583, 2410),
            "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/12/1583/2410.png"
        );
    }

    #[test]
    fn mercator_latitude_inverts_the_tile_y_formula() {
        // Mercator Y=0 is the equator; the tile-scheme limit is ~85.05°.
        assert!((mercator_latitude(0.0)).abs() < 1e-9);
        let limit = mercator_latitude(MERCATOR_HALF);
        assert!((limit - 85.051_128_779_806_6).abs() < 1e-6, "limit = {limit}");
    }
}
