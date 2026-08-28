use crate::modules::{IconKind, ModuleEvent, ToolDef};

pub fn tool() -> ToolDef {
    ToolDef {
        id: "WORKFLOWSTANDARDS",
        label: "Workflows &\nStandards",
        icon: IconKind::Svg(include_bytes!("../../../assets/icons/tool_palettes.svg")),
        event: ModuleEvent::Command("WORKFLOWSTANDARDS".to_string()),
    }
}
