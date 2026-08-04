// DONUT command — create a filled circular ring (thick LwPolyline).
//
// A donut is an LwPolyline with:
//   - 2 vertices at (cx ± r_avg, 0), both with bulge = 1.0  (two 180° CCW arcs)
//   - start_width = end_width = (outer - inner) / 2
//   - is_closed = true
//
// Workflow:
//   1. Type inner diameter (or 0 for a filled circle)
//   2. Type outer diameter
//   3. Click center point(s); Enter to finish

use acadrust::entities::{LwPolyline, LwVertex};
use acadrust::EntityType;
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult, WorkingPlane};

pub struct DonutCommand {
    state: DonutState,
    inner_r: f64,
    outer_r: f64,
    plane: WorkingPlane,
}

enum DonutState {
    AskInner,
    AskOuter,
    PlaceCenter,
}

impl DonutCommand {
    pub fn new() -> Self {
        Self {
            state: DonutState::AskInner,
            inner_r: 0.0,
            outer_r: 1.0,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for DonutCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "DONUT"
    }

    fn prompt(&self) -> String {
        match &self.state {
            DonutState::AskInner => t!("DONUT  Specify inside diameter <0>:").into_owned(),
            DonutState::AskOuter => t!("DONUT  Specify outside diameter:").into_owned(),
            DonutState::PlaceCenter => {
                t!("DONUT  Specify center of donut (Enter to exit):").into_owned()
            }
        }
    }

    fn wants_text_input(&self) -> bool {
        matches!(self.state, DonutState::AskInner | DonutState::AskOuter)
    }

    fn dyn_field(&self) -> crate::command::DynField {
        if matches!(self.state, DonutState::AskInner | DonutState::AskOuter) {
            crate::command::DynField::Scalar
        } else {
            crate::command::DynField::Point
        }
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let val: f64 = text
            .trim()
            .replace(',', ".")
            .parse()
            .ok()
            .filter(|&v: &f64| v >= 0.0)?;
        match &self.state {
            DonutState::AskInner => {
                self.inner_r = val / 2.0;
                self.state = DonutState::AskOuter;
                Some(CmdResult::NeedPoint)
            }
            DonutState::AskOuter => {
                if val <= 0.0 {
                    return None;
                }
                self.outer_r = val / 2.0;
                if self.inner_r > self.outer_r {
                    std::mem::swap(&mut self.inner_r, &mut self.outer_r);
                }
                self.state = DonutState::PlaceCenter;
                Some(CmdResult::NeedPoint)
            }
            _ => None,
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match &self.state {
            DonutState::PlaceCenter => {
                let center = self.plane.to_local(pt);
                let entity = make_donut(center.x, center.y, center.z, self.inner_r, self.outer_r);
                // Keep command active so user can place more donuts.
                CmdResult::CommitEntity(self.plane.place_entity(entity))
            }
            _ => CmdResult::NeedPoint,
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        match &self.state {
            DonutState::AskInner => {
                // Accept default 0 for inner diameter
                self.inner_r = 0.0;
                self.state = DonutState::AskOuter;
                CmdResult::NeedPoint
            }
            DonutState::PlaceCenter => CmdResult::Cancel,
            _ => CmdResult::Cancel,
        }
    }
}

fn make_donut(cx: f64, cy: f64, elevation: f64, inner_r: f64, outer_r: f64) -> EntityType {
    use acadrust::types::Vector2;
    let r_avg = (inner_r + outer_r) / 2.0;
    let width = outer_r - inner_r;

    let mut p = LwPolyline::new();
    p.is_closed = true;
    p.constant_width = width;
    p.elevation = elevation;

    // Vertex at (cx - r, cy) with bulge 1.0 (180° CCW arc to next vertex)
    let mut v0 = LwVertex::new(Vector2::new(cx - r_avg, cy));
    v0.bulge = 1.0;
    v0.start_width = width;
    v0.end_width = width;

    // Vertex at (cx + r, cy) with bulge 1.0 (second 180° arc back to v0)
    let mut v1 = LwVertex::new(Vector2::new(cx + r_avg, cy));
    v1.bulge = 1.0;
    v1.start_width = width;
    v1.end_width = width;

    p.vertices = vec![v0, v1];
    EntityType::LwPolyline(p)
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["DONUT"] });  // DonutCommand
