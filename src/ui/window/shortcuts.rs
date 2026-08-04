//! Editable keyboard shortcut table opened by CUI / SHORTCUTS.

use crate::app::Message;
use crate::t;
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Background, Element, Length, Theme};

#[derive(Clone, Copy, Debug)]
pub enum ShortcutField {
    Key,
    Command,
}

const GUTTER: f32 = 16.0;

fn muted_style(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.68)),
    }
}

pub fn view_window(
    rows: &[(String, String)],
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'_, Message> {
    let title = text(t!("Keyboard Shortcuts")).size(15);
    let hint = text(t!(
        "Type  SHORTCUTS SET <key> <cmd>  to add custom shortcuts."
    ))
    .size(11)
    .style(muted_style);
    let gutter = iced::Padding {
        top: 0.0,
        right: GUTTER,
        bottom: 0.0,
        left: 0.0,
    };

    let head = container(
        row![
            container(text(t!("Key")).size(11).style(muted_style))
                .width(Length::Fixed(180.0)),
            container(text(t!("Command")).size(11).style(muted_style)).width(sizing.width),
            Space::new().width(Length::Fixed(30.0)),
        ]
        .spacing(8),
    )
    .padding(gutter);

    let mut list = column![].spacing(3);
    for (idx, (key, command)) in rows.iter().enumerate() {
        let key_box = text_input("Ctrl+Key", key)
            .on_input(move |value| Message::ShortcutEditorInput {
                idx,
                field: ShortcutField::Key,
                value,
            })
            .size(13)
            .padding([3, 6])
            .width(Length::Fixed(180.0));
        let command_box = text_input(t!("command").as_ref(), command)
            .on_input(move |value| Message::ShortcutEditorInput {
                idx,
                field: ShortcutField::Command,
                value,
            })
            .size(13)
            .padding([3, 6])
            .width(sizing.width);
        let remove = button(crate::ui::icons::themed_danger_text(
            crate::ui::icons::CLOSE,
            12.0,
        ))
        .on_press(Message::ShortcutEditorRemove(idx))
        .padding([2, 6])
        .style(button::danger);
        list = list.push(
            row![key_box, command_box, remove]
                .spacing(8)
                .align_y(iced::Center),
        );
    }

    let add = button(text(format!("+ {}", t!("Add"))).size(12))
        .on_press(Message::ShortcutEditorAdd)
        .padding([4, 10])
        .style(button::secondary);
    let apply = button(text(t!("Apply")).size(12))
        .on_press(Message::ShortcutEditorApply)
        .padding([4, 16])
        .style(button::primary);

    container(
        column![
            title,
            hint,
            Space::new().height(6),
            head,
            scrollable(container(list).padding(gutter)).height(sizing.height),
            Space::new().height(6),
            row![add, Space::new().width(sizing.width), apply].align_y(iced::Center),
        ]
        .spacing(6)
        .width(sizing.width)
        .height(sizing.height),
    )
    .padding(12)
    .width(sizing.width)
    .height(sizing.height)
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(
            theme.palette().background.base.color,
        )),
        ..Default::default()
    })
    .into()
}
