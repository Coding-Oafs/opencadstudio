//! Status-bar customization and layout-list menu entries.

use iced::widget::{button, row, text};
use iced::{Background, Color, Element, Fill, Theme};

use crate::app::Message;
use crate::ui::statusbar::statusbar_config::{StatusBarConfig, StatusPill};
use crate::ui::statusbar::status_menu::Entry;

pub fn customization_entries(config: &StatusBarConfig) -> Vec<Entry<'static>> {
    StatusPill::ALL
        .iter()
        .map(|&pill| {
            Entry::stay(menu_row(
                pill.label(),
                config.is_visible(pill),
                Message::ToggleStatusPill(pill),
            ))
        })
        .collect()
}

pub fn layout_entries<'a>(
    layouts: &[String],
    current: &str,
) -> Vec<Entry<'a>> {
    layouts
        .iter()
        .map(|name| Entry::close(layout_row(name.clone(), name == current)))
        .collect()
}

fn layout_row<'a>(name: String, is_current: bool) -> Element<'a, Message> {
    let lbl = text(name.clone())
        .size(11)
        .color(if is_current { LABEL_ON } else { LABEL_OFF });
    button(row![lbl].align_y(iced::Center))
        .on_press(Message::LayoutSwitch(name))
        .style(move |_: &Theme, status| button::Style {
            background: Some(Background::Color(match (is_current, status) {
                (_, button::Status::Hovered) => ROW_HOVER,
                (true, _) => Color {
                    r: 0.18,
                    g: 0.26,
                    b: 0.36,
                    a: 1.0,
                },
                _ => Color::TRANSPARENT,
            })),
            ..Default::default()
        })
        .width(Fill)
        .padding([4, 12])
        .into()
}

fn menu_row(label: &'static str, checked: bool, msg: Message) -> Element<'static, Message> {
    let check = crate::ui::icons::check_cell(checked, CHECK_COLOR);

    let lbl = text(label)
        .size(11)
        .color(if checked { LABEL_ON } else { LABEL_OFF });

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
