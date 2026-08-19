//! Docked "Tool Palettes" panel: a searchable, grouped grid of command buttons.
//!
//! A palette button simply emits `Message::Command("<command>")` through the
//! same dispatcher the ribbon and command line use, so there is no separate
//! command plumbing — every button is one entry in a palette definition.
//!
//! Palettes are seeded with the built-in LiDAR and editing tool sets the
//! production workflow needs. They live in memory for v1; persisting a
//! user-edited set to JSON is a follow-up (see the plan doc).

use crate::app::Message;
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Background, Border, Color, Element, Fill, Length, Theme};

const PANEL_W: f32 = 232.0;
const PANEL_BG: Color = Color { r: 0.13, g: 0.13, b: 0.13, a: 1.0 };
const PANEL_BORDER: Color = Color { r: 0.22, g: 0.22, b: 0.24, a: 1.0 };
const TOOL_BG: Color = Color { r: 0.16, g: 0.16, b: 0.18, a: 1.0 };
const TOOL_HOVER: Color = Color { r: 0.22, g: 0.30, b: 0.42, a: 1.0 };
const TEXT: Color = Color { r: 0.88, g: 0.88, b: 0.88, a: 1.0 };
const DIM: Color = Color { r: 0.62, g: 0.62, b: 0.64, a: 1.0 };

/// One command button inside a palette.
#[derive(Debug, Clone)]
pub struct PaletteTool {
    pub label: String,
    pub command: String,
}

/// One named, grouped palette.
#[derive(Debug, Clone)]
pub struct Palette {
    pub name: String,
    pub groups: Vec<PaletteGroup>,
}

#[derive(Debug, Clone)]
pub struct PaletteGroup {
    pub title: String,
    pub tools: Vec<PaletteTool>,
}

impl Palette {
    #[cfg(test)]
    fn tool_count(&self) -> usize {
        self.groups.iter().map(|group| group.tools.len()).sum()
    }
}

/// Messages the palette panel emits. Wrapped in `Message::ToolPalettes`.
#[derive(Debug, Clone)]
pub enum ToolPalettesMsg {
    /// Select a different palette tab.
    Select(String),
    /// Run a command by clicking a tool button.
    Run(String),
    ToggleBar,
    Close,
    Search(String),
}

/// Panel state held on the app.
#[derive(Default)]
pub struct ToolPalettes {
    pub selected: Option<String>,
    pub search: String,
    /// Whether the narrow-window bar is expanded (full panel showing).
    pub expanded: bool,
    pub palettes: Vec<Palette>,
}

