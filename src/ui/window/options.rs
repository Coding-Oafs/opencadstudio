use crate::app::config::UiThemeConfig;
use crate::app::Message;
use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Space,
};
use iced::{Background, Border, Element, Theme};

pub fn view_window<'a>(
    default_save_format: &'a str,
    ui_theme: &'a UiThemeConfig,
    theme_color_inputs: &'a [String; 6],
    language: crate::i18n::Language,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    let selected_format = crate::io::SAVE_FORMAT_OPTIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == default_save_format);

    let theme_options = Theme::ALL
        .iter()
        .map(ToString::to_string)
        .chain(std::iter::once("Custom".to_string()))
        .collect::<Vec<_>>();
    let selected_theme = Some(ui_theme.name.clone());

    let palette = ui_theme.palette.to_iced();
    let colors = [
        (crate::tr!("options-color-background"), palette.background),
        (crate::tr!("options-color-text"), palette.text),
        (crate::tr!("options-color-primary"), palette.primary),
        (crate::tr!("options-color-success"), palette.success),
        (crate::tr!("options-color-warning"), palette.warning),
        (crate::tr!("options-color-danger"), palette.danger),
    ];

    let mut color_controls = column![].spacing(8);
    for (index, (label, color)) in colors.into_iter().enumerate() {
        let swatch = container(Space::new())
            .width(28)
            .height(22)
            .style(move |theme: &Theme| container::Style {
                background: Some(Background::Color(color)),
                border: Border {
                    color: theme.palette().background.strong.color,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
            });
        color_controls = color_controls.push(
            row![
                text(label).size(12).width(110),
                swatch,
                text_input("#RRGGBB", theme_color_inputs[index].as_str())
                    .on_input(move |value| Message::OptionsThemeColorChanged(index, value))
                    .width(130),
            ]
            .spacing(10)
            .align_y(iced::Center),
        );
    }

    let close = button(text(crate::tr!("action-close")).size(12))
        .on_press(Message::CloseModal)
        .padding([6, 18])
        .style(button::secondary);

    let content = column![
        text(crate::tr!("options-language-section")).size(15),
        Space::new().height(10),
        row![
            text(crate::tr!("options-language-label")).size(12).width(150),
            iced::widget::pick_list(
                Some(language),
                crate::i18n::Language::ALL,
                |value| value.to_string(),
            )
            .on_select(Message::LanguageChanged)
            .width(sizing.width),
        ]
        .spacing(12)
        .align_y(iced::Center),
        Space::new().height(22),
        text(crate::tr!("options-open-save-section")).size(15),
        Space::new().height(10),
        row![
            text(crate::tr!("options-default-save-format-label")).size(12).width(150),
            iced::widget::pick_list(
                selected_format,
                crate::io::SAVE_FORMAT_OPTIONS,
                |value| value.to_string(),
            )
            .on_select(|format: &str| Message::DefaultSaveFormatChanged(format.to_string()))
            .width(sizing.width),
        ]
        .spacing(12)
        .align_y(iced::Center),
        Space::new().height(8),
        text(crate::tr!("options-default-save-format-help"))
        .size(11)
        .width(sizing.width),
        Space::new().height(22),
        text(crate::tr!("options-theme-section")).size(15),
        Space::new().height(10),
        row![
            text(crate::tr!("options-theme-label")).size(12).width(150),
            iced::widget::pick_list(
                selected_theme,
                theme_options,
                |value| value.to_string(),
            )
            .on_select(Message::OptionsThemeChanged)
            .width(sizing.width),
        ]
        .spacing(12)
        .align_y(iced::Center),
        Space::new().height(8),
        text(crate::tr!("options-theme-help"))
        .size(11)
        .width(sizing.width),
        Space::new().height(12),
        color_controls,
    ]
    .spacing(0)
    .width(sizing.width);

    let body = column![
        scrollable(content).height(sizing.height),
        Space::new().height(12),
        row![Space::new().width(sizing.width), close],
    ]
    .width(sizing.width)
    .height(sizing.height);

    container(body)
        .style(container::rounded_box)
        .padding([16, 18])
        .width(sizing.width)
        .height(sizing.height)
        .into()
}
