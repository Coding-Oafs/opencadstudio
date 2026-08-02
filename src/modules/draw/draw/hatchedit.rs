// HATCHEDIT — edit an existing hatch entity's pattern, scale, or angle.
//
// Workflow:
//   1. Pick or pre-select a Hatch entity.
//   2. Enter options:
//        P <name>     — change pattern (ANSI31, SOLID, etc.)
//        S <value>    — change scale
//        A <degrees>  — change angle
//      Press Enter to apply changes.

use acadrust::Handle;
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult};

enum HatcheditStep {
    PickHatch,
    EditOptions {
        handle: Handle,
        name: String,
        scale: f32,
        angle: f32,
    },
}

pub struct HatcheditCommand {
    step: HatcheditStep,
}

impl HatcheditCommand {
    pub fn new() -> Self {
        Self {
            step: HatcheditStep::PickHatch,
        }
    }

    pub fn with_handle(handle: Handle, name: String, scale: f32, angle: f32) -> Self {
        Self {
            step: HatcheditStep::EditOptions {
                handle,
                name,
                scale,
                angle,
            },
        }
    }
}

impl CadCommand for HatcheditCommand {
    fn name(&self) -> &'static str {
        "HATCHEDIT"
    }

    fn prompt(&self) -> String {
        match &self.step {
            HatcheditStep::PickHatch => t!("HATCHEDIT  Select hatch:").into_owned(),
            HatcheditStep::EditOptions {
                name, scale, angle, ..
            } => {
                let scale = format!("{scale:.4}");
                let angle = format!("{angle:.1}");
                t!(
                    "HATCHEDIT  Pattern:%{name}  Scale:%{scale}  Angle:%{angle}  [P <pat> / S <scale> / A <angle> | Enter to apply]:",
                    name = name,
                    scale = scale,
                    angle = angle
                )
                .into_owned()
            }
        }
    }

    fn needs_entity_pick(&self) -> bool {
        matches!(self.step, HatcheditStep::PickHatch)
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        // Actual hatch model retrieval happens in commands.rs dispatch.
        // Store handle; name/scale/angle filled in by dispatch.
        self.step = HatcheditStep::EditOptions {
            handle,
            name: String::new(),
            scale: 1.0,
            angle: 0.0,
        };
        CmdResult::NeedPoint
    }

    fn wants_text_input(&self) -> bool {
        matches!(self.step, HatcheditStep::EditOptions { .. })
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let (handle, name, scale, angle) = match &mut self.step {
            HatcheditStep::EditOptions {
                handle,
                name,
                scale,
                angle,
            } => (*handle, name, scale, angle),
            _ => return None,
        };

        let text = text.trim().to_uppercase();

        if text.is_empty() {
            // Apply and exit
            return Some(CmdResult::HatcheditApply {
                handle,
                name: name.clone(),
                scale: *scale,
                angle: *angle,
            });
        }

        // Parse option: P/S/A followed by value
        if let Some(rest) = text.strip_prefix('P') {
            let n = rest.trim().to_string();
            if !n.is_empty() {
                *name = n;
            }
            return Some(CmdResult::NeedPoint);
        }
        if let Some(rest) = text.strip_prefix('S') {
            if let Ok(v) = rest.trim().replace(',', ".").parse::<f32>() {
                if v > 0.0 {
                    *scale = v;
                }
            }
            return Some(CmdResult::NeedPoint);
        }
        if let Some(rest) = text.strip_prefix('A') {
            if let Ok(v) = rest.trim().replace(',', ".").parse::<f32>() {
                *angle = v;
            }
            return Some(CmdResult::NeedPoint);
        }

        // Unrecognized — stay and re-prompt
        Some(CmdResult::NeedPoint)
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }
    fn on_enter(&mut self) -> CmdResult {
        // Enter without text → apply current settings
        let (handle, name, scale, angle) = match &self.step {
            HatcheditStep::EditOptions {
                handle,
                name,
                scale,
                angle,
            } => (*handle, name.clone(), *scale, *angle),
            _ => return CmdResult::Cancel,
        };
        CmdResult::HatcheditApply {
            handle,
            name,
            scale,
            angle,
        }
    }
    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["HATCHEDIT"] });  // HatcheditCommand
