use crate::app::Message;
use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{Background, Border, Color, Element, Fill, Theme};

pub fn view_window<'a>(default_save_format: &'a str) -> Element<'a, Message> {
    const BG: Color = Color {
        r: 0.15,
        g: 0.15,
        b: 0.17,
        a: 1.0,
    };
    const BORDER: Color = Color {
        r: 0.32,
        g: 0.32,
        b: 0.36,
        a: 1.0,
    };
    const TEXT: Color = Color {
        r: 0.90,
        g: 0.90,
        b: 0.90,
        a: 1.0,
    };
    const DIM: Color = Color {
        r: 0.60,
        g: 0.60,
        b: 0.64,
        a: 1.0,
    };

    let selected = crate::io::SAVE_FORMAT_OPTIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == default_save_format);

    let close = button(text("Close").size(12).color(TEXT))
        .on_press(Message::CloseModal)
        .padding([5, 16])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered | button::Status::Pressed => Color {
                    r: 0.34,
                    g: 0.34,
                    b: 0.38,
                    a: 1.0,
                },
                _ => Color {
                    r: 0.26,
                    g: 0.26,
                    b: 0.29,
                    a: 1.0,
                },
            })),
            text_color: TEXT,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });

    let body = column![
        text("Open and Save").size(15).color(TEXT),
        Space::new().height(16),
        row![
            text("Default save format:").size(12).color(TEXT).width(150),
            pick_list(crate::io::SAVE_FORMAT_OPTIONS, selected, |format: &str| {
                Message::DefaultSaveFormatChanged(format.to_string())
            })
            .width(Fill),
        ]
        .spacing(12)
        .align_y(iced::Center),
        Space::new().height(10),
        text(
            "Used when a new drawing is saved for the first time. Existing drawings keep their current file type and version."
        )
        .size(11)
        .color(DIM)
        .width(Fill),
        Space::new().height(Fill),
        row![Space::new().width(Fill), close],
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill);

    container(body)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BG)),
            ..Default::default()
        })
        .padding([16, 18])
        .width(Fill)
        .height(Fill)
        .into()
}
