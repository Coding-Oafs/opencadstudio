// Shapes dropdown — Rectangle and Polygon creation methods.
//
// Rectangle:
//   RECT     — Two Corners (axis-aligned)
//   RECT_ROT — Rotated (corner + adjacent corner + height)
//   RECT_CEN — Center Point + corner
//
// Polygon (regular N-gon, sides typed in command line):
//   POLY   — Inscribed in circle   (vertices ON the circle)
//   POLY_C — Circumscribed about circle (edges tangent to circle)
//   POLY_E — Edge (pick two endpoints of one edge)

use acadrust::entities::LwVertex;
use acadrust::types::Vector2;
use acadrust::{EntityType, LwPolyline};
use crate::t;

use crate::command::{CadCommand, CmdResult, WorkingPlane};
use crate::modules::draw::defaults;
use crate::modules::IconKind;
use crate::scene::model::wire_model::WireModel;
use glam::DVec3;

/// Four corners of a box centred at `c` with half-extents taken from `corner`,
/// axis-aligned in the active UCS (`ucs` = UCS→wire affine, identity = world).
fn ucs_box_around_center(c: DVec3, corner: DVec3, plane: WorkingPlane) -> [DVec3; 4] {
    let d = plane.vector_to_local(corner - c);
    let rx = plane.x * d.x.abs();
    let ry = plane.y * d.y.abs();
    [c - rx - ry, c + rx - ry, c + rx + ry, c - rx + ry]
}

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

// ── Icons ──────────────────────────────────────────────────────────────────

const ICON_RECT: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/shapes/rect.svg"));
const ICON_RECT_ROT: IconKind = IconKind::Svg(include_bytes!(
    "../../../../assets/icons/shapes/rect_rot.svg"
));
const ICON_RECT_CEN: IconKind = IconKind::Svg(include_bytes!(
    "../../../../assets/icons/shapes/rect_cen.svg"
));
const ICON_POLY_I: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/shapes/poly_i.svg"));
const ICON_POLY_C: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/shapes/poly_c.svg"));
const ICON_POLY_E: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/shapes/poly_e.svg"));

// ── Dropdown metadata ──────────────────────────────────────────────────────

pub const DROPDOWN_ID: &str = "SHAPES";

pub const DROPDOWN_ITEMS: &[(&str, &str, IconKind)] = &[
    ("RECT", "Rectangle - Two Corners", ICON_RECT),
    ("RECT_ROT", "Rectangle - Rotated", ICON_RECT_ROT),
    ("RECT_CEN", "Rectangle - Center", ICON_RECT_CEN),
    ("POLY", "Polygon - Inscribed", ICON_POLY_I),
    ("POLY_C", "Polygon - Circumscribed", ICON_POLY_C),
    ("POLY_E", "Polygon - Edge", ICON_POLY_E),
];

pub const ICON: IconKind = ICON_RECT;

// ── Shared geometry helpers ────────────────────────────────────────────────

fn make_pline(points: &[DVec3], plane: WorkingPlane) -> EntityType {
    let local: Vec<DVec3> = points.iter().map(|point| plane.to_local(*point)).collect();
    let elevation = local.first().map_or(0.0, |point| point.z);
    plane.place_entity(EntityType::LwPolyline(LwPolyline {
        vertices: local
            .iter()
            .map(|point| LwVertex::new(Vector2::new(point.x, point.y)))
            .collect(),
        elevation,
        is_closed: true,
        ..Default::default()
    }))
}

#[derive(Clone, Copy)]
struct RectStyle {
    chamfer_first: f64,
    chamfer_second: f64,
    fillet_radius: f64,
    width: f64,
    thickness: f64,
}

fn rectangle_corners(
    first: DVec3,
    cursor: DVec3,
    plane: WorkingPlane,
    rotation_deg: f64,
    fixed_dimensions: Option<(f64, f64)>,
) -> Option<[DVec3; 4]> {
    let first_local = plane.to_local(first);
    let cursor_local = plane.to_local(cursor);
    let angle = rotation_deg.to_radians();
    let axis_x = DVec3::new(angle.cos(), angle.sin(), 0.0);
    let axis_y = DVec3::new(-angle.sin(), angle.cos(), 0.0);
    let delta = cursor_local - first_local;
    let raw_width = delta.dot(axis_x);
    let raw_height = delta.dot(axis_y);
    let (width, height) = fixed_dimensions.map_or((raw_width, raw_height), |(w, h)| {
        (w.copysign(raw_width), h.copysign(raw_height))
    });
    if width.abs() <= 1.0e-9 || height.abs() <= 1.0e-9 {
        return None;
    }
    let local = [
        first_local,
        first_local + axis_x * width,
        first_local + axis_x * width + axis_y * height,
        first_local + axis_y * height,
    ];
    Some(local.map(|point| plane.to_world(point)))
}

