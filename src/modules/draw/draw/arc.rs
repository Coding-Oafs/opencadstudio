// Arc tool — ribbon dropdown + all OpenCADStudio arc creation methods.
//
// Methods (dropdown order):
//   ARC_3P  — 3-Point                (default)
//   ARC_SCE — Start, Center, End
//   ARC_SCA — Start, Center, Angle
//   ARC_SCL — Start, Center, Length of chord
//   ARC_SEA — Start, End, Angle      (sagitta-based pick)
//   ARC_SED — Start, End, Direction  (tangent at start)
//   ARC_SER — Start, End, Radius     (radius defined by dist cursor→start)
//   ARC_CSE — Center, Start, End
//   ARC_CSA — Center, Start, Angle
//   ARC_CSL — Center, Start, Length of chord
//   ARC_CONT— Continue                (tangent to previous line/arc; pick end only)

use acadrust::types::Vector3;
use acadrust::{Arc as CadArc, EntityType};
use crate::t;

use crate::command::{CadCommand, CmdResult, WorkingPlane};
use crate::modules::IconKind;
use crate::scene::model::wire_model::WireModel;
use glam::DVec3;

const TAU: f64 = std::f64::consts::TAU;

// ── Per-method SVG icons ───────────────────────────────────────────────────

const ICON_CSE: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_cse.svg"));
const ICON_3P: IconKind = IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_3p.svg"));
const ICON_SCE: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_sce.svg"));
const ICON_SCA: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_sca.svg"));
const ICON_SCL: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_scl.svg"));
const ICON_SEA: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_sea.svg"));
const ICON_SER: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_ser.svg"));
const ICON_SED: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_sed.svg"));
const ICON_CSA: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_csa.svg"));
const ICON_CSL: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_csl.svg"));
const ICON_CONT: IconKind =
    IconKind::Svg(include_bytes!("../../../../assets/icons/arc/arc_cont.svg"));

// ── Dropdown metadata ──────────────────────────────────────────────────────

pub const DROPDOWN_ID: &str = "ARC";

pub const DROPDOWN_ITEMS: &[(&str, &str, IconKind)] = &[
    ("ARC_3P", "3-Point", ICON_3P),
    ("ARC_SCE", "Start, Center, End", ICON_SCE),
    ("ARC_SCA", "Start, Center, Angle", ICON_SCA),
    ("ARC_SCL", "Start, Center, Length", ICON_SCL),
    ("ARC_SEA", "Start, End, Angle", ICON_SEA),
    ("ARC_SED", "Start, End, Direction", ICON_SED),
    ("ARC_SER", "Start, End, Radius", ICON_SER),
    ("ARC_CSE", "Center, Start, End", ICON_CSE),
    ("ARC_CSA", "Center, Start, Angle", ICON_CSA),
    ("ARC_CSL", "Center, Start, Length", ICON_CSL),
    ("ARC_CONT", "Continue", ICON_CONT),
];

/// Default icon — falls back to 3-Point before first use.
pub const ICON: IconKind = ICON_3P;

// ── Shared math helpers ────────────────────────────────────────────────────

/// Angle in radians from `center` to `pt`.
fn angle_xy(center: DVec3, pt: DVec3, plane: WorkingPlane) -> f64 {
    plane.angle(center, pt).unwrap_or(0.0)
}

/// Build a CCW arc polyline from `start_angle` to `end_angle` (radians).
fn arc_preview(
    center: DVec3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    plane: WorkingPlane,
) -> WireModel {
    let mut ea = end_angle;
    while ea < start_angle {
        ea += TAU;
    }
    let span = (ea - start_angle).min(TAU);
    let segs = ((span / TAU) * 64.0).ceil().max(4.0) as u32;
    let pts: Vec<[f64; 3]> = (0..=segs)
        .map(|i| {
            let a = start_angle + span * (i as f64 / segs as f64);
            let point = center + plane.x * (radius * a.cos()) + plane.y * (radius * a.sin());
            [point.x, point.y, point.z]
        })
        .collect();
    WireModel::solid_f64("rubber_band".into(), pts, WireModel::CYAN, false)
}

fn make_arc(
    center: DVec3,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
    plane: WorkingPlane,
) -> EntityType {
    let center = plane.to_local(center);
    plane.place_entity(EntityType::Arc(CadArc {
        center: Vector3::new(center.x, center.y, center.z),
        radius,
        start_angle,
        end_angle,
        ..Default::default()
    }))
}

fn line_wire(a: DVec3, b: DVec3) -> WireModel {
    WireModel::solid_f64(
        "rubber_band".into(),
        vec![[a.x, a.y, a.z], [b.x, b.y, b.z]],
        WireModel::CYAN,
        false,
    )
}

