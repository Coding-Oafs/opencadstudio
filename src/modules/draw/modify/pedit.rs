// PEDIT command — edit a polyline entity (#263).
//
// Flow:
//   Select polyline — picking a LINE or ARC instead asks "Turn it into one?"
//   (Yes converts in place and continues editing the result).
//   Options: Close / Open / Join / Width / Fit / Spline / Decurve / eXit.
//   Join gathers more segments (lines / arcs / polylines) and merges every
//   contiguous run through the JOIN machinery. (Vertex editing lives on the
//   polyline's grips, not in PEDIT.)
//   Fit smooths the segments into tangent-blended arcs, Spline replaces the
//   shape with a sampled cubic B-spline of the vertex frame, Decurve
//   straightens every segment.

use acadrust::entities::LwVertex;
use acadrust::types::Vector2;
use acadrust::{EntityType, Handle};
use glam::{DVec2, DVec3};
use crate::t;
use rustc_hash::FxHashMap as HashMap;

use crate::command::{CadCommand, CmdResult};

const TAU: f64 = std::f64::consts::TAU;

/// What PEDIT knows about a pickable entity, captured at dispatch.
#[derive(Clone, Copy)]
pub struct PeditTarget {
    /// LwPolyline / Polyline2D — a valid edit target.
    pub is_poly: bool,
    /// Line / Arc — offered for conversion on pick.
    pub convertible: bool,
}

enum Mode {
    PickTarget,
    Options,
    /// Picked a Line/Arc: asking "Turn it into one? [Yes/No]".
    ConvertPrompt(Handle),
    AwaitWidth,
    /// Join: gathering additional segments; Enter merges.
    JoinGather(Vec<Handle>),
}

pub struct PeditCommand {
    target: Option<Handle>,
    info: HashMap<u64, PeditTarget>,
    mode: Mode,
}

impl PeditCommand {
    pub fn new(info: HashMap<u64, PeditTarget>) -> Self {
        Self {
            target: None,
            info,
            mode: Mode::PickTarget,
        }
    }

    /// Adopt a pre-selected entity (pickfirst): a selected polyline skips the
    /// pick step, a selected line/arc goes straight to the convert prompt.
    pub fn with_preselection(mut self, handles: &[Handle]) -> Self {
        for &h in handles {
            let Some(info) = self.info.get(&h.value()).copied() else {
                continue;
            };
            if info.is_poly {
                self.target = Some(h);
                self.mode = Mode::Options;
                break;
            }
            if info.convertible {
                self.mode = Mode::ConvertPrompt(h);
                break;
            }
        }
        self
    }

}

