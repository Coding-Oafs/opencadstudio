//! Selection-cycling list box — shown at the cursor when a click lands on
//! two or more overlapping objects. Each row names a candidate; clicking it
//! adds that object to the current selection. Clicking outside dismisses it.

use iced::widget::{button, column, container, mouse_area, opaque, text};
use iced::{Background, Border, Color, Element, Fill, Length, Theme};

use crate::app::Message;

/// Full-canvas overlay: the list box anchored at `anchor` (canvas
/// coordinates) plus a transparent click-catcher that cancels.
pub fn cycle_popup_overlay(
    anchor: iced::Point,
    items: Vec<(acadrust::Handle, String)>,
) -> Element<'static, Message> {
    let rows: Vec<Element<'static, Message>> = items
        .into_iter()
        .map(|(handle, label)| item_row(handle, label))
        .collect();

    let panel = container(column(rows))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(PANEL_BG)),
            border: Border {
                color: PANEL_BORDER,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .width(Length::Fixed(150.0));

    let positioned = crate::ui::pin_at(anchor, opaque(panel));

    mouse_area(positioned).on_press(Message::CycleCancel).into()
}

fn item_row(handle: acadrust::Handle, label: String) -> Element<'static, Message> {
    let content = text(label).size(11).color(LABEL).align_y(iced::Center);
    let btn = button(content)
        .on_press(Message::CycleSelect(handle))
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => ROW_HOVER,
                _ => Color::TRANSPARENT,
            })),
            ..Default::default()
        })
        .width(Fill)
        .padding([4, 10]);
    // Highlight the underlying object while the cursor is over this row.
    mouse_area(btn)
        .on_enter(Message::CycleHover(Some(handle)))
        .on_exit(Message::CycleHoverExit(handle))
        .into()
}

// ── Colours ───────────────────────────────────────────────────────────────

const PANEL_BG: Color = Color {
    r: 0.15,
    g: 0.15,
    b: 0.15,
    a: 1.0,
};
const PANEL_BORDER: Color = Color {
    r: 0.32,
    g: 0.32,
    b: 0.32,
    a: 1.0,
};
const ROW_HOVER: Color = Color {
    r: 0.22,
    g: 0.45,
    b: 0.62,
    a: 1.0,
};
const LABEL: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
