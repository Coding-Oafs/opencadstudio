use iced::widget::text_input::{self, Status};
use iced::{Background, Border, Color, Element, Length, Theme};

/// A read-only value field: a text input with no `on_input` handler.
///
/// In the pinned iced fork, an input without `on_input` is treated as
/// disabled, so the caret is never drawn and typing does nothing — but mouse
/// click/drag selection and Ctrl+C / Ctrl+A still work. This gives a field the
/// user can select and copy but never edit or place a blinker in.
#[allow(dead_code)] // consumed by the read-only field unification tasks
pub fn field<'a, Message: Clone + 'a>(value: &'a str, size: f32, width: Length) -> Element<'a, Message> {
    iced::widget::text_input("", value)
        .size(size)
        .style(read_only_style)
        .padding([3, 6])
        .width(width)
        .into()
}

/// Muted "disabled input box" look: the same bordered box geometry as the
/// editable fields but with a quieter background and text so it reads as
/// read-only. The selection highlight is kept so copy stays discoverable.
#[allow(dead_code)] // used only via `field`, which the later tasks wire up
fn read_only_style(theme: &Theme, _status: Status) -> text_input::Style {
    let palette = theme.palette();
    text_input::Style {
        background: Background::Color(palette.background.weakest.color),
        border: Border {
            color: palette.background.neutral.color,
            width: 1.0,
            radius: 2.0.into(),
        },
        icon: Color::TRANSPARENT,
        placeholder: palette.background.base.text.scale_alpha(0.48),
        value: palette.background.base.text.scale_alpha(0.72),
        selection: palette.primary.base.color.scale_alpha(0.5),
    }
}
