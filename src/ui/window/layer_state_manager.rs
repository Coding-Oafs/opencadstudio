//! Layer State Manager — native DWG/DXF named layer-state UI.

use crate::app::Message;
use acadrust::{LayerState, LayerStateMask};
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Background, Border, Element, Fill, Theme};

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.65),
        ),
    }
}

fn button_style(accent: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if accent {
            button::primary(theme, status)
        } else {
            button::secondary(theme, status)
        }
    }
}

fn list_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if selected {
            button::primary(theme, status)
        } else {
            button::subtle(theme, status)
        }
    }
}

fn divider<'a>() -> Element<'a, Message> {
    container(Space::new().width(Fill).height(1))
        .width(Fill)
        .height(1)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.extended_palette().background.neutral.color,
            )),
            ..Default::default()
        })
        .into()
}

fn mask_summary(mask: LayerStateMask) -> String {
    let properties = [
        (LayerStateMask::ON, "On/Off"),
        (LayerStateMask::FROZEN, "Freeze"),
        (LayerStateMask::LOCKED, "Lock"),
        (LayerStateMask::PLOT, "Plot"),
        (LayerStateMask::COLOR, "Color"),
        (LayerStateMask::LINE_TYPE, "Linetype"),
        (LayerStateMask::LINE_WEIGHT, "Lineweight"),
        (LayerStateMask::PLOT_STYLE, "Plot style"),
        (LayerStateMask::TRANSPARENCY, "Transparency"),
    ];
    properties
        .into_iter()
        .filter_map(|(flag, label)| mask.contains(flag).then_some(label))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn view_window<'a>(
    states: Vec<LayerState>,
    selected: Option<&'a str>,
    name: &'a str,
    description: &'a str,
    filter: &'a str,
) -> Element<'a, Message> {
    let selected_state = selected.and_then(|selected| {
        states
            .iter()
            .find(|state| state.name.eq_ignore_ascii_case(selected))
    });
    let query = filter.trim().to_lowercase();
    let rows: Vec<Element<'_, Message>> = states
        .iter()
        .filter(|state| {
            query.is_empty()
                || state.name.to_lowercase().contains(&query)
                || state.description.to_lowercase().contains(&query)
        })
        .map(|state| {
            let is_selected =
                selected.is_some_and(|selected| state.name.eq_ignore_ascii_case(selected));
            let subtitle = if state.description.is_empty() {
                format!("{} layers", state.layers.len())
            } else {
                state.description.clone()
            };
            button(
                column![
                    text(state.name.clone()).size(12),
                    text(subtitle).size(10).style(muted),
                ]
                .spacing(2),
            )
            .on_press(Message::LayerStateManagerSelect(state.name.clone()))
            .style(list_style(is_selected))
            .padding([6, 9])
            .width(Fill)
            .into()
        })
        .collect();

    let empty: Element<'_, Message> = container(
        column![
            text(if states.is_empty() {
                "No layer states in this drawing"
            } else {
                "No matching layer states"
            })
            .size(11)
            .style(muted),
            text("Choose New to capture the current layer settings.")
                .size(10)
                .style(muted),
        ]
        .spacing(4),
    )
    .padding(14)
    .into();

    let state_list: Element<'_, Message> = if rows.is_empty() {
        empty
    } else {
        scrollable(column(rows).spacing(2)).height(Fill).into()
    };

    let left = container(
        column![
            text_input("Search layer states…", filter)
                .on_input(Message::LayerStateManagerFilter)
                .size(11)
                .padding([5, 8]),
            container(state_list)
                .width(Fill)
                .height(Fill)
                .padding(3)
                .style(|theme: &Theme| container::Style {
                    border: Border {
                        color: theme.extended_palette().background.neutral.color,
                        width: 1.0,
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                }),
        ]
        .spacing(8)
        .height(Fill),
    )
    .width(280)
    .height(Fill)
    .padding(iced::Padding {
        top: 12.0,
        right: 8.0,
        bottom: 12.0,
        left: 12.0,
    });

    let details = if let Some(state) = selected_state {
        column![
            text("Saved state details").size(13),
            row![
                text("Layers").size(10).style(muted).width(92),
                text(state.layers.len().to_string()).size(11),
            ]
            .spacing(8),
            row![
                text("Current layer").size(10).style(muted).width(92),
                text(if state.current_layer.is_empty() {
                    "—".to_string()
                } else {
                    state.current_layer.clone()
                })
                .size(11),
            ]
            .spacing(8),
            text("Restored properties").size(10).style(muted),
            text(mask_summary(state.mask)).size(11),
        ]
        .spacing(7)
    } else {
        column![
            text("New layer state").size(13),
            text("Save captures the current settings of every layer in the drawing.")
                .size(11)
                .style(muted),
        ]
        .spacing(7)
    };

    let restore = if selected_state.is_some() {
        button(text("Restore").size(11))
            .on_press(Message::LayerStateManagerRestore)
            .style(button_style(true))
    } else {
        button(text("Restore").size(11)).style(button_style(true))
    };
    let delete = if selected_state.is_some() {
        button(text("Delete").size(11))
            .on_press(Message::LayerStateManagerDelete)
            .style(button::danger)
    } else {
        button(text("Delete").size(11)).style(button::danger)
    };

    let right = container(
        column![
            row![
                button(text("New").size(11))
                    .on_press(Message::LayerStateManagerNew)
                    .style(button_style(false))
                    .padding([5, 12]),
                Space::new().width(Fill),
                restore.padding([5, 12]),
                delete.padding([5, 12]),
            ]
            .spacing(6)
            .align_y(iced::Center),
            divider(),
            text("Name").size(10).style(muted),
            text_input("Layer state name", name)
                .on_input(Message::LayerStateManagerName)
                .on_submit(Message::LayerStateManagerSave)
                .size(11)
                .padding([5, 8]),
            text("Description").size(10).style(muted),
            text_input("Optional description", description)
                .on_input(Message::LayerStateManagerDescription)
                .size(11)
                .padding([5, 8]),
            Space::new().height(8),
            details,
            Space::new().height(Fill),
            text("Layer states are stored inside the drawing and remain available after reopening it.")
                .size(10)
                .style(muted),
            button(text(if selected_state.is_some() {
                "Save / Update"
            } else {
                "Save New State"
            })
            .size(11))
            .on_press(Message::LayerStateManagerSave)
            .style(button_style(true))
            .padding([6, 14]),
        ]
        .spacing(7)
        .height(Fill),
    )
    .width(Fill)
    .height(Fill)
    .padding([12, 12]);

    container(row![left, right].height(Fill))
        .width(Fill)
        .height(Fill)
        .into()
}
