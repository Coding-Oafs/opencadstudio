//! Discoverable tool catalog. Curated forms complement (and never replace)
//! the registered command list and the existing ribbon definitions.
use crate::app::Message;
use crate::modules::{ModuleEvent, RibbonItem, ToolDef};
use iced::widget::{
    button, column, container, pick_list, row, scrollable, text, text_input, tooltip,
};
use iced::{Element, Fill};

#[derive(Clone)]
pub struct Field {
    pub label: &'static str,
    pub default: &'static str,
    pub kind: FieldKind,
}

#[derive(Clone)]
pub enum FieldKind {
    Number { min: f64, max: f64, integer: bool },
    Choice(&'static [&'static str]),
    Text,
}

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub category: String,
    pub command: String,
    pub description: String,
    pub fields: Vec<Field>,
}

impl Tool {
    pub fn command_with_values(&self, values: &[String]) -> Result<String, String> {
        if values.len() != self.fields.len() {
            return Err("Choose the tool again to reset its settings.".into());
        }
        for (field, value) in self.fields.iter().zip(values) {
            let value = value.trim();
            let valid = match &field.kind {
                FieldKind::Number { min, max, integer } => value.parse::<f64>().is_ok_and(|n| {
                    n.is_finite() && n >= *min && n <= *max && (!integer || n.fract() == 0.0)
                }),
                FieldKind::Choice(options) => options.contains(&value),
                FieldKind::Text => !value.is_empty() && !value.contains(['\n', '\r']),
            };
            if !valid {
                return Err(format!(
                    "Check {}. Enter a valid value before running this tool.",
                    field.label
                ));
            }
        }
        let mut parts = vec![self.command.clone()];
        for (field, value) in self.fields.iter().zip(values) {
            // Integer inputs such as 64.0 and 1e2 are valid whole numbers;
            // serialize them in the form expected by usize/u16 command parsers.
            parts.push(match field.kind {
                FieldKind::Number { integer: true, .. } => {
                    value.trim().parse::<f64>().unwrap().to_string()
                }
                _ => value.trim().to_string(),
            });
        }
        Ok(parts.join(" "))
    }
}

pub struct ToolCenter {
    pub tools: Vec<Tool>,
    pub search: String,
    pub category: String,
    pub selected: Option<usize>,
    pub values: Vec<String>,
}

impl Default for ToolCenter {
    fn default() -> Self {
        Self {
            tools: catalog(),
            search: String::new(),
            category: "All tools".into(),
            selected: None,
            values: vec![],
        }
    }
}

impl ToolCenter {
    pub fn select(&mut self, index: usize) {
        if let Some(tool) = self.tools.get(index) {
            self.values = tool.fields.iter().map(|f| f.default.to_string()).collect();
            self.selected = Some(index);
        }
    }
    pub fn matches(&self, tool: &Tool) -> bool {
        (self.category == "All tools" || self.category == tool.category)
            && self.search.split_whitespace().all(|word| {
                format!(
                    "{} {} {} {}",
                    tool.name, tool.command, tool.category, tool.description
                )
                .to_lowercase()
                .contains(&word.to_lowercase())
            })
    }
}

fn number(label: &'static str, default: &'static str, min: f64, max: f64, integer: bool) -> Field {
    Field {
        label,
        default,
        kind: FieldKind::Number { min, max, integer },
    }
}
fn positive(label: &'static str, default: &'static str) -> Field {
    number(label, default, 0.000001, 1e9, false)
}
fn choice(label: &'static str, default: &'static str, options: &'static [&'static str]) -> Field {
    Field {
        label,
        default,
        kind: FieldKind::Choice(options),
    }
}
fn input(label: &'static str) -> Field {
    Field {
        label,
        default: "",
        kind: FieldKind::Text,
    }
}

