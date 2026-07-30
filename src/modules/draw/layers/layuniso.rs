use crate::modules::{IconKind, ModuleEvent, ToolDef};

pub fn tool() -> ToolDef {
    ToolDef {
        id: "LAYUNISO",
        label: "Unisolate Layers",
        icon: IconKind::Svg(include_bytes!(
            "../../../../assets/icons/layers/layuniso.svg"
        )),
        event: ModuleEvent::Command("LAYUNISO".to_string()),
    }
}
