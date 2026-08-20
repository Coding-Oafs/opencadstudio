// WIPEOUT command — draw a polygonal mask or derive one from a closed polyline.

use acadrust::entities::{Wipeout, WipeoutClipType};
use acadrust::objects::{ObjectType, WipeoutVariables};
use acadrust::types::{Vector2, Vector3};
use acadrust::{CadDocument, EntityType, Handle};
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
    selected_polyline: Option<Handle>,
    frame_mode: i16,
}

#[derive(Clone, Copy, PartialEq)]
enum WipeoutMode {
    Draw,
    Polyline,
    Rectangular,
    Frames,
    ErasePolyline,
}

impl WipeoutCommand {
    pub fn new_polygonal(frame_mode: i16) -> Self {
        Self {
            mode: WipeoutMode::Draw,
            first: None,
            points: Vec::new(),
            plane: WorkingPlane::default(),
            selected_polyline: None,
            frame_mode: frame_mode.clamp(0, 2),
        }
    }

    pub fn new_polyline() -> Self {
        Self {
            mode: WipeoutMode::Polyline,
            first: None,
            points: Vec::new(),
            plane: WorkingPlane::default(),
            selected_polyline: None,
            frame_mode: 1,
        }
    }

    /// Kept for the legacy `WIPEOUT RECTANGULAR` command-line form.
    pub fn new_rectangular() -> Self {
        Self {
            mode: WipeoutMode::Rectangular,
            first: None,
            points: Vec::new(),
            plane: WorkingPlane::default(),
            selected_polyline: None,
            frame_mode: 1,
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
                t!("WIPEOUT  Specify first point or [Frames/Polyline] <Polyline>:").into_owned()
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
            WipeoutMode::Frames => t!(
                "WIPEOUT Frames  Enter frame mode [Off/On/DisplayButNotPlot] <%{mode}>:",
                mode = self.frame_mode
            )
            .into_owned(),
            WipeoutMode::ErasePolyline => {
                t!("WIPEOUT Polyline  Erase source polyline? [Yes/No] <No>:").into_owned()
            }
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        match self.mode {
            WipeoutMode::Draw if self.points.is_empty() => {
                vec![
                    CmdOption::new(t!("Frames").as_ref(), "F"),
                    CmdOption::new(t!("Polyline").as_ref(), "P"),
                ]
            }
            WipeoutMode::Draw => vec![
                CmdOption::new(t!("Undo").as_ref(), "U"),
                CmdOption::new(t!("Close").as_ref(), "C"),
            ],
            WipeoutMode::Frames => vec![
                CmdOption::new(t!("Off").as_ref(), "OFF"),
                CmdOption::new(t!("On").as_ref(), "ON"),
                CmdOption::new(t!("Display but not plot").as_ref(), "D"),
            ],
            WipeoutMode::ErasePolyline => vec![
                CmdOption::new(t!("Yes").as_ref(), "Y"),
                CmdOption::new(t!("No").as_ref(), "N"),
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
            WipeoutMode::Polyline | WipeoutMode::Frames | WipeoutMode::ErasePolyline => {
                CmdResult::NeedPoint
            }
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        match self.mode {
            WipeoutMode::Draw if self.points.is_empty() => {
                self.mode = WipeoutMode::Polyline;
                CmdResult::NeedPoint
            }
            WipeoutMode::Draw if self.points.len() >= 3 => self.finish_draw(),
            WipeoutMode::Frames => {
                CmdResult::Dispatch(format!("WIPEOUTFRAME {}", self.frame_mode))
            }
            WipeoutMode::ErasePolyline => self.finish_polyline(false),
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
            self.selected_polyline = Some(handle);
            self.mode = WipeoutMode::ErasePolyline;
            CmdResult::NeedPoint
        }
    }

    fn wants_text_input(&self) -> bool {
        matches!(
            self.mode,
            WipeoutMode::Draw | WipeoutMode::Frames | WipeoutMode::ErasePolyline
        )
    }

    fn point_step_accepts_keywords(&self) -> bool {
        matches!(
            self.mode,
            WipeoutMode::Draw | WipeoutMode::Frames | WipeoutMode::ErasePolyline
        )
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let text = text.trim().to_ascii_uppercase();
        match self.mode {
            WipeoutMode::Draw => match text.as_str() {
                "F" | "FRAMES" if self.points.is_empty() => {
                    self.mode = WipeoutMode::Frames;
                    Some(CmdResult::NeedPoint)
                }
                "P" | "POLYLINE" if self.points.is_empty() => {
                    self.mode = WipeoutMode::Polyline;
                    Some(CmdResult::NeedPoint)
                }
                "U" | "UNDO" if !self.points.is_empty() => Some(self.undo_point()),
                "C" | "CLOSE" if self.points.len() >= 3 => Some(self.finish_draw()),
                _ => None,
            },
            WipeoutMode::Frames => match text.as_str() {
                "0" | "OFF" => Some(CmdResult::Dispatch("WIPEOUTFRAME 0".into())),
                "1" | "ON" => Some(CmdResult::Dispatch("WIPEOUTFRAME 1".into())),
                "2" | "D" | "DISPLAYBUTNOTPLOT" => {
                    Some(CmdResult::Dispatch("WIPEOUTFRAME 2".into()))
                }
                _ => None,
            },
            WipeoutMode::ErasePolyline => match text.as_str() {
                "Y" | "YES" => Some(self.finish_polyline(true)),
                "N" | "NO" => Some(self.finish_polyline(false)),
                _ => None,
            },
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
            WipeoutMode::Polyline | WipeoutMode::Frames | WipeoutMode::ErasePolyline => None,
        }
    }
}

impl WipeoutCommand {
    fn finish_polyline(&self, erase_source: bool) -> CmdResult {
        self.selected_polyline.map_or(CmdResult::Cancel, |handle| {
            CmdResult::WipeoutFromPolyline {
                handle,
                erase_source,
            }
        })
    }
}

pub(crate) fn wipeout_frame_mode(document: &CadDocument) -> i16 {
    document
        .objects
        .values()
        .find_map(|object| match object {
            ObjectType::WipeoutVariables(value) => Some(value.display_frame),
            _ => None,
        })
        .unwrap_or(1)
        .clamp(0, 2)
}

pub(crate) fn set_wipeout_frame_mode(document: &mut CadDocument, mode: i16) {
    let mode = mode.clamp(0, 2);
    if let Some(value) = document.objects.values_mut().find_map(|object| match object {
        ObjectType::WipeoutVariables(value) => Some(value),
        _ => None,
    }) {
        value.display_frame = mode;
        return;
    }

    let owner = crate::scene::annotative::root_named_dict_handle(document);
    let handle = document.allocate_handle();
    let mut value = WipeoutVariables::new();
    value.handle = handle;
    value.owner = owner;
    value.display_frame = mode;
    document
        .objects
        .insert(handle, ObjectType::WipeoutVariables(value));
    if let Some(ObjectType::Dictionary(dictionary)) = document.objects.get_mut(&owner) {
        dictionary
            .entries
            .retain(|(name, _)| !name.eq_ignore_ascii_case("ACAD_WIPEOUT_VARS"));
        dictionary.add_entry("ACAD_WIPEOUT_VARS", handle);
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

fn boundary_is_simple(points: &[DVec3]) -> bool {
    fn orient(a: DVec3, b: DVec3, c: DVec3) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    fn on_segment(a: DVec3, b: DVec3, p: DVec3) -> bool {
        let eps = 1e-9;
        orient(a, b, p).abs() <= eps
            && p.x >= a.x.min(b.x) - eps
            && p.x <= a.x.max(b.x) + eps
            && p.y >= a.y.min(b.y) - eps
            && p.y <= a.y.max(b.y) + eps
    }
    fn intersects(a: DVec3, b: DVec3, c: DVec3, d: DVec3) -> bool {
        let ab_c = orient(a, b, c);
        let ab_d = orient(a, b, d);
        let cd_a = orient(c, d, a);
        let cd_b = orient(c, d, b);
        ((ab_c > 0.0 && ab_d < 0.0) || (ab_c < 0.0 && ab_d > 0.0))
            && ((cd_a > 0.0 && cd_b < 0.0) || (cd_a < 0.0 && cd_b > 0.0))
            || on_segment(a, b, c)
            || on_segment(a, b, d)
            || on_segment(c, d, a)
            || on_segment(c, d, b)
    }

    let count = points.len();
    for first in 0..count {
        let first_next = (first + 1) % count;
        for second in first + 1..count {
            let second_next = (second + 1) % count;
            if first == second
                || first_next == second
                || second_next == first
                || (first == 0 && second_next == 0)
            {
                continue;
            }
            if intersects(
                points[first],
                points[first_next],
                points[second],
                points[second_next],
            ) {
                return false;
            }
        }
    }
    true
}

fn make_poly_wipeout(points: &[DVec3]) -> Option<EntityType> {
    let points = clean_boundary(points);
    if points.len() < 3 {
        return None;
    }
    if !boundary_is_simple(&points) {
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

/// Build a wipeout boundary from a picked closed polyline without consuming
/// the source entity. Only straight, closed 2D polygonal boundaries qualify.
pub(crate) fn wipeout_from_polyline(entity: &EntityType) -> Option<EntityType> {
    fn from_ocs(points: &[DVec3], normal: (f64, f64, f64)) -> Option<EntityType> {
        let origin = crate::scene::view::transform::ocs_point_to_wcs((0.0, 0.0, 0.0), normal);
        let x = crate::scene::view::transform::ocs_point_to_wcs((1.0, 0.0, 0.0), normal);
        let y = crate::scene::view::transform::ocs_point_to_wcs((0.0, 1.0, 0.0), normal);
        let origin = DVec3::new(origin.0, origin.1, origin.2);
        let plane = WorkingPlane::new(
            origin,
            DVec3::new(x.0, x.1, x.2) - origin,
            DVec3::new(y.0, y.1, y.2) - origin,
        );
        make_poly_wipeout(points).map(|entity| plane.place_entity(entity))
    }

    match entity {
        EntityType::LwPolyline(polyline) => {
            if polyline.vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-12) {
                return None;
            }
            let raw: Vec<DVec3> = polyline
                .vertices
                .iter()
                .map(|vertex| DVec3::new(vertex.location.x, vertex.location.y, polyline.elevation))
                .collect();
            if !polyline.is_closed && !explicitly_closed(&raw) {
                return None;
            }
            from_ocs(
                &raw,
                (polyline.normal.x, polyline.normal.y, polyline.normal.z),
            )
        }
        EntityType::Polyline2D(polyline) => {
            if polyline.vertices.iter().any(|vertex| vertex.bulge.abs() > 1e-12) {
                return None;
            }
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
            from_ocs(
                &raw,
                (polyline.normal.x, polyline.normal.y, polyline.normal.z),
            )
        }
        _ => return None,
    }
}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["WIPEOUT"] });
