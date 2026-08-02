// DIMSPACE command — adjust the spacing between parallel linear/aligned dimensions.
//
// Workflow:
//   1. Select the base dimension
//   2. Select the other dimensions to space (click each, Enter to finish)
//   3. Enter spacing value (or 0 for automatic equal spacing)

use acadrust::Handle;
use glam::DVec3;

use crate::command::{CadCommand, CmdResult};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::t;

pub const ICON: IconKind = IconKind::Svg(include_bytes!("../../../assets/icons/dim_space.svg"));

pub fn tool() -> ToolDef {
    ToolDef {
        id: "DIMSPACE",
        label: "Dim Space",
        icon: ICON,
        event: ModuleEvent::Command("DIMSPACE".to_string()),
    }
}

enum Step {
    PickBase,
    PickOthers { base: Handle, others: Vec<Handle> },
    EnterSpacing { base: Handle, others: Vec<Handle> },
}

pub struct DimSpaceCommand {
    step: Step,
}

impl DimSpaceCommand {
    pub fn new() -> Self {
        Self {
            step: Step::PickBase,
        }
    }
}

impl CadCommand for DimSpaceCommand {
    fn name(&self) -> &'static str {
        "DIMSPACE"
    }

    fn prompt(&self) -> String {
        match &self.step {
            Step::PickBase => t!("DIMSPACE  Select base dimension:").into_owned(),
            Step::PickOthers { others, .. } => t!(
                "DIMSPACE  Select dimension to space (%{count} selected, Enter when done):",
                count = others.len()
            )
            .into_owned(),
            Step::EnterSpacing { .. } => t!("DIMSPACE  Enter value (0 = auto):").into_owned(),
        }
    }

    fn needs_entity_pick(&self) -> bool {
        matches!(self.step, Step::PickBase | Step::PickOthers { .. })
    }

    fn on_entity_pick(&mut self, handle: Handle, _pt: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        match &mut self.step {
            Step::PickBase => {
                self.step = Step::PickOthers {
                    base: handle,
                    others: vec![],
                };
                CmdResult::NeedPoint
            }
            Step::PickOthers { others, .. } => {
                if !others.contains(&handle) {
                    others.push(handle);
                }
                CmdResult::NeedPoint
            }
            _ => CmdResult::NeedPoint,
        }
    }

    fn wants_text_input(&self) -> bool {
        matches!(self.step, Step::EnterSpacing { .. })
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if let Step::EnterSpacing { base, others } = &self.step {
            let spacing: f64 = text.trim().parse().unwrap_or(0.0);
            let b = *base;
            let o = others.clone();
            // Emit sentinel for commands.rs to handle
            use acadrust::entities::XLine;
            let mut xl = XLine::default();
            let handles_str: Vec<String> = o.iter().map(|h| h.value().to_string()).collect();
            xl.common.layer = format!(
                "__DIMSPACE__{},{},{}",
                b.value(),
                handles_str.join(";"),
                spacing
            );
            return Some(CmdResult::ReplaceEntity(
                b,
                vec![acadrust::EntityType::XLine(xl)],
            ));
        }
        None
    }

    fn on_point(&mut self, _pt: DVec3) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        match &self.step {
            Step::PickOthers { base, others } if !others.is_empty() => {
                let b = *base;
                let o = others.clone();
                self.step = Step::EnterSpacing { base: b, others: o };
                CmdResult::NeedPoint
            }
            _ => CmdResult::Cancel,
        }
    }
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["DIMSPACE", "DSPACE"] });  // DimSpaceCommand
