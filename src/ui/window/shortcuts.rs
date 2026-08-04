//! Keyboard Shortcuts Reference window — fills the entire OS window.

use crate::app::Message;
use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{Background, Element, Theme};
use crate::t;
use std::borrow::Cow;

/// Display name of the primary accelerator modifier on this platform.
/// Mirrors the runtime binding in `app::view`, which uses
/// `Modifiers::command()` — the Cmd key on macOS, Ctrl elsewhere — so the
/// reference window shows the key the user actually presses.
#[cfg(target_os = "macos")]
const MOD: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl";

fn muted_style(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.68)),
    }
}

fn primary_style(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().primary.base.color),
    }
}

fn hdivider<'a>(width: iced::Length) -> Element<'a, Message> {
    container(Space::new().width(width).height(1))
        .width(width)
        .height(1)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.palette().background.neutral.color,
            )),
            ..Default::default()
        })
        .into()
}

fn shortcut_row<'a>(
    key: impl Into<Cow<'static, str>>,
    action: Cow<'static, str>,
) -> Element<'a, Message> {
    row![
        text(key.into())
            .size(11)
            .style(primary_style)
            .font(iced::Font::MONOSPACE)
            .width(160),
        text(action).size(11),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .padding([2, 0])
    .into()
}

fn section<'a>(title: impl Into<Cow<'static, str>>) -> Element<'a, Message> {
    container(text(title.into()).size(11).style(muted_style))
        .padding(iced::Padding {
            top: 6.0,
            right: 0.0,
            bottom: 2.0,
            left: 0.0,
        })
        .into()
}

pub fn view_window<'a>(
    overrides: &'a rustc_hash::FxHashMap<String, String>,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    // ── Toolbar ───────────────────────────────────────────────────────────
    let toolbar = container(
        row![
            text(t!("Type  SHORTCUTS SET <key> <cmd>  to add custom shortcuts."))
                .size(10)
                .style(muted_style),
        ]
        .align_y(iced::Center),
    )
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(
            theme.palette().background.weakest.color,
        )),
        ..Default::default()
    })
    .width(sizing.width)
    .padding([5, 10]);

    // ── Shortcut entries ──────────────────────────────────────────────────
    let mut rows: Vec<Element<'_, Message>> = vec![
        section(t!("── Function Keys ──────────────────────────────────────")),
        shortcut_row("F1", t!("Help")),
        shortcut_row("F2", t!("Toggle Command History")),
        shortcut_row("F3", t!("Toggle Object Snap")),
        shortcut_row("F7", t!("Toggle Grid")),
        shortcut_row("F8", t!("Toggle Ortho")),
        shortcut_row("F9", t!("Toggle Grid Snap")),
        shortcut_row("F10", t!("Toggle Polar Tracking")),
        shortcut_row("F11", t!("Toggle Object Snap Tracking")),
        shortcut_row("F12", t!("Toggle Dynamic Input")),
        section(t!(
            "── %{MOD} Shortcuts ──────────────────────────────────────",
            MOD = MOD
        )),
        shortcut_row(format!("{MOD}+0"), t!("Clean Screen")),
        shortcut_row(format!("{MOD}+1"), t!("Properties")),
        shortcut_row(format!("{MOD}+N"), t!("New Drawing")),
        shortcut_row(format!("{MOD}+O"), t!("Open File")),
        shortcut_row(format!("{MOD}+P"), t!("Plot")),
        shortcut_row(format!("{MOD}+Q"), t!("Close")),
        shortcut_row(format!("{MOD}+S"), t!("Save")),
        shortcut_row(format!("{MOD}+Shift+S"), t!("Save As")),
        shortcut_row(format!("{MOD}+Z"), t!("Undo")),
        shortcut_row(format!("{MOD}+Shift+Z / {MOD}+Y"), t!("Redo")),
        shortcut_row(format!("{MOD}+F / {MOD}+H"), t!("Find and Replace")),
        shortcut_row(format!("{MOD}+C"), t!("Copy to Clipboard")),
        shortcut_row(format!("{MOD}+Shift+C"), t!("Copy with Base Point")),
        shortcut_row(format!("{MOD}+X"), t!("Cut to Clipboard")),
        shortcut_row(format!("{MOD}+V"), t!("Paste from Clipboard")),
        shortcut_row(format!("{MOD}+Shift+V"), t!("Paste as Block")),
        section(t!("── Other Keys ──────────────────────────────────────────")),
        shortcut_row("Enter / Space", t!("Finalize command / Repeat last")),
        shortcut_row("Escape", t!("Cancel active command")),
        shortcut_row("Delete", t!("Delete selected entities")),
        shortcut_row("↑ / ↓", t!("Command history navigation")),
    ];

    // Custom overrides section
    rows.push(section(
        t!("── Custom Overrides (SHORTCUTS SET) ──────────────────"),
    ));
    if overrides.is_empty() {
        rows.push(
            text(t!("  (none — use: SHORTCUTS SET <key> <command>)"))
                .size(11)
                .style(muted_style)
                .into(),
        );
    } else {
        let mut sorted: Vec<_> = overrides.iter().collect();
        sorted.sort_by_key(|(k, _)| k.as_str());
        for (key, cmd) in sorted {
            rows.push(
                row![
                    text(key.as_str())
                        .size(11)
                        .style(primary_style)
                        .font(iced::Font::MONOSPACE)
                        .width(160),
                    text(cmd.as_str()).size(11),
                ]
                .spacing(8)
                .align_y(iced::Center)
                .padding([2, 0])
                .into(),
            );
        }
    }

    // ── Section headers styled separately ────────────────────────────────
    let content = scrollable(column(rows).spacing(3).padding([12, 16]))
        .width(sizing.width)
        .height(sizing.height);

    // ── Header row with accent ────────────────────────────────────────────
    let header = container(
        row![
            text(t!("Key")).size(10).style(primary_style).width(160),
            text(t!("Action")).size(10).style(primary_style),
        ]
        .spacing(8)
        .padding([4, 16]),
    )
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(
            theme.palette().primary.weak.color,
        )),
        ..Default::default()
    })
    .width(sizing.width);

    container(
        column![
            toolbar,
            hdivider(sizing.width),
            header,
            hdivider(sizing.width),
            content
        ]
        .spacing(0),
    )
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.palette().background.base.color,
            )),
            ..Default::default()
        })
        .width(sizing.width)
        .height(sizing.height)
        .into()
}