/// Circumscribed circle through three points.
fn circumcircle(
    a: DVec3,
    b: DVec3,
    c: DVec3,
    plane: WorkingPlane,
) -> Option<(DVec3, f64)> {
    let (a, b, c) = (plane.to_local(a), plane.to_local(b), plane.to_local(c));
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-9 {
        return None;
    }
    let ux = ((a.x * a.x + a.y * a.y) * (b.y - c.y)
        + (b.x * b.x + b.y * b.y) * (c.y - a.y)
        + (c.x * c.x + c.y * c.y) * (a.y - b.y))
        / d;
    let uy = ((a.x * a.x + a.y * a.y) * (c.x - b.x)
        + (b.x * b.x + b.y * b.y) * (a.x - c.x)
        + (c.x * c.x + c.y * c.y) * (b.x - a.x))
        / d;
    let center = DVec3::new(ux, uy, a.z);
    Some((plane.to_world(center), center.distance(a)))
}

/// True if `angle` lies inside the CCW arc [start..end] (radians).
fn ccw_contains(angle: f64, start: f64, end: f64) -> bool {
    let a = angle.rem_euclid(TAU);
    let s = start.rem_euclid(TAU);
    let e = end.rem_euclid(TAU);
    if s <= e {
        a >= s && a <= e
    } else {
        a >= s || a <= e
    }
}

/// Arc center+radius from two endpoints and a cursor (sagitta / bow-toward-cursor).
fn arc_from_sagitta(
    s: DVec3,
    e: DVec3,
    cursor: DVec3,
    flip_direction: bool,
    plane: WorkingPlane,
) -> Option<(DVec3, f64, f64, f64)> {
    let (s, e, cursor) = (plane.to_local(s), plane.to_local(e), plane.to_local(cursor));
    let chord_vec = e - s;
    let chord_len = (chord_vec.x * chord_vec.x + chord_vec.y * chord_vec.y).sqrt();
    if chord_len < 1e-6 {
        return None;
    }
    let unit_chord = DVec3::new(chord_vec.x / chord_len, chord_vec.y / chord_len, 0.0);
    let perp = DVec3::new(-unit_chord.y, unit_chord.x, 0.0);
    let mid = (s + e) * 0.5;
    let h = (cursor - mid).dot(perp); // signed sagitta
    if h.abs() < 1e-3 {
        return None;
    }
    let sweep = 4.0 * (2.0 * h.abs() / chord_len).atan();
    let included = -h.signum() * sweep * if flip_direction { -1.0 } else { 1.0 };
    arc_from_endpoints_angle(
        plane.to_world(s),
        plane.to_world(e),
        included,
        plane,
    )
}

/// Arc through two endpoints with a signed included angle. Positive angles
/// travel counter-clockwise from start to end; negative angles travel
/// clockwise and are stored by swapping the geometric endpoints.
fn arc_from_endpoints_angle(
    s: DVec3,
    e: DVec3,
    included: f64,
    plane: WorkingPlane,
) -> Option<(DVec3, f64, f64, f64)> {
    let (s_local, e_local) = (plane.to_local(s), plane.to_local(e));
    let chord = e_local - s_local;
    let length = chord.length();
    let sweep = included.abs();
    if length <= 1.0e-9 || sweep <= 1.0e-9 || sweep >= TAU - 1.0e-9 {
        return None;
    }
    let half_sin = (sweep * 0.5).sin().abs();
    if half_sin <= 1.0e-12 {
        return None;
    }
    let radius = length / (2.0 * half_sin);
    let unit = chord / length;
    let left = DVec3::new(-unit.y, unit.x, 0.0);
    let offset = length / (2.0 * (sweep * 0.5).tan());
    let center_local = (s_local + e_local) * 0.5 + left * offset * included.signum();
    let center = plane.to_world(center_local);
    let start = angle_xy(center, s, plane);
    let end = angle_xy(center, e, plane);
    if included > 0.0 {
        Some((center, radius, start, end))
    } else {
        Some((center, radius, end, start))
    }
}

/// Arc center+radius from start, end, and a radius-magnitude point (dist = dist(pt, start)).
fn arc_from_se_radius(
    s: DVec3,
    e: DVec3,
    radius_pt: DVec3,
    clockwise: bool,
    plane: WorkingPlane,
) -> Option<(DVec3, f64, f64, f64)> {
    let radius = plane.to_local(s).distance(plane.to_local(radius_pt));
    arc_from_endpoints_radius(s, e, radius, clockwise, plane)
}

fn arc_from_endpoints_radius(
    s: DVec3,
    e: DVec3,
    signed_radius: f64,
    clockwise: bool,
    plane: WorkingPlane,
) -> Option<(DVec3, f64, f64, f64)> {
    let chord_len = plane.to_local(s).distance(plane.to_local(e));
    let radius = signed_radius.abs();
    if radius <= 1.0e-9 || chord_len <= 1.0e-9 || radius < chord_len * 0.5 {
        return None;
    }
    let minor = 2.0 * (chord_len / (2.0 * radius)).asin();
    let mut included = if signed_radius >= 0.0 {
        minor
    } else {
        TAU - minor
    };
    if clockwise {
        included = -included;
    }
    arc_from_endpoints_angle(s, e, included, plane)
}