fn trimmed_rectangle_vertices(
    corners: [DVec3; 4],
    plane: WorkingPlane,
    style: RectStyle,
) -> Vec<(DVec3, f64)> {
    let use_fillet = style.fillet_radius > 1.0e-9;
    let use_chamfer = !use_fillet
        && (style.chamfer_first > 1.0e-9 || style.chamfer_second > 1.0e-9);
    if !use_fillet && !use_chamfer {
        return corners.into_iter().map(|point| (point, 0.0)).collect();
    }

    let local = corners.map(|point| plane.to_local(point));
    let area_twice: f64 = (0..4)
        .map(|index| {
            let a = local[index];
            let b = local[(index + 1) % 4];
            a.x * b.y - b.x * a.y
        })
        .sum();
    let bulge = area_twice.signum() * (std::f64::consts::PI / 8.0).tan();
    let mut out = Vec::with_capacity(8);
    for index in 0..4 {
        let corner = corners[index];
        let previous = corners[(index + 3) % 4];
        let next = corners[(index + 1) % 4];
        let incoming_length = (previous - corner).length();
        let outgoing_length = (next - corner).length();
        let max_trim = incoming_length.min(outgoing_length) * 0.499_999;
        let (incoming_trim, outgoing_trim, arc_bulge) = if use_fillet {
            let trim = style.fillet_radius.min(max_trim);
            (trim, trim, bulge)
        } else {
            (
                style.chamfer_first.min(max_trim),
                style.chamfer_second.min(max_trim),
                0.0,
            )
        };
        let incoming = corner + (previous - corner).normalize() * incoming_trim;
        let outgoing = corner + (next - corner).normalize() * outgoing_trim;
        out.push((incoming, arc_bulge));
        out.push((outgoing, 0.0));
    }
    out
}

fn make_rect_pline(corners: [DVec3; 4], plane: WorkingPlane, style: RectStyle) -> EntityType {
    let points = trimmed_rectangle_vertices(corners, plane, style);
    let local: Vec<(DVec3, f64)> = points
        .into_iter()
        .map(|(point, bulge)| (plane.to_local(point), bulge))
        .collect();
    let elevation = local.first().map_or(0.0, |(point, _)| point.z);
    let mut polyline = LwPolyline {
        vertices: local
            .iter()
            .map(|(point, bulge)| {
                let mut vertex = LwVertex::new(Vector2::new(point.x, point.y));
                vertex.bulge = *bulge;
                vertex
            })
            .collect(),
        elevation,
        is_closed: true,
        constant_width: style.width,
        thickness: style.thickness,
        ..Default::default()
    };
    let mut marker = acadrust::xdata::ExtendedDataRecord::new("OCS_RECTANGLE");
    marker.add_value(acadrust::xdata::XDataValue::Integer16(1));
    polyline.common.extended_data.add_record(marker);
    plane.place_entity(EntityType::LwPolyline(polyline))
}

fn rectangle_wire(corners: [DVec3; 4], plane: WorkingPlane, style: RectStyle) -> WireModel {
    let vertices = trimmed_rectangle_vertices(corners, plane, style);
    let mut points = Vec::new();
    for index in 0..vertices.len() {
        let (start, bulge) = vertices[index];
        let end = vertices[(index + 1) % vertices.len()].0;
        points.push([start.x, start.y, start.z]);
        if bulge.abs() > 1.0e-9 {
            let a = plane.to_local(start);
            let b = plane.to_local(end);
            if let Some(arc) = crate::entities::common::BulgeArc::from_bulge(
                [a.x, a.y],
                [b.x, b.y],
                bulge,
            ) {
                for step in 1..8 {
                    let point = arc.sample(step as f64 / 8.0);
                    let world = plane.to_world(DVec3::new(point[0], point[1], a.z));
                    points.push([world.x, world.y, world.z]);
                }
            }
        }
    }
    wire_loop(points)
}

fn wire_loop(pts: Vec<[f64; 3]>) -> WireModel {
    let mut p = pts;
    if let Some(&first) = p.first() {
        p.push(first);
    }
    WireModel::solid_f64("rubber_band".into(), p, WireModel::CYAN, false)
}

fn wire_seg(a: DVec3, b: DVec3) -> WireModel {
    WireModel::solid_f64(
        "rubber_band".into(),
        vec![[a.x, a.y, a.z], [b.x, b.y, b.z]],
        WireModel::CYAN,
        false,
    )
}

// ── Polygon geometry ───────────────────────────────────────────────────────

fn poly_verts(
    center: DVec3,
    vertex_r: f64,
    sides: u32,
    start_angle: f64,
    plane: WorkingPlane,
) -> Vec<DVec3> {
    (0..sides)
        .map(|i| {
            let a = start_angle + (i as f64) * TAU / sides as f64;
            center + plane.x * (vertex_r * a.cos()) + plane.y * (vertex_r * a.sin())
        })
        .collect()
}

fn poly_wire(
    center: DVec3,
    vertex_r: f64,
    sides: u32,
    start_angle: f64,
    plane: WorkingPlane,
) -> WireModel {
    let pts: Vec<[f64; 3]> = poly_verts(center, vertex_r, sides, start_angle, plane)
        .into_iter()
        .map(|point| [point.x, point.y, point.z])
        .collect();
    wire_loop(pts)
}

