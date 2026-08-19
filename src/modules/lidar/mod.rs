//! Production LiDAR workflow ribbon for native LAS/LAZ attachments.

use crate::modules::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

const ATTACH: &[u8] = include_bytes!("../../../assets/icons/pc_attach.svg");
const CLOUD: &[u8] = include_bytes!("../../../assets/icons/recap.svg");
const COLOR: &[u8] = include_bytes!("../../../assets/icons/color_palette.svg");
const POINT: &[u8] = include_bytes!("../../../assets/icons/point.svg");
const AUDIT: &[u8] = include_bytes!("../../../assets/icons/audit.svg");
const EXPORT: &[u8] = include_bytes!("../../../assets/icons/data_extract.svg");
const IMPORT: &[u8] = include_bytes!("../../../assets/icons/cui_import.svg");
const UNDO: &[u8] = include_bytes!("../../../assets/icons/clear_tool.svg");

fn tool(
    id: &'static str,
    label: &'static str,
    command: &'static str,
    icon: &'static [u8],
) -> ToolDef {
    ToolDef {
        id,
        label,
        icon: IconKind::Svg(icon),
        event: ModuleEvent::Command(command.to_string()),
    }
}

pub struct LidarModule;

impl CadModule for LidarModule {
    fn id(&self) -> &'static str {
        "lidar"
    }

    fn title(&self) -> &'static str {
        "LiDAR"
    }

    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: std::sync::OnceLock<Vec<RibbonGroup>> = std::sync::OnceLock::new();
        GROUPS.get_or_init(|| {
            vec![
                RibbonGroup {
                    title: "Cloud",
                    tools: vec![
                        RibbonItem::LargeTool(tool(
                            "LIDAR_ATTACH",
                            "Attach",
                            "POINTCLOUDATTACH",
                            ATTACH,
                        )),
                        RibbonItem::LargeTool(tool(
                            "LIDAR_ATTACH_FOLDER",
                            "Attach Folder",
                            "POINTCLOUDATTACHFOLDER",
                            ATTACH,
                        )),
                        RibbonItem::Dropdown {
                            id: "LIDAR_DENSITY",
                            icon: IconKind::Svg(POINT),
                            items: vec![
                                ("POINTCLOUDDENSITY AUTO", "Auto", IconKind::Svg(POINT)),
                                ("POINTCLOUDDENSITY 2", "1-in-2", IconKind::Svg(POINT)),
                                ("POINTCLOUDDENSITY 5", "1-in-5", IconKind::Svg(POINT)),
                                ("POINTCLOUDDENSITY 10", "1-in-10", IconKind::Svg(POINT)),
                                ("POINTCLOUDDENSITY FULL", "Full", IconKind::Svg(POINT)),
                            ],
                            default: "POINTCLOUDDENSITY AUTO",
                        },
                        RibbonItem::LargeTool(tool(
                            "LIDAR_MANAGER",
                            "Manager",
                            "POINTCLOUDMANAGER",
                            CLOUD,
                        )),
                        tool("LIDAR_INDEX", "Build LOD", "POINTCLOUDINDEX", CLOUD).into(),
                        tool("LIDAR_RESTORE", "Restore", "POINTCLOUDRESTORE", ATTACH).into(),
                        tool("LIDAR_INFO", "Cloud Info", "POINTCLOUDINFO", AUDIT).into(),
                        tool("LIDAR_STATS", "Statistics", "POINTCLOUDSTATS", AUDIT).into(),
                    ],
                },
                RibbonGroup {
                    title: "Display",
                    tools: vec![
                        RibbonItem::LargeDropdown {
                            id: "LIDAR_COLOR_MODE",
                            label: "Color By",
                            icon: IconKind::Svg(COLOR),
                            items: vec![
                                (
                                    "POINTCLOUDCOLOR CLASS",
                                    "Classification",
                                    IconKind::Svg(COLOR),
                                ),
                                ("POINTCLOUDCOLOR RGB", "RGB", IconKind::Svg(COLOR)),
                                (
                                    "POINTCLOUDCOLOR INTENSITY",
                                    "Intensity",
                                    IconKind::Svg(COLOR),
                                ),
                                (
                                    "POINTCLOUDCOLOR ELEVATION",
                                    "Elevation",
                                    IconKind::Svg(COLOR),
                                ),
                                ("POINTCLOUDCOLOR RETURN", "Return", IconKind::Svg(COLOR)),
                                (
                                    "POINTCLOUDCOLOR SOURCE",
                                    "Point Source",
                                    IconKind::Svg(COLOR),
                                ),
                            ],
                            default: "POINTCLOUDCOLOR CLASS",
                        },
                        RibbonItem::LargeDropdown {
                            id: "LIDAR_POINT_SIZE",
                            label: "Point Size",
                            icon: IconKind::Svg(POINT),
                            items: vec![
                                ("POINTCLOUDPOINTSIZE 1", "1 pixel", IconKind::Svg(POINT)),
                                ("POINTCLOUDPOINTSIZE 2", "2 pixels", IconKind::Svg(POINT)),
                                ("POINTCLOUDPOINTSIZE 3", "3 pixels", IconKind::Svg(POINT)),
                                ("POINTCLOUDPOINTSIZE 5", "5 pixels", IconKind::Svg(POINT)),
                            ],
                            default: "POINTCLOUDPOINTSIZE 3",
                        },
                    ],
                },
                RibbonGroup {
                    title: "Select",
                    tools: vec![
                        tool("LIDAR_PICK", "Point", "POINTCLOUDSELECTPOINT", POINT).into(),
                        tool("LIDAR_FENCE", "Fence", "POINTCLOUDSELECTBOX", POINT).into(),
                        tool("LIDAR_BRUSH", "Brush", "POINTCLOUDSELECTBRUSH", POINT).into(),
                        tool("LIDAR_SLICE", "Z Slice", "POINTCLOUDSELECTSLICE", POINT).into(),
                        tool("LIDAR_FILTER", "Filter", "POINTCLOUDSELECTFILTER", AUDIT).into(),
                        tool("LIDAR_SELECT_CLEAR", "Clear", "POINTCLOUDSELECTCLEAR", UNDO).into(),
                    ],
                },
                RibbonGroup {
                    title: "Classify",
                    tools: vec![
                        RibbonItem::LargeDropdown {
                            id: "LIDAR_ASSIGN_CLASS",
                            label: "Assign Class",
                            icon: IconKind::Svg(POINT),
                            items: vec![
                                (
                                    "POINTCLOUDCLASSIFYSELECTION 1",
                                    "Unclassified (1)",
                                    IconKind::Svg(POINT),
                                ),
                                (
                                    "POINTCLOUDCLASSIFYSELECTION 2",
                                    "Ground (2)",
                                    IconKind::Svg(POINT),
                                ),
                                (
                                    "POINTCLOUDCLASSIFYSELECTION 6",
                                    "Building (6)",
                                    IconKind::Svg(POINT),
                                ),
                                (
                                    "POINTCLOUDCLASSIFYSELECTION 7",
                                    "Low Point (7)",
                                    IconKind::Svg(POINT),
                                ),
                                (
                                    "POINTCLOUDCLASSIFYSELECTION 9",
                                    "Water (9)",
                                    IconKind::Svg(POINT),
                                ),
                                (
                                    "POINTCLOUDCLASSIFYSELECTION 18",
                                    "High Noise (18)",
                                    IconKind::Svg(POINT),
                                ),
                            ],
                            default: "POINTCLOUDCLASSIFYSELECTION 2",
                        },
                        tool("LIDAR_AUTO_GROUND", "Auto Ground", "POINTCLOUDGROUND", CLOUD).into(),
                        tool("LIDAR_AUTO_NOISE", "Auto Noise", "POINTCLOUDNOISE 2.0 4 7", UNDO)
                            .into(),
                        tool("LIDAR_CONTOURS", "Contours", "POINTCLOUDCONTOUR 1", CLOUD).into(),
                        RibbonItem::Dropdown {
                            id: "LIDAR_FLAGS",
                            icon: IconKind::Svg(AUDIT),
                            items: vec![
                                (
                                    "POINTCLOUDFLAGSELECTION WITHHELD ON",
                                    "Set Withheld",
                                    IconKind::Svg(AUDIT),
                                ),
                                (
                                    "POINTCLOUDFLAGSELECTION WITHHELD OFF",
                                    "Clear Withheld",
                                    IconKind::Svg(AUDIT),
                                ),
                                (
                                    "POINTCLOUDFLAGSELECTION OVERLAP ON",
                                    "Set Overlap",
                                    IconKind::Svg(AUDIT),
                                ),
                                (
                                    "POINTCLOUDFLAGSELECTION KEY ON",
                                    "Set Key Point",
                                    IconKind::Svg(AUDIT),
                                ),
                                (
                                    "POINTCLOUDFLAGSELECTION SYNTHETIC ON",
                                    "Set Synthetic",
                                    IconKind::Svg(AUDIT),
                                ),
                            ],
                            default: "POINTCLOUDFLAGSELECTION WITHHELD ON",
                        },
                        tool("LIDAR_UNDO", "Undo Edit", "POINTCLOUDUNDO", UNDO).into(),
                    ],
                },
                RibbonGroup {
                    title: "Section",
                    tools: vec![
                        tool("LIDAR_SECTION", "Draw Section", "POINTCLOUDSECTION", CLOUD).into(),
                        tool(
                            "LIDAR_SECTION_VIEW",
                            "Section View",
                            "POINTCLOUDSECTIONVIEW",
                            CLOUD,
                        )
                        .into(),
                        tool(
                            "LIDAR_SECTION_WIDER",
                            "Wider Band",
                            "POINTCLOUDSECTIONWIDTH 2.0",
                            CLOUD,
                        )
                        .into(),
                        tool(
                            "LIDAR_SECTION_NARROWER",
                            "Narrower Band",
                            "POINTCLOUDSECTIONWIDTH 0.5",
                            CLOUD,
                        )
                        .into(),
                        tool(
                            "LIDAR_SECTION_STEP",
                            "Step +1",
                            "POINTCLOUDSECTIONMOVE 1",
                            CLOUD,
                        )
                        .into(),
                        tool(
                            "LIDAR_SECTION_CLEAR",
                            "Clear Section",
                            "POINTCLOUDSECTIONCLEAR",
                            UNDO,
                        )
                        .into(),
                    ],
                },
                RibbonGroup {
                    title: "Interchange",
                    tools: vec![
                        RibbonItem::LargeTool(tool(
                            "LIDAR_EXPORT",
                            "Export LAS/LAZ",
                            "POINTCLOUDEXPORT",
                            EXPORT,
                        )),
                        RibbonItem::LargeTool(tool(
                            "LIDAR_EXPORT_ALL",
                            "Export Merged",
                            "POINTCLOUDEXPORTALL",
                            EXPORT,
                        )),
                        tool(
                            "LIDAR_PTC_IMPORT",
                            "Import PTC",
                            "POINTCLOUDPTCIMPORT",
                            IMPORT,
                        )
                        .into(),
                        tool(
                            "LIDAR_PTC_EXPORT",
                            "Export PTC",
                            "POINTCLOUDPTCEXPORT",
                            EXPORT,
                        )
                        .into(),
                        tool("LIDAR_MNU_IMPORT", "Import MNU", "MNUIMPORT", IMPORT).into(),
                        tool("LIDAR_MNU_EXPORT", "Export MNU", "MNUEXPORT", EXPORT).into(),
                        tool("LIDAR_CANCEL", "Cancel Job", "POINTCLOUDEXPORTCANCEL", UNDO).into(),
                    ],
                },
            ]
        })
    }
}