impl CadCommand for PeditCommand {
    fn name(&self) -> &'static str {
        "PEDIT"
    }

    fn prompt(&self) -> String {
        match &self.mode {
            Mode::PickTarget => {
                t!("PEDIT  Select polyline (or a line/arc to convert):").into_owned()
            }
            Mode::ConvertPrompt(_) => t!(
                "PEDIT  Object is not a polyline. Turn it into one?  [Yes/No] <Y>:"
            )
            .into_owned(),
            Mode::AwaitWidth => t!("PEDIT  Specify new width:").into_owned(),
            Mode::JoinGather(list) => t!(
                "PEDIT Join  Select objects to join (%{count} picked), Enter to merge:",
                count = list.len().saturating_sub(1)
            )
            .into_owned(),
            Mode::Options => t!("PEDIT  Enter option:").into_owned(),
        }
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        match &self.mode {
            Mode::Options => vec![
                CmdOption::new(t!("Close").as_ref(), "C"),
                CmdOption::new(t!("Open").as_ref(), "O"),
                CmdOption::new(t!("Join").as_ref(), "J"),
                CmdOption::new(t!("Width").as_ref(), "W"),
                CmdOption::new(t!("Fit").as_ref(), "F"),
                CmdOption::new(t!("Spline").as_ref(), "S"),
                CmdOption::new(t!("Decurve").as_ref(), "D"),
                CmdOption::new(t!("eXit").as_ref(), "X"),
            ],
            Mode::ConvertPrompt(_) => {
                vec![CmdOption::new(t!("Yes").as_ref(), "Y"), CmdOption::new(t!("No").as_ref(), "N")]
            }
            Mode::JoinGather(_) => vec![CmdOption::enter(t!("Join").as_ref())],
            _ => vec![],
        }
    }

    fn needs_entity_pick(&self) -> bool {
        matches!(self.mode, Mode::PickTarget)
    }

    fn is_selection_gathering(&self) -> bool {
        // Join uses the normal selection system, so single picks AND
        // window/crossing boxes both gather objects.
        matches!(self.mode, Mode::JoinGather(_))
    }

    fn on_selection_complete(&mut self, handles: Vec<Handle>) -> CmdResult {
        if let (Some(target), Mode::JoinGather(list)) = (self.target, &mut self.mode) {
            list.clear();
            list.push(target);
            for h in handles {
                if h != target && self.info.contains_key(&h.value()) && !list.contains(&h) {
                    list.push(h);
                }
            }
        }
        CmdResult::NeedPoint
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        match &mut self.mode {
            Mode::PickTarget => {
                let Some(info) = self.info.get(&handle.value()).copied() else {
                    return CmdResult::NeedPoint;
                };
                if info.is_poly {
                    self.target = Some(handle);
                    self.mode = Mode::Options;
                } else if info.convertible {
                    self.mode = Mode::ConvertPrompt(handle);
                }
                CmdResult::NeedPoint
            }
            _ => CmdResult::NeedPoint,
        }
    }

    fn on_entity_replaced(&mut self, old: Handle, new_handles: &[Handle]) {
        // A Yes-conversion (or Break) replaced the entity — adopt the first
        // piece as the live target and carry its bookkeeping over.
        if let Some(&nh) = new_handles.first() {
            self.info.remove(&old.value());
            self.info.insert(
                nh.value(),
                PeditTarget {
                    is_poly: true,
                    convertible: false,
                },
            );
            self.target = Some(nh);
            self.mode = Mode::Options;
        }
    }

    fn wants_text_input(&self) -> bool {
        !matches!(self.mode, Mode::PickTarget)
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let up = text.trim().to_uppercase();
        match &mut self.mode {
            Mode::PickTarget => None,
            Mode::ConvertPrompt(handle) => {
                let handle = *handle;
                match up.as_str() {
                    "Y" | "YES" | "" => Some(CmdResult::PeditOp {
                        handle,
                        op: PeditOp::ConvertToPolyline,
                    }),
                    "N" | "NO" => {
                        self.mode = Mode::PickTarget;
                        Some(CmdResult::NeedPoint)
                    }
                    _ => Some(CmdResult::NeedPoint),
                }
            }
            Mode::AwaitWidth => {
                let handle = self.target?;
                let w: f64 = up
                    .replace(',', ".")
                    .parse()
                    .ok()
                    .filter(|&v: &f64| v >= 0.0)?;
                self.mode = Mode::Options;
                Some(CmdResult::PeditOp {
                    handle,
                    op: PeditOp::SetWidth(w),
                })
            }
            Mode::JoinGather(_) => None,
            Mode::Options => {
                let handle = self.target?;
                match up.as_str() {
                    "X" | "EXIT" => Some(CmdResult::Cancel),
                    "C" | "CLOSE" => Some(CmdResult::PeditOp {
                        handle,
                        op: PeditOp::SetClosed(true),
                    }),
                    "O" | "OPEN" => Some(CmdResult::PeditOp {
                        handle,
                        op: PeditOp::SetClosed(false),
                    }),
                    "W" | "WIDTH" => {
                        self.mode = Mode::AwaitWidth;
                        Some(CmdResult::NeedPoint)
                    }
                    "J" | "JOIN" => {
                        self.mode = Mode::JoinGather(vec![handle]);
                        Some(CmdResult::NeedPoint)
                    }
                    "F" | "FIT" => Some(CmdResult::PeditOp {
                        handle,
                        op: PeditOp::Fit,
                    }),
                    "S" | "SPLINE" => Some(CmdResult::PeditOp {
                        handle,
                        op: PeditOp::Spline,
                    }),
                    "D" | "DECURVE" => Some(CmdResult::PeditOp {
                        handle,
                        op: PeditOp::Decurve,
                    }),
                    _ => {
                        // Inline shorthand `W <value>`.
                        if let Some(rest) = up.strip_prefix("W ") {
                            let w: f64 = rest.trim().replace(',', ".").parse().ok()?;
                            if w >= 0.0 {
                                return Some(CmdResult::PeditOp {
                                    handle,
                                    op: PeditOp::SetWidth(w),
                                });
                            }
                        }
                        None
                    }
                }
            }
        }
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        match &self.mode {
            Mode::JoinGather(list) if list.len() >= 2 => CmdResult::JoinEntities(list.clone()),
            Mode::JoinGather(_) => {
                self.mode = Mode::Options;
                CmdResult::NeedPoint
            }
            Mode::ConvertPrompt(h) => CmdResult::PeditOp {
                handle: *h,
                op: PeditOp::ConvertToPolyline,
            },
            _ => CmdResult::Cancel,
        }
    }
}

