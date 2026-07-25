//! Modal overlay shown while a CAD file is being loaded.
//!
//! Displays the file name, size, current phase, measured progress, and a
//! Cancel button.

use iced::time::Instant;
use iced::widget::{button, column, container, row, stack, text, Space};
use iced::{Background, Border, Color, Element, Fill, Length, Theme};
use std::sync::atomic::Ordering;

use crate::app::{
    Message, OpenProgress, OPEN_PHASE_CACHING, OPEN_PHASE_FINALIZING, OPEN_PHASE_PARSING,
    OPEN_PHASE_READING, OPEN_PHASE_XREF,
};

const CARD_WIDTH: f32 = 420.0;
const BAR_TRACK_WIDTH: f32 = 380.0;
const BAR_TRACK_HEIGHT: f32 = 6.0;

fn phase_label(phase: u8) -> &'static str {
    match phase {
        OPEN_PHASE_READING => "Reading file…",
        OPEN_PHASE_PARSING => "Parsing entities…",
        OPEN_PHASE_XREF => "Loading references…",
        OPEN_PHASE_CACHING => "Building scene caches…",
        OPEN_PHASE_FINALIZING => "Finalizing…",
        _ => "Working…",
    }
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn view<'a>(progress: &'a OpenProgress, _now: Instant) -> Element<'a, Message> {
    let phase = progress.state.phase.load(Ordering::Acquire);
    let basis_points = progress
        .state
        .basis_points
        .load(Ordering::Relaxed)
        .min(10000);
    let fraction = basis_points as f32 / 10000.0;
    let fill_width = BAR_TRACK_WIDTH * fraction;
    let trailing = (BAR_TRACK_WIDTH - fill_width).max(0.0);

    let bar_fill: Element<'_, Message> = container(
        Space::new()
            .width(Length::Fixed(fill_width))
            .height(Length::Fixed(BAR_TRACK_HEIGHT)),
    )
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(Color {
                r: 0.30,
                g: 0.62,
                b: 0.95,
                a: 1.0,
            })),
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

    let bar_value: Element<'_, Message> = row![
        bar_fill,
        Space::new()
            .width(Length::Fixed(trailing))
            .height(Length::Fixed(BAR_TRACK_HEIGHT)),
    ]
    .into();

    let bar_track: Element<'_, Message> = container(
        stack![
            container(Space::new().width(Length::Fixed(BAR_TRACK_WIDTH)).height(Length::Fixed(BAR_TRACK_HEIGHT)))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(Color {
                        r: 0.18,
                        g: 0.18,
                        b: 0.18,
                        a: 1.0,
                    })),
                    border: Border {
                        radius: 3.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            bar_value,
        ]
        .width(Length::Fixed(BAR_TRACK_WIDTH))
        .height(Length::Fixed(BAR_TRACK_HEIGHT)),
    )
    .into();

    // ── Card body ────────────────────────────────────────────────────────
    let title = text("Opening file")
        .size(15)
        .color(Color::WHITE);

    let name_line = text(format!(
        "{}  ({})",
        progress.name,
        format_size(progress.size_bytes)
    ))
    .size(13)
    .color(Color {
        r: 0.82,
        g: 0.82,
        b: 0.82,
        a: 1.0,
    });

    let phase_line = text(format!(
        "{}  {:.1}%",
        phase_label(phase),
        basis_points as f32 / 100.0
    ))
        .size(12)
        .color(Color {
            r: 0.70,
            g: 0.80,
            b: 0.95,
            a: 1.0,
        });

    let cancel_btn: Element<'_, Message> = button(text("Cancel").size(12).color(Color::WHITE))
        .on_press(Message::OpenCancel)
        .style(|_: &Theme, status| {
            let bg = match status {
                button::Status::Hovered => Color {
                    r: 0.32,
                    g: 0.32,
                    b: 0.32,
                    a: 1.0,
                },
                button::Status::Pressed => Color {
                    r: 0.42,
                    g: 0.18,
                    b: 0.18,
                    a: 1.0,
                },
                _ => Color {
                    r: 0.22,
                    g: 0.22,
                    b: 0.22,
                    a: 1.0,
                },
            };
            button::Style {
                background: Some(Background::Color(bg)),
                border: Border {
                    color: Color {
                        r: 0.40,
                        g: 0.40,
                        b: 0.40,
                        a: 1.0,
                    },
                    width: 1.0,
                    radius: 3.0.into(),
                },
                text_color: Color::WHITE,
                ..Default::default()
            }
        })
        .padding([4, 14])
        .into();

    let cancel_row: Element<'_, Message> = container(cancel_btn).align_right(Fill).into();

    let card = container(
        column![title, name_line, bar_track, phase_line, cancel_row]
            .spacing(10)
            .width(Length::Fixed(CARD_WIDTH)),
    )
    .padding([18, 22])
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(Color {
            r: 0.13,
            g: 0.13,
            b: 0.13,
            a: 0.98,
        })),
        border: Border {
            color: Color {
                r: 0.45,
                g: 0.45,
                b: 0.45,
                a: 1.0,
            },
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    });

    // ── Backdrop (click-blocker + dim) ────────────────────────────────────
    let backdrop: Element<'_, Message> = container(Space::new().width(Fill).height(Fill))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.55,
            })),
            ..Default::default()
        })
        .width(Fill)
        .height(Fill)
        .into();

    let centered: Element<'_, Message> = container(card)
        .center_x(Fill)
        .center_y(Fill)
        .width(Fill)
        .height(Fill)
        .into();

    stack![backdrop, centered].width(Fill).height(Fill).into()
}
