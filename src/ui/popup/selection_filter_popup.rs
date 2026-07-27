//! Selection-filter status menu — choose which entity types are selectable.
//! A checked row means that type can be picked; unchecking it excludes the
//! type from interactive selection. Opened from the FILTER status pill.

use rustc_hash::FxHashSet as HashSet;

use iced::widget::{button, container, row, text};
use iced::{Background, Border, Color, Element, Fill, Theme};

use crate::app::Message;
use crate::ui::statusbar::status_menu::Entry;

/// - `types`: entity-type names present in the current layout.
/// - `excluded`: types currently filtered out (unchecked).
pub fn menu_entries(
    types: Vec<String>,
    excluded: &HashSet<String>,
) -> Vec<Entry<'static>> {
    // "Select All / Clear All" header, mirroring the OSNAP popup: Select All
    // clears every exclusion, Clear All excludes every present type.
    let has_types = !types.is_empty();
    let all_included = excluded.is_empty();
    let all_excluded = has_types && types.iter().all(|t| excluded.contains(t));
    let header = row![
        header_btn(
            "Select All",
            Message::SelectionFilterSelectAll,
            has_types && !all_included,
        ),
        header_btn(
            "Clear All",
            Message::SelectionFilterClearAll,
            has_types && !all_excluded,
        ),
    ]
    .spacing(1)
    .padding([4u16, 8]);

    let divider = container(iced::widget::Space::new().height(1))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(DIVIDER)),
            ..Default::default()
        })
        .width(Fill)
        .padding([0, 4]);

    let rows: Vec<Entry<'static>> = if types.is_empty() {
        vec![Entry::stay(empty_row())]
    } else {
        types
            .into_iter()
            .map(|name| {
                let included = !excluded.contains(&name);
                Entry::stay(type_row(name, included))
            })
            .collect()
    };

    let mut entries = vec![Entry::stay(header), Entry::stay(divider)];
    entries.extend(rows);
    entries
}

fn type_row(name: String, included: bool) -> Element<'static, Message> {
    let check = crate::ui::icons::check_cell(included, CHECK_COLOR);

    let lbl = text(name.clone())
        .size(11)
        .color(if included { LABEL_ON } else { LABEL_OFF });

    let content = row![check, lbl].spacing(6).align_y(iced::Center);

    button(content)
        .on_press(Message::ToggleSelectionFilterType(name))
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

fn empty_row() -> Element<'static, Message> {
    container(text("No objects").size(11).color(LABEL_OFF))
        .padding([4, 10])
        .into()
}

fn header_btn(label: &str, msg: Message, enabled: bool) -> Element<'_, Message> {
    let color = if enabled {
        Color { r: 0.70, g: 0.70, b: 0.70, a: 1.0 }
    } else {
        Color { r: 0.38, g: 0.38, b: 0.38, a: 1.0 }
    };
    let b = button(text(label).size(10).color(color));
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
const ROW_HOVER: Color = Color {
    r: 0.22,
    g: 0.22,
    b: 0.22,
    a: 1.0,
};
const DIVIDER: Color = Color {
    r: 0.28,
    g: 0.28,
    b: 0.28,
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
const LABEL_ON: Color = Color {
    r: 0.92,
    g: 0.92,
    b: 0.92,
    a: 1.0,
};
const LABEL_OFF: Color = Color {
    r: 0.6,
    g: 0.6,
    b: 0.6,
    a: 1.0,
};