// ── Op enum (used in CmdResult) ────────────────────────────────────────────

#[derive(Clone)]
pub enum PeditOp {
    SetClosed(bool),
    SetWidth(f64),
    /// Replace the picked Line/Arc with an equivalent LwPolyline (#263).
    ConvertToPolyline,
    Fit,
    Spline,
    Decurve,
}

// ── Apply logic (pure entity edits; driver handles convert/break/marker) ──

pub fn apply_pedit(entity: &mut EntityType, op: &PeditOp) -> bool {
    match op {
        PeditOp::SetClosed(closed) => match entity {
            EntityType::LwPolyline(p) => {
                p.is_closed = *closed;
                true
            }
            EntityType::Polyline2D(p) => {
                if *closed {
                    p.close();
                } else {
                    p.flags.set_closed(false);
                }
                true
            }
            _ => false,
        },
        PeditOp::SetWidth(w) => match entity {
            EntityType::LwPolyline(p) => {
                p.constant_width = *w;
                for v in &mut p.vertices {
                    v.start_width = *w;
                    v.end_width = *w;
                }
                true
            }
            _ => false,
        },
        PeditOp::Fit => match entity {
            EntityType::LwPolyline(p) => fit_curve(p),
            _ => false,
        },
        PeditOp::Spline => match entity {
            EntityType::LwPolyline(p) => spline_smooth(p),
            _ => false,
        },
        PeditOp::Decurve => match entity {
            EntityType::LwPolyline(p) => {
                for v in &mut p.vertices {
                    v.bulge = 0.0;
                }
                true
            }
            _ => false,
        },
        // Handled by the driver (it replaces the entity, not edits in place).
        PeditOp::ConvertToPolyline => false,
    }
}

/// A Line or Arc as an equivalent 2-vertex LwPolyline (common carried over,
/// handle NULL for the replace flow). `None` for anything else.
pub fn convert_to_polyline(entity: &EntityType) -> Option<EntityType> {
    let mut pl = acadrust::LwPolyline::new();
    match entity {
        EntityType::Line(l) => {
            pl.common = l.common.clone();
            pl.vertices = vec![
                LwVertex::new(Vector2::new(l.start.x, l.start.y)),
                LwVertex::new(Vector2::new(l.end.x, l.end.y)),
            ];
        }
        EntityType::Arc(a) => {
            pl.common = a.common.clone();
            let (sa, ea) = (a.start_angle, a.end_angle);
            let sweep = {
                let s = (ea - sa).rem_euclid(TAU);
                if s.abs() < 1e-12 {
                    TAU
                } else {
                    s
                }
            };
            let p0 = (
                a.center.x + a.radius * sa.cos(),
                a.center.y + a.radius * sa.sin(),
            );
            let p1 = (
                a.center.x + a.radius * ea.cos(),
                a.center.y + a.radius * ea.sin(),
            );
            let mut v0 = LwVertex::new(Vector2::new(p0.0, p0.1));
            v0.bulge = (sweep / 4.0).tan();
            pl.vertices = vec![v0, LwVertex::new(Vector2::new(p1.0, p1.1))];
        }
        _ => return None,
    }
    pl.common.handle = Handle::NULL;
    Some(EntityType::LwPolyline(pl))
}