/// Arc center+radius from start, end, and a tangent-direction point at start.
fn arc_from_direction(
    s: DVec3,
    e: DVec3,
    dir_pt: DVec3,
    plane: WorkingPlane,
) -> Option<(DVec3, f64)> {
    let (s, e, dir_pt) = (
        plane.to_local(s),
        plane.to_local(e),
        plane.to_local(dir_pt),
    );
    let t = (dir_pt - s).normalize_or_zero();
    if t.length_squared() < 1e-12 {
        return None;
    }
    let perp_t = DVec3::new(-t.y, t.x, 0.0); // rotate tangent 90° CCW
    let chord = e - s;
    let denom = perp_t.x * chord.x + perp_t.y * chord.y;
    if denom.abs() < 1e-9 {
        return None;
    }
    let lambda = chord.length_squared() * 0.5 / denom;
    let center = s + perp_t * lambda;
    Some((plane.to_world(center), center.distance(s)))
}

/// Tangent-continuing arc: starts at `s` leaving in unit direction `t` (forward,
/// G1-continuous with the previous entity) and passes through `e`. Returns
/// `(center, radius, start_angle, end_angle)` with the CCW sweep oriented so the
/// drawn arc actually leaves `s` along `+t` (not the reflex complement).
///
/// `arc_from_direction` treats the tangent as an undirected line (±t give the
/// same circle), so on its own the CCW-only renderer would draw the wrong arc
/// whenever `e` lies to the right of `+t`. We fix the sweep direction here by
/// checking the circle's CCW travel direction at `s` and swapping the angles
/// when it opposes `+t`. `flip` (Ctrl) selects the complementary arc — the other
/// way around the same circle.
fn arc_continue(
    s: DVec3,
    t: DVec3,
    e: DVec3,
    flip: bool,
    plane: WorkingPlane,
) -> Option<(DVec3, f64, f64, f64)> {
    let (center, radius) = arc_from_direction(s, e, s + t, plane)?;
    // CCW travel direction on this circle at s = (s - center) rotated +90°.
    let rs = plane.vector_to_local(s - center);
    let t = plane.vector_to_local(t);
    let travel_s = DVec3::new(-rs.y, rs.x, 0.0);
    let forward = travel_s.dot(t) >= 0.0;
    // Draw CCW s→e when the CCW-from-s arc leaves along +t; else CCW e→s. `flip`
    // inverts the choice to draw the complementary arc.
    let (sa, ea) = if forward ^ flip {
        (angle_xy(center, s, plane), angle_xy(center, e, plane))
    } else {
        (angle_xy(center, e, plane), angle_xy(center, s, plane))
    };
    Some((center, radius, sa, ea))
}

/// Continuation anchor for `ARC_CONT`: the point where drawing a line, arc or
/// planar polyline ended plus the unit tangent leaving it.
/// `last` (the final pick point) disambiguates which endpoint the drawing ended
/// on — essential because a committed arc stores geometric CCW start/end angles,
/// not the direction the pen travelled. Returns `None` for other entity kinds.
pub fn continue_anchor(entity: &EntityType, last: Option<DVec3>) -> Option<(DVec3, DVec3)> {
    if !matches!(
        entity,
        EntityType::Line(_)
            | EntityType::Arc(_)
            | EntityType::LwPolyline(_)
            | EntityType::Polyline2D(_)
    ) {
        return None;
    }
    let curve = crate::entities::curve::entity_curve(entity)?;
    let start = DVec3::from_array(curve.point_at(0.0));
    let end = DVec3::from_array(curve.point_at(1.0));
    let use_end = last.map_or(true, |point| point.distance(end) <= point.distance(start));
    let (point, tangent) = if use_end {
        (end, DVec3::from_array(curve.tangent_at(1.0)))
    } else {
        (start, -DVec3::from_array(curve.tangent_at(0.0)))
    };
    let tangent = tangent.normalize_or_zero();
    (tangent.length_squared() > 1.0e-12).then_some((point, tangent))
}

/// Compute end_angle from a chord-length pick (SCL / CSL semantics).
/// Positive length selects the minor arc; negative length selects the major.
fn end_angle_from_chord_len(start_angle: f64, chord: f64, r: f64) -> Option<f64> {
    if r <= 0.0 || chord == 0.0 || chord.abs() > 2.0 * r {
        return None;
    }
    let minor = 2.0 * (chord.abs() / (2.0 * r)).asin();
    let sweep = if chord > 0.0 { minor } else { TAU - minor };
    Some(start_angle + sweep)
}

// ── Command 1: Center, Start, End ─────────────────────────────────────────

pub struct ArcCommand {
    step: u8,
    c: DVec3,
    r: f64,
    sa: f64,
    cw: bool,
    plane: WorkingPlane,
}

