//! Plot / Print dialog — a full plot setup surface rendered as an in-canvas
//! modal (Plan B). Bundles printer choice, paper, scale, offset, plot style,
//! quality and output options into one dialog; on commit it either sends the
//! current layout to a system printer (with the chosen options) or writes a
//! PDF. Styled to match the other OCS dialogs (dark pills + fields).

use crate::app::Message;
use crate::io::paper_sizes::PaperSize;
use iced::widget::{
    button, checkbox, column, container, row, scrollable, text, text_input, Space,
};
use iced::{Background, Border, Element, Length, Theme};

/// Sentinel entries in the printer dropdown (not real printer names).
pub const OUT_DEFAULT: &str = "System default printer";
pub const OUT_PDF: &str = "Save to PDF file…";

/// Top-of-list entries: no page setup (defaults + PDF), and the last-used
/// settings captured when the dialog opened.
pub const SETUP_NONE: &str = "<none>";
pub const SETUP_PREV: &str = "<previous>";

/// One of the many boolean plot options (folded into a single message so the
/// dialog needn't carry a variant per checkbox).
#[derive(Debug, Clone, Copy)]
pub enum PlotFlag {
    Center,
    ScaleLw,
    UpsideDown,
    Mono,
    Lineweights,
    WithStyles,
    Transparency,
    PaperspaceLast,
    Stamp,
    SaveLayout,
}

/// Every edit the Plot dialog can emit. Wrapped in `Message::PlotDlg` so the
/// top-level match stays a single arm.
#[derive(Debug, Clone)]
pub enum PlotDlgMsg {
    Close,
    Commit,
    Preview,
    PrinterProperties,
    Printer(String),
    Paper(String),
    Orientation(String),
    Area(String),
    Scale(String),
    Quality(String),
    Shade(String),
    Copies(String),
    OffsetX(String),
    OffsetY(String),
    Dpi(String),
    Flag(PlotFlag),
    LoadStyle,
    ClearStyle,
    PickWindow,
    // ── Named page-setup manager ─────────────────────────────────────────
    /// Pick a named page setup (loads its values into the editor).
    SelectSetup(String),
    /// Write the current editor values into the active layout.
    SetCurrent,
    /// Create a new named page setup from the current editor values.
    NewSetup,
    /// Duplicate the selected page setup.
    CopySetup,
    /// Begin an inline rename of the given page setup row.
    RenameStart(String),
    /// Delete the selected named page setup.
    DeleteSetup,
    /// Live edit of the new/rename name field.
    NameInput(String),
    /// Confirm the new/rename name.
    NameCommit,
    /// Cancel the new/rename name row.
    NameCancel,
}

/// Transient state backing the Plot dialog. Seeded from the layout's plot
/// settings when the dialog opens; consumed on commit.
// The persisted fields form the "plot" section of the app config
// ([`crate::app::config`]); `#[serde(skip)]` marks the runtime-only fields
// (discovered printers, live page/offset choices, name-entry state) so only the
// user's print preferences are written, matching the former plot.txt subset.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlotDialogState {
    /// Printer names discovered on the system (via `lpstat`), never the
    /// sentinels.
    #[serde(skip)]
    pub printers: Vec<String>,
    /// Chosen printer name, or `None` for the system default.
    pub printer: Option<String>,
    /// Output goes to a PDF file instead of a printer.
    pub to_file: bool,
    #[serde(skip)]
    pub paper: String,
    #[serde(skip)]
    pub orientation: String,
    pub upside_down: bool,
    pub copies: String,
    pub area: String,
    #[serde(skip)]
    pub center: bool,
    #[serde(skip)]
    pub offset_x: String,
    #[serde(skip)]
    pub offset_y: String,
    pub scale: String,
    pub scale_lw: bool,
    pub quality: String,
    pub dpi: String,
    pub shade: String,
    pub mono: bool,
    pub lineweights: bool,
    pub with_styles: bool,
    pub transparency: bool,
    pub paperspace_last: bool,
    pub stamp: bool,
    pub save_layout: bool,
    /// Display name of the active plot style table ("" = none).
    pub style_name: String,
    /// The selected setup references a style table that is not loaded.
    #[serde(skip)]
    pub style_missing: bool,
    /// Named page setups in the document (refreshed when the dialog opens).
    #[serde(skip)]
    pub page_setups: Vec<String>,
    /// Currently selected named page setup ("" = none / current layout).
    #[serde(skip)]
    pub selected_setup: String,
    /// When `Some`, a name-entry row is showing (for New / Rename).
    #[serde(skip)]
    pub name_input: Option<String>,
    /// `true` when `name_input` is renaming the selected setup, else creating.
    #[serde(skip)]
    pub name_rename: bool,
    /// Whether the dialog was opened from a paper-space layout.
    #[serde(skip)]
    pub paper_space: bool,
}

