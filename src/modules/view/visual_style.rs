// The visual styles a viewport can be drawn in, in one list.
//
// Three places offer the same choice and used to describe it separately: the
// Visual Style dropdown on the ribbon, the render-mode picker with its preview
// cube, and the VISUALSTYLES command. The ribbon's copy had drifted furthest —
// it named four styles, two of which reached nothing (its "Shaded" carried the
// id `SOLID`, which is the 2D solid *drawing* command, and `HIDDEN` matched no
// command at all). Naming each style once, next to the command that applies it,
// is what keeps the three in step. (#621)

use std::sync::OnceLock;

use acadrust::entities::ViewportRenderMode as Mode;

use crate::modules::IconKind;

const WIREFRAME_ICON: IconKind =
    IconKind::Svg(include_bytes!("../../../assets/icons/wireframe.svg"));
const HIDDEN_ICON: IconKind = IconKind::Svg(include_bytes!("../../../assets/icons/hidden.svg"));
const SHADED_ICON: IconKind = IconKind::Svg(include_bytes!("../../../assets/icons/solid.svg"));

pub struct VisualStyle {
    pub mode: Mode,
    /// What the user sees, wherever the style is offered.
    pub label: &'static str,
    /// The command line that applies it. Ribbon items dispatch this verbatim,
    /// so it doubles as the item's identity for the checkmark and the
    /// last-used-tool memory. The keyword is the render mode's own name — there
    /// is one set of styles now, so there is one set of names for them.
    pub command: &'static str,
    pub icon: IconKind,
}

/// Every style, in the order they are offered — wireframes, then hidden line,
/// then the shaded ones, each pair plain before with-edges.
pub const VISUAL_STYLES: &[VisualStyle] = &[
    VisualStyle {
        mode: Mode::Wireframe2D,
        label: "Wireframe 2D",
        command: "VISUALSTYLES WIREFRAME2D",
        icon: WIREFRAME_ICON,
    },
    VisualStyle {
        mode: Mode::Wireframe3D,
        label: "Wireframe 3D",
        command: "VISUALSTYLES WIREFRAME3D",
        icon: WIREFRAME_ICON,
    },
    VisualStyle {
        mode: Mode::HiddenLine,
        label: "Hidden Line",
        command: "VISUALSTYLES HIDDENLINE",
        icon: HIDDEN_ICON,
    },
    VisualStyle {
        mode: Mode::FlatShaded,
        label: "Flat Shaded",
        command: "VISUALSTYLES FLATSHADED",
        icon: SHADED_ICON,
    },
    VisualStyle {
        mode: Mode::GouraudShaded,
        label: "Gouraud Shaded",
        command: "VISUALSTYLES GOURAUDSHADED",
        icon: SHADED_ICON,
    },
    VisualStyle {
        mode: Mode::FlatShadedWithEdges,
        label: "Flat Shaded + Edges",
        command: "VISUALSTYLES FLATSHADEDWITHEDGES",
        icon: SHADED_ICON,
    },
    VisualStyle {
        mode: Mode::GouraudShadedWithEdges,
        label: "Gouraud Shaded + Edges",
        command: "VISUALSTYLES GOURAUDSHADEDWITHEDGES",
        icon: SHADED_ICON,
    },
];

impl VisualStyle {
    /// The bare keyword, without the verb its `command` spells out.
    pub fn keyword(&self) -> &'static str {
        self.command
            .strip_prefix("VISUALSTYLES ")
            .unwrap_or(self.command)
    }
}

/// The choices an interactive style prompt offers, in table order.
pub fn keyword_choices() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    VISUAL_STYLES
        .iter()
        .map(|style| (style.label, style.keyword(), None))
        .collect()
}

/// The prompt those choices are announced with. Built once from the table so
/// the line the user reads cannot list something the picker does not offer.
pub fn keyword_prompt() -> &'static str {
    static PROMPT: OnceLock<String> = OnceLock::new();
    PROMPT
        .get_or_init(|| {
            let listed: Vec<&str> = VISUAL_STYLES.iter().map(|style| style.label).collect();
            format!("Visual style  [{}]:", listed.join(" / "))
        })
        .as_str()
}

pub fn label_for(mode: Mode) -> &'static str {
    VISUAL_STYLES
        .iter()
        .find(|style| style.mode == mode)
        .map(|style| style.label)
        .unwrap_or("Wireframe 2D")
}

/// The style a style keyword names. Only the seven exist; nothing maps onto a
/// nearest neighbour, because there is no longer anything else to map from.
pub fn mode_for_keyword(keyword: &str) -> Option<Mode> {
    let keyword = keyword.trim().to_uppercase();
    VISUAL_STYLES
        .iter()
        .find(|style| style.keyword() == keyword)
        .map(|style| style.mode)
}