impl ArcCommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            c: DVec3::ZERO,
            r: 0.0,
            sa: 0.0,
            cw: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.cw = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_CSE"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC  Specify center:").into_owned(),
            1 => t!("ARC  Specify start point:").into_owned(),
            _ => {
                let cx = format!("{:.2}", self.c.x);
                let cy = format!("{:.2}", self.c.y);
                let r = format!("{:.3}", self.r);
                t!(
                    "ARC  Specify end point  [c=(%{cx},%{cy}) r=%{r}]:",
                    cx = cx,
                    cy = cy,
                    r = r
                )
                .into_owned()
            }
        }
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        // Alternate construction methods, offered only at the first (center)
        // step. Each keyword hands off to the dedicated variant command.
        if self.step == 0 {
            vec![
                CmdOption::new("SCE", "SCE"),
                CmdOption::new("SCA", "SCA"),
                CmdOption::new("SEA", "SEA"),
                CmdOption::new("SER", "SER"),
                CmdOption::new("CSA", "CSA"),
                CmdOption::new("3P", "3P"),
            ]
        } else {
            vec![]
        }
    }

    fn point_step_accepts_keywords(&self) -> bool {
        self.step == 0
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.c = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.r = self.c.distance(pt);
                self.sa = angle_xy(self.c, pt, self.plane);
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let ea = angle_xy(self.c, pt, self.plane);
                let e = if self.cw {
                    make_arc(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    make_arc(self.c, self.r, self.sa, ea, self.plane)
                };
                CmdResult::CommitAndExit(e)
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        // At the center step, keyword options switch construction method by
        // handing off to the dedicated variant command.
        if self.step == 0 {
            return match text.trim().to_uppercase().as_str() {
                "SCE" => Some(CmdResult::Dispatch("ARC_SCE".into())),
                "SCA" => Some(CmdResult::Dispatch("ARC_SCA".into())),
                "SEA" => Some(CmdResult::Dispatch("ARC_SEA".into())),
                "SER" => Some(CmdResult::Dispatch("ARC_SER".into())),
                "CSA" => Some(CmdResult::Dispatch("ARC_CSA".into())),
                "3P" => Some(CmdResult::Dispatch("ARC_3P".into())),
                _ => None,
            };
        }
        None
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.c, pt)),
            2 => {
                let ea = angle_xy(self.c, pt, self.plane);
                Some(if self.cw {
                    arc_preview(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    arc_preview(self.c, self.r, self.sa, ea, self.plane)
                })
            }
            _ => None,
        }
    }
}

// ── Command 2: 3-Point  (ARC_3P) ──────────────────────────────────────────

pub struct Arc3PCommand {
    pts: Vec<DVec3>,
    plane: WorkingPlane,
}

impl Arc3PCommand {
    pub fn new() -> Self {
        Self {
            pts: Vec::new(),
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for Arc3PCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "ARC_3P"
    }
    fn prompt(&self) -> String {
        match self.pts.len() {
            0 => t!("ARC 3P  Specify start point:").into_owned(),
            1 => t!("ARC 3P  Specify second point on arc:").into_owned(),
            _ => t!("ARC 3P  Specify end point:").into_owned(),
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        self.pts.push(pt);
        if self.pts.len() < 3 {
            return CmdResult::NeedPoint;
        }
        let (p1, p2, p3) = (self.pts[0], self.pts[1], self.pts[2]);
        match circumcircle(p1, p2, p3, self.plane) {
            None => {
                self.pts.pop();
                CmdResult::NeedPoint
            } // collinear — retry
            Some((center, radius)) => {
                let a1 = angle_xy(center, p1, self.plane);
                let a2 = angle_xy(center, p2, self.plane);
                let a3 = angle_xy(center, p3, self.plane);
                // Choose arc direction so that p2 lies on the arc from p1 to p3.
                let (sa, ea) = if ccw_contains(a2, a1, a3) {
                    (a1, a3)
                } else {
                    (a3, a1)
                };
                CmdResult::CommitAndExit(make_arc(center, radius, sa, ea, self.plane))
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        if self.pts.is_empty() {
            CmdResult::Dispatch("ARC_CONT".into())
        } else {
            CmdResult::Cancel
        }
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.pts.len() {
            0 => None,
            1 => Some(line_wire(self.pts[0], pt)),
            _ => {
                let (p1, p2) = (self.pts[0], self.pts[1]);
                if let Some((center, radius)) = circumcircle(p1, p2, pt, self.plane) {
                    let a1 = angle_xy(center, p1, self.plane);
                    let a2 = angle_xy(center, p2, self.plane);
                    let a3 = angle_xy(center, pt, self.plane);
                    let (sa, ea) = if ccw_contains(a2, a1, a3) {
                        (a1, a3)
                    } else {
                        (a3, a1)
                    };
                    Some(arc_preview(center, radius, sa, ea, self.plane))
                } else {
                    Some(WireModel::solid_f64(
                        "rubber_band".into(),
                        vec![[p1.x, p1.y, p1.z], [p2.x, p2.y, p2.z], [pt.x, pt.y, pt.z]],
                        WireModel::CYAN,
                        false,
                    ))
                }
            }
        }
    }
}

// ── Command 3: Start, Center, End  (ARC_SCE) ──────────────────────────────

pub struct ArcSCECommand {
    step: u8,
    s: DVec3,
    c: DVec3,
    r: f64,
    sa: f64,
    cw: bool,
    plane: WorkingPlane,
}

impl ArcSCECommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            s: DVec3::ZERO,
            c: DVec3::ZERO,
            r: 0.0,
            sa: 0.0,
            cw: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcSCECommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.cw = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_SCE"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC SCE  Specify start point:").into_owned(),
            1 => t!("ARC SCE  Specify center:").into_owned(),
            _ => {
                let r = format!("{:.3}", self.r);
                t!("ARC SCE  Specify end point  [r=%{r}]:", r = r).into_owned()
            }
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.s = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.c = pt;
                self.r = pt.distance(self.s);
                self.sa = angle_xy(pt, self.s, self.plane);
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let ea = angle_xy(self.c, pt, self.plane);
                let e = if self.cw {
                    make_arc(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    make_arc(self.c, self.r, self.sa, ea, self.plane)
                };
                CmdResult::CommitAndExit(e)
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.s, pt)),
            2 => {
                let ea = angle_xy(self.c, pt, self.plane);
                Some(if self.cw {
                    arc_preview(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    arc_preview(self.c, self.r, self.sa, ea, self.plane)
                })
            }
            _ => None,
        }
    }
}

