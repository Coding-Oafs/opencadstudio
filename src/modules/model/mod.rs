// Model module — 3D solid modelling.
//
//   Model group  : create primitive solids (box, cylinder, cone, sphere, …)
//                  as ACIS Solid3D entities via acadrust's primitive builders.
//   Solids group : build solids from 2D profiles (EXTRUDE / REVOLVE / SWEEP /
//                  LOFT, implemented in `modules::insert::solid3d_cmds`).
//   Design group : combine solids with the kernel boolean operations
//                  (union / subtract / intersect).

pub mod boolean_cmd;
pub mod primitive_cmd;

use crate::modules::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

pub struct ModelModule;

const BOX_ICON: &[u8] = include_bytes!("../../../assets/icons/box3d.svg");
const CYLINDER_ICON: &[u8] = include_bytes!("../../../assets/icons/cylinder3d.svg");
const CONE_ICON: &[u8] = include_bytes!("../../../assets/icons/cone3d.svg");
const SPHERE_ICON: &[u8] = include_bytes!("../../../assets/icons/sphere3d.svg");
const WEDGE_ICON: &[u8] = include_bytes!("../../../assets/icons/wedge3d.svg");
const TORUS_ICON: &[u8] = include_bytes!("../../../assets/icons/torus3d.svg");
// No dedicated profile-tool SVGs exist; each Solids tool reuses the closest
// existing 3D icon rather than inventing binary assets.
const EXTRUDE_ICON: &[u8] = include_bytes!("../../../assets/icons/solid.svg");
const REVOLVE_ICON: &[u8] = include_bytes!("../../../assets/icons/torus3d.svg");
const SWEEP_ICON: &[u8] = include_bytes!("../../../assets/icons/array_path.svg");
const LOFT_ICON: &[u8] = include_bytes!("../../../assets/icons/wireframe.svg");
const UNION_ICON: &[u8] = include_bytes!("../../../assets/icons/union.svg");
const SUBTRACT_ICON: &[u8] = include_bytes!("../../../assets/icons/subtract.svg");
const INTERSECT_ICON: &[u8] = include_bytes!("../../../assets/icons/intersect.svg");

/// Helper to declare a ribbon tool that fires a named command.
fn tool(id: &'static str, label: &'static str, icon: &'static [u8]) -> ToolDef {
    ToolDef {
        id,
        label,
        icon: IconKind::Svg(icon),
        event: ModuleEvent::Command(id.to_string()),
    }
}

impl CadModule for ModelModule {
    fn id(&self) -> &'static str {
        "model"
    }
    fn title(&self) -> &'static str {
        "Model"
    }

    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: std::sync::OnceLock<Vec<RibbonGroup>> = std::sync::OnceLock::new();
        GROUPS.get_or_init(|| {
            vec![
                RibbonGroup {
                    title: "Modeling tools",
                    tools: vec![RibbonItem::LargeTool(ToolDef {
                        id: "MODEL_TOOL_CENTER", label: "All modeling\ntools",
                        icon: IconKind::Svg(EXTRUDE_ICON),
                        event: ModuleEvent::Command("TOOLCENTER MODEL".into()),
                    })],
                },
                RibbonGroup {
                    title: "Model",
                    tools: vec![
                        RibbonItem::LargeTool(tool("BOX", "Box", BOX_ICON)),
                        RibbonItem::LargeTool(tool("CYLINDER", "Cylinder", CYLINDER_ICON)),
                        RibbonItem::LargeTool(tool("CONE", "Cone", CONE_ICON)),
                        RibbonItem::LargeTool(tool("SPHERE", "Sphere", SPHERE_ICON)),
                        RibbonItem::Dropdown {
                            id: "MODEL_MORE",
                            icon: IconKind::Svg(WEDGE_ICON),
                            items: vec![
                                ("WEDGE", "Wedge", IconKind::Svg(WEDGE_ICON)),
                                ("TORUS", "Torus", IconKind::Svg(TORUS_ICON)),
                                ("PYRAMID", "Pyramid", IconKind::Svg(CONE_ICON)),
                                ("POLYSOLID", "Wall / polysolid", IconKind::Svg(BOX_ICON)),
                            ],
                            default: "WEDGE",
                        },
                    ],
                },
                RibbonGroup {
                    title: "Solids",
                    tools: vec![
                        RibbonItem::LargeTool(tool("EXTRUDE", "Extrude", EXTRUDE_ICON)),
                        RibbonItem::LargeTool(tool("REVOLVE", "Revolve", REVOLVE_ICON)),
                        RibbonItem::LargeTool(tool("SWEEP", "Sweep", SWEEP_ICON)),
                        RibbonItem::LargeTool(tool("LOFT", "Loft", LOFT_ICON)),
                    ],
                },
                RibbonGroup {
                    title: "Design",
                    tools: vec![
                        RibbonItem::LargeTool(tool("UNION", "Union", UNION_ICON)),
                        RibbonItem::LargeTool(tool("SUBTRACT", "Subtract", SUBTRACT_ICON)),
                        RibbonItem::LargeTool(tool("INTERSECT", "Intersect", INTERSECT_ICON)),
                    ],
                },
                RibbonGroup {
                    title: "3D editing",
                    tools: vec![
                        tool("3DMOVE", "Move in 3D", EXTRUDE_ICON).into(),
                        tool("3DROTATE", "Rotate in 3D", REVOLVE_ICON).into(),
                        tool("3DMIRROR", "Mirror in 3D", LOFT_ICON).into(),
                        tool("3DALIGN", "Align in 3D", LOFT_ICON).into(),
                        tool("SLICE", "Slice", SUBTRACT_ICON).into(),
                        tool("SECTION", "Cross-section", LOFT_ICON).into(),
                        tool("PRESSPULL", "Press / pull", EXTRUDE_ICON).into(),
                        tool("THICKEN", "Thicken", EXTRUDE_ICON).into(),
                        tool("INTERFERE", "Interference", INTERSECT_ICON).into(),
                    ],
                },
                RibbonGroup {
                    title: "Mesh repair",
                    tools: vec![
                        tool("MESHDIAGNOSE", "Diagnose mesh", LOFT_ICON).into(),
                        tool("MESHORIENT", "Orient faces", LOFT_ICON).into(),
                        tool("MESHFLIP", "Flip normals", LOFT_ICON).into(),
                        tool("TOOLCENTER MESHWELD", "Weld vertices…", LOFT_ICON).into(),
                        tool("TOOLCENTER MESHFILL", "Fill holes…", LOFT_ICON).into(),
                        tool("TOOLCENTER MESHDELETEFACE", "Delete faces…", LOFT_ICON).into(),
                    ],
                },
            ]
        })
    }
}