fn angle_xy(from: DVec3, to: DVec3, plane: WorkingPlane) -> f64 {
    plane.angle(from, to).unwrap_or(0.0)
}

fn plane_distance(from: DVec3, to: DVec3, plane: WorkingPlane) -> f64 {
    let delta = plane.vector_to_local(to - from);
    delta.x.hypot(delta.y)
}

// ── Command: Rectangle — Two Corners  (RECT) ──────────────────────────────

#[derive(Clone, Copy)]
enum RectStep {
    FirstCorner,
    Opposite,
    ChamferFirst,
    ChamferSecond,
    Elevation,
    Fillet,
    Thickness,
    Width,
    Rotation,
    AreaValue,
    AreaBasis(f64),
    AreaDimension { area: f64, by_length: bool },
    DimensionsLength,
    DimensionsWidth(f64),
    PlaceSized { width: f64, height: f64 },
}

pub struct RectCommand {
    step: RectStep,
    first: Option<DVec3>,
    plane: WorkingPlane,
    chamfer_first: f64,
    chamfer_second: f64,
    fillet_radius: f64,
    elevation: f64,
    thickness: f64,
    width: f64,
    rotation_deg: f64,
}

impl RectCommand {
    pub fn new() -> Self {
        Self {
            step: RectStep::FirstCorner,
            first: None,
            plane: WorkingPlane::default(),
            chamfer_first: defaults::get_rect_chamfer1().max(0.0),
            chamfer_second: defaults::get_rect_chamfer2().max(0.0),
            fillet_radius: defaults::get_rect_fillet().max(0.0),
            elevation: defaults::get_rect_elevation(),
            thickness: defaults::get_rect_thickness(),
            width: defaults::get_rect_width().max(0.0),
            rotation_deg: defaults::get_rect_rotation(),
        }
    }

    fn style(&self) -> RectStyle {
        RectStyle {
            chamfer_first: self.chamfer_first,
            chamfer_second: self.chamfer_second,
            fillet_radius: self.fillet_radius,
            width: self.width,
            thickness: self.thickness,
        }
    }

    fn finish(&self, cursor: DVec3, fixed_dimensions: Option<(f64, f64)>) -> CmdResult {
        let Some(first) = self.first else {
            return CmdResult::NeedPoint;
        };
        let Some(corners) = rectangle_corners(
            first,
            cursor,
            self.plane,
            self.rotation_deg,
            fixed_dimensions,
        ) else {
            return CmdResult::NeedPoint;
        };
        CmdResult::CommitAndExit(make_rect_pline(corners, self.plane, self.style()))
    }
}