// ── Command 4: Start, Center, Angle  (ARC_SCA) ────────────────────────────
// Interactive: cursor direction from center defines span.  Typing: degrees of span.

pub struct ArcSCACommand {
    step: u8,
    s: DVec3,
    c: DVec3,
    r: f64,
    sa: f64,
    cw: bool,
    plane: WorkingPlane,
}

impl ArcSCACommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            s: DVec3::ZERO,
            c: DVec3::ZERO,
            r: 0.0,
            sa: 0.0,
            cw: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcSCACommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.cw = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_SCA"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC SCA  Specify start point:").into_owned(),
            1 => t!("ARC SCA  Specify center:").into_owned(),
            _ => {
                let sa = format!("{:.1}°", self.sa.to_degrees());
                t!(
                    "ARC SCA  Click end direction or type arc span in degrees  [start=%{sa}]:",
                    sa = sa
                )
                .into_owned()
            }
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.s = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.c = pt;
                self.r = pt.distance(self.s);
                self.sa = angle_xy(pt, self.s, self.plane);
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let ea = angle_xy(self.c, pt, self.plane);
                let e = if self.cw {
                    make_arc(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    make_arc(self.c, self.r, self.sa, ea, self.plane)
                };
                CmdResult::CommitAndExit(e)
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step == 2 {
            let span: f64 = text.trim().replace(',', ".").parse().ok()?;
            let ea = self.sa + span.to_radians();
            let entity = if span < 0.0 {
                make_arc(self.c, self.r, ea, self.sa, self.plane)
            } else {
                make_arc(self.c, self.r, self.sa, ea, self.plane)
            };
            return Some(CmdResult::CommitAndExit(entity));
        }
        None
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Included angle (span) at the centre. Typed value is the span, handled
        // by on_text_input; the box previews the live span via dyn_live_value.
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.c),
            fields: vec![DynFieldSpec::new(DynRole::Angle)],
            guide: DynGuide::Polar,
            ref_point: Some(
                self.c + self.plane.x * self.sa.cos() + self.plane.y * self.sa.sin(),
            ),
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        (self.step == 2).then(|| {
            crate::command::dyn_display_angle_deg(
                (angle_xy(self.c, cursor, self.plane) - self.sa) as f32,
            ) as f64
        })
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.s, pt)),
            2 => {
                let ea = angle_xy(self.c, pt, self.plane);
                Some(if self.cw {
                    arc_preview(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    arc_preview(self.c, self.r, self.sa, ea, self.plane)
                })
            }
            _ => None,
        }
    }
}

// ── Command 5: Start, Center, Length  (ARC_SCL) ───────────────────────────
// "Length" = chord length from start to end of arc.
// Interactive: cursor distance from start_pt drives the chord length.

pub struct ArcSCLCommand {
    step: u8,
    s: DVec3,
    c: DVec3,
    r: f64,
    sa: f64,
    cw: bool,
    plane: WorkingPlane,
}

