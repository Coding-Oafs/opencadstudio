//! A frontend home for elevation, maps and native spatial projects.
use crate::modules::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};
const TERRAIN: &[u8] = include_bytes!("../../../assets/icons/wireframe.svg");
const MAP: &[u8] = include_bytes!("../../../assets/icons/view_top.svg");
const INFO: &[u8] = include_bytes!("../../../assets/icons/audit.svg");
fn tool(command: &'static str, label: &'static str, icon: &'static [u8]) -> RibbonItem {
    RibbonItem::LargeTool(ToolDef {
        id: command,
        label,
        icon: IconKind::Svg(icon),
        event: ModuleEvent::Command(command.into()),
    })
}
pub struct TerrainModule;
impl CadModule for TerrainModule {
    fn id(&self) -> &'static str {
        "terrain"
    }
    fn title(&self) -> &'static str {
        "Terrain & GIS"
    }
    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: std::sync::OnceLock<Vec<RibbonGroup>> = std::sync::OnceLock::new();
        GROUPS.get_or_init(|| {
            vec![
                RibbonGroup {
                    title: "Project",
                    tools: vec![tool("PROJECTEXPLORER", "Project\nexplorer", INFO), tool("JOBSPANEL", "Background\njobs", INFO)],
                },
                RibbonGroup {
                    title: "Elevation",
                    tools: vec![
                        tool("TERRAINPANEL", "Terrain\nworkspace", TERRAIN),
                        tool("TERRAIN INFO", "Pick\nelevation", INFO),
                    ],
                },
                RibbonGroup {
                    title: "Maps & coordinates",
                    tools: vec![
                        tool("TOOLCENTER TERRAIN", "Map & CRS\ntools", MAP),
                        tool("BASEMAP ARCGIS", "Aerial\nimagery", MAP),
                        tool("BASEMAP STREETS", "Street\nmap", MAP),
                        tool("BASEMAP OFF", "Hide map", MAP),
                    ],
                },
                RibbonGroup {
                    title: "GIS",
                    tools: vec![
                        tool("GISIMPORT", "Import GIS", MAP),
                        tool("TOOLCENTER GIS", "GIS & project\ntools", INFO),
                    ],
                },
                RibbonGroup {
                    title: "Scene",
                    tools: vec![
                        tool("VIEW ISO", "3D view", TERRAIN),
                        tool("ZOOM EXTENTS", "Fit scene", MAP),
                    ],
                },
            ]
        })
    }
}
