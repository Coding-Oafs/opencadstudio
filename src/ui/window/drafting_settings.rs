use crate::app::settings::IsoPlane;
use crate::app::Message;
use crate::snap::{Snapper, ALL_SNAP_MODES};
use iced::widget::{button, checkbox, column, container, row, scrollable, text, Space};
use iced::{Element, Fill};

pub fn view_window<'a>(
    snapper: &'a Snapper,
    grid: bool,
    grid_snap: bool,
    ortho: bool,
    polar: bool,
    otrack: bool,
    isometric: bool,
    iso_plane: IsoPlane,
    snap_angle_deg: f32,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    let toggle = |value: bool, label: std::borrow::Cow<'a, str>, message: Message| {
        row![
            checkbox(value).on_toggle(move |_| message.clone()).size(15),
            text(label).size(12),
        ]
        .spacing(7)
        .align_y(iced::Center)
    };

    let drafting_modes = column![
        text(crate::t!("Drafting modes")).size(15),
        Space::new().height(8),
        toggle(grid, crate::t!("Grid display"), Message::ToggleGrid),
        toggle(grid_snap, crate::t!("Grid snap"), Message::ToggleGridSnap),
        toggle(ortho, crate::t!("Ortho"), Message::ToggleOrtho),
        toggle(polar, crate::t!("Polar tracking"), Message::TogglePolar),
        toggle(otrack, crate::t!("Object snap tracking"), Message::ToggleOTrack),
    ]
    .spacing(6);

    let mut planes = row![].spacing(5);
    for plane in IsoPlane::ALL {
        planes = planes.push(
            button(text(crate::t!(plane.label())).size(11))
                .on_press(Message::SetIsoPlane(plane))
                .style(if isometric && iso_plane == plane {
                    button::primary
                } else {
                    button::secondary
                })
                .padding([5, 12]),
        );
    }
    let isometric_controls = column![
        text(crate::t!("Isometric drafting")).size(15),
        Space::new().height(8),
        toggle(
            isometric,
            crate::t!("Enable isometric drafting"),
            Message::ToggleIsometricDrafting,
        ),
        planes,
        text(crate::t!("F5 cycles Left, Top, and Right.")).size(11),
        Space::new().height(6),
        row![
            text(crate::t!("Rotation: %{angle}°", angle = snap_angle_deg)).size(11),
            button(text(crate::t!("Reset rotation")).size(10))
                .on_press(Message::ResetDraftingRotation)
                .style(button::secondary)
                .padding([4, 10]),
        ]
        .spacing(10)
        .align_y(iced::Center),
    ]
    .spacing(7);

    let mut snap_modes = column![
        text(crate::t!("Object snap modes")).size(15),
        Space::new().height(8),
        toggle(
            snapper.snap_enabled,
            crate::t!("Enable object snap"),
            Message::ToggleSnapEnabled,
        ),
        row![
            button(text(crate::t!("Select All")).size(10))
                .on_press(Message::SnapSelectAll)
                .style(button::secondary)
                .padding([4, 10]),
            button(text(crate::t!("Clear All")).size(10))
                .on_press(Message::SnapClearAll)
                .style(button::secondary)
                .padding([4, 10]),
        ]
        .spacing(6),
    ]
    .spacing(5);
    for &(snap_type, _, label) in ALL_SNAP_MODES {
        snap_modes = snap_modes.push(
            row![
                checkbox(snapper.is_on(snap_type))
                    .on_toggle(move |_| Message::ToggleSnap(snap_type))
                    .size(14),
                text(crate::t!(label)).size(11),
            ]
            .spacing(7)
            .align_y(iced::Center),
        );
    }

    let close = button(text(crate::tr!("action", "close")).size(12))
        .on_press(Message::CloseModal)
        .padding([6, 18])
        .style(button::secondary);
    let content = column![
        drafting_modes,
        Space::new().height(20),
        isometric_controls,
        Space::new().height(20),
        snap_modes,
    ]
    .width(sizing.width);
    let body = column![
        scrollable(content).spacing(8).height(sizing.height),
        Space::new().height(12),
        row![Space::new().width(Fill), close],
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
