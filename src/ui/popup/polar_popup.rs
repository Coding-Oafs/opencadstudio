//! Polar-tracking angle status menu.

use iced::widget::{button, container, row, text, text_input};
use iced::{Background, Color, Element, Fill, Length, Theme};

use crate::app::Message;
use crate::ui::statusbar::status_menu::Entry;

/// Angle increments offered in the picker, in degrees. Matches the common
/// drafting set and adds the fine 1° step requested in #264.
const PRESETS: &[f32] = &[90.0, 45.0, 30.0, 22.5, 18.0, 15.0, 10.0, 5.0, 1.0];

/// Format an angle without a trailing `.0` (so `22.5°` but `15°`).
pub fn angle_label(deg: f32) -> String {
    if (deg.fract()).abs() < 1e-3 {
        format!("{:.0}°", deg)
    } else {
        format!("{deg}°")
    }
}

pub fn menu_entries<'a>(
    current: f32,
    custom: &'a str,
) -> Vec<Entry<'a>> {
    let mut entries: Vec<Entry<'a>> = PRESETS
        .iter()
        .map(|&deg| {
            let active = (current - deg).abs() < 1e-3;
            Entry::close(angle_row(deg, active))
        })
        .collect();

    // Free-entry custom angle: type a value and press Enter to apply.
    let custom_field = text_input("Custom…", custom)
        .on_input(Message::PolarCustomInput)
        .on_submit(Message::SubmitPolarCustom)
        .size(11)
        .padding([2, 6])
        .width(Length::Fixed(58.0));
    let custom_row = container(
        row![
            custom_field,
            text("°").size(11).color(LABEL_OFF),
        ]
        .spacing(4)
        .align_y(iced::Center),
    )
    .padding([5, 10]);
    entries.push(Entry::stay(custom_row));
    entries
}

fn angle_row<'a>(deg: f32, active: bool) -> Element<'a, Message> {
    let check = crate::ui::icons::check_cell(active, CHECK_COLOR);

    let lbl = text(angle_label(deg))
        .size(11)
        .color(if active { LABEL_ON } else { LABEL_OFF });

    let content = row![check, lbl].spacing(6).align_y(iced::Center);

    button(content)
        .on_press(Message::SetPolarAngle(deg))
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
