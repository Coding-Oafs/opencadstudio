// Center mark tool — interactive command.
//
// Command: DIMCENTER (aliases DCE, CENTERMARK) — pick a Circle or Arc and draw
// a small cross of two Line entities through its center. Each arm of the cross
// extends a distance of `radius * 0.2` from the center in the four cardinal
// directions, so the result is a horizontal line from (cx - m, cy) to
// (cx + m, cy) and a vertical line from (cx, cy - m) to (cx, cy + m), with
// `m = radius * 0.2`. Both lines are committed at once and the command ends.

use acadrust::types::Vector3;
use acadrust::{EntityType, Handle, Line};
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult};
use crate::modules::{IconKind, ModuleEvent, ToolDef};

// ── Ribbon definition ─────────────────────────────────────────────────────

#[allow(dead_code)] // ribbon definition ready for wiring; command works via the command line
pub fn tool() -> ToolDef {
    ToolDef {
        id: "DIMCENTER",
        label: "Center Mark",
        icon: IconKind::Svg(include_bytes!("../../../../assets/icons/line.svg")),
        event: ModuleEvent::Command("DIMCENTER".to_string()),
    }
}

// ── Command implementation ────────────────────────────────────────────────

pub struct DimCenterCommand {
    /// The picked entity, injected by the host before `on_entity_pick` runs.
    /// `None` until the host injects it.
    picked: Option<EntityType>,
}

impl DimCenterCommand {
    pub fn new() -> Self {
        Self { picked: None }
    }

    /// Extract (center, radius) from a Circle or Arc; `None` for anything else.
    fn center_radius(entity: &EntityType) -> Option<(DVec3, f64, DVec3, DVec3)> {
        match entity {
            EntityType::Circle(circle) => {
                let normal = (circle.normal.x, circle.normal.y, circle.normal.z);
                let center = crate::scene::view::transform::ocs_point_to_wcs(
                    (circle.center.x, circle.center.y, circle.center.z),
                    normal,
                );
                let (x, y) = crate::scene::view::transform::ocs_axes(normal);
                Some((
                    DVec3::new(center.0, center.1, center.2),
                    circle.radius,
                    DVec3::new(x.0, x.1, x.2),
                    DVec3::new(y.0, y.1, y.2),
                ))
            }
            EntityType::Arc(arc) => {
                let normal = (arc.normal.x, arc.normal.y, arc.normal.z);
                let center = crate::scene::view::transform::ocs_point_to_wcs(
                    (arc.center.x, arc.center.y, arc.center.z),
                    normal,
                );
                let (x, y) = crate::scene::view::transform::ocs_axes(normal);
                Some((
                    DVec3::new(center.0, center.1, center.2),
                    arc.radius,
                    DVec3::new(x.0, x.1, x.2),
                    DVec3::new(y.0, y.1, y.2),
                ))
            }
            _ => None,
        }
    }

    /// Build the two cross lines from a center and radius.
    fn build_cross(center: DVec3, radius: f64, x: DVec3, y: DVec3) -> Vec<EntityType> {
        let m = radius * 0.2;
        let to_vector = |point: DVec3| Vector3::new(point.x, point.y, point.z);
        let horizontal = Line::from_points(to_vector(center - x * m), to_vector(center + x * m));
        let vertical = Line::from_points(to_vector(center - y * m), to_vector(center + y * m));
        vec![EntityType::Line(horizontal), EntityType::Line(vertical)]
    }
}

impl CadCommand for DimCenterCommand {
    fn name(&self) -> &'static str {
        "DIMCENTER"
    }

    fn prompt(&self) -> String {
        t!("DIMCENTER  Select arc or circle:").into_owned()
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
        match self.picked.as_ref().and_then(Self::center_radius) {
            Some((center, radius, x, y)) => {
                let lines = Self::build_cross(center, radius, x, y);
                CmdResult::ReplaceMany(vec![], lines)
            }
            // Picked something that is not a circle or arc — keep prompting.
            None => CmdResult::NeedPoint,
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
    names: &["DIMCENTER", "DCE", "CENTERMARK"]
}); // DimCenterCommand