impl CadCommand for RectCommand {
    fn name(&self) -> &'static str {
        "RECT"
    }
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn prompt(&self) -> String {
        match self.step {
            RectStep::FirstCorner => crate::t!("RECT  Specify first corner:").into_owned(),
            RectStep::Opposite => crate::t!("RECT  Specify opposite corner:").into_owned(),
            RectStep::ChamferFirst => format!(
                "RECT  Specify first chamfer distance <{}>:",
                crate::entities::common::format_length(self.chamfer_first)
            ),
            RectStep::ChamferSecond => format!(
                "RECT  Specify second chamfer distance <{}>:",
                crate::entities::common::format_length(self.chamfer_second)
            ),
            RectStep::Elevation => {
                format!(
                    "RECT  Specify elevation <{}>:",
                    crate::entities::common::format_length(self.elevation)
                )
            }
            RectStep::Fillet => {
                format!(
                    "RECT  Specify fillet radius <{}>:",
                    crate::entities::common::format_length(self.fillet_radius)
                )
            }
            RectStep::Thickness => {
                format!(
                    "RECT  Specify thickness <{}>:",
                    crate::entities::common::format_length(self.thickness)
                )
            }
            RectStep::Width => format!(
                "RECT  Specify width <{}>:",
                crate::entities::common::format_length(self.width)
            ),
            RectStep::Rotation => {
                format!(
                    "RECT  Specify rotation angle <{}>:",
                    crate::entities::common::format_angle(self.rotation_deg.to_radians())
                )
            }
            RectStep::AreaValue => "RECT  Specify rectangle area:".to_string(),
            RectStep::AreaBasis(_) => {
                "RECT  Calculate dimensions based on [Length / Width] <Length>:".to_string()
            }
            RectStep::AreaDimension { by_length: true, .. } => {
                "RECT  Specify rectangle length:".to_string()
            }
            RectStep::AreaDimension { by_length: false, .. } => {
                "RECT  Specify rectangle width:".to_string()
            }
            RectStep::DimensionsLength => "RECT  Specify rectangle length:".to_string(),
            RectStep::DimensionsWidth(_) => "RECT  Specify rectangle width:".to_string(),
            RectStep::PlaceSized { .. } => {
                "RECT  Specify orientation from the first corner:".to_string()
            }
        }
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        match self.step {
            RectStep::FirstCorner => vec![
                CmdOption::new("Chamfer", "CHAMFER"),
                CmdOption::new("Elevation", "ELEVATION"),
                CmdOption::new("Fillet", "FILLET"),
                CmdOption::new("Thickness", "THICKNESS"),
                CmdOption::new("Width", "WIDTH"),
            ],
            RectStep::Opposite => vec![
                CmdOption::new("Area", "AREA"),
                CmdOption::new("Dimensions", "DIMENSIONS"),
                CmdOption::new("Rotation", "ROTATION"),
            ],
            RectStep::AreaBasis(_) => vec![
                CmdOption::new("Length", "LENGTH"),
                CmdOption::new("Width", "WIDTH"),
            ],
            _ => vec![],
        }
    }

    fn point_step_accepts_keywords(&self) -> bool {
        matches!(self.step, RectStep::FirstCorner | RectStep::Opposite)
    }

    fn wants_text_input(&self) -> bool {
        !matches!(self.step, RectStep::PlaceSized { .. })
    }

    fn dyn_field(&self) -> crate::command::DynField {
        match self.step {
            RectStep::Rotation => crate::command::DynField::Angle,
            RectStep::ChamferFirst
            | RectStep::ChamferSecond
            | RectStep::Elevation
            | RectStep::Fillet
            | RectStep::Thickness
            | RectStep::Width
            | RectStep::AreaValue
            | RectStep::AreaBasis(_)
            | RectStep::AreaDimension { .. }
            | RectStep::DimensionsLength
            | RectStep::DimensionsWidth(_) => crate::command::DynField::Scalar,
            _ => crate::command::DynField::Point,
        }
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let upper = text.trim().to_uppercase();
        match self.step {
            RectStep::FirstCorner => {
                return match upper.as_str() {
                    "C" | "CHAMFER" => {
                        self.step = RectStep::ChamferFirst;
                        Some(CmdResult::NeedPoint)
                    }
                    "E" | "ELEVATION" => {
                        self.step = RectStep::Elevation;
                        Some(CmdResult::NeedPoint)
                    }
                    "F" | "FILLET" => {
                        self.step = RectStep::Fillet;
                        Some(CmdResult::NeedPoint)
                    }
                    "T" | "THICKNESS" => {
                        self.step = RectStep::Thickness;
                        Some(CmdResult::NeedPoint)
                    }
                    "W" | "WIDTH" => {
                        self.step = RectStep::Width;
                        Some(CmdResult::NeedPoint)
                    }
                    _ => None,
                };
            }
            RectStep::Opposite => {
                return match upper.as_str() {
                    "A" | "AREA" => {
                        self.step = RectStep::AreaValue;
                        Some(CmdResult::NeedPoint)
                    }
                    "D" | "DIMENSIONS" => {
                        self.step = RectStep::DimensionsLength;
                        Some(CmdResult::NeedPoint)
                    }
                    "R" | "ROTATION" => {
                        self.step = RectStep::Rotation;
                        Some(CmdResult::NeedPoint)
                    }
                    _ => None,
                };
            }
            RectStep::AreaBasis(area) => {
                return match upper.as_str() {
                    "L" | "LENGTH" => {
                        self.step = RectStep::AreaDimension {
                            area,
                            by_length: true,
                        };
                        Some(CmdResult::NeedPoint)
                    }
                    "W" | "WIDTH" => {
                        self.step = RectStep::AreaDimension {
                            area,
                            by_length: false,
                        };
                        Some(CmdResult::NeedPoint)
                    }
                    _ => None,
                };
            }
            _ => {}
        }

        if matches!(self.step, RectStep::Rotation) {
            let angle = crate::entities::common::parse_typed_angle(text)?;
            self.rotation_deg = angle.to_degrees();
            defaults::set_rect_rotation(self.rotation_deg);
            self.step = RectStep::Opposite;
            return Some(CmdResult::NeedPoint);
        }
        let value = if matches!(self.step, RectStep::AreaValue) {
            text.trim().replace(',', ".").parse().ok()?
        } else {
            crate::entities::common::parse_typed_length(text)?
        };
        match self.step {
            RectStep::ChamferFirst if value >= 0.0 => {
                self.chamfer_first = value;
                defaults::set_rect_chamfer1(value);
                self.step = RectStep::ChamferSecond;
            }
            RectStep::ChamferSecond if value >= 0.0 => {
                self.chamfer_second = value;
                self.fillet_radius = 0.0;
                defaults::set_rect_chamfer2(value);
                defaults::set_rect_fillet(0.0);
                self.step = RectStep::FirstCorner;
            }
            RectStep::Elevation => {
                self.elevation = value;
                defaults::set_rect_elevation(value);
                self.step = RectStep::FirstCorner;
            }
            RectStep::Fillet if value >= 0.0 => {
                self.fillet_radius = value;
                self.chamfer_first = 0.0;
                self.chamfer_second = 0.0;
                defaults::set_rect_fillet(value);
                defaults::set_rect_chamfer1(0.0);
                defaults::set_rect_chamfer2(0.0);
                self.step = RectStep::FirstCorner;
            }
            RectStep::Thickness => {
                self.thickness = value;
                defaults::set_rect_thickness(value);
                self.step = RectStep::FirstCorner;
            }
            RectStep::Width if value >= 0.0 => {
                self.width = value;
                defaults::set_rect_width(value);
                self.step = RectStep::FirstCorner;
            }
            RectStep::AreaValue if value > 0.0 => {
                self.step = RectStep::AreaBasis(value);
            }
            RectStep::AreaDimension { area, by_length } if value > 0.0 => {
                let (width, height) = if by_length {
                    (value, area / value)
                } else {
                    (area / value, value)
                };
                self.step = RectStep::PlaceSized { width, height };
            }
            RectStep::DimensionsLength if value > 0.0 => {
                self.step = RectStep::DimensionsWidth(value);
            }
            RectStep::DimensionsWidth(length) if value > 0.0 => {
                self.step = RectStep::PlaceSized {
                    width: length,
                    height: value,
                };
            }
            _ => return None,
        }
        Some(CmdResult::NeedPoint)
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            RectStep::FirstCorner => {
                let mut local = self.plane.to_local(pt);
                local.z = self.elevation;
                self.first = Some(self.plane.to_world(local));
                self.step = RectStep::Opposite;
                CmdResult::NeedPoint
            }
            RectStep::Opposite => self.finish(pt, None),
            RectStep::PlaceSized { width, height } => {
                self.finish(pt, Some((width, height)))
            }
            RectStep::Rotation => {
                let Some(first) = self.first else {
                    return CmdResult::NeedPoint;
                };
                let delta = self.plane.vector_to_local(pt - first);
                if delta.x.hypot(delta.y) <= 1.0e-9 {
                    return CmdResult::NeedPoint;
                }
                self.rotation_deg = delta.y.atan2(delta.x).to_degrees();
                defaults::set_rect_rotation(self.rotation_deg);
                self.step = RectStep::Opposite;
                CmdResult::NeedPoint
            }
            _ => CmdResult::NeedPoint,
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        match self.step {
            RectStep::FirstCorner => CmdResult::Cancel,
            RectStep::ChamferFirst => {
                self.step = RectStep::ChamferSecond;
                CmdResult::NeedPoint
            }
            RectStep::ChamferSecond => {
                self.fillet_radius = 0.0;
                defaults::set_rect_fillet(0.0);
                self.step = RectStep::FirstCorner;
                CmdResult::NeedPoint
            }
            RectStep::Elevation
            | RectStep::Fillet
            | RectStep::Thickness
            | RectStep::Width => {
                self.step = RectStep::FirstCorner;
                CmdResult::NeedPoint
            }
            RectStep::Rotation => {
                self.step = RectStep::Opposite;
                CmdResult::NeedPoint
            }
            RectStep::AreaBasis(area) => {
                self.step = RectStep::AreaDimension {
                    area,
                    by_length: true,
                };
                CmdResult::NeedPoint
            }
            _ => CmdResult::NeedPoint,
        }
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        let first = self.first?;
        let fixed_dimensions = match self.step {
            RectStep::PlaceSized { width, height } => Some((width, height)),
            RectStep::Opposite => None,
            _ => return None,
        };
        let corners = rectangle_corners(
            first,
            pt,
            self.plane,
            self.rotation_deg,
            fixed_dimensions,
        )?;
        Some(rectangle_wire(corners, self.plane, self.style()))
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        if !matches!(self.step, RectStep::Opposite) || self.rotation_deg.abs() > 1.0e-9 {
            return None;
        }
        self.first.map(|first| DynSpec {
            anchor: DynAnchor::Point(first),
            fields: vec![
                DynFieldSpec::new(DynRole::Width),
                DynFieldSpec::new(DynRole::Height),
            ],
            guide: DynGuide::RectSides,
            ref_point: None,
        })
    }
}

