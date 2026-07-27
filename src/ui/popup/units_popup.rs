//! Drawing-units status menu.

use iced::widget::{button, row, text};
use iced::{Background, Color, Element, Fill, Theme};

use crate::app::Message;
use crate::ui::statusbar::status_menu::Entry;

/// Units offered in the picker: (INSUNITS code, menu label).
const UNITS: &[(i16, &str)] = &[
    (0, "Unitless"),
    (4, "Millimeters"),
    (5, "Centimeters"),
    (6, "Meters"),
    (7, "Kilometers"),
    (1, "Inches"),
    (2, "Feet"),
    (3, "Miles"),
    (10, "Yards"),
];

/// Short label shown on the status-bar pill for an INSUNITS code.
pub fn unit_short(code: i16) -> &'static str {
    match code {
        1 => "in",
        2 => "ft",
        3 => "mi",
        4 => "mm",
        5 => "cm",
        6 => "m",
        7 => "km",
        10 => "yd",
        0 => "Unitless",
        _ => "Unit",
    }
}

pub fn menu_entries(current: i16) -> Vec<Entry<'static>> {
    UNITS
        .iter()
        .map(|&(code, label)| {
            Entry::close(unit_row(
                label,
                code == current,
                Message::SetDrawingUnits(code),
            ))
        })
        .collect()
}

fn unit_row(label: &'static str, active: bool, msg: Message) -> Element<'static, Message> {
    let check = crate::ui::icons::check_cell(active, CHECK_COLOR);

    let lbl = text(label)
        .size(11)
        .color(if active { LABEL_ON } else { LABEL_OFF });

    let content = row![check, lbl].spacing(6).align_y(iced::Center);

    button(content)
        .on_press(msg)
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => ROW_HOVER,
                _ => Color::TRANSPARENT,
            })),
            ..Default::default()
        })
        .width(Fill)
        .padding([4, 10])
        .into()
}

// ── Colours ───────────────────────────────────────────────────────────────

const ROW_HOVER: Color = Color {
    r: 0.22,
    g: 0.22,
    b: 0.22,
    a: 1.0,
};
const CHECK_COLOR: Color = Color {
    r: 0.35,
    g: 0.75,
    b: 1.00,
    a: 1.0,
};
const LABEL_ON: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
const LABEL_OFF: Color = Color {
    r: 0.65,
    g: 0.65,
    b: 0.65,
    a: 1.0,
};