impl ArcSCLCommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            s: DVec3::ZERO,
            c: DVec3::ZERO,
            r: 0.0,
            sa: 0.0,
            cw: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcSCLCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.cw = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_SCL"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC SCL  Specify start point:").into_owned(),
            1 => t!("ARC SCL  Specify center:").into_owned(),
            _ => {
                let r = format!("{:.3}", self.r);
                t!(
                    "ARC SCL  Click chord end or type chord length  [r=%{r}]:",
                    r = r
                )
                .into_owned()
            }
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.s = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.c = pt;
                self.r = pt.distance(self.s);
                self.sa = angle_xy(pt, self.s, self.plane);
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let chord = self.s.distance(pt);
                let Some(ea) = end_angle_from_chord_len(self.sa, chord, self.r) else {
                    return CmdResult::NeedPoint;
                };
                let entity = if self.cw {
                    make_arc(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    make_arc(self.c, self.r, self.sa, ea, self.plane)
                };
                CmdResult::CommitAndExit(entity)
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step == 2 {
            let chord: f64 = text.trim().replace(',', ".").parse().ok()?;
            if chord != 0.0 {
                let ea = end_angle_from_chord_len(self.sa, chord, self.r)?;
                return Some(CmdResult::CommitAndExit(make_arc(
                    self.c, self.r, self.sa, ea, self.plane,
                )));
            }
        }
        None
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Chord length from the start point (typed → on_text_input).
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.s),
            fields: vec![DynFieldSpec::new(DynRole::Distance)],
            guide: DynGuide::Radius,
            ref_point: None,
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        (self.step == 2).then(|| self.s.distance(cursor))
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.s, pt)),
            2 => {
                let chord = self.s.distance(pt);
                let ea = end_angle_from_chord_len(self.sa, chord, self.r)?;
                Some(if self.cw {
                    arc_preview(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    arc_preview(self.c, self.r, self.sa, ea, self.plane)
                })
            }
            _ => None,
        }
    }
}

// ── Command 6: Start, End, Angle  (ARC_SEA) ───────────────────────────────
// Interactive: cursor distance from chord defines sagitta → arc shape.

pub struct ArcSEACommand {
    step: u8,
    s: DVec3,
    e: DVec3,
    ctrl: bool,
    plane: WorkingPlane,
}

impl ArcSEACommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            s: DVec3::ZERO,
            e: DVec3::ZERO,
            ctrl: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcSEACommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.ctrl = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_SEA"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC SEA  Specify start point:").into_owned(),
            1 => t!("ARC SEA  Specify end point:").into_owned(),
            _ => t!("ARC SEA  Specify angle (move cursor perpendicular to chord):").into_owned(),
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.s = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.e = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => match arc_from_sagitta(self.s, self.e, pt, self.ctrl, self.plane) {
                Some((center, radius, sa, ea)) => {
                    CmdResult::CommitAndExit(make_arc(center, radius, sa, ea, self.plane))
                }
                None => CmdResult::Cancel,
            },
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step != 2 {
            return None;
        }
        let included = text.trim().replace(',', ".").parse::<f64>().ok()?.to_radians();
        let (center, radius, sa, ea) =
            arc_from_endpoints_angle(self.s, self.e, included, self.plane)?;
        Some(CmdResult::CommitAndExit(make_arc(
            center, radius, sa, ea, self.plane,
        )))
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point((self.s + self.e) * 0.5),
            fields: vec![DynFieldSpec::new(DynRole::Angle)],
            guide: DynGuide::None,
            ref_point: Some(self.s),
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.s, pt)),
            2 => {
                if let Some((center, radius, sa, ea)) =
                    arc_from_sagitta(self.s, self.e, pt, self.ctrl, self.plane)
                {
                    Some(arc_preview(center, radius, sa, ea, self.plane))
                } else {
                    Some(line_wire(self.s, self.e))
                }
            }
            _ => None,
        }
    }
}

// ── Command 7: Start, End, Radius  (ARC_SER) ──────────────────────────────
// Interactive: radius = distance(cursor, start_point).

pub struct ArcSERCommand {
    step: u8,
    s: DVec3,
    e: DVec3,
    ctrl: bool,
    plane: WorkingPlane,
}

impl ArcSERCommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            s: DVec3::ZERO,
            e: DVec3::ZERO,
            ctrl: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcSERCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.ctrl = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_SER"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC SER  Specify start point:").into_owned(),
            1 => t!("ARC SER  Specify end point:").into_owned(),
            _ => t!("ARC SER  Click radius point or type radius value:").into_owned(),
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.s = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.e = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => match arc_from_se_radius(self.s, self.e, pt, self.ctrl, self.plane) {
                Some((center, radius, sa, ea)) => {
                    CmdResult::CommitAndExit(make_arc(center, radius, sa, ea, self.plane))
                }
                None => CmdResult::Cancel,
            },
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step == 2 {
            let r: f64 = text.trim().replace(',', ".").parse().ok()?;
            let (center, radius, sa, ea) =
                arc_from_endpoints_radius(self.s, self.e, r, false, self.plane)?;
            return Some(CmdResult::CommitAndExit(make_arc(
                center, radius, sa, ea, self.plane,
            )));
        }
        None
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Radius value (typed → on_text_input); the preview arc is the guide.
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.s),
            fields: vec![DynFieldSpec::new(DynRole::Radius)],
            guide: DynGuide::None,
            ref_point: None,
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        if self.step != 2 {
            return None;
        }
        arc_from_se_radius(self.s, self.e, cursor, self.ctrl, self.plane)
            .map(|(_, r, _, _)| r)
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.s, pt)),
            2 => {
                if let Some((center, radius, sa, ea)) =
                    arc_from_se_radius(self.s, self.e, pt, self.ctrl, self.plane)
                {
                    Some(arc_preview(center, radius, sa, ea, self.plane))
                } else {
                    Some(line_wire(self.s, self.e))
                }
            }
            _ => None,
        }
    }
}