/// The default built-in palettes, seeded on first open. These surface the
/// commands already implemented by the LiDAR and editing modules — no new
/// command plumbing.
pub fn default_palettes() -> Vec<Palette> {
    vec![
        Palette {
            name: "LiDAR View".into(),
            groups: vec![
                PaletteGroup {
                    title: "Views".into(),
                    tools: vec![
                        tool("Plan (Top)", "VIEW TOP"),
                        tool("Front", "VIEW FRONT"),
                        tool("Right", "VIEW RIGHT"),
                        tool("Isometric", "VIEW ISO"),
                        tool("Perspective", "PERSP"),
                        tool("Orthographic", "PARALLEL"),
                    ],
                },
                PaletteGroup {
                    title: "Display".into(),
                    tools: vec![
                        tool("Color: Class", "POINTCLOUDCOLOR CLASS"),
                        tool("Color: RGB", "POINTCLOUDCOLOR RGB"),
                        tool("Color: Intensity", "POINTCLOUDCOLOR INTENSITY"),
                        tool("Color: Elevation", "POINTCLOUDCOLOR ELEVATION"),
                        tool("Point Size 2", "POINTCLOUDPOINTSIZE 2"),
                        tool("Point Size 5", "POINTCLOUDPOINTSIZE 5"),
                    ],
                },
            ],
        },
        Palette {
            name: "LiDAR Select".into(),
            groups: vec![
                PaletteGroup {
                    title: "Select".into(),
                    tools: vec![
                        tool("Pick Point", "POINTCLOUDSELECTPOINT"),
                        tool("Window Box", "POINTCLOUDSELECTBOX"),
                        tool("Brush", "POINTCLOUDSELECTBRUSH"),
                        tool("Fence", "POINTCLOUDSELECTFENCE"),
                        tool("Z Slice", "POINTCLOUDSELECTSLICE"),
                        tool("Clear", "POINTCLOUDSELECTCLEAR"),
                    ],
                },
                PaletteGroup {
                    title: "Classify".into(),
                    tools: vec![
                        tool("Ground (2)", "POINTCLOUDCLASSIFYSELECTION 2"),
                        tool("Building (6)", "POINTCLOUDCLASSIFYSELECTION 6"),
                        tool("Vegetation (5)", "POINTCLOUDCLASSIFYSELECTION 5"),
                        tool("Water (9)", "POINTCLOUDCLASSIFYSELECTION 9"),
                        tool("Low Point (7)", "POINTCLOUDCLASSIFYSELECTION 7"),
                        tool("Noise (18)", "POINTCLOUDCLASSIFYSELECTION 18"),
                        tool("Auto Ground", "POINTCLOUDGROUND"),
                        tool("Auto Noise", "POINTCLOUDNOISE 2.0 4 7"),
                    ],
                },
                PaletteGroup {
                    title: "Flags".into(),
                    tools: vec![
                        tool("Set Withheld", "POINTCLOUDFLAGSELECTION WITHHELD ON"),
                        tool("Clear Withheld", "POINTCLOUDFLAGSELECTION WITHHELD OFF"),
                        tool("Set Key Point", "POINTCLOUDFLAGSELECTION KEY ON"),
                        tool("Set Overlap", "POINTCLOUDFLAGSELECTION OVERLAP ON"),
                    ],
                },
                PaletteGroup {
                    title: "Section".into(),
                    tools: vec![
                        tool("Draw Section", "POINTCLOUDSECTION"),
                        tool("Section View", "POINTCLOUDSECTIONVIEW"),
                        tool("Step +1", "POINTCLOUDSECTIONMOVE 1"),
                        tool("Step -1", "POINTCLOUDSECTIONMOVE -1"),
                        tool("Wider Band", "POINTCLOUDSECTIONWIDTH 2.0"),
                        tool("Narrower Band", "POINTCLOUDSECTIONWIDTH 0.5"),
                        tool("Clear Section", "POINTCLOUDSECTIONCLEAR"),
                    ],
                },
            ],
        },
        Palette {
            name: "Edit".into(),
            groups: vec![
                PaletteGroup {
                    title: "Modify".into(),
                    tools: vec![
                        tool("Move", "MOVE"),
                        tool("Copy", "COPY"),
                        tool("Rotate", "ROTATE"),
                        tool("Scale", "SCALE"),
                        tool("Mirror", "MIRROR"),
                        tool("Offset", "OFFSET"),
                        tool("Trim", "TRIM"),
                        tool("Extend", "EXTEND"),
                        tool("Fillet", "FILLET"),
                    ],
                },
                PaletteGroup {
                    title: "Draw".into(),
                    tools: vec![
                        tool("Line", "LINE"),
                        tool("Polyline", "PLINE"),
                        tool("Circle", "CIRCLE"),
                        tool("Rectangle", "RECTANG"),
                        tool("Point", "POINT"),
                    ],
                },
            ],
        },
    ]
}

fn tool(label: &'static str, command: &'static str) -> PaletteTool {
    PaletteTool {
        label: label.into(),
        command: command.into(),
    }
}