// ── Command: Rectangle — Rotated  (RECT_ROT) ──────────────────────────────
//   Step 0: pick corner A
//   Step 1: pick adjacent corner B  (defines one edge direction + length)
//   Step 2: pick height point  (perpendicular offset from the A-B edge)

pub struct RectRotCommand {
    step: u8,
    a: DVec3,
    b: DVec3,
    plane: WorkingPlane,
}

impl RectRotCommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            a: DVec3::ZERO,
            b: DVec3::ZERO,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for RectRotCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "RECT_ROT"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => crate::t!("RECT ROT  Specify first corner:").into_owned(),
            1 => crate::t!("RECT ROT  Specify adjacent corner (defines edge direction):").into_owned(),
            _ => crate::t!("RECT ROT  Specify height (perpendicular pick):").into_owned(),
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.a = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                if plane_distance(self.a, pt, self.plane) <= 1.0e-9 {
                    return CmdResult::NeedPoint;
                }
                self.b = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let (a, b, pt) = (
                    self.plane.to_local(self.a),
                    self.plane.to_local(self.b),
                    self.plane.to_local(pt),
                );
                let dir = (b - a).normalize_or_zero();
                let perp = DVec3::new(-dir.y, dir.x, 0.0);
                let h = (pt - b).dot(perp); // signed height
                if h.abs() <= 1.0e-9 {
                    return CmdResult::NeedPoint;
                }
                let c = b + perp * h;
                let d = a + perp * h;
                let corners = [a, b, c, d].map(|point| self.plane.to_world(point));
                CmdResult::CommitAndExit(make_pline(&corners, self.plane))
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(wire_seg(self.a, pt)),
            2 => {
                let (a, b, pt) = (
                    self.plane.to_local(self.a),
                    self.plane.to_local(self.b),
                    self.plane.to_local(pt),
                );
                let dir = (b - a).normalize_or_zero();
                let perp = DVec3::new(-dir.y, dir.x, 0.0);
                let h = (pt - b).dot(perp);
                let c = b + perp * h;
                let d = a + perp * h;
                let points = [a, b, c, d].map(|point| self.plane.to_world(point));
                Some(wire_loop(
                    points.into_iter().map(|p| [p.x, p.y, p.z]).collect(),
                ))
            }
            _ => None,
        }
    }

    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Step 0: corner A (point). Step 1: adjacent corner — the base edge,
        // needs direction + length (legacy polar). Step 2: height — measured
        // square to the fixed base edge A→B, so show the perpendicular drop
        // and take the perpendicular distance (no angle).
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.b),
            fields: vec![DynFieldSpec::new(DynRole::Distance)],
            guide: DynGuide::PerpDim,
            ref_point: Some(self.a),
        })
    }

    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        // Live height = perpendicular distance from the cursor to the base edge.
        (self.step == 2).then(|| {
            let dir = self
                .plane
                .vector_to_local(self.b - self.a)
                .normalize_or_zero();
            let perp = DVec3::new(-dir.y, dir.x, 0.0);
            self.plane.vector_to_local(cursor - self.b).dot(perp).abs()
        })
    }
}

