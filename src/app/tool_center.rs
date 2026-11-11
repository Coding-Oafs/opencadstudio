use super::{Message, ModalKind, OpenCADStudio};
use iced::Task;

impl OpenCADStudio {
    pub(super) fn open_tool_center(&mut self, category: &str) -> Task<Message> {
        self.tool_center.category = match category.to_ascii_uppercase().as_str() {
            "LIDAR" => "LiDAR",
            "MODEL" => "Model",
            "TERRAIN" => "Terrain",
            "GIS" => "GIS & projects",
            _ => "All tools",
        }
        .into();
        self.tool_center.search.clear();
        self.tool_center.selected = None;
        // Refresh dynamic plugin commands when opening; do not leave new tools
        // stranded in autocomplete after a plugin is enabled.
        self.tool_center.tools = crate::ui::window::tool_center::catalog();
        for name in &self.command_line.dynamic_commands {
            if !self.tool_center.tools.iter().any(|t| t.command == *name) {
                self.tool_center
                    .tools
                    .push(crate::ui::window::tool_center::Tool {
                        name: name.clone(),
                        category: "Plugins".into(),
                        command: name.clone(),
                        description: "Tool from a loaded plugin. Start it and follow its prompts."
                            .into(),
                        fields: vec![],
                    });
            }
        }
        if let Some(index) = self
            .tool_center
            .tools
            .iter()
            .position(|t| t.command.eq_ignore_ascii_case(category))
        {
            self.tool_center.category = self.tool_center.tools[index].category.clone();
            self.tool_center.select(index);
        }
        self.ribbon.close_dropdown();
        self.active_modal = Some(ModalKind::ToolCenter);
        self.reset_modal_geometry();
        iced::widget::operation::focus(iced::widget::Id::new("tool-center-search"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn open_terrain_panel(&mut self) -> Task<Message> {
        if self.tabs[self.active_tab].is_start {
            self.command_line
                .push_info("Create or open a drawing, then open Terrain & elevation.");
            return Task::none();
        }
        let tab_id = self.tabs[self.active_tab].id;
        if self.terrain_panel.tab_id != Some(tab_id) {
            self.terrain_panel = Default::default();
            self.terrain_panel.tab_id = Some(tab_id);
        }
        if let Some(crs) = &self.tabs[self.active_tab].spatial.drawing_crs {
            self.terrain_panel.values[3] =
                crs.epsg.map(|epsg| epsg.to_string()).unwrap_or_default();
        }
        self.terrain_panel.zoom = self.tabs[self.active_tab]
            .terrain_request
            .zoom
            .unwrap_or(12);
        self.terrain_panel.values[6] = self.tabs[self.active_tab]
            .terrain_request
            .mesh_max_vertices
            .to_string();
        self.terrain_panel.error.clear();
        self.ribbon.close_dropdown();
        self.active_modal = Some(ModalKind::TerrainPanel);
        self.reset_modal_geometry();
        Task::none()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn terrain_panel_data(&self) -> crate::ui::window::terrain_panel::TerrainView {
        let tab = &self.tabs[self.active_tab];
        crate::ui::window::terrain_panel::TerrainView {
            crs: format!("Current: {} • Working units: {}", tab.spatial.drawing_crs.as_ref().map(|c| c.label()).unwrap_or_else(|| "not set".into()), tab.spatial.working_unit.label()),
            has_crs: tab.spatial.drawing_crs.is_some(),
            has_model: tab.terrain.is_some(),
            selected: tab.scene.selected_entities().len(),
            model: tab.terrain.as_ref().map(|t| format!("{} • {} • zoom {} • {} tiles ({} failed) • elevations in {}", t.source, t.crs_label, t.zoom, t.tiles, t.failed_tiles, t.elevation_unit)).unwrap_or_else(|| "No elevation loaded. Download a site or drawing area above.".into()),
            extent: self.tabs[self.active_tab].terrain_request.bounds.map(|b| format!("Current download bounds: {:.2}, {:.2} to {:.2}, {:.2}", b[0], b[1], b[2], b[3])).unwrap_or_else(|| "Drawing area uses the map extent, or the drawing geometry if no map extent is set.".into()),
            progress: self.terrain_job.as_ref().map(|job| job.snapshot()),
            feedback: self.command_line.history.last().map(|entry| entry.text.clone()).unwrap_or_default(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn terrain_panel_action(
        &mut self,
        action: crate::ui::window::terrain_panel::TerrainAction,
    ) -> Task<Message> {
        use crate::ui::window::terrain_panel::TerrainAction;
        let command = match self.terrain_panel.command(&action) {
            Ok(command) => command,
            Err(error) => {
                self.terrain_panel.error = error;
                return Task::none();
            }
        };
        self.terrain_panel.error.clear();
        if matches!(
            action,
            TerrainAction::FetchSite | TerrainAction::FetchExtent
        ) {
            let mesh_command = match self.terrain_panel.command(&TerrainAction::Mesh) {
                Ok(command) => command,
                Err(error) => {
                    self.terrain_panel.error = error;
                    return Task::none();
                }
            };
            self.tabs[self.active_tab].terrain_request.mesh_max_vertices = mesh_command
                .split_whitespace()
                .last()
                .unwrap()
                .parse()
                .unwrap();
            self.tabs[self.active_tab].terrain_request.zoom = Some(self.terrain_panel.zoom);
            self.tabs[self.active_tab].terrain_request.cell_size = None;
        }
        if matches!(
            action,
            TerrainAction::PickBounds | TerrainAction::Inspect | TerrainAction::Drape
        ) {
            self.active_modal = None;
            self.command_line.history_open = true;
        }
        self.dispatch_command(&command)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::modules::ModuleEvent;
    use crate::ui::window::terrain_panel::TerrainAction;

    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app
    }

    #[test]
    fn tool_center_ribbon_preserves_selection_and_opens_the_requested_form() {
        let mut app = app();
        app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 10,0"}"#);
        app.automation_op(r#"{"op":"select","type":"Line"}"#);
        let _ = app.update(Message::RibbonToolClick {
            tool_id: "LIDAR_DRAPE_SETTINGS".into(),
            event: ModuleEvent::Command("TOOLCENTER POINTCLOUDDRAPE".into()),
        });
        assert!(app.active_modal == Some(ModalKind::ToolCenter));
        let tool = &app.tool_center.tools[app.tool_center.selected.unwrap()];
        assert_eq!(tool.command, "POINTCLOUDDRAPE");
        assert_eq!(app.tool_center.values, ["1", "0", "ABORT"]);
        assert_eq!(app.tabs[app.active_tab].scene.selected_entities().len(), 1);
    }

    #[test]
    fn terrain_panel_blocks_bad_input_and_hands_point_picking_to_viewport() {
        let mut app = app();
        let _ = app.update(Message::TerrainPanelOpen);
        let _ = app.update(Message::TerrainPanelAction(TerrainAction::FetchSite));
        assert!(!app.terrain_panel.error.is_empty());
        assert!(app.terrain_job.is_none());
        assert!(app.active_modal == Some(ModalKind::TerrainPanel));
        let _ = app.update(Message::TerrainPanelAction(TerrainAction::PickBounds));
        assert!(app.active_modal.is_none());
        assert_eq!(
            app.tabs[app.active_tab].active_cmd.as_ref().unwrap().name(),
            "TERRAIN BOUNDS"
        );
    }

    #[test]
    fn terrain_settings_and_site_inputs_do_not_leak_between_drawings() {
        let mut app = app();
        let _ = app.dispatch_command("TERRAIN BOUNDS 1 2 3 4");
        let _ = app.dispatch_command("TERRAIN ZOOM 15");
        let _ = app.open_terrain_panel();
        app.terrain_panel.values[0] = "-71".into();
        let original = app.active_tab;
        app.tabs
            .push(super::super::document::DocumentTab::new_drawing(500));
        app.active_tab = app.tabs.len() - 1;
        let _ = app.open_terrain_panel();
        assert!(app.tabs[app.active_tab].terrain_request.bounds.is_none());
        assert!(app.tabs[app.active_tab].terrain_request.zoom.is_none());
        assert!(app.terrain_panel.values[0].is_empty());
        assert_eq!(
            app.tabs[original].terrain_request.bounds,
            Some([1., 2., 3., 4.])
        );
    }

    #[test]
    fn changing_coordinates_invalidates_terrain_extent() {
        let mut app = app();
        let _ = app.dispatch_command("TERRAIN BOUNDS 1 2 3 4");
        let _ = app.drawing_crs_command("32619");
        assert!(app.tabs[app.active_tab].terrain_request.bounds.is_none());
        let _ = app.dispatch_command("TERRAIN BOUNDS 10 20 30 40");
        let _ = app.drawing_crs_command("32619");
        assert_eq!(
            app.tabs[app.active_tab].terrain_request.bounds,
            Some([10., 20., 30., 40.])
        );
    }

    #[test]
    fn model_form_reaches_backend_and_keeps_error_visible() {
        let mut app = app();
        let _ = app.open_tool_center("MESHWELD");
        let tool = &app.tool_center.tools[app.tool_center.selected.unwrap()];
        let command = tool.command_with_values(&["0.01".into()]).unwrap();
        let _ = app.update(Message::ToolCenterRun(command));
        assert!(app.active_modal.is_none());
        assert!(app.command_line.history_open);
        assert!(app
            .command_line
            .history
            .iter()
            .any(|entry| entry.text.contains("MESHWELD")));
        assert!(!app
            .command_line
            .history
            .iter()
            .any(|entry| entry.text.contains("Unknown command")));
    }

    #[test]
    fn terrain_drape_button_creates_geometry_and_supports_undo() {
        let mut app = app();
        app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 1,1"}"#);
        app.automation_op(r#"{"op":"select","type":"Line"}"#);
        let surface = ocs_pointcloud::RasterSurface {
            origin: [0., 0.],
            cell_size: 1.,
            columns: 3,
            rows: 3,
            values: vec![10.; 9],
            no_data: -9999.,
            counts: vec![1; 9],
        };
        app.tabs[app.active_tab].terrain = Some(std::sync::Arc::new(super::super::terrain::TerrainDataset {
            surface,
            zoom: 12,
            tiles: 1,
            failed_tiles: 0,
            crs_label: "test".into(),
            elevation_unit: "meters".into(),
            source: "test fixture".into(),
            cache_key: None,
        }));
        let before = app.tabs[app.active_tab].scene.document.entities().count();
        let _ = app.open_terrain_panel();
        let _ = app.update(Message::TerrainPanelAction(TerrainAction::Drape));
        assert!(app.active_modal.is_none());
        assert_eq!(
            app.tabs[app.active_tab].scene.document.entities().count(),
            before + 1
        );
        assert_eq!(app.automation_op(r#"{"op":"undo"}"#)["total"], before);
    }
}
