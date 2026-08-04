// Centre line tool — interactive command.
//
// Command: CENTERLINE — pick two straight Line entities and draw a single
// centre line between them, committed as one Line entity.
//
//   * If the two picked lines are (near) parallel, the result is the midline:
//     a line running between the two, centred on the average of their
//     midpoints, oriented along the averaged direction, with a length equal to
//     the longer of the two source lines.
//   * If the two picked lines cross, the result is the bisector of the acute
//     angle at their intersection point, drawn symmetrically either side of
//     that point.
//
// The command picks the first line on the first click, stores it, prompts for
// the second line, then computes the centre line and ends.

use acadrust::types::Vector3;
use acadrust::{EntityType, Handle, Line};
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult, WorkingPlane};
use crate::modules::{IconKind, ModuleEvent, ToolDef};

// ── Ribbon definition ─────────────────────────────────────────────────────

#[allow(dead_code)] // ribbon definition ready for wiring; command works via the command line
pub fn tool() -> ToolDef {
    ToolDef {
        id: "CENTERLINE",
        label: "Center Line",
        icon: IconKind::Svg(include_bytes!("../../../../assets/icons/line.svg")),
        event: ModuleEvent::Command("CENTERLINE".to_string()),
    }
}

// ── Geometry helpers ──────────────────────────────────────────────────────

/// A line reduced to the quantities the centre-line maths needs.
#[derive(Clone, Copy)]
struct LineGeom {
    start: DVec3,
    end: DVec3,
}

impl LineGeom {
    fn from_line(line: &Line) -> Self {
        Self {
            start: DVec3::new(line.start.x, line.start.y, line.start.z),
            end: DVec3::new(line.end.x, line.end.y, line.end.z),
        }
    }

    fn midpoint(&self) -> DVec3 {
        (self.start + self.end) * 0.5
    }

    fn length(&self) -> f64 {
        (self.end - self.start).length()
    }

    /// Normalized direction in 3D, or `None` if the line is degenerate.
    fn dir(&self) -> Option<DVec3> {
        let d = self.end - self.start;
        let len = d.length();
        if len <= 1e-9 {
            None
        } else {
            Some(d / len)
        }
    }
}

/// 2D cross product of the XY components of two vectors.
fn cross_xy(a: DVec3, b: DVec3) -> f64 {
    a.x * b.y - a.y * b.x
}

fn to_v3(p: DVec3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}

/// Compute the centre line between two source lines. `None` when the result
/// would be degenerate (zero-length sources, or coincident parallel lines that
/// give no usable direction).
fn compute_center_line(a: LineGeom, b: LineGeom) -> Option<Line> {
    let da = a.dir()?;
    let db = b.dir()?;

    // Average Z of the four endpoints keeps the result on a sensible plane even
    // when the picked lines sit at slightly different elevations.
    let avg_z = (a.start.z + a.end.z + b.start.z + b.end.z) * 0.25;

    let parallel = cross_xy(da, db).abs() <= 1e-9;

    if parallel {
        // Midline: average direction (flip db so the two add constructively),
        // centred on the average of the two midpoints, length = longer source.
        let db_aligned = if da.dot(db) < 0.0 { -db } else { db };
        let avg = da + db_aligned;
        let dir = if avg.length() <= 1e-9 {
            da
        } else {
            avg / avg.length()
        };
        let dir = DVec3::new(dir.x, dir.y, 0.0);
        let dir = if dir.length() <= 1e-9 {
            return None;
        } else {
            dir / dir.length()
        };

        let mid = (a.midpoint() + b.midpoint()) * 0.5;
        let half = a.length().max(b.length()) * 0.5;
        if half <= 1e-9 {
            return None;
        }
        let center = DVec3::new(mid.x, mid.y, avg_z);
        let p0 = center - dir * half;
        let p1 = center + dir * half;
        Some(Line::from_points(to_v3(p0), to_v3(p1)))
    } else {
        // Intersecting: bisector of the acute angle at the intersection point.
        // Solve the 2D line-line intersection in XY.
        //   a.start + t*da = b.start + s*db
        let r = da; // direction of line a
        let s = db; // direction of line b
        let denom = cross_xy(r, s);
        if denom.abs() <= 1e-12 {
            return None;
        }
        let qp = b.start - a.start;
        let t = cross_xy(qp, s) / denom;
        let ix = a.start.x + r.x * t;
        let iy = a.start.y + r.y * t;
        let p = DVec3::new(ix, iy, avg_z);

        // Orient both directions away from the intersection point, choosing the
        // half of each line whose midpoint lies on the side we keep, so the two
        // oriented directions bisect the acute angle between the lines.
        let a_away = orient_away(p, a);
        let b_away = orient_away(p, b);
        // For an acute-angle bisector, flip b_away if the two oriented dirs open
        // obtuse (their dot is negative).
        let b_oriented = if a_away.dot(b_away) < 0.0 {
            -b_away
        } else {
            b_away
        };
        let bis = a_away + b_oriented;
        let bis = DVec3::new(bis.x, bis.y, 0.0);
        if bis.length() <= 1e-9 {
            return None;
        }
        let dir = bis / bis.length();

        let half = ((a.length() + b.length()) * 0.5) * 0.5;
        if half <= 1e-9 {
            return None;
        }
        let p0 = p - dir * half;
        let p1 = p + dir * half;
        Some(Line::from_points(to_v3(p0), to_v3(p1)))
    }
}