/// Build the docked panel element from the palette state.
pub fn view(state: &ToolPalettes) -> Element<'_, Message> {
    let query = state.search.trim().to_lowercase();
    let selected = state
        .selected
        .clone()
        .or_else(|| state.palettes.first().map(|p| p.name.clone()));

    let tabs: Element<'_, Message> = if state.palettes.len() <= 1 {
        Space::new().into()
    } else {
        let mut r = row![].spacing(2);
        for palette in &state.palettes {
            let active = selected.as_deref() == Some(palette.name.as_str());
            let name = palette.name.clone();
            r = r.push(
                button(text(&palette.name).size(11).color(TEXT))
                    .on_press(Message::ToolPalettes(ToolPalettesMsg::Select(name)))
                    .padding([3, 8])
                    .style(move |_: &Theme, status| button::Style {
                        background: Some(Background::Color(if active {
                            TOOL_HOVER
                        } else {
                            match status {
                                button::Status::Hovered | button::Status::Pressed => TOOL_HOVER,
                                _ => TOOL_BG,
                            }
                        })),
                        border: Border {
                            color: PANEL_BORDER,
                            width: 1.0,
                            radius: 3.0.into(),
                        },
                        text_color: TEXT,
                        ..Default::default()
                    }),
            );
        }
        r.into()
    };

    let body: Element<'_, Message> = match selected.as_deref().and_then(|name| {
        state.palettes.iter().find(|p| p.name == name)
    }) {
        None => container(text("No palettes").size(12).color(DIM))
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .into(),
        Some(palette) => {
            let mut col = column![].spacing(10).padding(6);
            for group in &palette.groups {
                let mut group_col = column![].spacing(4);
                group_col = group_col.push(text(&group.title).size(10).color(DIM));
                for t in &group.tools {
                    if !query.is_empty() && !t.label.to_lowercase().contains(&query) {
                        continue;
                    }
                    let command = t.command.clone();
                    let label = t.label.clone();
                    group_col = group_col.push(
                        button(text(label).size(12).color(TEXT).width(Fill))
                            .on_press(Message::ToolPalettes(ToolPalettesMsg::Run(command)))
                            .padding([6, 10])
                            .width(Fill)
                            .style(|_: &Theme, status| button::Style {
                                background: Some(Background::Color(match status {
                                    button::Status::Hovered | button::Status::Pressed => {
                                        TOOL_HOVER
                                    }
                                    _ => TOOL_BG,
                                })),
                                border: Border {
                                    color: PANEL_BORDER,
                                    width: 1.0,
                                    radius: 3.0.into(),
                                },
                                text_color: TEXT,
                                ..Default::default()
                            }),
                    );
                }
                col = col.push(group_col);
            }
            scrollable(col).width(Fill).height(Fill).into()
        }
    };

    let title_bar = container(
        row![
            text("Tool Palettes").size(12).color(TEXT),
            iced::widget::Space::new().width(Fill),
            icon_button("✕", ToolPalettesMsg::Close),
        ]
        .spacing(4)
        .align_y(iced::Center),
    )
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(TOOL_BG)),
        border: Border {
            color: PANEL_BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .width(Fill)
    .padding([5, 6]);

    let search_input = text_input("Search tools…", &state.search)
        .on_input(|v| Message::ToolPalettes(ToolPalettesMsg::Search(v)))
        .padding([4, 8])
        .size(12);

    container(column![title_bar, tabs, search_input, body].spacing(6))
        .width(Length::Fixed(PANEL_W))
        .height(Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(PANEL_BG)),
            border: Border {
                color: PANEL_BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn icon_button<'a>(label: &'static str, msg: ToolPalettesMsg) -> Element<'a, Message> {
    button(text(label).size(14).color(Color::WHITE))
        .on_press(Message::ToolPalettes(msg))
        .width(Length::Fixed(24.0))
        .height(Length::Fixed(24.0))
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered | button::Status::Pressed => TOOL_HOVER,
                _ => Color::TRANSPARENT,
            })),
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            text_color: Color::WHITE,
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palettes_are_nonempty_and_commands_present() {
        let palettes = default_palettes();
        assert!(!palettes.is_empty());
        assert!(palettes.iter().all(|p| p.tool_count() > 0));
        // A few representative tools must be present, proving the seed content
        // wires real command strings.
        let commands: Vec<&str> = palettes
            .iter()
            .flat_map(|p| p.groups.iter())
            .flat_map(|g| g.tools.iter())
            .map(|t| t.command.as_str())
            .collect();
        for expected in ["VIEW TOP", "POINTCLOUDCLASSIFYSELECTION 2", "MOVE"] {
            assert!(commands.contains(&expected), "missing {expected}");
        }
    }
}