pub fn curated() -> Vec<Tool> {
    let mut tools = Vec::new();
    let mut add =
        |category: &str, name: &str, command: &str, description: &str, fields: Vec<Field>| {
            tools.push(Tool {
                category: category.into(),
                name: name.into(),
                command: command.into(),
                description: description.into(),
                fields,
            })
        };
    #[cfg(not(target_arch = "wasm32"))]
    {
        add("Terrain", "Terrain workspace", "TERRAINPANEL", "Set a site location, download global elevation, inspect terrain and drape selected objects using labeled controls.", vec![]);
        add("Terrain", "Drawing coordinate system", "CRS", "Assign the coordinate system used by this drawing. Use the EPSG code supplied with your survey or map. Existing point clouds are transformed when supported.", vec![number("EPSG code", "", 1., 65535., true)]);
        add(
            "Terrain",
            "Use attached cloud coordinates",
            "CRS LAS",
            "Use the active LAS/LAZ source coordinate system for the drawing.",
            vec![],
        );
        add(
            "Terrain",
            "Aerial imagery",
            "BASEMAP ARCGIS",
            "Display the aerial imagery underlay in the drawing coordinate system.",
            vec![],
        );
        add(
            "Terrain",
            "Street map",
            "BASEMAP STREETS",
            "Display the street map underlay.",
            vec![],
        );
        add(
            "Terrain",
            "Map detail",
            "BASEMAP ZOOM",
            "Set the source tile zoom for the basemap.",
            vec![number("Map zoom", "15", 0., 22., true)],
        );
        add(
            "Terrain",
            "Pick map extent",
            "BASEMAP BOUNDS",
            "Pick two corners in the drawing to limit the map coverage.",
            vec![],
        );
        add(
            "Terrain",
            "Coordinate information",
            "SPATIALINFO",
            "Report the current drawing coordinate system, units and map extent.",
            vec![],
        );
        for (name, command, description) in [
            ("Attach point cloud", "POINTCLOUDATTACH", "Choose a LAS/LAZ/COPC point cloud file."),
            ("Attach folder", "POINTCLOUDATTACHFOLDER", "Choose a folder of point clouds to attach."),
            ("Cloud manager", "POINTCLOUDMANAGER", "Manage attached sources, visibility, classification and saved sections."),
            ("Classify ground", "POINTCLOUDGROUND", "Classify ground points in the active cloud before generating a bare-earth surface."),
            ("Classify urban scene", "POINTCLOUDURBANCLASSIFY CURRENT", "Run the bundled classification worker on the current dataset."),
            ("Classify a folder", "POINTCLOUDURBANCLASSIFYFOLDER", "Choose input and output folders for batch classification."),
            ("Select point", "POINTCLOUDSELECTPOINT", "Click a cloud point to select it for classification."),
            ("Select box", "POINTCLOUDSELECTBOX", "Pick a box around cloud points to select them."),
            ("Select with brush", "POINTCLOUDSELECTBRUSH", "Paint a selection over the point cloud."),
            ("Clear cloud selection", "POINTCLOUDSELECTCLEAR", "Clear selected cloud points."),
            ("Draw section", "POINTCLOUDSECTION", "Pick a section line through the cloud."),
            ("Open section view", "POINTCLOUDSECTIONVIEW", "Inspect the active section in a fixed world-space view."),
            ("Clear section", "POINTCLOUDSECTIONCLEAR", "Remove the active section clipping band."),
            ("Validate breaklines", "POINTCLOUDBREAKLINECHECK", "Select 3D polylines first. Check and register terrain breaklines before surface draping."),
            ("Export cloud", "POINTCLOUDEXPORT", "Choose an output LAS/LAZ file for the active cloud."),
            ("Export merged clouds", "POINTCLOUDEXPORTALL", "Choose an output file for all attached sources."),
            ("Export 3D Tiles", "POINTCLOUD3DTILES", "Export a local point tileset. This interchange format is experimental."),
            ("Build display detail", "POINTCLOUDINDEX", "Build the multiresolution display cache for the cloud."),
            ("Index progress", "POINTCLOUDINDEXSTATUS", "Show progress of display-cache generation."),
            ("Cancel indexing", "POINTCLOUDINDEXCANCEL", "Cancel display-cache generation."),
            ("Export progress", "POINTCLOUDEXPORTSTATUS", "Show the active export job status."),
            ("Cancel export", "POINTCLOUDEXPORTCANCEL", "Cancel the current point-cloud export."),
            ("Undo cloud edit", "POINTCLOUDUNDO", "Undo the latest point-cloud classification edit."),
            ("Cloud statistics", "POINTCLOUDSTATS", "Report point counts and classification statistics."),
        ] { add("LiDAR", name, command, description, vec![]); }
        for (name, cmd, help) in [
            ("Export bare-earth elevation (DTM)", "POINTCLOUDDTM", "Save a raster terrain surface using ground points. Classify ground first. Cell size is in drawing units."),
            ("Export top-surface elevation (DSM)", "POINTCLOUDDSM", "Save a raster surface including above-ground features. Cell size is in drawing units."),
            ("Export hillshade", "POINTCLOUDHILLSHADE", "Save a shaded relief raster from the cloud surface. Cell size is in drawing units."),
        ] { add("LiDAR", name, cmd, help, vec![positive("Cell size (drawing units)", "1")]); }
        add(
            "LiDAR",
            "Generate contours",
            "POINTCLOUDCONTOUR",
            "Create contour geometry from the cloud surface at the selected elevation interval.",
            vec![positive("Contour interval (drawing units)", "1")],
        );
        add("LiDAR", "Drape objects onto cloud terrain", "POINTCLOUDDRAPE", "Select curves, polylines or meshes first. Creates draped copies; originals remain. ABORT stops at missing coverage; SKIP omits uncovered objects; CLAMP uses the nearest boundary. Uses global terrain if no cloud is attached.", vec![positive("Curve sample spacing", "1"), number("Vertical offset", "0", -1e9, 1e9, false), choice("Outside coverage", "ABORT", &["ABORT", "SKIP", "CLAMP"])]);
        add(
            "LiDAR",
            "Query surface elevation",
            "POINTCLOUDSURFACEAT",
            "Inspect surface elevation and vertical distance at drawing coordinates.",
            vec![
                number("X", "", -1e12, 1e12, false),
                number("Y", "", -1e12, 1e12, false),
                number("Reference Z", "0", -1e12, 1e12, false),
            ],
        );
        add(
            "LiDAR",
            "Section width",
            "POINTCLOUDSECTIONWIDTH",
            "Set the active section band's width in drawing units.",
            vec![positive("Band width", "10")],
        );
        add(
            "LiDAR",
            "Move section",
            "POINTCLOUDSECTIONMOVE",
            "Move the active section by a signed distance in drawing units.",
            vec![number("Distance (+ or −)", "1", -1e9, 1e9, false)],
        );
        add(
            "LiDAR",
            "Cloud color",
            "POINTCLOUDCOLOR",
            "Choose how cloud points are colored.",
            vec![choice(
                "Color by",
                "CLASS",
                &["CLASS", "RGB", "INTENSITY", "ELEVATION", "RETURN", "SOURCE"],
            )],
        );
        add(
            "LiDAR",
            "Point size",
            "POINTCLOUDPOINTSIZE",
            "Set the displayed point diameter in pixels.",
            vec![number("Pixels", "3", 1., 10., true)],
        );
        add(
            "LiDAR",
            "Display budget",
            "POINTCLOUDBUDGET",
            "Choose a point-cloud display memory budget suited to your computer.",
            vec![choice("Budget", "AUTO", &["AUTO", "LOW", "MEDIUM", "HIGH"])],
        );
        add("LiDAR", "Assign selected point class", "POINTCLOUDCLASSIFYSELECTION", "Select cloud points first. Common classes: 1 unclassified, 2 ground, 5 high vegetation, 6 building, 7 noise, 9 water, 18 high noise.", vec![number("Class number", "2", 0., 255., true)]);
        add(
            "LiDAR",
            "Classify isolated noise",
            "POINTCLOUDNOISE",
            "Classify points with too few neighbors inside the chosen radius.",
            vec![
                positive("Search radius", "2"),
                number("Minimum neighbors", "4", 1., 100000., true),
                number("Noise class", "7", 0., 255., true),
            ],
        );
        for (name, cmd, help) in [
            (
                "New spatial project",
                "SPATIALPROJECTNEW",
                "Create a spatial project for source references and workflow data.",
            ),
            (
                "Open spatial project",
                "SPATIALPROJECTOPEN",
                "Choose an existing spatial project.",
            ),
            (
                "Save spatial project",
                "SPATIALPROJECTSAVE",
                "Save spatial project references and metadata.",
            ),
            (
                "Import GIS layers",
                "GISIMPORT",
                "Choose a GeoPackage or GeoJSON file.",
            ),
            (
                "List GIS layers",
                "GISLAYERS",
                "List native GIS layers in the active project.",
            ),
            (
                "Check GIS topology",
                "GISTOPOLOGY",
                "Check native feature geometry for topology problems.",
            ),
            (
                "Workflows and standards",
                "WORKFLOWSTANDARDS",
                "Open the workflow and company standards editor. Advanced definitions use JSON.",
            ),
        ] {
            add("GIS & projects", name, cmd, help, vec![]);
        }
        add("GIS & projects", "Export GIS layer", "GISEXPORT", "Export a named native layer. Use its layer name from List GIS layers and a .gpkg or .geojson output path.", vec![input("Layer name"), input("Output file path")]);
        add("GIS & projects", "Transform GIS layer", "GISTRANSFORM", "Transform a native layer to another coordinate system. Use its layer name from List GIS layers.", vec![input("Layer name"), number("Target EPSG", "", 1., 65535., true)]);
    }
    for (name, cmd, help) in [
        ("Box", "BOX", "Pick the base corners and height to create a solid box."),
        ("Cylinder", "CYLINDER", "Pick the base center, radius and height."),
        ("Cone", "CONE", "Pick the base center, radius and height."),
        ("Sphere", "SPHERE", "Pick a center and radius."),
        ("Pyramid", "PYRAMID", "Create a pyramid using the guided viewport prompts."),
        ("Wall / polysolid", "POLYSOLID", "Draw a wall-like solid with width and height."),
        ("Extrude profile", "EXTRUDE", "Select planar profiles and specify extrusion height."),
        ("Revolve profile", "REVOLVE", "Revolve a profile around a picked axis."),
        ("Sweep along path", "SWEEP", "Sweep a profile along a path."),
        ("Loft profiles", "LOFT", "Create a solid through successive profiles."),
        ("Union solids", "UNION", "Combine selected solids into one solid."),
        ("Subtract solids", "SUBTRACT", "Select the solids to keep, then the solids to subtract."),
        ("Intersect solids", "INTERSECT", "Keep the overlapping volume of selected solids."),
        ("Move in 3D", "3DMOVE", "Move selected objects between picked 3D points."),
        ("Rotate in 3D", "3DROTATE", "Rotate objects about a picked 3D axis."),
        ("Mirror in 3D", "3DMIRROR", "Reflect objects through a picked plane."),
        ("Align in 3D", "3DALIGN", "Align objects using source and destination points."),
        ("Slice solid", "SLICE", "Cut selected solids with a plane using the viewport prompts."),
        ("Cross-section", "SECTION", "Create a planar section of selected geometry."),
        ("Check interference", "INTERFERE", "Find overlapping volumes between solids."),
        ("Press / pull", "PRESSPULL", "Push or pull a closed boundary to create 3D geometry."),
        ("Thicken surface", "THICKEN", "Add thickness to supported surfaces using the guided prompts."),
        ("Flatten geometry", "FLATTEN", "Project selected objects onto the working plane. This changes the selected geometry; Undo restores it."),
        ("Diagnose mesh", "MESHDIAGNOSE", "Select meshes first. Report topology, open boundaries and face indices before repair."),
        ("Flip mesh normals", "MESHFLIP", "Reverse all face directions in selected meshes. Undo restores the originals."),
        ("Orient mesh faces", "MESHORIENT", "Make face orientation consistent in selected meshes."),
        ("Mass properties", "MASSPROP", "Report mass properties for supported selected solids."),
        ("Export STL", "EXPORTSTL", "Export mesh geometry to an STL file."),
        ("Export STEP", "EXPORTSTEP", "Export mesh geometry to a STEP file."),
        ("Export 3MF", "EXPORT3MF", "Export mesh geometry to a 3MF file."),
    ] { add("Model", name, cmd, help, vec![]); }
    add("Model", "Weld mesh vertices", "MESHWELD", "Merge coincident or nearby vertices in selected meshes. Tolerance is in drawing units; zero merges exact duplicates.", vec![number("Weld tolerance", "0.000001", 0., 1e6, false)]);
    add("Model", "Fill mesh holes", "MESHFILL", "Fill boundary loops in selected meshes up to the specified number of edges. Diagnose the mesh first.", vec![number("Maximum boundary edges", "64", 3., 1000000., true)]);
    add("Model", "Delete mesh faces", "MESHDELETEFACE", "Select a mesh and diagnose it first. Enter space-separated face indices starting at 0. Undo restores removed faces.", vec![input("Face indices (for example 0 1 2)")]);
    tools
}