/// Unit direction from `p` toward the farther endpoint of `line` (the side that
/// extends away from the intersection point).
fn orient_away(p: DVec3, line: LineGeom) -> DVec3 {
    let to_start = line.start - p;
    let to_end = line.end - p;
    let pick = if to_end.length() >= to_start.length() {
        to_end
    } else {
        to_start
    };
    let len = pick.length();
    if len <= 1e-9 {
        // Fall back to the raw line direction if the pick degenerates.
        line.dir().unwrap_or(DVec3::X)
    } else {
        pick / len
    }
}

// ── Command implementation ────────────────────────────────────────────────

pub struct CenterLineCommand {
    /// First picked Line's geometry, captured once the first pick lands.
    first: Option<LineGeom>,
    /// The entity injected by the host before `on_entity_pick` runs; `None`
    /// until the host injects it.
    picked: Option<EntityType>,
    plane: WorkingPlane,
}

impl CenterLineCommand {
    pub fn new() -> Self {
        Self {
            first: None,
            picked: None,
            plane: WorkingPlane::default(),
        }
    }

    /// Extract a `LineGeom` from a picked entity; `None` for anything else.
    fn as_line(&self, entity: &EntityType) -> Option<LineGeom> {
        match entity {
            EntityType::Line(line) => {
                let mut geom = LineGeom::from_line(line);
                geom.start = self.plane.to_local(geom.start);
                geom.end = self.plane.to_local(geom.end);
                Some(geom)
            }
            _ => None,
        }
    }
}

impl CadCommand for CenterLineCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "CENTERLINE"
    }

    fn prompt(&self) -> String {
        if self.first.is_none() {
            t!("CENTERLINE  Select first line:").into_owned()
        } else {
            t!("CENTERLINE  Select second line:").into_owned()
        }
    }

    fn needs_entity_pick(&self) -> bool {
        true
    }

    fn inject_before_entity_pick(&self) -> bool {
        true
    }

    fn inject_picked_entity(&mut self, entity: EntityType) {
        self.picked = Some(entity);
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        // Resolve the just-picked entity; ignore the pick if it isn't a Line.
        let picked = self.picked.take();
        let geom = match picked.as_ref().and_then(|entity| self.as_line(entity)) {
            Some(g) => g,
            None => return CmdResult::NeedPoint,
        };
        // Reject zero-length picks rather than corrupting the result.
        if geom.length() <= 1e-9 {
            return CmdResult::NeedPoint;
        }

        match self.first {
            None => {
                // First line captured; keep prompting for the second.
                self.first = Some(geom);
                CmdResult::NeedPoint
            }
            Some(first) => match compute_center_line(first, geom) {
                Some(line) => CmdResult::CommitAndExit(
                    self.plane.place_entity(EntityType::Line(line)),
                ),
                // Degenerate combination (coincident / no usable result):
                // keep prompting for a usable second line.
                None => CmdResult::NeedPoint,
            },
        }
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration {
    names: &["CENTERLINE"]
}); // CenterLineCommand
