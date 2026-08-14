//! Click-first manager for an attached LAS/LAZ point cloud.

use crate::app::Message;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Background, Element, Length, Theme};

#[derive(Clone, Debug, Default)]
pub struct PointCloudManagerData {
    pub attached: bool,
    pub source: String,
    pub source_points: u64,
    pub displayed_points: usize,
    pub sample_label: String,
    pub pending_edits: usize,
    pub transactions: usize,
    pub active_selection: u64,
    pub selection_sets: usize,
    pub class_count: usize,
    pub color_mode: String,
    pub point_size_px: f32,
    pub crs_declared: bool,
    pub indexed: bool,
    pub index_running: bool,
    pub cache: String,
    pub export_progress: Option<(u64, u64)>,
    pub sidecar_available: bool,
    pub selection_filter: String,
}

fn action(label: &'static str, command: &'static str, enabled: bool) -> Element<'static, Message> {
    let button = button(text(label).size(12))
        .padding([5, 10])
        .style(button::secondary);
    if enabled {
        button
            .on_press(Message::Command(command.to_string()))
            .into()
    } else {
        button.into()
    }
}

fn status(label: &'static str, value: String) -> Element<'static, Message> {
    row![
        container(text(label).size(11)).width(Length::Fixed(128.0)),
        text(value).size(12),
    ]
    .spacing(8)
    .into()
}

fn section<'a>(
    title: &'static str,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(column![text(title).size(14), content.into()].spacing(7))
        .padding(10)
        .width(Length::Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.palette().background.weak.color)),
            border: iced::Border {
                color: theme.palette().background.strong.color,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .into()
}

