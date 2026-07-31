// Rotate tool — ribbon definition + interactive command.
//
// Command:  ROTATE (RO)
//   Requires at least one entity selected before starting.
//   Step 1: pick rotation center
//   Step 2: specify a relative rotation angle, or choose Reference
//
//   Reference rotation: the reference angle may be typed or measured between
//   two points. The new absolute angle is then typed or picked from the center;
//   the applied rotation is new-angle - reference-angle.

use acadrust::Handle;
use glam::DVec3;

use crate::command::{CadCommand, CmdResult, DynField, EntityTransform};
use crate::modules::draw::defaults;
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::scene::model::wire_model::WireModel;

// ── Ribbon definition ──────────────────────────────────────────────────────

pub fn tool() -> ToolDef {
    ToolDef {
        id: "ROTATE",
        label: "Rotate",
        icon: IconKind::Svg(include_bytes!("../../../../assets/icons/rotate.svg")),
        event: ModuleEvent::Command("ROTATE".to_string()),
    }
}

// ── Command implementation ─────────────────────────────────────────────────

enum Step {
    Center,
    Angle { center: DVec3 },
    RefFirst { center: DVec3 },
    RefSecond { center: DVec3, first: DVec3 },
    RefNew { center: DVec3, ref_angle: f64 },
}

pub struct RotateCommand {
    handles: Vec<Handle>,
    wire_models: Vec<WireModel>,
    step: Step,
    default_angle: f64, // degrees
}

impl RotateCommand {
    pub fn new(handles: Vec<Handle>, wire_models: Vec<WireModel>) -> Self {
        Self {
            handles,
            wire_models,
            step: Step::Center,
            default_angle: defaults::get_rotate_angle(),
        }
    }

    fn commit(&self, center: DVec3, angle_rad: f64) -> CmdResult {
        defaults::set_rotate_angle(angle_rad.to_degrees());
        CmdResult::TransformSelected(
            self.handles.clone(),
            EntityTransform::Rotate { center, angle_rad },
        )
    }
}

