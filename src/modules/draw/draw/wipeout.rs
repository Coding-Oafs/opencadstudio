// WIPEOUT command — draw a polygonal mask or derive one from a closed polyline.

use acadrust::entities::{Wipeout, WipeoutClipType};
use acadrust::types::{Vector2, Vector3};
use acadrust::{EntityType, Handle};
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdOption, CmdResult, WorkingPlane};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::scene::model::wire_model::WireModel;

pub const ICON: IconKind = IconKind::Svg(include_bytes!("../../../../assets/icons/wipeout.svg"));

pub fn tool() -> ToolDef {
    ToolDef {
        id: "WIPEOUT",
        label: "Wipeout",
        icon: ICON,
        event: ModuleEvent::Command("WIPEOUT".to_string()),
    }
}

pub struct WipeoutCommand {
    mode: WipeoutMode,
    first: Option<DVec3>,
    points: Vec<DVec3>,
    plane: WorkingPlane,
}

#[derive(Clone, Copy, PartialEq)]
enum WipeoutMode {
    Draw,
    Polyline,
    Rectangular,
}

impl WipeoutCommand {
    pub fn new_polygonal() -> Self {
        Self {
            mode: WipeoutMode::Draw,
            first: None,
            points: Vec::new(),
            plane: WorkingPlane::default(),
        }
    }

    pub fn new_polyline() -> Self {
        Self {
            mode: WipeoutMode::Polyline,
            first: None,
            points: Vec::new(),
            plane: WorkingPlane::default(),
        }
    }

    /// Kept for the legacy `WIPEOUT RECTANGULAR` command-line form.
    pub fn new_rectangular() -> Self {
        Self {
            mode: WipeoutMode::Rectangular,
            first: None,
            points: Vec::new(),
            plane: WorkingPlane::default(),
        }
    }

    fn finish_draw(&self) -> CmdResult {
        let local: Vec<DVec3> = self
            .points
            .iter()
            .map(|point| self.plane.to_local(*point))
            .collect();
        match make_poly_wipeout(&local) {
            Some(entity) => CmdResult::CommitAndExit(self.plane.place_entity(entity)),
            None => CmdResult::NeedPoint,
        }
    }

    fn undo_point(&mut self) -> CmdResult {
        self.points.pop();
        CmdResult::NeedPoint
    }
}