pub fn view_window(
    data: PointCloudManagerData,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
    let attached = data.attached;
    let has_selection = attached && data.active_selection > 0;
    let export_running = data.export_progress.is_some();
    let export_status = data.export_progress.map_or_else(
        || "Idle".to_string(),
        |(completed, total)| {
            let percent = if total == 0 {
                100.0
            } else {
                completed as f64 / total as f64 * 100.0
            };
            format!("{completed}/{total} points ({percent:.1}%)")
        },
    );

    let overview = if attached {
        column![
            status("Source", data.source),
            status("Source points", data.source_points.to_string()),
            status(
                "GPU display",
                format!("{} ({})", data.displayed_points, data.sample_label),
            ),
            status(
                "Edits",
                format!(
                    "{} points / {} transactions",
                    data.pending_edits, data.transactions
                ),
            ),
            status(
                "Selection",
                format!(
                    "{} active / {} saved sets",
                    data.active_selection, data.selection_sets
                ),
            ),
            status("Class table", format!("{} classes", data.class_count)),
            status(
                "Coordinates",
                if data.crs_declared {
                    "CRS metadata present"
                } else {
                    "CRS not declared"
                }
                .to_string(),
            ),
            status(
                "LOD cache",
                if data.index_running {
                    "Building (cancel available)".to_string()
                } else if data.indexed {
                    format!("Ready: {}", data.cache)
                } else {
                    "Not built".to_string()
                },
            ),
            status("Export", export_status),
            row![
                action("Attach / Replace", "POINTCLOUDATTACH", true),
                action(
                    "Restore Sidecar",
                    "POINTCLOUDRESTORE",
                    data.sidecar_available
                ),
                action(
                    "Build / Open LOD",
                    "POINTCLOUDINDEX",
                    attached && !data.index_running
                ),
                action("Cancel Index", "POINTCLOUDINDEXCANCEL", data.index_running),
                action("Info", "POINTCLOUDINFO", attached),
                action("Statistics", "POINTCLOUDSTATS", attached),
            ]
            .spacing(6),
        ]
        .spacing(4)
    } else {
        column![
            text("No LAS/LAZ point cloud is attached to this drawing.").size(12),
            row![
                action("Attach LAS/LAZ", "POINTCLOUDATTACH", true),
                action(
                    "Restore Drawing Sidecar",
                    "POINTCLOUDRESTORE",
                    data.sidecar_available
                ),
            ]
            .spacing(6),
        ]
        .spacing(8)
    };

    let display = column![
        status(
            "Current",
            format!("{} / {:.1} px", data.color_mode, data.point_size_px),
        ),
        row![
            action("Class", "POINTCLOUDCOLOR CLASS", attached),
            action("RGB", "POINTCLOUDCOLOR RGB", attached),
            action("Intensity", "POINTCLOUDCOLOR INTENSITY", attached),
            action("Elevation", "POINTCLOUDCOLOR ELEVATION", attached),
            action("Return", "POINTCLOUDCOLOR RETURN", attached),
            action("Source", "POINTCLOUDCOLOR SOURCE", attached),
        ]
        .spacing(6),
        row![
            text("Point size:").size(11),
            action("1 px", "POINTCLOUDPOINTSIZE 1", attached),
            action("2 px", "POINTCLOUDPOINTSIZE 2", attached),
            action("3 px", "POINTCLOUDPOINTSIZE 3", attached),
            action("5 px", "POINTCLOUDPOINTSIZE 5", attached),
        ]
        .spacing(6)
        .align_y(iced::Center),
    ]
    .spacing(7);

    let edit = column![
        text(
            "Selection tools prompt for survey coordinates; edits target stable LAS source indices."
        )
        .size(11),
        status("Active filter", data.selection_filter),
        row![
            action("Single Point", "POINTCLOUDSELECTPOINT", attached),
            action("3D Fence", "POINTCLOUDSELECTBOX", attached),
            action("3D Brush", "POINTCLOUDSELECTBRUSH", attached),
            action("Elevation Slice", "POINTCLOUDSELECTSLICE", attached),
            action("Set Filter", "POINTCLOUDSELECTFILTER", attached),
            action("Clear Filter", "POINTCLOUDSELECTFILTER CLEAR", attached),
            action("Clear Selection", "POINTCLOUDSELECTCLEAR", has_selection),
        ]
        .spacing(6),
        row![
            action(
                "Unclassified 1",
                "POINTCLOUDCLASSIFYSELECTION 1",
                has_selection
            ),
            action("Ground 2", "POINTCLOUDCLASSIFYSELECTION 2", has_selection),
            action("Building 6", "POINTCLOUDCLASSIFYSELECTION 6", has_selection),
            action(
                "Low Noise 7",
                "POINTCLOUDCLASSIFYSELECTION 7",
                has_selection
            ),
            action("Water 9", "POINTCLOUDCLASSIFYSELECTION 9", has_selection),
            action(
                "High Noise 18",
                "POINTCLOUDCLASSIFYSELECTION 18",
                has_selection
            ),
        ]
        .spacing(6),
        row![
            action(
                "Withhold",
                "POINTCLOUDFLAGSELECTION WITHHELD ON",
                has_selection
            ),
            action(
                "Clear Withheld",
                "POINTCLOUDFLAGSELECTION WITHHELD OFF",
                has_selection
            ),
            action(
                "Overlap",
                "POINTCLOUDFLAGSELECTION OVERLAP ON",
                has_selection
            ),
            action("Key Point", "POINTCLOUDFLAGSELECTION KEY ON", has_selection),
            action(
                "Synthetic",
                "POINTCLOUDFLAGSELECTION SYNTHETIC ON",
                has_selection
            ),
            action(
                "Undo Point Edit",
                "POINTCLOUDUNDO",
                attached && data.transactions > 0
            ),
        ]
        .spacing(6),
    ]
    .spacing(7);

    let interchange = column![
        row![
            action("Import .ptc", "POINTCLOUDPTCIMPORT", attached),
            action("Export .ptc", "POINTCLOUDPTCEXPORT", attached),
            action("Import .mnu", "MNUIMPORT", true),
            action("Export .mnu", "MNUEXPORT", true),
        ]
        .spacing(6),
        row![
            action(
                "Export LAS/LAZ",
                "POINTCLOUDEXPORT",
                attached && !export_running
            ),
            action("Export Status", "POINTCLOUDEXPORTSTATUS", export_running),
            action("Cancel Export", "POINTCLOUDEXPORTCANCEL", export_running),
            action("Detach", "POINTCLOUDDETACH", attached),
            Space::new().width(Length::Fill),
            button(text("Close").size(12))
                .on_press(Message::CloseModal)
                .padding([5, 14])
                .style(button::primary),
        ]
        .spacing(6)
        .align_y(iced::Center),
    ]
    .spacing(7);

    let body = column![
        section("Attachment and jobs", overview),
        section("GPU display", display),
        section("Selection and sparse edits", edit),
        section("Interchange and output", interchange),
    ]
    .spacing(8)
    .width(Length::Fill);

    container(scrollable(body).height(sizing.height))
        .padding(12)
        .width(sizing.width)
        .height(sizing.height)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.palette().background.base.color)),
            ..Default::default()
        })
        .into()
}