// ── Curve fitting ─────────────────────────────────────────────────────────

fn vert_xy(v: &LwVertex) -> DVec2 {
    DVec2::new(v.location.x, v.location.y)
}

/// Wrap an angle into (-pi, pi].
fn wrap_angle(mut a: f64) -> f64 {
    while a > std::f64::consts::PI {
        a -= TAU;
    }
    while a <= -std::f64::consts::PI {
        a += TAU;
    }
    a
}

/// Bulge of the arc `a` -> `b` whose tangent AT `a` is `t` (entry form).
fn bulge_entry(a: DVec2, t: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    if d.length_squared() < 1e-12 {
        return 0.0;
    }
    (wrap_angle(d.y.atan2(d.x) - t.y.atan2(t.x)) / 2.0).tan()
}

/// Bulge of the arc `a` -> `b` whose tangent AT `b` is `t` (exit form).
fn bulge_exit(a: DVec2, b: DVec2, t: DVec2) -> f64 {
    let d = b - a;
    if d.length_squared() < 1e-12 {
        return 0.0;
    }
    (wrap_angle(t.y.atan2(t.x) - d.y.atan2(d.x)) / 2.0).tan()
}

/// PEDIT Fit: replace every segment with a BIARC — two arcs that leave the
/// start vertex along its tangent, meet each other tangentially at an
/// inserted knee vertex, and arrive at the end vertex along ITS tangent.
/// Both segments at a vertex share that vertex's tangent (the average of the
/// neighbouring chords), so the whole run is tangent-continuous — arcs
/// mutually tangent everywhere, which a single arc per segment cannot do.
fn fit_curve(p: &mut acadrust::LwPolyline) -> bool {
    let n = p.vertices.len();
    if n < 3 {
        return false;
    }
    let pts: Vec<DVec2> = p.vertices.iter().map(vert_xy).collect();
    let chord = |i: usize| -> DVec2 {
        let a = pts[i % n];
        let b = pts[(i + 1) % n];
        (b - a).normalize_or_zero()
    };
    // Per-vertex tangents.
    let tangent = |i: usize| -> DVec2 {
        if !p.is_closed && i == 0 {
            return chord(0);
        }
        if !p.is_closed && i == n - 1 {
            return chord(n - 2);
        }
        let prev = chord((i + n - 1) % n);
        let cur = chord(i % n);
        let sum = prev + cur;
        if sum.length_squared() < 1e-12 {
            cur
        } else {
            sum.normalize()
        }
    };
    let seg_count = if p.is_closed { n } else { n - 1 };
    let mut out: Vec<LwVertex> = Vec::with_capacity(seg_count * 2 + 1);
    for i in 0..seg_count {
        let p0 = pts[i % n];
        let p1 = pts[(i + 1) % n];
        let t0 = tangent(i % n);
        let t1 = tangent((i + 1) % n);
        let d = p1 - p0;
        let src = &p.vertices[i % n];
        let mut push = |q: DVec2, bulge: f64| {
            let mut v = LwVertex::new(Vector2::new(q.x, q.y));
            v.bulge = bulge;
            v.start_width = src.start_width;
            v.end_width = src.end_width;
            out.push(v);
        };
        if d.length_squared() < 1e-12 {
            push(p0, 0.0);
            continue;
        }
        // Both tangents already aligned with the chord: keep it straight.
        let dn = d.normalize();
        if (dn - t0).length_squared() < 1e-12 && (dn - t1).length_squared() < 1e-12 {
            push(p0, 0.0);
            continue;
        }
        // Classic equal-parameter biarc: apexes A = p0 + k*t0, B = p1 - k*t1
        // and the knee M at their midpoint. Tangency at M needs |AB| = 2k,
        // i.e. 2(1 - t0.t1) k^2 + 2 (d.(t0+t1)) k - |d|^2 = 0 — A and B are
        // then the tangent-line apexes of their arcs, so both arcs' tangents
        // at M run along AB and the pair is tangent-continuous.
        let dot_tt = t0.dot(t1).clamp(-1.0, 1.0);
        let b_lin = d.dot(t0 + t1);
        let a_quad = 2.0 * (1.0 - dot_tt);
        let k = if a_quad.abs() > 1e-9 {
            let disc = b_lin * b_lin + a_quad * d.length_squared();
            if disc < 0.0 {
                push(p0, bulge_entry(p0, t0, p1));
                continue;
            }
            (-b_lin + disc.sqrt()) / a_quad
        } else if b_lin.abs() > 1e-9 {
            // Parallel tangents: the quadratic degenerates to one root.
            d.length_squared() / (2.0 * b_lin)
        } else {
            // Anti-parallel S with no along-tangent reach: symmetric split.
            (d.length_squared() / 4.0).sqrt()
        };
        if !k.is_finite() || k <= 1e-9 {
            push(p0, bulge_entry(p0, t0, p1));
            continue;
        }
        let m = (p0 + t0 * k + p1 - t1 * k) * 0.5;
        push(p0, bulge_entry(p0, t0, m));
        push(m, bulge_exit(m, p1, t1));
    }
    if !p.is_closed {
        let mut last = p.vertices[n - 1].clone();
        last.bulge = 0.0;
        out.push(last);
    }
    if out.len() < 2 {
        return false;
    }
    p.vertices = out;
    true
}

