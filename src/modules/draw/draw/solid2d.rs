// 2D solid tool — interactive command.
//
// Command: SOLID (reachable as SO / SOLID2D — the bare SOLID verb currently
// toggles the shaded display) — pick three or four corner points and commit a
// filled triangle or quadrilateral. Four points may be picked either around
// the perimeter (the natural order shown by the rubber band) or in AutoCAD's
// traditional Z pattern. The command normalizes both forms to the DXF corner
// order before committing. Enter after the third point commits a triangle.

use acadrust::entities::Solid;
use acadrust::types::Vector3;
use acadrust::EntityType;

use crate::command::{CadCommand, CmdResult};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::scene::model::wire_model::WireModel;
use glam::DVec3;

// ── Ribbon definition ─────────────────────────────────────────────────────

#[allow(dead_code)] // ribbon definition ready for wiring; command works via the command line
pub fn tool() -> ToolDef {
    ToolDef {
        id: "SOLID2D",
        label: "2D Solid",
        icon: IconKind::Svg(include_bytes!("../../../../assets/icons/line.svg")),
        event: ModuleEvent::Command("SOLID2D".to_string()),
    }
}

// ── Command implementation ────────────────────────────────────────────────

pub struct Solid2dCommand {
    /// Corner points picked so far (3 → triangle, 4 → quadrilateral).
    points: Vec<DVec3>,
}

impl Solid2dCommand {
    pub fn new() -> Self {
        Self { points: Vec::new() }
    }

    fn v3(p: DVec3) -> Vector3 {
        Vector3::new(p.x, p.y, p.z)
    }

    /// True when walking the four points in pick order produces a bow-tie.
    /// SOLID2D currently commits into the default world-XY OCS, so use that
    /// same plane when deciding whether the last two points need swapping.
    fn pick_order_crosses(points: &[DVec3]) -> bool {
        if points.len() != 4 {
            return false;
        }
        let orient = |a: DVec3, b: DVec3, c: DVec3| {
            (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
        };
        let crosses = |a: DVec3, b: DVec3, c: DVec3, d: DVec3| {
            let ab_c = orient(a, b, c);
            let ab_d = orient(a, b, d);
            let cd_a = orient(c, d, a);
            let cd_b = orient(c, d, b);
            ((ab_c > 0.0 && ab_d < 0.0) || (ab_c < 0.0 && ab_d > 0.0))
                && ((cd_a > 0.0 && cd_b < 0.0) || (cd_a < 0.0 && cd_b > 0.0))
        };
        crosses(points[0], points[1], points[2], points[3])
            || crosses(points[1], points[2], points[3], points[0])
    }

    /// Convert either perimeter picks (1-2-3-4) or classic SOLID Z picks
    /// (1-2-4-3 around the perimeter) into the entity's DXF Z order.
    fn solid_from_four_points(points: &[DVec3]) -> Solid {
        let perimeter_order = !Self::pick_order_crosses(points);
        let (third, fourth) = if perimeter_order {
            (points[3], points[2])
        } else {
            (points[2], points[3])
        };
        Solid::new(
            Self::v3(points[0]),
            Self::v3(points[1]),
            Self::v3(third),
            Self::v3(fourth),
        )
    }
}

impl CadCommand for Solid2dCommand {
    fn name(&self) -> &'static str {
        "SOLID"
    }

    fn prompt(&self) -> String {
        match self.points.len() {
            0 => "SOLID  Specify first point:".to_string(),
            1 => "SOLID  Specify second point:".to_string(),
            2 => "SOLID  Specify third point:".to_string(),
            _ => "SOLID  Specify fourth point:".to_string(),
        }
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        // After three corners Enter commits a triangle; offer a Done button.
        if self.points.len() == 3 {
            vec![CmdOption::enter("Done")]
        } else {
            Vec::new()
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        self.points.push(pt);
        if self.points.len() == 4 {
            let solid = Self::solid_from_four_points(&self.points);
            CmdResult::CommitAndExit(EntityType::Solid(solid))
        } else {
            CmdResult::NeedPoint
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.points.len() == 3 {
            let p = &self.points;
            let solid = Solid::triangle(Self::v3(p[0]), Self::v3(p[1]), Self::v3(p[2]));
            CmdResult::CommitAndExit(EntityType::Solid(solid))
        } else {
            CmdResult::Cancel
        }
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        if self.points.is_empty() {
            return None;
        }
        // Outline the corners picked so far plus the cursor, closed back to the
        // first point, as a rubber-band hint.
        let mut preview = self.points.clone();
        preview.push(pt);
        if preview.len() == 4 && Self::pick_order_crosses(&preview) {
            preview.swap(2, 3);
        }
        let mut pts: Vec<[f64; 3]> = preview.iter().map(|p| [p.x, p.y, p.z]).collect();
        pts.push([preview[0].x, preview[0].y, preview[0].z]);
        Some(WireModel::solid_f64(
            "rubber_band".to_string(),
            pts,
            WireModel::CYAN,
            false,
        ))
    }
}

// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration {
    names: &["SO", "SOLID", "SOLID2D"]
}); // Solid2dCommand