pub fn help_for(command: &str) -> Option<String> {
    let command = command.strip_prefix("TOOLCENTER ").unwrap_or(command);
    match command {
        "MODEL" => {
            return Some(
                "Browse all modeling tools, mesh repair settings and 3D editing operations.".into(),
            )
        }
        "LIDAR" => {
            return Some(
                "Browse point-cloud surfaces, classification, sections, display and export tools."
                    .into(),
            )
        }
        "TERRAIN" => {
            return Some("Find maps, drawing coordinates and terrain elevation tools.".into())
        }
        "GIS" => return Some("Import native feature layers and manage spatial projects.".into()),
        "TERRAIN INFO" => {
            return Some(
                "Pick a point in the drawing to inspect elevation on the loaded terrain.".into(),
            )
        }
        _ => {}
    }
    static HELP: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    HELP.get_or_init(curated)
        .iter()
        .find(|tool| tool.command == command)
        .map(|tool| tool.description.clone())
}

pub fn catalog() -> Vec<Tool> {
    let mut tools = curated();
    let guided: std::collections::HashSet<String> =
        tools.iter().map(|tool| tool.command.clone()).collect();
    for module in crate::modules::registry::all_modules() {
        for group in module.ribbon_groups() {
            let category = match module.id() {
                "model" => "Model",
                "lidar" => "LiDAR",
                "terrain" => "Terrain",
                _ => module.title(),
            };
            let mut add = |command: &str, label: &str| {
                if command.starts_with("TOOLCENTER")
                    || tools.iter().any(|tool| tool.command == command)
                {
                    return;
                }
                tools.push(Tool { name: label.replace('\n', " "), category: category.into(), command: command.into(), description: format!("{} > {}. Start this tool, then follow the prompts near the cursor or above the command entry.", module.title(), group.title), fields: vec![] });
            };
            let command_tool = |tool: &ToolDef| match &tool.event {
                ModuleEvent::Command(cmd) => Some((cmd.clone(), tool.label)),
                _ => None,
            };
            for item in &group.tools {
                match item {
                    RibbonItem::Tool(t)
                    | RibbonItem::LargeTool(t)
                    | RibbonItem::PropertiesGroup { match_prop: t } => {
                        if let Some((cmd, label)) = command_tool(t) {
                            add(&cmd, label);
                        }
                    }
                    RibbonItem::Dropdown { items, .. }
                    | RibbonItem::LargeDropdown { items, .. } => {
                        for (cmd, label, _) in items {
                            add(cmd, label);
                        }
                    }
                    RibbonItem::LayerComboGroup { row2, row3 } => {
                        for t in row2.iter().chain(row3) {
                            if let Some((cmd, label)) = command_tool(t) {
                                add(&cmd, label);
                            }
                        }
                    }
                    RibbonItem::StyleComboGroup {
                        manager_cmd, rows, ..
                    } => {
                        if let Some(cmd) = manager_cmd {
                            add(cmd, "Style manager");
                        }
                        for t in rows.iter().flatten() {
                            if let Some((cmd, label)) = command_tool(t) {
                                add(&cmd, label);
                            }
                        }
                    }
                }
            }
        }
    }
    for name in crate::command::all_registered_command_names() {
        if !tools.iter().any(|tool| tool.command == name) {
            tools.push(Tool { name: name.into(), category: "Advanced commands".into(), command: name.into(), description: "Registered command or alias. Start it to see its prompt or usage in the command history. Search the other categories for guided tools with labeled settings.".into(), fields: vec![] });
        }
    }
    tools.sort_by_key(|tool| (!guided.contains(&tool.command), tool.name.to_lowercase()));
    tools
}