impl Default for PlotDialogState {
    fn default() -> Self {
        Self {
            printers: Vec::new(),
            printer: None,
            to_file: false,
            paper: "A4".into(),
            orientation: "Landscape".into(),
            upside_down: false,
            copies: "1".into(),
            area: "Window".into(),
            center: true,
            offset_x: "0.0".into(),
            offset_y: "0.0".into(),
            scale: "Fit".into(),
            scale_lw: true,
            quality: "Normal".into(),
            dpi: "300".into(),
            shade: "As displayed".into(),
            mono: false,
            lineweights: true,
            with_styles: true,
            transparency: false,
            paperspace_last: false,
            stamp: false,
            save_layout: false,
            style_name: String::new(),
            style_missing: false,
            page_setups: Vec::new(),
            selected_setup: String::new(),
            name_input: None,
            name_rename: false,
            paper_space: false,
        }
    }
}

impl PlotDialogState {
    /// Copy the plot-setting fields (paper, scale, output options, …) from
    /// `o`, leaving list / rename / runtime UI state untouched. Used to restore
    /// the `<previous>` snapshot.
    pub fn copy_settings_from(&mut self, o: &PlotDialogState) {
        self.printer = o.printer.clone();
        self.to_file = o.to_file;
        self.paper = o.paper.clone();
        self.orientation = o.orientation.clone();
        self.upside_down = o.upside_down;
        self.copies = o.copies.clone();
        self.area = o.area.clone();
        self.center = o.center;
        self.offset_x = o.offset_x.clone();
        self.offset_y = o.offset_y.clone();
        self.scale = o.scale.clone();
        self.scale_lw = o.scale_lw;
        self.quality = o.quality.clone();
        self.dpi = o.dpi.clone();
        self.shade = o.shade.clone();
        self.mono = o.mono;
        self.lineweights = o.lineweights;
        self.with_styles = o.with_styles;
        self.transparency = o.transparency;
        self.paperspace_last = o.paperspace_last;
        self.stamp = o.stamp;
        self.save_layout = o.save_layout;
        self.style_name = o.style_name.clone();
        self.style_missing = o.style_missing;
    }

}

fn btn(accent: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, st| {
        let palette = theme.palette();
        let pair = match (accent, st) {
            (true, button::Status::Hovered | button::Status::Pressed) => palette.primary.strong,
            (false, button::Status::Hovered | button::Status::Pressed) => {
                palette.background.strong
            }
            (true, _) => palette.primary.base,
            _ => palette.background.weak,
        };
        button::Style {
        background: Some(Background::Color(pair.color)),
        text_color: pair.text,
        border: Border {
            color: palette.background.neutral.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow::default(),
        snap: false,
        }
    }
}

fn field_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.palette();
    let border = match status {
        text_input::Status::Focused { .. } => palette.primary.base.color,
        _ => palette.background.neutral.color,
    };
    text_input::Style {
        background: Background::Color(palette.background.base.color),
        border: Border { color: border, width: 1.0, radius: 3.0.into() },
        icon: palette.background.base.text,
        placeholder: palette.background.base.text.scale_alpha(0.48),
        value: palette.background.base.text,
        selection: palette.primary.base.color.scale_alpha(0.5),
    }
}

