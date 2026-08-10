// DIVIDE command — place Point entities at N equal intervals along an entity.
// MEASURE command — place Point entities at fixed-distance intervals along an entity.

use acadrust::entities::Point as PointEnt;
use cadkernel::space::PlanarCurve;
use acadrust::types::Vector3;
use acadrust::{EntityType, Handle};
use glam::DVec3;
use crate::entities::curve::entity_curve;
use crate::t;

use crate::command::{CadCommand, CmdResult};

// ── DIVIDE ─────────────────────────────────────────────────────────────────

pub struct DivideCommand {
    target: Option<Handle>,
    waiting_for_n: bool,
}

impl DivideCommand {
    pub fn new() -> Self {
        Self {
            target: None,
            waiting_for_n: false,
        }
    }
}

impl CadCommand for DivideCommand {
    fn name(&self) -> &'static str {
        "DIVIDE"
    }

    fn prompt(&self) -> String {
        if self.target.is_none() {
            t!("DIVIDE  Select object to divide:").into_owned()
        } else {
            t!("DIVIDE  Enter number of segments:").into_owned()
        }
    }

    fn needs_entity_pick(&self) -> bool {
        self.target.is_none()
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        self.target = Some(handle);
        self.waiting_for_n = true;
        CmdResult::NeedPoint
    }

    fn wants_text_input(&self) -> bool {
        self.waiting_for_n
    }

    fn dyn_field(&self) -> crate::command::DynField {
        if self.waiting_for_n {
            crate::command::DynField::Scalar
        } else {
            crate::command::DynField::Point
        }
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let n: usize = text.trim().parse().ok().filter(|&n| n >= 2)?;
        let handle = self.target?;
        self.waiting_for_n = false;
        Some(CmdResult::DivideEntity { handle, n })
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

// ── MEASURE ────────────────────────────────────────────────────────────────

pub struct MeasureCommand {
    target: Option<Handle>,
    waiting_for_dist: bool,
}

impl MeasureCommand {
    pub fn new() -> Self {
        Self {
            target: None,
            waiting_for_dist: false,
        }
    }
}

impl CadCommand for MeasureCommand {
    fn name(&self) -> &'static str {
        "MEASURE"
    }

    fn prompt(&self) -> String {
        if self.target.is_none() {
            t!("MEASURE  Select object to measure:").into_owned()
        } else {
            t!("MEASURE  Specify segment length:").into_owned()
        }
    }

    fn needs_entity_pick(&self) -> bool {
        self.target.is_none()
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        self.target = Some(handle);
        self.waiting_for_dist = true;
        CmdResult::NeedPoint
    }

    fn wants_text_input(&self) -> bool {
        self.waiting_for_dist
    }

    fn dyn_field(&self) -> crate::command::DynField {
        if self.waiting_for_dist {
            crate::command::DynField::Scalar
        } else {
            crate::command::DynField::Point
        }
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let dist: f64 = text
            .trim()
            .replace(',', ".")
            .parse()
            .ok()
            .filter(|&d: &f64| d > 0.0)?;
        let handle = self.target?;
        self.waiting_for_dist = false;
        Some(CmdResult::MeasureEntity {
            handle,
            segment_length: dist,
        })
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

// ── Geometry ───────────────────────────────────────────────────────────────

/// Compute N-1 equally spaced points along the entity (DIVIDE).
pub fn divide_entity(entity: &EntityType, n: usize) -> Vec<EntityType> {
    if n < 2 {
        return vec![];
    }
    let Some((curve, total)) = measurable(entity) else {
        return vec![];
    };
    let step = total / n as f64;
    (1..n)
        .map(|k| make_point(curve.point_at_distance(step * k as f64)))
        .collect()
}

/// Compute points at fixed `segment_length` intervals along the entity (MEASURE).
pub fn measure_entity(entity: &EntityType, segment_length: f64) -> Vec<EntityType> {
    if segment_length <= 0.0 {
        return vec![];
    }
    let Some((curve, total)) = measurable(entity) else {
        return vec![];
    };
    let mut pts = Vec::new();
    let mut walked = segment_length;
    while walked < total - 1e-6 {
        pts.push(make_point(curve.point_at_distance(walked)));
        walked += segment_length;
    }
    pts
}

fn make_point(pos: [f64; 3]) -> EntityType {
    let mut p = PointEnt::new();
    p.location = Vector3::new(pos[0], pos[1], pos[2]);
    EntityType::Point(p)
}

/// The entity's curve and its length, or `None` for anything that cannot be
/// walked along — a hatch, a block, an unbounded ray.
fn measurable(entity: &EntityType) -> Option<(PlanarCurve, f64)> {
    let curve = entity_curve(entity)?;
    let total = curve.curve.length();
    (total.is_finite() && total > 1e-10).then_some((curve, total))
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["DIVIDE"] });  // DivideCommand
inventory::submit!(crate::command::CommandRegistration { names: &["MEASURE"] });  // MeasureCommand
