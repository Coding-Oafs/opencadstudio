//! Click-first manager for an attached LAS/LAZ point cloud.

use crate::app::Message;
use iced::widget::{
    button, checkbox, column, container, row, scrollable, slider, text, text_input, Space,
};
use iced::{Background, Element, Length, Theme};

#[derive(Clone, Debug, Default)]
pub struct PointCloudClassRow {
    pub code: u8,
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
    pub locked: bool,
    pub total: u64,
    pub withheld: u64,
    pub overlap: u64,
    pub key_points: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PointCloudAuditRow {
    pub created_unix_ms: u64,
    pub action: String,
    pub detail: String,
}

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
    pub section_width_map_units: i32,
    pub crs_declared: bool,
    pub indexed: bool,
    pub index_running: bool,
    pub cache: String,
    pub export_progress: Option<(u64, u64)>,
    pub urban_job_running: bool,
    pub urban_status: String,
    pub sidecar_available: bool,
    pub selection_filter: String,
    pub resident_tiles: usize,
    pub resident_points: usize,
    pub visible_tiles: usize,
    pub cpu_memory_bytes: usize,
    pub gpu_memory_bytes: usize,
    pub lod_label: String,
    pub pending_tile_requests: usize,
    pub cancelled_tile_requests: u64,
    pub stale_tile_results: u64,
    pub crs_label: String,
    pub survey_readiness: String,
    pub class_rows: Vec<PointCloudClassRow>,
    pub audit_rows: Vec<PointCloudAuditRow>,
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
            status(
                "Streaming",
                format!(
                    "{} visible / {} resident tiles; {} resident points",
                    data.visible_tiles, data.resident_tiles, data.resident_points
                ),
            ),
            status("LOD", data.lod_label),
            status(
                "Memory",
                format!(
                    "CPU {:.1} MiB / GPU {:.1} MiB",
                    data.cpu_memory_bytes as f64 / (1024.0 * 1024.0),
                    data.gpu_memory_bytes as f64 / (1024.0 * 1024.0)
                ),
            ),
            status(
                "Tile requests",
                format!(
                    "{} pending / {} cancelled / {} stale",
                    data.pending_tile_requests,
                    data.cancelled_tile_requests,
                    data.stale_tile_results
                ),
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
            action("4 px", "POINTCLOUDPOINTSIZE 4", attached),
            action("5 px", "POINTCLOUDPOINTSIZE 5", attached),
            action("6 px", "POINTCLOUDPOINTSIZE 6", attached),
            action("8 px", "POINTCLOUDPOINTSIZE 8", attached),
            action("10 px", "POINTCLOUDPOINTSIZE 10", attached),
        ]
        .spacing(6)
        .align_y(iced::Center),
        row![
            text("Slice width:").size(11),
            slider(1..=1024, data.section_width_map_units, move |value| {
                Message::Command(format!("POINTCLOUDSECTIONWIDTH {value}"))
            })
            .step(1)
            .width(Length::Fixed(220.0)),
            text(format!("{} map units", data.section_width_map_units)).size(11),
        ]
        .spacing(6)
        .align_y(iced::Center),
    ]
    .spacing(7);

    let mut class_rows = column![row![
        text("Show").size(10).width(36),
        text("Code / name").size(10).width(Length::Fixed(245.0)),
        text("R").size(10),
        text("G").size(10),
        text("B").size(10),
        text("Displayed statistics").size(10),
    ]
    .spacing(8)
    .align_y(iced::Center)]
    .spacing(4);
    for class in data.class_rows {
        let code = class.code;
        let color = class.color;
        class_rows = class_rows.push(
            row![
                checkbox(class.visible)
                    .on_toggle(move |visible| Message::PointCloudClassVisibilityChanged(
                        code, visible
                    ))
                    .size(15)
                    .width(36),
                row![
                    text(code.to_string()).size(11).width(30),
                    text_input("Class name", &class.name)
                        .on_input(move |name| Message::PointCloudClassNameChanged(code, name))
                        .size(11)
                        .padding([3, 5])
                        .width(Length::Fixed(205.0)),
                ]
                .spacing(4),
                slider(0..=255, color[0], move |value| {
                    Message::PointCloudClassColorChanged(code, 0, value)
                })
                .step(1)
                .width(Length::Fixed(74.0)),
                slider(0..=255, color[1], move |value| {
                    Message::PointCloudClassColorChanged(code, 1, value)
                })
                .step(1)
                .width(Length::Fixed(74.0)),
                slider(0..=255, color[2], move |value| {
                    Message::PointCloudClassColorChanged(code, 2, value)
                })
                .step(1)
                .width(Length::Fixed(74.0)),
                text(format!(
                    "{} pts · W{} O{} K{}",
                    class.total, class.withheld, class.overlap, class.key_points
                ))
                .size(10)
                .width(Length::Fill),
                button(text("Delete").size(10))
                    .on_press_maybe((!class.locked).then_some(Message::PointCloudClassRemove(code)))
                    .padding([3, 6])
                    .style(button::secondary),
            ]
            .spacing(6)
            .align_y(iced::Center),
        );
    }
    class_rows = class_rows.push(
        row![
            action("Add Class", "POINTCLOUDCLASSADD", attached),
            text("Changes are saved to the drawing sidecar and .ptc export.").size(10),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );

    let mut audit_rows = column![].spacing(3);
    for entry in data.audit_rows.iter().rev().take(20) {
        audit_rows = audit_rows.push(
            text(format!(
                "{}  {} — {}",
                entry.created_unix_ms, entry.action, entry.detail
            ))
            .size(10),
        );
    }
    if data.audit_rows.is_empty() {
        audit_rows = audit_rows.push(text("No persisted audit entries yet.").size(10));
    }

    let coordinates = column![
        status("Declared CRS", data.crs_label),
        status("Survey safeguard", data.survey_readiness),
        text("Reprojection transforms XY through a selected EPSG definition and preserves Z values unless a separately verified vertical transformation is performed.").size(10),
        row![
            action("Inspect CRS", "POINTCLOUDCRS", attached),
            action("Reproject Copy", "POINTCLOUDREPROJECT", attached),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let urban = column![
        status(
            "Status",
            if data.urban_status.is_empty() {
                "Ready".to_string()
            } else {
                data.urban_status
            },
        ),
        status(
            "Boston profile",
            "Buildings ON · Roads ON · Vegetation ON".to_string(),
        ),
        status(
            "Settings",
            "Road edge +1 ft · Tree radius 12 ft · full-density source stream".to_string(),
        ),
        status(
            "Output",
            "classified\\*_classified.laz · ASPRS display + UPCP label".to_string(),
        ),
        text("The source LAZ is never overwritten. Original classes are retained in source_classification; Boston reference data is current and may differ from the 2013–2014 survey epoch.")
            .size(10),
        row![
            action(
                "Classify Current Tile",
                "POINTCLOUDURBANCLASSIFY CURRENT",
                attached && !data.urban_job_running,
            ),
            action(
                "Classify Source Folder",
                "POINTCLOUDURBANCLASSIFY FOLDER",
                attached && !data.urban_job_running,
            ),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let edit = column![
        text("Viewport tools select displayed points in screen space; edits target stable LAS source indices.")
        .size(11),
        status("Active filter", data.selection_filter),
        row![
            action("Single Point", "POINTCLOUDSELECTPOINT", attached),
            action("Screen Window", "POINTCLOUDSELECTBOX", attached),
            action("Polygon Fence", "POINTCLOUDSELECTFENCE", attached),
            action("32 px Brush", "POINTCLOUDSELECTBRUSH", attached),
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
            action(
                "Vegetation 5",
                "POINTCLOUDCLASSIFYSELECTION 5",
                has_selection
            ),
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
        section("CRS and survey safeguards", coordinates),
        section("Urban classification", urban),
        section("Selection and sparse edits", edit),
        section("Editable class table and displayed statistics", class_rows),
        section("Point-cloud edit audit", audit_rows),
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