// ── Command 8: Start, End, Direction  (ARC_SED) ───────────────────────────
// Interactive: cursor position defines tangent direction at start (cursor − start).

pub struct ArcSEDCommand {
    step: u8,
    s: DVec3,
    e: DVec3,
    ctrl: bool,
    plane: WorkingPlane,
}

impl ArcSEDCommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            s: DVec3::ZERO,
            e: DVec3::ZERO,
            ctrl: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcSEDCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.ctrl = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_SED"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC SED  Specify start point:").into_owned(),
            1 => t!("ARC SED  Specify end point:").into_owned(),
            _ => t!("ARC SED  Specify tangent direction at start:").into_owned(),
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.s = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.e = pt;
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => match arc_continue(self.s, pt - self.s, self.e, self.ctrl, self.plane) {
                Some((center, radius, sa, ea)) => {
                    CmdResult::CommitAndExit(make_arc(center, radius, sa, ea, self.plane))
                }
                None => CmdResult::Cancel,
            },
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step != 2 {
            return None;
        }
        let angle = text.trim().replace(',', ".").parse::<f64>().ok()?.to_radians();
        let tangent = self.plane.x * angle.cos() + self.plane.y * angle.sin();
        let (center, radius, sa, ea) =
            arc_continue(self.s, tangent, self.e, self.ctrl, self.plane)?;
        Some(CmdResult::CommitAndExit(make_arc(
            center, radius, sa, ea, self.plane,
        )))
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.s),
            fields: vec![DynFieldSpec::new(DynRole::Angle)],
            guide: DynGuide::Polar,
            ref_point: Some(self.s + self.plane.x),
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.s, pt)),
            2 => {
                if let Some((center, radius, sa, ea)) =
                    arc_continue(self.s, pt - self.s, self.e, self.ctrl, self.plane)
                {
                    Some(arc_preview(center, radius, sa, ea, self.plane))
                } else {
                    Some(line_wire(self.s, self.e))
                }
            }
            _ => None,
        }
    }
}

// ── Command 9: Center, Start, Angle  (ARC_CSA) ────────────────────────────
// Interactive: angle direction indicated by cursor position relative to center.

pub struct ArcCSACommand {
    step: u8,
    c: DVec3,
    r: f64,
    sa: f64,
    cw: bool,
    plane: WorkingPlane,
}

impl ArcCSACommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            c: DVec3::ZERO,
            r: 0.0,
            sa: 0.0,
            cw: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcCSACommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.cw = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_CSA"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC CSA  Specify center:").into_owned(),
            1 => t!("ARC CSA  Specify start point:").into_owned(),
            _ => {
                let sa = format!("{:.1}°", self.sa.to_degrees());
                t!(
                    "ARC CSA  Click end direction or type arc span in degrees  [start=%{sa}]:",
                    sa = sa
                )
                .into_owned()
            }
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.c = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.r = self.c.distance(pt);
                self.sa = angle_xy(self.c, pt, self.plane);
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let ea = angle_xy(self.c, pt, self.plane);
                let e = if self.cw {
                    make_arc(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    make_arc(self.c, self.r, self.sa, ea, self.plane)
                };
                CmdResult::CommitAndExit(e)
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step == 2 {
            let span: f64 = text.trim().replace(',', ".").parse().ok()?;
            let ea = self.sa + span.to_radians();
            let entity = if span < 0.0 {
                make_arc(self.c, self.r, ea, self.sa, self.plane)
            } else {
                make_arc(self.c, self.r, self.sa, ea, self.plane)
            };
            return Some(CmdResult::CommitAndExit(entity));
        }
        None
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.c),
            fields: vec![DynFieldSpec::new(DynRole::Angle)],
            guide: DynGuide::Polar,
            ref_point: Some(
                self.c + self.plane.x * self.sa.cos() + self.plane.y * self.sa.sin(),
            ),
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        (self.step == 2).then(|| {
            crate::command::dyn_display_angle_deg(
                (angle_xy(self.c, cursor, self.plane) - self.sa) as f32,
            ) as f64
        })
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.c, pt)),
            2 => {
                let ea = angle_xy(self.c, pt, self.plane);
                Some(if self.cw {
                    arc_preview(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    arc_preview(self.c, self.r, self.sa, ea, self.plane)
                })
            }
            _ => None,
        }
    }
}

// ── Command 10: Center, Start, Length  (ARC_CSL) ──────────────────────────
// "Length" = chord from start to end.  Interactive: dist(cursor, start_pt) = chord.

pub struct ArcCSLCommand {
    step: u8,
    c: DVec3,
    s: DVec3,
    r: f64,
    sa: f64,
    cw: bool,
    plane: WorkingPlane,
}

