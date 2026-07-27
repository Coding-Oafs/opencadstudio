//! OpenCADStudio-style OSNAP status menu.

use iced::widget::{button, container, row, text};
use iced::{Background, Border, Color, Element, Fill, Length, Theme};

use crate::app::Message;
use crate::snap::{SnapType, Snapper, ALL_SNAP_MODES};
use crate::ui::statusbar::status_menu::Entry;

pub fn menu_entries<'a>(snapper: &'a Snapper) -> Vec<Entry<'a>> {
    let all_on = snapper.all_on();
    let none_on = snapper.none_on();

    let header = row![
        header_btn("Select All", Message::SnapSelectAll, !all_on),
        header_btn("Clear All", Message::SnapClearAll, !none_on),
    ]
    .spacing(1)
    .padding([4u16, 8]);

    // Divider
    let divider = container(iced::widget::Space::new().height(1))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(DIVIDER)),
            ..Default::default()
        })
        .width(Fill)
        .padding([0, 4]);

    let mut entries = vec![Entry::stay(header), Entry::stay(divider)];
    for &(snap_type, _glyph, label) in ALL_SNAP_MODES {
        entries.push(Entry::stay(snap_row(
            snap_type,
            label,
            snapper.is_on(snap_type),
        )));
    }
    entries
}

// ── Individual snap row ───────────────────────────────────────────────────

fn snap_row<'a>(snap_type: SnapType, label: &'a str, active: bool) -> Element<'a, Message> {
    let checkmark = crate::ui::icons::check_cell(active, CHECK_COLOR);

    // SVG marker (not a Unicode glyph) so the symbols render on the web build,
    // whose bundled font lacks them and showed tofu boxes. (#138)
    let icon_el = container(crate::ui::icons::tinted::<Message>(
        crate::ui::icons::osnap(snap_type),
        13.0,
        ICON_COLOR,
    ))
    .width(Length::Fixed(16.0))
    .align_x(iced::Center);

    let label_el = text(label)
        .size(11)
        .color(if active { LABEL_ON } else { LABEL_OFF });

    let content = row![checkmark, icon_el, label_el]
        .spacing(4)
        .align_y(iced::Center);

    button(content)
        .on_press(Message::ToggleSnap(snap_type))
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => ROW_HOVER,
                _ => Color::TRANSPARENT,
            })),
            ..Default::default()
        })
        .width(Fill)
        .padding([3, 8])
        .into()
}

fn header_btn(label: &str, msg: Message, enabled: bool) -> Element<'_, Message> {
    let b = button(text(label).size(10).color(if enabled {
        Color {
            r: 0.70,
            g: 0.70,
            b: 0.70,
            a: 1.0,
        }
    } else {
        Color {
            r: 0.38,
            g: 0.38,
            b: 0.38,
            a: 1.0,
        }
    }));
    let b = if enabled { b.on_press(msg) } else { b };
    b.style(|_: &Theme, status| button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => ROW_HOVER,
            _ => BTN_BG,
        })),
        border: Border {
            color: PANEL_BORDER,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..Default::default()
    })
    .padding([3, 8])
    .into()
}

// ── Colours ───────────────────────────────────────────────────────────────

const PANEL_BORDER: Color = Color {
    r: 0.32,
    g: 0.32,
    b: 0.32,
    a: 1.0,
};
const DIVIDER: Color = Color {
    r: 0.28,
    g: 0.28,
    b: 0.28,
    a: 1.0,
};
const ROW_HOVER: Color = Color {
    r: 0.24,
    g: 0.24,
    b: 0.24,
    a: 1.0,
};
const BTN_BG: Color = Color {
    r: 0.20,
    g: 0.20,
    b: 0.20,
    a: 1.0,
};
const CHECK_COLOR: Color = Color {
    r: 0.35,
    g: 0.75,
    b: 1.00,
    a: 1.0,
};
const ICON_COLOR: Color = Color {
    r: 0.25,
    g: 0.75,
    b: 0.45,
    a: 1.0,
}; // green icon
const LABEL_ON: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
const LABEL_OFF: Color = Color {
    r: 0.52,
    g: 0.52,
    b: 0.52,
    a: 1.0,
};
