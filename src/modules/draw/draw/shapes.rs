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
use crate::modules::IconKind;
use crate::scene::model::wire_model::WireModel;
use glam::DVec3;

/// Build the four corners of an axis-aligned box between opposite corners `a`
/// and `b`, axis-aligned in the active UCS (`ucs` = UCS→wire affine, identity =
/// world). The two given corners stay put; the other two are placed square to
/// the UCS axes instead of the world axes.
fn ucs_box_corners(a: DVec3, b: DVec3, plane: WorkingPlane) -> [DVec3; 4] {
    let au = plane.to_local(a);
    let bu = plane.to_local(b);
    [
        plane.to_world(DVec3::new(au.x, au.y, au.z)),
        plane.to_world(DVec3::new(bu.x, au.y, au.z)),
        plane.to_world(DVec3::new(bu.x, bu.y, au.z)),
        plane.to_world(DVec3::new(au.x, bu.y, au.z)),
    ]
}

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

pub struct RectCommand {
    a: Option<DVec3>,
    plane: WorkingPlane,
}

impl RectCommand {
    pub fn new() -> Self {
        Self {
            a: None,
            plane: WorkingPlane::default(),
        }
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
        if self.a.is_none() {
            crate::t!("RECT  Specify first corner:").into_owned()
        } else {
            crate::t!("RECT  Specify opposite corner:").into_owned()
        }
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        // First corner also offers the alternate rectangle methods; later steps
        // are plain point picks. (#304)
        if self.a.is_none() {
            vec![
                CmdOption::new("Rotation", "ROTATION"),
                CmdOption::new("Center", "CENTER"),
            ]
        } else {
            vec![]
        }
    }

    fn point_step_accepts_keywords(&self) -> bool {
        self.a.is_none()
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        // At the first corner, keyword options hand off to the dedicated
        // variant command. (#304)
        if self.a.is_none() {
            return match text.trim().to_uppercase().as_str() {
                "R" | "ROTATION" => Some(CmdResult::Dispatch("RECT_ROT".into())),
                "C" | "CENTER" => Some(CmdResult::Dispatch("RECT_CEN".into())),
                _ => None,
            };
        }
        None
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.a {
            None => {
                self.a = Some(pt);
                CmdResult::NeedPoint
            }
            Some(a) => {
                let c = ucs_box_corners(a, pt, self.plane);
                CmdResult::CommitAndExit(make_pline(&c, self.plane))
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
        let a = self.a?;
        let c = ucs_box_corners(a, pt, self.plane);
        Some(wire_loop(vec![
            [c[0].x, c[0].y, c[0].z],
            [c[1].x, c[1].y, c[1].z],
            [c[2].x, c[2].y, c[2].z],
            [c[3].x, c[3].y, c[3].z],
        ]))
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Opposite corner: enter width and height (signed deltas from the first
        // corner), with the rectangle drawn as the guide. First corner is a
        // normal point pick.
        self.a.map(|a| DynSpec {
            anchor: DynAnchor::Point(a),
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
            sides: 6,
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
            sides: 6,
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
            sides: 6,
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


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["POLY_C"] });  // PolyCCommand
inventory::submit!(crate::command::CommandRegistration { names: &["POLY", "POLYGON"] });  // PolyCommand
inventory::submit!(crate::command::CommandRegistration { names: &["POLY_E"] });  // PolyECommand
inventory::submit!(crate::command::CommandRegistration { names: &["RECT_CEN"] });  // RectCenCommand
inventory::submit!(crate::command::CommandRegistration { names: &["RECT", "RECTANG"] });  // RectCommand
inventory::submit!(crate::command::CommandRegistration { names: &["RECT_ROT"] });  // RectRotCommand
