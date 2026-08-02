// DIMBREAK command — insert a break in a dimension/extension line where
// another object crosses it.
//
// Simplified workflow:
//   1. Select the dimension to break
//   2. Select the object that crosses it (or Enter for AUTO mode on all crossings)
//
// Because breaking a dimension line requires deep geometric intersection logic
// that depends on the renderer, this implementation records the target handles
// and emits a ReplaceEntity sentinel that commands.rs intercepts to apply the break.

use acadrust::Handle;
use glam::DVec3;

use crate::command::{CadCommand, CmdResult};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::t;

pub const ICON: IconKind = IconKind::Svg(include_bytes!("../../../assets/icons/dim_break.svg"));

pub fn tool() -> ToolDef {
    ToolDef {
        id: "DIMBREAK",
        label: "Dim Break",
        icon: ICON,
        event: ModuleEvent::Command("DIMBREAK".to_string()),
    }
}

enum Step {
    PickDim,
    PickCrossing { dim_handle: Handle },
}

pub struct DimBreakCommand {
    step: Step,
}

impl DimBreakCommand {
    pub fn new() -> Self {
        Self {
            step: Step::PickDim,
        }
    }
}

impl CadCommand for DimBreakCommand {
    fn name(&self) -> &'static str {
        "DIMBREAK"
    }

    fn prompt(&self) -> String {
        match &self.step {
            Step::PickDim => t!("DIMBREAK  Select dimension to break:").into_owned(),
            Step::PickCrossing { .. } => {
                t!("DIMBREAK  Select object to break at, or Enter for Auto:").into_owned()
            }
        }
    }

    fn needs_entity_pick(&self) -> bool {
        true
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        match &self.step {
            Step::PickDim => {
                self.step = Step::PickCrossing { dim_handle: handle };
                CmdResult::NeedPoint
            }
            Step::PickCrossing { dim_handle } => {
                let dim_h = *dim_handle;
                // Emit sentinel: the actual break logic is in commands.rs
                use acadrust::entities::XLine;
                let mut xl = XLine::default();
                xl.common.layer = format!("__DIMBREAK__{},{}", dim_h.value(), handle.value());
                CmdResult::ReplaceEntity(dim_h, vec![acadrust::EntityType::XLine(xl)])
            }
        }
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        // Auto mode: apply to all crossings
        if let Step::PickCrossing { dim_handle } = &self.step {
            let dim_h = *dim_handle;
            use acadrust::entities::XLine;
            let mut xl = XLine::default();
            xl.common.layer = format!("__DIMBREAK_AUTO__{}", dim_h.value());
            return CmdResult::ReplaceEntity(dim_h, vec![acadrust::EntityType::XLine(xl)]);
        }
        CmdResult::Cancel
    }
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["DIMBREAK"] });  // DimBreakCommand