impl CadCommand for WipeoutCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "WIPEOUT"
    }

    fn prompt(&self) -> String {
        match self.mode {
            WipeoutMode::Draw if self.points.is_empty() => {
                t!("WIPEOUT  Specify first point or [Polyline]:").into_owned()
            }
            WipeoutMode::Draw => {
                let n = self.points.len();
                t!(
                    "WIPEOUT  Specify next point or [Undo/Close] (%{n} points):",
                    n = n
                )
                .into_owned()
            }
            WipeoutMode::Polyline => {
                t!("WIPEOUT Polyline  Select a closed planar polyline:").into_owned()
            }
            WipeoutMode::Rectangular if self.first.is_none() => {
                t!("WIPEOUT Rectangular  Specify first corner:").into_owned()
            }
            WipeoutMode::Rectangular => {
                t!("WIPEOUT Rectangular  Specify opposite corner:").into_owned()
            }
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        match self.mode {
            WipeoutMode::Draw if self.points.is_empty() => {
                vec![CmdOption::new(t!("Polyline").as_ref(), "P")]
            }
            WipeoutMode::Draw => vec![
                CmdOption::new(t!("Undo").as_ref(), "U"),
                CmdOption::new(t!("Close").as_ref(), "C"),
            ],
            _ => Vec::new(),
        }
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        match self.mode {
            WipeoutMode::Draw => {
                if let Some(first) = self.points.first() {
                    let distance_squared =
                        (point.x - first.x).powi(2) + (point.y - first.y).powi(2);
                    if self.points.len() >= 3 && distance_squared < 1e-12 {
                        return self.finish_draw();
                    }
                }
                self.points.push(point);
                CmdResult::NeedPoint
            }
            WipeoutMode::Rectangular => {
                if let Some(first) = self.first {
                    let first = self.plane.to_local(first);
                    let point = self.plane.to_local(point);
                    CmdResult::CommitAndExit(
                        self.plane.place_entity(make_rect_wipeout(first, point)),
                    )
                } else {
                    self.first = Some(point);
                    CmdResult::NeedPoint
                }
            }
            WipeoutMode::Polyline => CmdResult::NeedPoint,
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        match self.mode {
            WipeoutMode::Draw if self.points.len() >= 3 => self.finish_draw(),
            _ => CmdResult::Cancel,
        }
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn needs_entity_pick(&self) -> bool {
        self.mode == WipeoutMode::Polyline
    }

    fn on_entity_pick(&mut self, handle: Handle, _point: DVec3) -> CmdResult {
        if handle.is_null() {
            CmdResult::NeedPoint
        } else {
            CmdResult::WipeoutFromPolyline(handle)
        }
    }

    fn wants_text_input(&self) -> bool {
        self.mode == WipeoutMode::Draw
    }

    fn point_step_accepts_keywords(&self) -> bool {
        self.mode == WipeoutMode::Draw
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.mode != WipeoutMode::Draw {
            return None;
        }
        match text.trim().to_ascii_uppercase().as_str() {
            "P" | "POLYLINE" if self.points.is_empty() => {
                self.mode = WipeoutMode::Polyline;
                Some(CmdResult::NeedPoint)
            }
            "U" | "UNDO" if !self.points.is_empty() => {
                Some(self.undo_point())
            }
            "C" | "CLOSE" if self.points.len() >= 3 => {
                Some(self.finish_draw())
            }
            _ => None,
        }
    }

    fn on_undo_step(&mut self) -> Option<CmdResult> {
        if self.mode == WipeoutMode::Draw && !self.points.is_empty() {
            Some(self.undo_point())
        } else {
            None
        }
    }

    fn window_corner_pick(&self) -> bool {
        self.mode == WipeoutMode::Rectangular
    }

    fn window_first_corner(&self) -> Option<DVec3> {
        (self.mode == WipeoutMode::Rectangular)
            .then_some(self.first)
            .flatten()
    }

    fn on_mouse_move(&mut self, point: DVec3) -> Option<WireModel> {
        match self.mode {
            WipeoutMode::Draw => {
                let first = *self.points.first()?;
                let mut preview = self.points.clone();
                preview.push(point);
                preview.push(first);
                Some(WireModel::solid_f64(
                    "wipeout_preview".into(),
                    preview
                        .iter()
                        .map(|point| [point.x, point.y, point.z])
                        .collect(),
                    WireModel::CYAN,
                    false,
                ))
            }
            WipeoutMode::Rectangular => {
                let first = self.first?;
                let corners = {
                    let first_local = self.plane.to_local(first);
                    let point_local = self.plane.to_local(point);
                    [
                        first_local,
                        DVec3::new(point_local.x, first_local.y, first_local.z),
                        DVec3::new(point_local.x, point_local.y, first_local.z),
                        DVec3::new(first_local.x, point_local.y, first_local.z),
                        first_local,
                    ]
                    .map(|corner| self.plane.to_world(corner))
                };
                Some(WireModel::solid_f64(
                    "wipeout_preview".into(),
                    corners.iter().map(|p| [p.x, p.y, p.z]).collect(),
                    WireModel::CYAN,
                    false,
                ))
            }
            WipeoutMode::Polyline => None,
        }
    }
}

fn make_rect_wipeout(first: DVec3, second: DVec3) -> EntityType {
    let corner1 = Vector3::new(
        first.x.min(second.x),
        first.y.min(second.y),
        first.z,
    );
    let corner2 = Vector3::new(
        first.x.max(second.x),
        first.y.max(second.y),
        first.z,
    );
    EntityType::Wipeout(Wipeout::from_corners(corner1, corner2))
}

fn same_point(first: DVec3, second: DVec3) -> bool {
    (first - second).length_squared() < 1e-18
}

fn clean_boundary(points: &[DVec3]) -> Vec<DVec3> {
    let mut clean = Vec::with_capacity(points.len());
    for point in points.iter().copied().filter(|point| point.is_finite()) {
        if clean.last().is_none_or(|last| !same_point(*last, point)) {
            clean.push(point);
        }
    }
    if clean.len() > 1 && same_point(clean[0], *clean.last().unwrap()) {
        clean.pop();
    }
    clean
}

fn make_poly_wipeout(points: &[DVec3]) -> Option<EntityType> {
    let points = clean_boundary(points);
    if points.len() < 3 {
        return None;
    }
    let z = points[0].z;
    if points.iter().any(|point| (point.z - z).abs() > 1e-7) {
        return None;
    }

    let mut min_x = points[0].x;
    let mut min_y = points[0].y;
    let mut max_x = points[0].x;
    let mut max_y = points[0].y;
    for point in points.iter().skip(1) {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    if width < 1e-9 || height < 1e-9 {
        return None;
    }

    // Stable signed area around a local origin rejects collinear boundaries
    // without losing precision on large world coordinates.
    let origin = points[0];
    let mut twice_area = 0.0;
    for index in 0..points.len() {
        let a = points[index] - origin;
        let b = points[(index + 1) % points.len()] - origin;
        twice_area += a.x * b.y - b.x * a.y;
    }
    if twice_area.abs() <= width * height * 1e-10 {
        return None;
    }

    // Wipeout clip vertices use image-pixel coordinates centred on the image:
    // X grows right, Y grows up while the stored V vector points down.
    let clip_boundary_vertices = points
        .iter()
        .map(|point| {
            let normalized_x = (point.x - min_x) / width;
            let normalized_y = (point.y - min_y) / height;
            Vector2::new(normalized_x - 0.5, 0.5 - normalized_y)
        })
        .collect();
    let mut wipeout = Wipeout::new();
    wipeout.insertion_point = Vector3::new(min_x, min_y, z);
    wipeout.u_vector = Vector3::new(width, 0.0, 0.0);
    wipeout.v_vector = Vector3::new(0.0, height, 0.0);
    wipeout.size = Vector2::new(1.0, 1.0);
    wipeout.clip_type = WipeoutClipType::Polygonal;
    wipeout.clip_boundary_vertices = clip_boundary_vertices;
    wipeout.clipping_enabled = true;
    Some(EntityType::Wipeout(wipeout))
}

fn explicitly_closed(points: &[DVec3]) -> bool {
    points.len() >= 4 && same_point(points[0], *points.last().unwrap())
}

fn sample_2d_polyline(
    vertices: &[([f64; 2], f64)],
    elevation: f64,
    normal: (f64, f64, f64),
) -> Vec<DVec3> {
    if vertices.len() < 3 {
        return Vec::new();
    }
    let to_wcs = |point: [f64; 2]| {
        let point = crate::scene::view::transform::ocs_point_to_wcs(
            (point[0], point[1], elevation),
            normal,
        );
        DVec3::new(point.0, point.1, point.2)
    };
    let mut points = vec![to_wcs(vertices[0].0)];
    for index in 0..vertices.len() {
        let (start, bulge) = vertices[index];
        let end_index = (index + 1) % vertices.len();
        let end = vertices[end_index].0;
        if let Some(arc) =
            crate::entities::common::BulgeArc::from_bulge(start, end, bulge)
        {
            let steps = ((arc.sweep.abs() / std::f64::consts::TAU * 64.0).ceil()
                as usize)
                .clamp(4, 64);
            for step in 1..=steps {
                if end_index == 0 && step == steps {
                    break;
                }
                points.push(to_wcs(arc.sample(step as f64 / steps as f64)));
            }
        } else if end_index != 0 {
            points.push(to_wcs(end));
        }
    }
    points
}

/// Build a wipeout boundary from a picked closed polyline without consuming
/// the source entity. Bulged 2D segments are sampled into the polygonal mask.
pub(crate) fn wipeout_from_polyline(entity: &EntityType) -> Option<EntityType> {
    let points = match entity {
        EntityType::LwPolyline(polyline) => {
            let raw: Vec<DVec3> = polyline
                .vertices
                .iter()
                .map(|vertex| DVec3::new(vertex.location.x, vertex.location.y, polyline.elevation))
                .collect();
            if !polyline.is_closed && !explicitly_closed(&raw) {
                return None;
            }
            let vertices: Vec<([f64; 2], f64)> = polyline
                .vertices
                .iter()
                .map(|vertex| {
                    (
                        [vertex.location.x, vertex.location.y],
                        vertex.bulge,
                    )
                })
                .collect();
            sample_2d_polyline(
                &vertices,
                polyline.elevation,
                (
                    polyline.normal.x,
                    polyline.normal.y,
                    polyline.normal.z,
                ),
            )
        }
        EntityType::Polyline2D(polyline) => {
            let raw: Vec<DVec3> = polyline
                .vertices
                .iter()
                .map(|vertex| {
                    DVec3::new(
                        vertex.location.x,
                        vertex.location.y,
                        polyline.elevation,
                    )
                })
                .collect();
            if !polyline.is_closed() && !explicitly_closed(&raw) {
                return None;
            }
            let vertices: Vec<([f64; 2], f64)> = polyline
                .vertices
                .iter()
                .map(|vertex| {
                    (
                        [vertex.location.x, vertex.location.y],
                        vertex.bulge,
                    )
                })
                .collect();
            sample_2d_polyline(
                &vertices,
                polyline.elevation,
                (
                    polyline.normal.x,
                    polyline.normal.y,
                    polyline.normal.z,
                ),
            )
        }
        EntityType::Polyline(polyline) => {
            let points: Vec<DVec3> = polyline
                .vertices
                .iter()
                .map(|vertex| {
                    DVec3::new(
                        vertex.location.x,
                        vertex.location.y,
                        vertex.location.z,
                    )
                })
                .collect();
            if !polyline.is_closed() && !explicitly_closed(&points) {
                return None;
            }
            points
        }
        EntityType::Polyline3D(polyline) => {
            let points: Vec<DVec3> = polyline
                .vertices
                .iter()
                .map(|vertex| {
                    DVec3::new(
                        vertex.position.x,
                        vertex.position.y,
                        vertex.position.z,
                    )
                })
                .collect();
            if !polyline.flags.closed && !explicitly_closed(&points) {
                return None;
            }
            points
        }
        _ => return None,
    };
    make_poly_wipeout(&points)
}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["WIPEOUT"] });
