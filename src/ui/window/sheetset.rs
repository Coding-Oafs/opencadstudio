//! Docked "Sheet Set Manager": an ordered list of sheets (drawing + layout)
//! that open/activate through the existing multi-tab and layout machinery.
//!
//! Persistence is a simple, documented JSON file (not AutoCAD `.dst`), per the
//! approved plan. Activating a sheet opens its drawing (if not already open)
//! and switches to its layout.

use crate::app::Message;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Length, Theme};
use serde::{Deserialize, Serialize};

const PANEL_W: f32 = 260.0;
const PANEL_BG: Color = Color { r: 0.13, g: 0.13, b: 0.13, a: 1.0 };
const PANEL_BORDER: Color = Color { r: 0.22, g: 0.22, b: 0.24, a: 1.0 };
const ROW_BG: Color = Color { r: 0.16, g: 0.16, b: 0.18, a: 1.0 };
const ROW_HOVER: Color = Color { r: 0.22, g: 0.30, b: 0.42, a: 1.0 };
const TEXT: Color = Color { r: 0.88, g: 0.88, b: 0.88, a: 1.0 };
const DIM: Color = Color { r: 0.62, g: 0.62, b: 0.64, a: 1.0 };

/// One sheet: a named drawing + layout, optionally its own title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub name: String,
    pub drawing: String,
    pub layout: String,
}

/// A sheet set: an ordered collection of sheets plus its own display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetSet {
    pub name: String,
    pub sheets: Vec<Sheet>,
}

impl SheetSet {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// Messages the panel emits. Wrapped in `Message::SheetSet`.
#[derive(Debug, Clone)]
pub enum SheetSetMsg {
    /// Activate a sheet: open its drawing (if needed) and switch layout.
    Activate(usize),
    /// Remove a sheet from the set.
    Remove(usize),
    /// Rename the set.
    RenameSet(String),
    /// Add the current tab's drawing + active layout as a sheet.
    AddCurrent(String),
    ToggleBar,
    Close,
}

/// Panel state held on the app.
#[derive(Default)]
pub struct SheetSetState {
    pub set: Option<SheetSet>,
    pub expanded: bool,
    /// Name to use when adding the current drawing (from the file dialog or
    /// current layout).
    pub pending_name: String,
}

/// Build the docked panel element.
pub fn view(state: &SheetSetState) -> Element<'_, Message> {
    let title_bar = container(
        row![
            text("Sheet Set Manager").size(12).color(TEXT),
            iced::widget::Space::new().width(Fill),
            icon_button("✕", SheetSetMsg::Close),
        ]
        .spacing(4)
        .align_y(iced::Center),
    )
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(ROW_BG)),
        border: Border {
            color: PANEL_BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .width(Fill)
    .padding([5, 6]);

    let body: Element<'_, Message> = match &state.set {
        None => container(text("No sheet set loaded.").size(12).color(DIM))
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .into(),
        Some(set) => {
            let mut col = column![].spacing(2).padding(4);
            // Set name + rename field.
            col = col.push(
                text_input("Sheet set name", &set.name)
                    .on_input(|v| Message::SheetSet(SheetSetMsg::RenameSet(v)))
                    .padding([4, 8])
                    .size(12),
            );
            col = col.push(
                text("Add current drawing:")
                    .size(10)
                    .color(DIM),
            );
            col = col.push(
                text_input("Sheet name (defaults to layout)", &state.pending_name)
                    .on_input(|v| Message::SheetSet(SheetSetMsg::AddCurrent(v)))
                    .padding([4, 8])
                    .size(12),
            );
            col = col.push(
                button(text("Add current sheet").size(12).color(TEXT))
                    .on_press(Message::SheetSet(SheetSetMsg::AddCurrent(
                        state.pending_name.clone(),
                    )))
                    .padding([4, 8])
                    .style(row_style),
            );
            // Sheet list.
            let mut list = column![].spacing(2);
            for (index, sheet) in set.sheets.iter().enumerate() {
                list = list.push(sheet_row(index, sheet));
            }
            if set.sheets.is_empty() {
                list = list.push(
                    container(text("No sheets yet — add the current drawing.")
                        .size(11)
                        .color(DIM))
                    .padding(6),
                );
            }
            col = col.push(scrollable(list).width(Fill).height(Fill));
            col.into()
        }
    };

    container(column![title_bar, body].spacing(6))
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

fn sheet_row(index: usize, sheet: &Sheet) -> Element<'_, Message> {
    let activate = sheet.name.clone();
    let remove = sheet.name.clone();
    container(
        row![
            column![
                text(&sheet.name).size(12).color(TEXT),
                text(format!("{} — {}", sheet.drawing, sheet.layout))
                    .size(10)
                    .color(DIM),
            ]
            .spacing(1)
            .width(Fill),
            button(text("Open").size(11).color(TEXT))
                .on_press(Message::SheetSet(SheetSetMsg::Activate(index)))
                .padding([2, 6])
                .style(row_style),
            button(text("✕").size(11).color(DIM))
                .on_press(Message::SheetSet(SheetSetMsg::Remove(index)))
                .padding([2, 6])
                .style(row_style),
        ]
        .spacing(4)
        .align_y(iced::Center),
    )
    .padding([4, 6])
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(if activate == remove {
            ROW_BG
        } else {
            ROW_BG
        })),
        border: Border {
            color: PANEL_BORDER,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn row_style(_: &Theme, status: iced::widget::button::Status) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: Some(Background::Color(match status {
            iced::widget::button::Status::Hovered
            | iced::widget::button::Status::Pressed => ROW_HOVER,
            _ => ROW_BG,
        })),
        border: Border {
            color: PANEL_BORDER,
            width: 1.0,
            radius: 3.0.into(),
        },
        text_color: TEXT,
        ..Default::default()
    }
}

fn icon_button<'a>(label: &'static str, msg: SheetSetMsg) -> Element<'a, Message> {
    button(text(label).size(14).color(Color::WHITE))
        .on_press(Message::SheetSet(msg))
        .width(Length::Fixed(24.0))
        .height(Length::Fixed(24.0))
        .style(|_: &Theme, status| iced::widget::button::Style {
            background: Some(Background::Color(match status {
                iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
                    ROW_HOVER
                }
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
    fn sheet_set_round_trips_json() {
        let set = SheetSet {
            name: "Project".into(),
            sheets: vec![Sheet {
                name: "Sheet 1".into(),
                drawing: "C:\\x\\a.dwg".into(),
                layout: "Layout1".into(),
            }],
        };
        let json = set.to_json();
        let parsed = SheetSet::from_json(&json).unwrap();
        assert_eq!(parsed, set);
    }
}