impl CadCommand for RotateCommand {
    fn name(&self) -> &'static str {
        "ROTATE"
    }

    fn prompt(&self) -> String {
        match &self.step {
            Step::Center => format!(
                "ROTATE  Specify rotation center  [{} objects]:",
                self.handles.len()
            ),
            Step::Angle { .. } => format!(
                "ROTATE  Specify rotation angle  <{:.4}>:",
                self.default_angle
            ),
            Step::RefFirst { .. } => {
                "ROTATE  Specify first reference point or type reference angle:".into()
            }
            Step::RefSecond { .. } => "ROTATE  Specify second reference point:".into(),
            Step::RefNew { ref_angle, .. } => format!(
                "ROTATE  Specify new absolute angle  [ref={:.1}°]:",
                ref_angle.to_degrees()
            ),
        }
    }

    fn options(&self) -> Vec<crate::command::CmdOption> {
        use crate::command::CmdOption;
        match self.step {
            Step::Angle { .. } => vec![CmdOption::new("Reference", "R")],
            _ => vec![],
        }
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match &self.step {
            Step::Center => {
                self.step = Step::Angle { center: pt };
                CmdResult::NeedPoint
            }
            Step::Angle { center } => {
                let center = *center;
                let direction = pt - center;
                if direction.x.hypot(direction.y) <= f64::EPSILON {
                    return CmdResult::NeedPoint;
                }
                let angle_rad = direction.y.atan2(direction.x);
                self.commit(center, angle_rad)
            }
            Step::RefFirst { center } => {
                let center = *center;
                self.step = Step::RefSecond { center, first: pt };
                CmdResult::NeedPoint
            }
            Step::RefSecond { center, first } => {
                let direction = pt - *first;
                if direction.x.hypot(direction.y) <= f64::EPSILON {
                    return CmdResult::NeedPoint;
                }
                let center = *center;
                let ref_angle = direction.y.atan2(direction.x);
                self.step = Step::RefNew { center, ref_angle };
                CmdResult::NeedPoint
            }
            Step::RefNew { center, ref_angle } => {
                let center = *center;
                let direction = pt - center;
                if direction.x.hypot(direction.y) <= f64::EPSILON {
                    return CmdResult::NeedPoint;
                }
                let new_angle = direction.y.atan2(direction.x);
                self.commit(center, new_angle - *ref_angle)
            }
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        // At the normal angle step, Enter uses the stored default angle.
        if let Step::Angle { center } = &self.step {
            let center = *center;
            return self.commit(center, self.default_angle.to_radians());
        }
        CmdResult::Cancel
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let t = text.trim();
        match &self.step {
            Step::Angle { center } => {
                let center = *center;
                let low = t.to_ascii_lowercase();
                if low == "r" || low == "reference" {
                    self.step = Step::RefFirst { center };
                    return Some(CmdResult::NeedPoint);
                }
                // The value already carries the correct sign when it comes
                // from dynamic input.
                let deg: f64 = t.replace(',', ".").parse().ok()?;
                Some(self.commit(center, deg.to_radians()))
            }
            Step::RefFirst { center } => {
                let center = *center;
                let ref_deg: f64 = t.replace(',', ".").parse().ok()?;
                self.step = Step::RefNew {
                    center,
                    ref_angle: ref_deg.to_radians(),
                };
                Some(CmdResult::NeedPoint)
            }
            Step::RefNew { center, ref_angle } => {
                let (center, ref_angle) = (*center, *ref_angle);
                let new_deg: f64 = t.replace(',', ".").parse().ok()?;
                Some(self.commit(center, new_deg.to_radians() - ref_angle))
            }
            Step::Center | Step::RefSecond { .. } => None,
        }
    }

    fn on_preview_wires(&mut self, pt: DVec3) -> Vec<WireModel> {
        let (center, angle_rad) = match &self.step {
            Step::Angle { center } => {
                let direction = pt - *center;
                if direction.x.hypot(direction.y) <= f64::EPSILON {
                    return vec![];
                }
                (*center, direction.y.atan2(direction.x))
            }
            Step::RefSecond { first, .. } => {
                return vec![WireModel::solid(
                    "rubber_band".into(),
                    vec![
                        [first.x as f32, first.y as f32, first.z as f32],
                        [pt.x as f32, pt.y as f32, pt.z as f32],
                    ],
                    WireModel::CYAN,
                    false,
                )];
            }
            Step::RefNew { center, ref_angle } => {
                let direction = pt - *center;
                if direction.x.hypot(direction.y) <= f64::EPSILON {
                    return vec![];
                }
                (*center, direction.y.atan2(direction.x) - *ref_angle)
            }
            _ => return vec![],
        };
        // Object ghosts rotated to the new angle. The rotation sweep arc is
        // drawn by the dynamic-input overlay (polar guide), not here.
        self.wire_models
            .iter()
            .map(|w| w.rotated(center.as_vec3(), angle_rad as f32))
            .collect()
    }

    fn dyn_field(&self) -> DynField {
        match self.step {
            Step::Angle { .. } | Step::RefNew { .. } => DynField::Angle,
            Step::RefFirst { .. } => DynField::Scalar,
            _ => DynField::Point,
        }
    }

    fn dyn_spec(&self) -> Option<crate::command::DynSpec> {
        use crate::command::{DynAnchor, DynFieldSpec, DynGuide, DynRole, DynSpec};
        // Both normal rotation and Reference's new angle are absolute cursor
        // directions from the center. Reference mode subtracts its stored
        // reference angle only when previewing or committing the transform.
        match self.step {
            Step::Angle { center } | Step::RefNew { center, .. } => Some(DynSpec {
                anchor: DynAnchor::Point(center),
                fields: vec![DynFieldSpec::new(DynRole::Angle)],
                guide: DynGuide::Polar,
                ref_point: Some(center + DVec3::X),
            }),
            _ => None,
        }
    }

    fn dyn_commit_as_text(&self) -> bool {
        matches!(self.step, Step::Angle { .. } | Step::RefFirst { .. })
    }

    fn dyn_live_value(&self, cursor: DVec3) -> Option<f64> {
        match self.step {
            Step::Angle { center } | Step::RefNew { center, .. } => {
                let direction = cursor - center;
                (direction.x.hypot(direction.y) > f64::EPSILON).then(|| {
                    crate::command::dyn_display_angle_deg(
                        direction.y.atan2(direction.x) as f32,
                    ) as f64
                })
            }
            _ => None,
        }
    }
}
