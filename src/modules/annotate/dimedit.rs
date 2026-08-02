// DIMEDIT command — edit dimension text or reset it to the measured value.
//
// Workflow:
//   1. Pick a dimension entity
//   2. Enter new text (blank = reset to auto-measured value, "<>" = measured value placeholder)

use acadrust::Handle;
use glam::DVec3;

use crate::command::{CadCommand, CmdResult};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::t;

pub const ICON: IconKind = IconKind::Svg(include_bytes!("../../../assets/icons/dim_edit.svg"));

pub fn tool() -> ToolDef {
    ToolDef {
        id: "DIMEDIT",
        label: "Dim Edit",
        icon: ICON,
        event: ModuleEvent::Command("DIMEDIT".to_string()),
    }
}

enum Step {
    PickDim,
    EnterText { handle: Handle },
}

pub struct DimEditCommand {
    step: Step,
}

impl DimEditCommand {
    pub fn new() -> Self {
        Self {
            step: Step::PickDim,
        }
    }
}

impl CadCommand for DimEditCommand {
    fn name(&self) -> &'static str {
        "DIMEDIT"
    }

    fn prompt(&self) -> String {
        match &self.step {
            Step::PickDim => t!("DIMEDIT  Select dimension:").into_owned(),
            Step::EnterText { .. } => {
                t!("DIMEDIT  Enter text override (blank = reset to measured):").into_owned()
            }
        }
    }

    fn needs_entity_pick(&self) -> bool {
        matches!(self.step, Step::PickDim)
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        self.step = Step::EnterText { handle };
        CmdResult::NeedPoint
    }

    fn wants_text_input(&self) -> bool {
        matches!(self.step, Step::EnterText { .. })
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let handle = match &self.step {
            Step::EnterText { handle } => *handle,
            _ => return None,
        };
        // Empty = reset to auto-measured, "<>" = keep measured, else override
        let new_text = text.trim().to_string();
        Some(CmdResult::DdeditEntity { handle, new_text })
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }
    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["DIMEDIT"] });  // DimEditCommand
