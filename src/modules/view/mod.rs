// View module — viewport tools, navigation, visual styles, palettes, interface.

mod cascade;
mod file_tabs;
mod layout_tabs;
pub mod limits;
mod orbit;
mod ortho;
mod pan;
mod persp;
pub mod plot_window;
mod properties_palette;
pub mod quick_print;
mod sheetset;
mod tile_horiz;
mod tile_vert;
mod tool_palettes;
pub mod ucs_cmd;
mod ucs_icon;
mod view_front;
mod view_iso;
mod view_right;
mod view_top;
mod viewcube;
pub mod visual_style;
mod vports_config;
mod vports_join;
mod vports_named;
mod vports_restore;
mod zoom_ext;
mod zoom_in;
mod zoom_out;
pub mod zoom_window;

use crate::modules::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

pub struct ViewModule;

impl CadModule for ViewModule {
    fn id(&self) -> &'static str {
        "view"
    }
    fn title(&self) -> &'static str {
        "View"
    }

    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: std::sync::OnceLock<Vec<RibbonGroup>> = std::sync::OnceLock::new();
        GROUPS.get_or_init(|| {
            vec![
                // ── Viewport Tools ───────────────────────────────────────────────
                RibbonGroup {
                    title: "Viewport Tools",
                    tools: vec![
                        RibbonItem::LargeTool(ucs_icon::tool()),
                        RibbonItem::LargeTool(viewcube::tool()),
                    ],
                },
                // ── Navigate ─────────────────────────────────────────────────────
                RibbonGroup {
                    title: "Navigate",
                    tools: vec![
                        RibbonItem::LargeTool(ToolDef {
                            id: "SELECTTOOL",
                            label: "Select",
                            icon: IconKind::Svg(include_bytes!("../../../assets/icons/point.svg")),
                            event: ModuleEvent::Command("SELECTTOOL".to_string()),
                        }),
                        RibbonItem::LargeTool(ToolDef {
                            id: "NAVIGATOR",
                            label: "Navigator",
                            icon: IconKind::Svg(include_bytes!("../../../assets/icons/pan.svg")),
                            event: ModuleEvent::Command("NAVIGATOR".to_string()),
                        }),
                        RibbonItem::LargeTool(zoom_ext::tool()),
                        RibbonItem::Tool(zoom_window::tool()),
                        RibbonItem::Tool(zoom_in::tool()),
                        RibbonItem::Tool(zoom_out::tool()),
                        RibbonItem::Tool(pan::tool()),
                        RibbonItem::Tool(orbit::tool()),
                    ],
                },
                // ── Model Viewports ───────────────────────────────────────────────
                RibbonGroup {
                    title: "Model Viewports",
                    tools: vec![
                        RibbonItem::LargeTool(vports_config::tool()),
                        RibbonItem::Tool(vports_named::tool()),
                        RibbonItem::Tool(vports_join::tool()),
                        RibbonItem::Tool(vports_restore::tool()),
                    ],
                },
                // ── Visual Style ──────────────────────────────────────────────────
                RibbonGroup {
                    // WIREFRAME and SOLID ids are special-cased in ribbon.rs
                    // for toggle-state highlighting based on Ribbon::wireframe.
                    title: "Visual Style",
                    tools: vec![RibbonItem::LargeDropdown {
                        id: "VISUAL_STYLE",
                        label: "Visual\nStyle",
                        icon: visual_style::VISUAL_STYLES[0].icon,
                        items: visual_style::VISUAL_STYLES
                            .iter()
                            .map(|style| (style.command, style.label, style.icon))
                            .collect(),
                        default: visual_style::VISUAL_STYLES[0].command,
                    }],
                },
                // ── Projection ────────────────────────────────────────────────────
                RibbonGroup {
                    // ORTHO and PERSP ids are special-cased in ribbon.rs
                    // for toggle-state highlighting based on Camera::projection.
                    title: "Projection",
                    tools: vec![
                        RibbonItem::LargeTool(ortho::tool()),
                        RibbonItem::LargeTool(persp::tool()),
                    ],
                },
                // ── Preset Views ──────────────────────────────────────────────────
                RibbonGroup {
                    title: "Preset",
                    tools: vec![
                        RibbonItem::Tool(view_top::tool()),
                        RibbonItem::Tool(view_front::tool()),
                        RibbonItem::Tool(view_right::tool()),
                        RibbonItem::Tool(view_iso::tool()),
                    ],
                },
                // ── Palettes ──────────────────────────────────────────────────────
                RibbonGroup {
                    title: "Palettes",
                    tools: vec![
                        RibbonItem::LargeTool(tool_palettes::tool()),
                        RibbonItem::LargeTool(properties_palette::tool()),
                        RibbonItem::LargeTool(sheetset::tool()),
                    ],
                },
                // ── Interface ─────────────────────────────────────────────────────
                RibbonGroup {
                    title: "Interface",
                    tools: vec![
                        RibbonItem::LargeTool(file_tabs::tool()),
                        RibbonItem::LargeTool(layout_tabs::tool()),
                        RibbonItem::Tool(tile_horiz::tool()),
                        RibbonItem::Tool(tile_vert::tool()),
                        RibbonItem::Tool(cascade::tool()),
                    ],
                },
                // ── Basemap ───────────────────────────────────────────────────────
                RibbonGroup {
                    title: "Basemap",
                    tools: vec![
                        RibbonItem::LargeDropdown {
                            id: "BASEMAP_PROVIDER",
                            label: "Imagery",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/underlay_layers.svg"
                            )),
                            items: vec![
                                (
                                    "BASEMAP ARCGIS",
                                    "World Imagery",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_layers.svg"
                                    )),
                                ),
                                (
                                    "BASEMAP STREETS",
                                    "Street Map",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_layers.svg"
                                    )),
                                ),
                                (
                                    "BASEMAP GOOGLE",
                                    "Google Hybrid",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_layers.svg"
                                    )),
                                ),
                                (
                                    "BASEMAP OFF",
                                    "Off",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_layers.svg"
                                    )),
                                ),
                            ],
                            default: "BASEMAP ARCGIS",
                        },
                        RibbonItem::LargeDropdown {
                            id: "BASEMAP_PROJECTION",
                            label: "Projection",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/underlay_frames.svg"
                            )),
                            items: vec![
                                (
                                    "BASEMAP PROJ DRAWING",
                                    "From Drawing",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_frames.svg"
                                    )),
                                ),
                                (
                                    "BASEMAP PROJ LAS",
                                    "From LAS",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_frames.svg"
                                    )),
                                ),
                                (
                                    "BASEMAP PROJ DEFAULT",
                                    "Web Mercator",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/underlay_frames.svg"
                                    )),
                                ),
                            ],
                            default: "BASEMAP PROJ DRAWING",
                        },
                        RibbonItem::Tool(ToolDef {
                            id: "CRS",
                            label: "Drawing CRS",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/underlay_frames.svg"
                            )),
                            event: ModuleEvent::Command("CRS".to_string()),
                        }),
                        RibbonItem::LargeDropdown {
                            id: "WORKING_UNITS",
                            label: "Working Units",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/dim_linear.svg"
                            )),
                            items: vec![
                                (
                                    "WORKINGUNITS METERS",
                                    "Meters",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/dim_linear.svg"
                                    )),
                                ),
                                (
                                    "WORKINGUNITS CENTIMETERS",
                                    "Centimeters",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/dim_linear.svg"
                                    )),
                                ),
                                (
                                    "WORKINGUNITS FEET",
                                    "Feet",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/dim_linear.svg"
                                    )),
                                ),
                                (
                                    "WORKINGUNITS INCHES",
                                    "Inches",
                                    IconKind::Svg(include_bytes!(
                                        "../../../assets/icons/dim_linear.svg"
                                    )),
                                ),
                            ],
                            default: "WORKINGUNITS METERS",
                        },
                        RibbonItem::Tool(ToolDef {
                            id: "BASEMAP_ZOOMIN",
                            label: "Zoom In",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/zoom_in.svg"
                            )),
                            event: ModuleEvent::Command("BASEMAP ZOOMIN".to_string()),
                        }),
                        RibbonItem::Tool(ToolDef {
                            id: "BASEMAP_ZOOMOUT",
                            label: "Zoom Out",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/zoom_out.svg"
                            )),
                            event: ModuleEvent::Command("BASEMAP ZOOMOUT".to_string()),
                        }),
                        RibbonItem::Tool(ToolDef {
                            id: "BASEMAP",
                            label: "Basemap",
                            icon: IconKind::Svg(include_bytes!(
                                "../../../assets/icons/snap_underlays.svg"
                            )),
                            event: ModuleEvent::Command("BASEMAP".to_string()),
                        }),
                    ],
                },
                // ── Plot ──────────────────────────────────────────────────────────
                // Model space has no paper-space side toolbar, so Page Setup
                // (format/orientation/pick window for PLOTWINDOW) needs an
                // entry here too.
                RibbonGroup {
                    title: "Plot",
                    tools: vec![RibbonItem::Tool(ToolDef {
                        id: "PAGESETUP",
                        label: "Page Setup",
                        icon: IconKind::Svg(include_bytes!("../../../assets/icons/pagesetup.svg")),
                        event: ModuleEvent::Command("PAGESETUP".to_string()),
                    })],
                },
            ]
        })
    }
}