// ── Command: Rectangle — Center  (RECT_CEN) ───────────────────────────────
//   Step 0: pick center
//   Step 1: pick any corner  (half-width = |cx|, half-height = |cy|)

pub struct RectCenCommand {
    center: Option<DVec3>,
    plane: WorkingPlane,
}

impl RectCenCommand {
    pub fn new() -> Self {
        Self {
            center: None,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for RectCenCommand {
    fn name(&self) -> &'static str {
        "RECT_CEN"
    }
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn prompt(&self) -> String {
        if self.center.is_none() {
            t!("RECT CEN  Specify center point:").into_owned()
        } else {
            t!("RECT CEN  Specify corner point:").into_owned()
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.center {
            None => {
                self.center = Some(pt);
                CmdResult::NeedPoint
            }
            Some(c) => {
                let delta = self.plane.vector_to_local(pt - c);
                if delta.x.abs() <= 1.0e-9 || delta.y.abs() <= 1.0e-9 {
                    return CmdResult::NeedPoint;
                }
                let q = ucs_box_around_center(c, pt, self.plane);
                CmdResult::CommitAndExit(make_pline(&q, self.plane))
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        let c = self.center?;
        let q = ucs_box_around_center(c, pt, self.plane);
        Some(wire_loop(vec![
            [q[0].x, q[0].y, q[0].z],
            [q[1].x, q[1].y, q[1].z],
            [q[2].x, q[2].y, q[2].z],
            [q[3].x, q[3].y, q[3].z],
        ]))
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Corner from the centre gives the half-width / half-height; show them
        // on dotted axis legs out of the centre.
        self.center.map(|c| DynSpec {
            anchor: DynAnchor::Point(c),
            fields: vec![
                DynFieldSpec::new(DynRole::Width),
                DynFieldSpec::new(DynRole::Height),
            ],
            guide: DynGuide::AxisDelta,
            ref_point: None,
        })
    }
}

// ── Command: Polygon — Inscribed  (POLY) ──────────────────────────────────
//   Type number of sides (default 6) → pick center → pick vertex
//   Vertices lie ON the circle of the picked radius.

pub struct PolyCommand {
    sides: u32,
    step: u8,
    center: DVec3,
    plane: WorkingPlane,
}

impl PolyCommand {
    pub fn new() -> Self {
        Self {
            sides: defaults::get_polygon_sides() as u32,
            step: 0,
            center: DVec3::ZERO,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for PolyCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "POLY"
    }

    fn wants_text_input(&self) -> bool {
        self.step == 0
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        // The sides step also offers the alternate polygon methods; later steps
        // are plain point picks. (#304)
        if self.step == 0 {
            vec![
                CmdOption::new("Circumscribed", "CIRCUMSCRIBED"),
                CmdOption::new("Edge", "EDGE"),
            ]
        } else {
            vec![]
        }
    }

    fn point_step_accepts_keywords(&self) -> bool {
        self.step == 0
    }

    fn dyn_field(&self) -> crate::command::DynField {
        if self.step == 0 {
            crate::command::DynField::Scalar
        } else {
            crate::command::DynField::Point
        }
    }

    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Vertex on the circle: radius from the centre + rotation angle.
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.center),
            fields: vec![
                DynFieldSpec::new(DynRole::Radius),
                DynFieldSpec::new(DynRole::Angle),
            ],
            guide: DynGuide::Polar,
            ref_point: None,
        })
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        // At the sides step, keyword options hand off to the dedicated variant
        // command; numeric input still sets the side count. (#304)
        if self.step == 0 {
            match text.trim().to_uppercase().as_str() {
                "C" | "CIRCUMSCRIBED" => return Some(CmdResult::Dispatch("POLY_C".into())),
                "E" | "EDGE" => return Some(CmdResult::Dispatch("POLY_E".into())),
                _ => {}
            }
        }
        if let Ok(n) = text.trim().parse::<u32>() {
            if (3..=1024).contains(&n) {
                self.sides = n;
                defaults::set_polygon_sides(n as f64);
            }
        }
        self.step = 1;
        Some(CmdResult::NeedPoint)
    }

    fn prompt(&self) -> String {
        match self.step {
            0 => crate::tf!("POLYGON  Enter number of sides <{}>:", self.sides).into_owned(),
            1 => crate::tf!("POLYGON  Specify center [{} sides]:", self.sides).into_owned(),
            _ => crate::tf!(
                "POLYGON  Specify vertex on circle [{} sides inscribed]:",
                self.sides
            )
            .into_owned(),
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                // User clicked without typing sides: use default, treat click as center.
                self.center = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            1 => {
                self.center = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let r = plane_distance(self.center, pt, self.plane);
                let sa = angle_xy(self.center, pt, self.plane);
                let vertices = poly_verts(self.center, r, self.sides, sa, self.plane);
                CmdResult::CommitAndExit(make_pline(&vertices, self.plane))
            }
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.step == 0 {
            self.step = 1;
            return CmdResult::NeedPoint;
        }
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        if self.step < 2 {
            return None;
        }
        let r = plane_distance(self.center, pt, self.plane);
        let sa = angle_xy(self.center, pt, self.plane);
        Some(poly_wire(self.center, r, self.sides, sa, self.plane))
    }
}

// ── Command: Polygon — Circumscribed  (POLY_C) ────────────────────────────
//   Type sides → pick center → pick edge-midpoint (on the inscribed circle).
//   vertex_radius = inradius / cos(π/N).

pub struct PolyCCommand {
    sides: u32,
    step: u8,
    center: DVec3,
    plane: WorkingPlane,
}

impl PolyCCommand {
    pub fn new() -> Self {
        Self {
            sides: defaults::get_polygon_sides() as u32,
            step: 0,
            center: DVec3::ZERO,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for PolyCCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "POLY_C"
    }

    fn wants_text_input(&self) -> bool {
        self.step == 0
    }

    fn dyn_field(&self) -> crate::command::DynField {
        if self.step == 0 {
            crate::command::DynField::Scalar
        } else {
            crate::command::DynField::Point
        }
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if let Ok(n) = text.trim().parse::<u32>() {
            if (3..=1024).contains(&n) {
                self.sides = n;
                defaults::set_polygon_sides(n as f64);
            }
        }
        self.step = 1;
        Some(CmdResult::NeedPoint)
    }

    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Edge-midpoint distance (apothem) from the centre + rotation.
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.center),
            fields: vec![
                DynFieldSpec::new(DynRole::Radius),
                DynFieldSpec::new(DynRole::Angle),
            ],
            guide: DynGuide::Polar,
            ref_point: None,
        })
    }