fn muted_style(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.68)),
    }
}

fn hdivider<'a>(width: Length) -> Element<'a, Message> {
    container(Space::new().width(width).height(1))
        .width(width)
        .height(1)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.palette().background.neutral.color
            )),
            ..Default::default()
        })
        .into()
}

fn section_label<'a>(s: &'static str) -> Element<'a, Message> {
    text(s).size(11).style(muted_style).into()
}

/// A `label : dropdown` row. `ctor` turns the picked string into a dialog
/// message.
fn drop_row<'a>(
    label: &'a str,
    options: Vec<String>,
    selected: Option<String>,
    ctor: fn(String) -> PlotDlgMsg,
    width: Length,
) -> Element<'a, Message> {
    let pl = iced::widget::pick_list(selected, options, |value| value.to_string())
        .on_select(move |s| Message::PlotDlg(ctor(s)))
        .text_size(12)
        .padding([3, 6])
        .width(width);
    row![text(label).size(11).style(muted_style).width(92), pl]
        .spacing(8)
        .align_y(iced::Center)
    .into()
}

fn drop_row_enabled<'a>(
    label: &'a str,
    options: Vec<String>,
    selected: Option<String>,
    ctor: fn(String) -> PlotDlgMsg,
    width: Length,
    enabled: bool,
) -> Element<'a, Message> {
    if enabled {
        return drop_row(label, options, selected, ctor, width);
    }
    row![
        text(label).size(11).style(muted_style).width(92),
        container(text(selected.unwrap_or_default()).size(12).style(muted_style))
            .padding([4, 7])
            .width(width),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .into()
}

/// A `label : text field` row.
fn field_row<'a>(
    label: &'a str,
    value: &'a str,
    ctor: fn(String) -> PlotDlgMsg,
    width: u16,
) -> Element<'a, Message> {
    row![
        text(label).size(11).style(muted_style).width(92),
        text_input("", value)
            .on_input(move |s| Message::PlotDlg(ctor(s)))
            .style(field_style)
            .size(12)
            .width(width as f32),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .into()
}

fn field_row_enabled<'a>(
    label: &'a str,
    value: &'a str,
    ctor: fn(String) -> PlotDlgMsg,
    width: u16,
    enabled: bool,
) -> Element<'a, Message> {
    if enabled {
        return field_row(label, value, ctor, width);
    }
    row![
        text(label).size(11).style(muted_style).width(92),
        container(text(value.to_string()).size(12).style(muted_style))
            .padding([4, 7])
            .width(width as f32),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .into()
}

/// A single option checkbox bound to a `PlotFlag`.
fn check<'a>(label: &'a str, on: bool, flag: PlotFlag) -> Element<'a, Message> {
    checkbox(on)
        .label(label)
        .on_toggle(move |_| Message::PlotDlg(PlotDlgMsg::Flag(flag)))
        .size(14)
        .text_size(11)
        .into()
}

fn check_enabled<'a>(
    label: &'a str,
    on: bool,
    flag: PlotFlag,
    enabled: bool,
) -> Element<'a, Message> {
    if enabled {
        return check(label, on, flag);
    }
    checkbox(on)
        .label(label)
        .size(14)
        .text_size(11)
        .style(checkbox::primary)
        .into()
}

