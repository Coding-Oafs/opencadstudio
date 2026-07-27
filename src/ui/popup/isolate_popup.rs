//! Isolate / Hide / End Isolation status menu.

use iced::widget::{button, row, text};
use iced::{Background, Color, Element, Fill, Theme};

use crate::app::Message;
use crate::ui::statusbar::status_menu::Entry;

/// - `has_selection`: enables Isolate / Hide (they act on the selection).
/// - `isolation_active`: enables End Isolation (something is hidden).
pub fn menu_entries(
    has_selection: bool,
    isolation_active: bool,
) -> Vec<Entry<'static>> {
    vec![
        action_entry(
            "Isolate Objects",
            has_selection,
            Message::Command("ISOLATEOBJECTS".to_string()),
        ),
        action_entry(
            "Hide Objects",
            has_selection,
            Message::Command("HIDEOBJECTS".to_string()),
        ),
        action_entry(
            "End Isolation",
            isolation_active,
            Message::Command("UNISOLATEOBJECTS".to_string()),
        ),
    ]
}

fn action_entry(label: &'static str, enabled: bool, msg: Message) -> Entry<'static> {
    let row = action_row(label, enabled, msg);
    if enabled {
        Entry::close(row)
    } else {
        Entry::stay(row)
    }
}

fn action_row(label: &'static str, enabled: bool, msg: Message) -> Element<'static, Message> {
    let lbl = text(label)
        .size(11)
        .color(if enabled { LABEL_ON } else { LABEL_OFF });
    let content = row![lbl].align_y(iced::Center);

    let mut btn = button(content)
        .style(move |_: &Theme, status| button::Style {
            background: Some(Background::Color(match (enabled, status) {
                (true, button::Status::Hovered) => ROW_HOVER,
                _ => Color::TRANSPARENT,
            })),
            ..Default::default()
        })
        .width(Fill)
        .padding([4, 12]);
    if enabled {
        btn = btn.on_press(msg);
    }
    btn.into()
}

// ── Colours ───────────────────────────────────────────────────────────────

const ROW_HOVER: Color = Color {
    r: 0.22,
    g: 0.22,
    b: 0.22,
    a: 1.0,
};
const LABEL_ON: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
const LABEL_OFF: Color = Color {
    r: 0.5,
    g: 0.5,
    b: 0.5,
    a: 1.0,
};