    fn prompt(&self) -> String {
        match self.step {
            0 => crate::tf!("POLYGON C  Enter number of sides <{}>:", self.sides).into_owned(),
            1 => crate::tf!("POLYGON C  Specify center [{} sides]:", self.sides).into_owned(),
            _ => crate::tf!(
                "POLYGON C  Specify edge-midpoint radius [{} sides circumscribed]:",
                self.sides
            )
            .into_owned(),
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.center = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            1 => {
                self.center = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let inradius = plane_distance(self.center, pt, self.plane);
                let vr = inradius / (PI / self.sides as f64).cos();
                // The picked pt is at the midpoint of an edge; the vertex is
                // offset by half a sector (π/N) from that direction.
                let edge_angle = angle_xy(self.center, pt, self.plane);
                let sa = edge_angle + PI / self.sides as f64;
                let vertices = poly_verts(
                    self.center,
                    vr,
                    self.sides,
                    sa,
                    self.plane,
                );
                CmdResult::CommitAndExit(make_pline(&vertices, self.plane))
            }
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.step == 0 {
            self.step = 1;
            return CmdResult::NeedPoint;
        }
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        if self.step < 2 {
            return None;
        }
        let inradius = plane_distance(self.center, pt, self.plane);
        let vr = inradius / (PI / self.sides as f64).cos();
        let sa = angle_xy(self.center, pt, self.plane) + PI / self.sides as f64;
        Some(poly_wire(self.center, vr, self.sides, sa, self.plane))
    }
}

// ── Command: Polygon — Edge  (POLY_E) ─────────────────────────────────────
//   Type sides → pick edge start A → pick edge end B.
//   Center is computed from the edge and the polygon geometry.

pub struct PolyECommand {
    sides: u32,
    step: u8,
    a: DVec3,
    plane: WorkingPlane,
}

impl PolyECommand {
    pub fn new() -> Self {
        Self {
            sides: defaults::get_polygon_sides() as u32,
            step: 0,
            a: DVec3::ZERO,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for PolyECommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "POLY_E"
    }

    fn wants_text_input(&self) -> bool {
        self.step == 0
    }

    fn dyn_field(&self) -> crate::command::DynField {
        if self.step == 0 {
            crate::command::DynField::Scalar
        } else {
            crate::command::DynField::Point
        }
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if let Ok(n) = text.trim().parse::<u32>() {
            if (3..=1024).contains(&n) {
                self.sides = n;
                defaults::set_polygon_sides(n as f64);
            }
        }
        self.step = 1;
        Some(CmdResult::NeedPoint)
    }

    fn prompt(&self) -> String {
        match self.step {
            0 => {
                let n = self.sides;
                t!("POLYGON E  Enter number of sides <%{n}>:", n = n).into_owned()
            }
            1 => {
                let n = self.sides;
                t!(
                    "POLYGON E  Specify first endpoint of edge [%{n} sides]:",
                    n = n
                )
                .into_owned()
            }
            _ => {
                let n = self.sides;
                t!(
                    "POLYGON E  Specify second endpoint of edge [%{n} sides]:",
                    n = n
                )
                .into_owned()
            }
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.a = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            1 => {
                self.a = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                if let Some((center, vr, sa)) =
                    edge_poly_params(self.a, pt, self.sides, self.plane)
                {
                    let vertices = poly_verts(center, vr, self.sides, sa, self.plane);
                    CmdResult::CommitAndExit(make_pline(&vertices, self.plane))
                } else {
                    CmdResult::Cancel
                }
            }
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.step == 0 {
            self.step = 1;
            return CmdResult::NeedPoint;
        }
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        if self.step < 2 {
            return None;
        }
        if let Some((center, vr, sa)) =
            edge_poly_params(self.a, pt, self.sides, self.plane)
        {
            Some(poly_wire(center, vr, self.sides, sa, self.plane))
        } else {
            Some(wire_seg(self.a, pt))
        }
    }
}

/// Compute polygon center, vertex-radius and start-angle from two edge endpoints.
/// The polygon is placed on the left side of A→B (CCW convention).
fn edge_poly_params(
    a: DVec3,
    b: DVec3,
    sides: u32,
    plane: WorkingPlane,
) -> Option<(DVec3, f64, f64)> {
    let (a, b) = (plane.to_local(a), plane.to_local(b));
    let edge_len = a.distance(b);
    if edge_len < 1e-6 {
        return None;
    }
    // vertex_radius = edge_len / (2 * sin(π/N))
    let vr = edge_len / (2.0 * (PI / sides as f64).sin());
    // inradius = vr * cos(π/N) = edge_len / (2 * tan(π/N))
    let inradius = vr * (PI / sides as f64).cos();
    // Center: on the left perpendicular bisector of A→B
    let dir = (b - a) / edge_len;
    let perp = DVec3::new(-dir.y, dir.x, 0.0); // CCW left
    let mid = (a + b) * 0.5;
    let center = mid + perp * inradius;
    // First vertex = A
    let center_world = plane.to_world(center);
    let sa = angle_xy(center_world, plane.to_world(a), plane);
    Some((center_world, vr, sa))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_variants_share_last_valid_side_count() {
        let mut inscribed = PolyCommand::new();
        let _ = inscribed.on_text_input("7");
        assert_eq!(PolyCCommand::new().sides, 7);
        assert_eq!(PolyECommand::new().sides, 7);

        let mut circumscribed = PolyCCommand::new();
        let _ = circumscribed.on_text_input("8");
        assert_eq!(PolyCommand::new().sides, 8);
        assert_eq!(PolyECommand::new().sides, 8);

        let mut edge = PolyECommand::new();
        let _ = edge.on_text_input("9");
        assert_eq!(PolyCommand::new().sides, 9);
        assert_eq!(PolyCCommand::new().sides, 9);

        let _ = edge.on_text_input("2");
        assert_eq!(PolyCommand::new().sides, 9);
    }
}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["POLY_C"] });  // PolyCCommand
inventory::submit!(crate::command::CommandRegistration { names: &["POLY", "POLYGON"] });  // PolyCommand
inventory::submit!(crate::command::CommandRegistration { names: &["POLY_E"] });  // PolyECommand
inventory::submit!(crate::command::CommandRegistration { names: &["RECT_CEN"] });  // RectCenCommand
inventory::submit!(crate::command::CommandRegistration { names: &["RECT", "RECTANG"] });  // RectCommand
inventory::submit!(crate::command::CommandRegistration { names: &["RECT_ROT"] });  // RectRotCommand