fn check_static<'a>(label: &'a str, on: bool) -> Element<'a, Message> {
    checkbox(on)
        .label(label)
        .size(14)
        .text_size(11)
        .style(checkbox::primary)
        .into()
}

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(10)
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
                background: Some(Background::Color(palette.background.weak.color)),
                border: Border {
                    color: palette.background.neutral.color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn view_window(
    s: &PlotDialogState,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'_, Message> {
    let width = sizing.width;
    let height = sizing.height;
    let action = if s.to_file { "Export PDF" } else { "Print" };
    let is_special = s.selected_setup == SETUP_NONE || s.selected_setup == SETUP_PREV;
    let sel_is_layout = s.selected_setup.len() >= 2
        && s.selected_setup.starts_with('*')
        && s.selected_setup.ends_with('*');
    let can_copy = !s.selected_setup.is_empty() && !is_special;
    let is_named = can_copy && !sel_is_layout;

    // ── Page setup ────────────────────────────────────────────────────────
    let setup_selected = (!s.selected_setup.is_empty()).then(|| s.selected_setup.clone());
    let mut setup_actions = row![
        text("Name").size(11).style(muted_style).width(56),
        iced::widget::pick_list(setup_selected, s.page_setups.clone(), |value| value.to_string())
            .on_select(|value| Message::PlotDlg(PlotDlgMsg::SelectSetup(value)))
            .text_size(12)
            .padding([3, 6])
            .width(Length::Fill),
        button(text("Add…").size(11))
            .on_press(Message::PlotDlg(PlotDlgMsg::NewSetup))
            .style(btn(false))
            .padding([4, 10]),
    ]
    .spacing(6)
    .align_y(iced::Center);
    if can_copy {
        setup_actions = setup_actions.push(
            button(text("Copy").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::CopySetup))
                .style(btn(false))
                .padding([4, 10]),
        );
    }
    if is_named {
        setup_actions = setup_actions
            .push(
                button(text("Rename").size(11))
                    .on_press(Message::PlotDlg(PlotDlgMsg::RenameStart(
                        s.selected_setup.clone(),
                    )))
                    .style(btn(false))
                    .padding([4, 10]),
            )
            .push(
            button(text("Delete").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::DeleteSetup))
                .style(btn(false))
                    .padding([4, 10]),
            );
    }
    let name_editor: Element<'_, Message> = if let Some(value) = s.name_input.as_deref() {
        row![
            text(if s.name_rename { "Setup name" } else { "New setup" })
                .size(11)
                .style(muted_style)
                .width(82),
            text_input("", value)
                .on_input(|value| Message::PlotDlg(PlotDlgMsg::NameInput(value)))
                .on_submit(Message::PlotDlg(PlotDlgMsg::NameCommit))
                .style(field_style)
                .size(12)
                .width(Length::Fill),
            button(text("Save").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::NameCommit))
                .style(btn(true))
                .padding([4, 10]),
            button(text("Cancel").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::NameCancel))
                .style(btn(false))
                .padding([4, 10]),
        ]
        .spacing(6)
        .align_y(iced::Center)
        .into()
    } else {
        Space::new().height(0).into()
    };
    let setup_panel = panel(
        column![
            section_label("Page setup"),
            setup_actions,
            name_editor,
        ]
        .spacing(7),
    );

    // ── Printer / plotter ─────────────────────────────────────────────────
    let mut printer_opts = vec![OUT_DEFAULT.to_string()];
    printer_opts.extend(s.printers.iter().cloned());
    printer_opts.push(OUT_PDF.to_string());
    let printer_sel = if s.to_file {
        Some(OUT_PDF.to_string())
    } else {
        Some(s.printer.clone().unwrap_or_else(|| OUT_DEFAULT.to_string()))
    };
    let paper_opts: Vec<String> = PaperSize::ALL.iter().map(|p| p.label().to_string()).collect();
    let paper_note: Element<'_, Message> = if s.area == "Layout" {
        text("Layout plots the current sheet; Apply to layout updates its paper size.")
            .size(10)
            .style(muted_style)
            .width(width)
            .into()
    } else {
        Space::new().height(0).into()
    };
    let mut output_row = row![
        text("Output").size(11).style(muted_style).width(92),
        iced::widget::pick_list(printer_sel, printer_opts, |value| value.to_string())
            .on_select(|value| Message::PlotDlg(PlotDlgMsg::Printer(value)))
            .text_size(12)
            .padding([3, 6])
            .width(Length::Fill),
    ]
    .spacing(8)
    .align_y(iced::Center);
    if !s.to_file {
        output_row = output_row.push(
            button(text("Properties…").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::PrinterProperties))
                .style(btn(false))
                .padding([4, 8]),
        );
    }
    let destination = if s.to_file {
        "Destination: PDF file"
    } else if s.printer.is_some() {
        "Destination: selected system printer"
    } else {
        "Destination: system default printer"
    };
    let printer_panel = panel(
        column![
            section_label("Printer / plotter"),
            output_row,
            field_row_enabled("Copies", &s.copies, PlotDlgMsg::Copies, 60, !s.to_file),
            text(destination).size(10).style(muted_style),
        ]
        .spacing(7),
    );

    // ── Paper, area, offset, scale ────────────────────────────────────────
    let paper_panel = panel(column![
        section_label("Paper"),
        drop_row("Size", paper_opts, Some(s.paper.clone()), PlotDlgMsg::Paper, width),
        paper_note,
    ].spacing(7));

    let mut area_options = strs(&["Extents", "Display", "Window"]);
    if s.paper_space {
        area_options.insert(0, "Layout".to_string());
    }
    let mut area_row = row![
        text("What to plot").size(11).style(muted_style).width(92),
        iced::widget::pick_list(Some(s.area.clone()), area_options, |value| value.to_string())
            .on_select(|value| Message::PlotDlg(PlotDlgMsg::Area(value)))
            .text_size(12)
            .padding([3, 6])
            .width(Length::Fill),
    ]
    .spacing(8)
    .align_y(iced::Center);
    if s.area == "Window" {
        area_row = area_row.push(
            button(text("Pick…").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::PickWindow))
                .style(btn(false))
                .padding([4, 10]),
        );
    }
    let common_area = s.area != "Layout";
    let area_panel = panel(column![
        section_label("Plot area"),
        area_row,
        section_label("Plot offset"),
        column![
            field_row_enabled("X (mm)", &s.offset_x, PlotDlgMsg::OffsetX, 70, common_area && !s.center),
            field_row_enabled("Y (mm)", &s.offset_y, PlotDlgMsg::OffsetY, 70, common_area && !s.center),
        ]
        .spacing(7),
        check_enabled("Center the plot", s.center, PlotFlag::Center, common_area),
        section_label("Plot scale"),
        drop_row_enabled(
            "Scale",
            strs(&["Fit", "1:1", "1:2", "1:5", "1:10", "1:20", "1:50", "1:100", "2:1"]),
            Some(if common_area { s.scale.clone() } else { "1:1".into() }),
            PlotDlgMsg::Scale,
            width,
            common_area,
        ),
        check_enabled("Scale lineweights", s.scale_lw, PlotFlag::ScaleLw, common_area),
    ].spacing(7));

    // ── Style and shaded viewport settings ───────────────────────────────
    let style_label = if s.style_name.is_empty() {
        "(none)".to_string()
    } else if s.style_missing {
        format!("{} (not loaded)", s.style_name)
    } else {
        s.style_name.clone()
    };
    let style_panel = panel(column![
        section_label("Plot style table (pen assignments)"),
        row![
            container(text(style_label).size(12))
                .style(|theme: &Theme| {
                    let palette = theme.palette();
                    container::Style {
                    background: Some(Background::Color(palette.background.base.color)),
                    border: Border {
                        color: palette.background.neutral.color,
                        width: 1.0,
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                    }
                })
                .padding([4, 8])
                .width(width),
            button(text("Load…").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::LoadStyle))
                .style(btn(false))
                .padding([4, 10]),
            button(text("Clear").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::ClearStyle))
                .style(btn(false))
                .padding([4, 10]),
        ]
        .spacing(6)
        .align_y(iced::Center),
    ].spacing(7));

    let shaded_panel = panel(column![
        section_label("Shaded viewport options"),
        drop_row(
            "Shade plot",
            strs(&["As displayed", "Wireframe"]),
            Some(s.shade.clone()),
            PlotDlgMsg::Shade,
            width,
        ),
        drop_row(
            "Quality",
            strs(&["Draft", "Preview", "Normal", "Presentation", "Maximum", "Custom"]),
            Some(s.quality.clone()),
            PlotDlgMsg::Quality,
            width,
        ),
        field_row_enabled("DPI", &s.dpi, PlotDlgMsg::Dpi, 70, s.quality == "Custom"),
        text("Vector PDF stays resolution-independent; quality controls printer rasterization.")
            .size(10)
            .style(muted_style)
            .width(width),
        text("Hidden-line and rendered raster modes need a raster viewport backend.")
            .size(10)
            .style(muted_style)
            .width(width),
    ].spacing(7));

    // ── Output options and orientation ────────────────────────────────────
    let options_panel = panel(column![
        section_label("Plot options"),
        row![
            column![
                check_static("Plot in background", true),
                check("Object lineweights", s.lineweights, PlotFlag::Lineweights),
                check_enabled(
                    "Plot with styles",
                    s.with_styles,
                    PlotFlag::WithStyles,
                    !s.style_name.is_empty(),
                ),
                check("Monochrome", s.mono, PlotFlag::Mono),
                check("Plot transparency", s.transparency, PlotFlag::Transparency),
            ]
            .spacing(6)
            .width(width),
            column![
                check_enabled(
                    "Paper space last",
                    s.paperspace_last,
                    PlotFlag::PaperspaceLast,
                    s.paper_space,
                ),
                check_static("Hide paper objects (unavailable)", false),
                check("Plot stamp", s.stamp, PlotFlag::Stamp),
                check_enabled(
                    "Save changes to layout",
                    s.save_layout,
                    PlotFlag::SaveLayout,
                    s.paper_space,
                ),
            ]
            .spacing(6)
            .width(width),
        ]
        .spacing(10),
    ].spacing(7));

    let orientation_panel = panel(column![
        section_label("Drawing orientation"),
        drop_row(
            "Orientation",
            strs(&["Portrait", "Landscape"]),
            Some(s.orientation.clone()),
            PlotDlgMsg::Orientation,
            width,
        ),
        check("Plot upside-down", s.upside_down, PlotFlag::UpsideDown),
    ].spacing(7));

    let left = column![printer_panel, paper_panel, area_panel]
        .spacing(10)
        .width(width);
    let right = column![style_panel, shaded_panel, options_panel, orientation_panel]
        .spacing(10)
        .width(width);
    let detail = scrollable(
        container(column![
            setup_panel,
            row![left, right].spacing(12).width(width),
        ].spacing(10))
        .padding(12),
    )
    .width(width)
    .height(height);

    let mut actions = row![
        button(text("Preview").size(11))
            .on_press(Message::PlotDlg(PlotDlgMsg::Preview))
            .style(btn(false))
            .padding([5, 14]),
        Space::new().width(Length::Fill),
    ]
    .spacing(6)
    .align_y(iced::Center);
    if s.paper_space {
        actions = actions.push(
            button(text("Apply to layout").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::SetCurrent))
                .style(btn(false))
                .padding([5, 12]),
        );
    }
    actions = actions
        .push(
            button(text("Cancel").size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::Close))
                .style(btn(false))
                .padding([5, 14]),
        )
        .push(
            button(text(action).size(11))
                .on_press(Message::PlotDlg(PlotDlgMsg::Commit))
                .style(btn(true))
                .padding([5, 18]),
        );
    let footer = container(actions)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.palette().background.weak.color)),
            ..Default::default()
        })
        .padding([7, 10])
        .width(width);

    container(column![detail, hdivider(width), footer].spacing(0))
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.palette().background.base.color
            )),
            ..Default::default()
        })
        .width(width)
        .height(height)
        .into()
}