impl ArcCSLCommand {
    pub fn new() -> Self {
        Self {
            step: 0,
            c: DVec3::ZERO,
            s: DVec3::ZERO,
            r: 0.0,
            sa: 0.0,
            cw: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcCSLCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.cw = ctrl;
    }

    fn name(&self) -> &'static str {
        "ARC_CSL"
    }
    fn prompt(&self) -> String {
        match self.step {
            0 => t!("ARC CSL  Specify center:").into_owned(),
            1 => t!("ARC CSL  Specify start point:").into_owned(),
            _ => {
                let r = format!("{:.3}", self.r);
                t!(
                    "ARC CSL  Click chord end or type chord length  [r=%{r}]:",
                    r = r
                )
                .into_owned()
            }
        }
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.step {
            0 => {
                self.c = pt;
                self.step = 1;
                CmdResult::NeedPoint
            }
            1 => {
                self.s = pt;
                self.r = self.c.distance(pt);
                self.sa = angle_xy(self.c, pt, self.plane);
                self.step = 2;
                CmdResult::NeedPoint
            }
            _ => {
                let chord = self.s.distance(pt);
                let Some(ea) = end_angle_from_chord_len(self.sa, chord, self.r) else {
                    return CmdResult::NeedPoint;
                };
                let entity = if self.cw {
                    make_arc(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    make_arc(self.c, self.r, self.sa, ea, self.plane)
                };
                CmdResult::CommitAndExit(entity)
            }
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn enter_accepts_default_start(&self) -> bool {
        false
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.step == 2 {
            let chord: f64 = text.trim().replace(',', ".").parse().ok()?;
            if chord != 0.0 {
                let ea = end_angle_from_chord_len(self.sa, chord, self.r)?;
                return Some(CmdResult::CommitAndExit(make_arc(
                    self.c, self.r, self.sa, ea, self.plane,
                )));
            }
        }
        None
    }
    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Chord length from the start point (typed → on_text_input).
        (self.step == 2).then(|| DynSpec {
            anchor: DynAnchor::Point(self.s),
            fields: vec![DynFieldSpec::new(DynRole::Distance)],
            guide: DynGuide::Radius,
            ref_point: None,
        })
    }
    fn dyn_commit_as_text(&self) -> bool {
        self.step == 2
    }
    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        (self.step == 2).then(|| self.s.distance(cursor))
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match self.step {
            1 => Some(line_wire(self.c, pt)),
            2 => {
                let chord = self.s.distance(pt);
                let ea = end_angle_from_chord_len(self.sa, chord, self.r)?;
                Some(if self.cw {
                    arc_preview(self.c, self.r, ea, self.sa, self.plane)
                } else {
                    arc_preview(self.c, self.r, self.sa, ea, self.plane)
                })
            }
            _ => None,
        }
    }
}


// ── Command 11: Continue  (ARC_CONT) ──────────────────────────────────────
// Tangent-continuing arc from the previous line, arc or planar polyline
// endpoint. The start point and tangent are seeded at dispatch (from
// `continue_anchor`); the user picks only the end point, so the arc joins the
// previous entity smoothly.

pub struct ArcContCommand {
    s: DVec3,
    tangent: DVec3,
    /// Live Ctrl state (set via `set_ctrl`): flips the arc to the other way.
    ctrl: bool,
    plane: WorkingPlane,
}

impl ArcContCommand {
    pub fn new(s: DVec3, tangent: DVec3) -> Self {
        Self {
            s,
            tangent,
            ctrl: false,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for ArcContCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "ARC_CONT"
    }
    fn prompt(&self) -> String {
        t!("ARC Continue  Specify end point  [Ctrl = flip direction]:").into_owned()
    }
    fn set_ctrl(&mut self, ctrl: bool) {
        self.ctrl = ctrl;
    }
    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match arc_continue(self.s, self.tangent, pt, self.ctrl, self.plane) {
            Some((center, radius, sa, ea)) => {
                let arc = make_arc(center, radius, sa, ea, self.plane);
                CmdResult::CommitAndExit(arc)
            }
            // A degenerate pick should keep the one-shot command active so the
            // user can choose a valid end point.
            None => CmdResult::NeedPoint,
        }
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        match arc_continue(self.s, self.tangent, pt, self.ctrl, self.plane) {
            Some((center, radius, sa, ea)) => {
                Some(arc_preview(center, radius, sa, ea, self.plane))
            }
            None => Some(line_wire(self.s, pt)),
        }
    }
}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["ARC", "ARC_3P"] });  // Arc3PCommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_CSA"] });  // ArcCSACommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_CSL"] });  // ArcCSLCommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_CSE"] });  // ArcCommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_SCA"] });  // ArcSCACommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_SCE"] });  // ArcSCECommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_SCL"] });  // ArcSCLCommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_SEA"] });  // ArcSEACommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_SED"] });  // ArcSEDCommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_SER"] });  // ArcSERCommand
inventory::submit!(crate::command::CommandRegistration { names: &["ARC_CONT"] });  // ArcContCommand