pub fn view<'a>(
    state: &'a ToolCenter,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    let mut categories = vec!["All tools".to_string()];
    categories.extend(state.tools.iter().map(|t| t.category.clone()));
    categories.sort();
    categories.dedup();
    let mut list = column![].spacing(4);
    let mut count = 0;
    for (index, tool) in state
        .tools
        .iter()
        .enumerate()
        .filter(|(_, t)| state.matches(t))
    {
        count += 1;
        let selected = state.selected == Some(index);
        let item =
            button(column![text(&tool.name).size(13), text(&tool.category).size(10)].spacing(3))
                .width(Fill)
                .padding(9)
                .on_press(Message::ToolCenterSelect(index))
                .style(if selected {
                    button::primary
                } else {
                    button::text
                });
        list = list.push(tooltip(
            item,
            text(tool.description.clone()).size(12).width(300),
            tooltip::Position::Right,
        ));
    }
    if count == 0 {
        list = list.push(
            text("No matching tools. Try 'surface', 'mesh', 'section' or another category.")
                .size(13),
        );
    }
    let mut detail =
        column![text("Choose a tool to see what it does and its settings.").size(14)].spacing(12);
    if let Some(tool) = state.selected.and_then(|index| state.tools.get(index)) {
        detail = column![text(&tool.name).size(22), text(&tool.description).size(13)].spacing(12);
        for (index, (field, value)) in tool.fields.iter().zip(&state.values).enumerate() {
            let editor: Element<'a, Message> = match &field.kind {
                FieldKind::Choice(options) => {
                    pick_list(Some(value.as_str()), options.to_vec(), |s| s.to_string())
                        .on_select(move |value| Message::ToolCenterValue(index, value.to_string()))
                        .width(Fill)
                        .into()
                }
                _ => text_input(field.default, value)
                    .on_input(move |v| Message::ToolCenterValue(index, v))
                    .padding(8)
                    .into(),
            };
            detail = detail.push(column![text(field.label).size(12), editor].spacing(4));
        }
        let command = tool.command_with_values(&state.values);
        if let Err(error) = &command {
            detail = detail.push(text(error.clone()).size(12));
        }
        detail = detail.push(
            button("Start tool")
                .padding([10, 18])
                .on_press_maybe(command.ok().map(Message::ToolCenterRun)),
        );
        detail = detail.push(text(format!("Command: {}", tool.command)).size(11));
        detail = detail.push(text("This window closes when the tool starts so you can pick objects in the drawing. Results and next steps appear in the command history.").size(12));
    }
    column![
        row![
            text_input(
                "Search by task: drape, terrain, repair, section…",
                &state.search
            )
            .id(iced::widget::Id::new("tool-center-search"))
            .on_input(Message::ToolCenterSearch)
            .padding(9)
            .width(Fill),
            pick_list(Some(state.category.clone()), categories, |s| s.clone())
                .on_select(Message::ToolCenterCategory)
        ]
        .spacing(10),
        text(format!(
            "{count} tools • Select objects before opening tools that operate on a selection."
        ))
        .size(12),
        row![
            scrollable(list)
                .width(iced::Length::FillPortion(2))
                .height(Fill),
            scrollable(container(detail).padding(12))
                .width(iced::Length::FillPortion(3))
                .height(Fill)
        ]
        .spacing(12),
        row![
            text("Esc closes this window.").size(11),
            iced::widget::Space::new().width(Fill),
            button("Close").on_press(Message::CloseModal)
        ]
        .spacing(10)
    ]
    .spacing(10)
    .padding(16)
    .width(sizing.width)
    .height(sizing.height)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_registered_command_has_a_searchable_home() {
        let catalog = catalog();
        for name in crate::command::all_registered_command_names() {
            assert!(catalog.iter().any(|t| t.command == name), "Missing {name}");
        }
        let unique: std::collections::HashSet<_> = catalog.iter().map(|t| &t.command).collect();
        assert_eq!(unique.len(), catalog.len());
    }
    #[test]
    fn forms_reject_nonfinite_out_of_range_and_injected_values() {
        let tools = curated();
        let weld = tools.iter().find(|t| t.command == "MESHWELD").unwrap();
        for bad in ["NaN", "inf", "-1", "1\nERASE"] {
            assert!(weld.command_with_values(&[bad.into()]).is_err());
        }
        assert_eq!(
            weld.command_with_values(&["0.01".into()]).unwrap(),
            "MESHWELD 0.01"
        );
        let fill = tools.iter().find(|t| t.command == "MESHFILL").unwrap();
        assert!(fill.command_with_values(&["3.5".into()]).is_err());
    }
    #[test]
    fn natural_language_search_finds_hidden_surface_tools() {
        let mut state = ToolCenter::default();
        state.search = "bare earth".into();
        assert!(state.tools.iter().any(|t| state.matches(t)));
    }
}