/// PEDIT Spline: replace the shape with a sampled uniform cubic B-spline of
/// the vertex frame (8 samples per span; a closed frame wraps around).
fn spline_smooth(p: &mut acadrust::LwPolyline) -> bool {
    let n = p.vertices.len();
    if n < 3 {
        return false;
    }
    let pts: Vec<DVec2> = p.vertices.iter().map(vert_xy).collect();
    const S: usize = 8;
    let bspline = |p0: DVec2, p1: DVec2, p2: DVec2, p3: DVec2, t: f64| -> DVec2 {
        let t2 = t * t;
        let t3 = t2 * t;
        (p0 * (-t3 + 3.0 * t2 - 3.0 * t + 1.0)
            + p1 * (3.0 * t3 - 6.0 * t2 + 4.0)
            + p2 * (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0)
            + p3 * t3)
            / 6.0
    };
    let mut out: Vec<DVec2> = Vec::new();
    if p.is_closed {
        for i in 0..n {
            let p0 = pts[(i + n - 1) % n];
            let p1 = pts[i];
            let p2 = pts[(i + 1) % n];
            let p3 = pts[(i + 2) % n];
            for s in 0..S {
                out.push(bspline(p0, p1, p2, p3, s as f64 / S as f64));
            }
        }
    } else {
        // Clamp the ends by repeating the end control points so the curve
        // starts and finishes on them.
        let at = |i: isize| -> DVec2 { pts[i.clamp(0, n as isize - 1) as usize] };
        for i in -1..(n as isize - 2) {
            let (p0, p1, p2, p3) = (at(i), at(i + 1), at(i + 2), at(i + 3));
            let last_span = i == n as isize - 3;
            let steps = if last_span { S + 1 } else { S };
            for s in 0..steps {
                out.push(bspline(p0, p1, p2, p3, s as f64 / S as f64));
            }
        }
        // Pin the exact endpoints.
        if let (Some(v), Some(q)) = (out.first_mut(), pts.first()) {
            *v = *q;
        }
        if let (Some(v), Some(q)) = (out.last_mut(), pts.last()) {
            *v = *q;
        }
    }
    if out.len() < 2 {
        return false;
    }
    let w = p.constant_width;
    p.vertices = out
        .into_iter()
        .map(|q| {
            let mut v = LwVertex::new(Vector2::new(q.x, q.y));
            v.start_width = w;
            v.end_width = w;
            v
        })
        .collect();
    true
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["PEDIT"] });  // PeditCommand
