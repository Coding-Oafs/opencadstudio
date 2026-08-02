//! Multiline Style Manager window — fills the entire OS window.

use crate::app::Message;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Theme};
use crate::t;
use std::borrow::Cow;

fn muted_style(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.68)),
    }
}

pub fn view_window<'a>(
    styles: Vec<String>,
    selected: &'a str,
    selected_style: Option<&'a acadrust::objects::MLineStyle>,
    current_style: String,
    rename_active: Option<&'a str>,
    rename_buf: &'a str,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    // ── Right: Details panel ──────────────────────────────────────────────
    let info_row = |label: Cow<'static, str>, val: String| -> Element<'_, Message> {
        row![
            text(label).size(11).style(muted_style).width(120),
            text(val).size(11),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    };

    let details: Element<'_, Message> = if let Some(s) = selected_style {
        let elem_rows: Vec<Element<'_, Message>> = s
            .elements
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                let color_str = match &e.color {
                    acadrust::types::Color::ByLayer => "ByLayer".into(),
                    acadrust::types::Color::None => "None".into(),
                    acadrust::types::Color::ByBlock => "ByBlock".into(),
                    acadrust::types::Color::Index(i) => format!("ACI {i}"),
                    acadrust::types::Color::Rgb { r, g, b } => format!("#{r:02X}{g:02X}{b:02X}"),
                };
                let lt = if e.linetype.is_empty() {
                    "ByLayer"
                } else {
                    &e.linetype
                };
                row![
                    text(format!("  {idx}:")).size(10).style(muted_style).width(24),
                    text(format!("{:+.3}", e.offset)).size(10).width(70),
                    text(color_str).size(10).width(90),
                    text(lt).size(10),
                ]
                .spacing(4)
                .align_y(iced::Center)
                .into()
            })
            .collect();

        let mut col_items: Vec<Element<'_, Message>> = vec![
            info_row(t!("Name:"), s.name.clone()),
            info_row(t!("Elements:"), s.elements.len().to_string()),
            text(t!("  Off   Color        Ltype"))
                .size(10)
                .style(muted_style)
                .into(),
        ];
        col_items.extend(elem_rows);
        scrollable(column(col_items).spacing(6).padding([12, 12]))
            .height(sizing.height)
            .into()
    } else {
        container(text(t!("Select a style to view details.")).size(11).style(muted_style))
            .padding([12, 12])
            .into()
    };

    let right_panel = container(details)
        .width(sizing.width)
        .height(sizing.height);

    crate::ui::style::style_manager::view(crate::ui::style::style_manager::Scaffold {
        sizing,
        kind: crate::app::StyleKind::MLine,
        styles: &styles,
        selected,
        current: Some(current_style.as_str()),
        rename_active,
        rename_buf,
        on_new: Message::MlStyleDialogNew,
        on_copy: Message::MlStyleDialogCopy,
        on_delete: Message::MlStyleDialogDelete,
        on_select: Message::MlStyleDialogSelect,
        on_set_current: Message::MlStyleDialogSetCurrent,
        on_apply: Message::MlStyleApply,
        editor: right_panel.into(),
    })
}
