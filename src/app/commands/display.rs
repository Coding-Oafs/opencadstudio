use super::*;

impl OpenCADStudio {
    pub(super) fn dispatch_display(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        match cmd {
            // Interactive pan: left-drag pans the view until Esc. The only pan
            // path when there is no middle mouse button (trackpad / web).
            "PAN" => {
                self.tabs[i].pan_mode = true;
                self.clear_navigation_hover(i);
                self.command_line.push_output(
                    crate::t!("PAN: drag with the left mouse button. Press Esc to exit.").as_ref(),
                );
            }

            // ArcGIS-style combined navigation: left-drag pans, Shift+left-
            // drag uses the existing orbit path, and the wheel keeps zooming.
            "NAVIGATOR" => {
                self.tabs[i].pan_mode = true;
                self.clear_navigation_hover(i);
                self.command_line.push_output(
                    "Navigator: drag to pan, Shift+drag to orbit, and use the wheel to zoom. Press Esc to exit.",
                );
            }

            // Dedicated selection affordance. Normal viewport selection code
            // is already the default; arming it explicitly exits navigation
            // and keeps the ribbon tool highlighted.
            "SELECTTOOL" => {
                self.tabs[i].selection_tool_mode = true;
                self.clear_navigation_hover(i);
                self.command_line.push_output(
                    "Selection tool active: click, window, crossing, or lasso to select features.",
                );
            }

            // ── TABLE cell editing ─────────────────────────────────────────────
            // TABLE CELL <row> <col> <text> — set text for a cell in the selected Table
            cmd if cmd.starts_with("TABLE ") => {
                let rest = cmd.trim_start_matches("TABLE").trim();
                let sub_up = rest.split_whitespace().next().unwrap_or("").to_uppercase();
                if sub_up == "CELL" {
                    let parts: Vec<&str> = rest.splitn(4, char::is_whitespace).collect();
                    // parts: ["CELL", "<row>", "<col>", "<text>"]
                    let row_res = parts.get(1).and_then(|s| s.parse::<usize>().ok());
                    let col_res = parts.get(2).and_then(|s| s.parse::<usize>().ok());
                    let text = parts.get(3).copied().unwrap_or("");
                    match (row_res, col_res) {
                        (Some(row), Some(col)) => {
                            let selected_handles: Vec<acadrust::Handle> = self.tabs[i]
                                .scene
                                .selected_entities()
                                .iter()
                                .map(|(h, _)| *h)
                                .collect();
                            let mut found = false;
                            for sh in &selected_handles {
                                if let Some(acadrust::EntityType::Table(tbl)) = self.tabs[i]
                                    .scene
                                    .document
                                    .entities_mut()
                                    .find(|e| e.common().handle == *sh)
                                {
                                    if tbl.set_cell_text(row, col, text) {
                                        found = true;
                                    }
                                }
                            }
                            if found {
                                self.push_undo_snapshot(i, "TABLE CELL");
                                self.tabs[i].dirty = true;
                                self.command_line.push_output(
                                    crate::tf!("TABLE CELL: set [{row},{col}] = \"{text}\".")
                                        .as_ref(),
                                );
                            } else {
                                self.command_line.push_error(
                                    crate::t!("TABLE CELL: select a Table entity first, or row/col out of range.").as_ref()
                                );
                            }
                        }
                        _ => {
                            self.command_line.push_info(
                                crate::t!("Usage: TABLE CELL <row> <col> <text>").as_ref(),
                            );
                        }
                    }
                } else {
                    self.command_line.push_info(
                        "Usage: TABLE  (creates new table)  or  TABLE CELL <row> <col> <text>",
                    );
                }
            }

            // ── UCSICON — toggle UCS icon visibility on all viewports ────────────
            // UCSICON ON       — show UCS icon in all viewports
            // UCSICON OFF      — hide UCS icon in all viewports
            // UCSICON NOORIGIN — show icon but not at origin (show at corner)
            // UCSICON ORIGIN   — show icon at UCS origin
            "UCSICON" => {
                use crate::command::KeywordCommand;
                let c = KeywordCommand::new(
                    "UCSICON",
                    "UCSICON  [On / Off / NoOrigin / Origin]:",
                    vec![
                        ("On", "ON", None),
                        ("Off", "OFF", None),
                        ("NoOrigin", "NOORIGIN", None),
                        ("Origin", "ORIGIN", None),
                    ],
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("UCSICON ") => {
                let sub = cmd.split_whitespace().nth(1).unwrap_or("").to_uppercase();
                match sub.as_str() {
                    "ON" | "OFF" | "NOORIGIN" | "ORIGIN" => {
                        self.push_undo_snapshot(i, "UCSICON");
                        let visible = sub != "OFF";
                        let at_origin = sub == "ORIGIN";
                        // Update model-space icon flags.
                        self.show_ucs_icon = visible;
                        self.ribbon.set_ucs_icon(visible);
                        if sub == "NOORIGIN" || sub == "ORIGIN" {
                            self.ucs_icon_at_origin = at_origin;
                        }
                        let mut count = 0usize;
                        for entity in self.tabs[i].scene.document.entities_mut() {
                            if let acadrust::EntityType::Viewport(vp) = entity {
                                vp.status.ucs_icon_visible = visible;
                                if sub == "NOORIGIN" || sub == "ORIGIN" {
                                    vp.status.ucs_icon_at_origin = at_origin;
                                }
                                count += 1;
                            }
                        }
                        self.tabs[i].dirty = true;
                        self.command_line.push_output(
                            crate::tf!("UCSICON {sub}: updated {count} viewport(s) + model space.")
                                .as_ref(),
                        );
                    }
                    "" => {
                        // Bare UCSICON toggles visibility.
                        self.push_undo_snapshot(i, "UCSICON");
                        let visible = !self.show_ucs_icon;
                        self.show_ucs_icon = visible;
                        self.ribbon.set_ucs_icon(visible);
                        for entity in self.tabs[i].scene.document.entities_mut() {
                            if let acadrust::EntityType::Viewport(vp) = entity {
                                vp.status.ucs_icon_visible = visible;
                            }
                        }
                        self.tabs[i].dirty = true;
                        let state = if visible { "ON" } else { "OFF" };
                        self.command_line
                            .push_output(crate::tf!("UCSICON {state}").as_ref());
                    }
                    _ => {
                        self.command_line.push_info(
                            crate::t!("Usage: UCSICON ON | OFF | NOORIGIN | ORIGIN").as_ref(),
                        );
                    }
                }
            }

            // ── NAVVCUBE — toggle ViewCube visibility ────────────────────────────
            "NAVVCUBE" => {
                return Some(Task::done(Message::ToggleViewCube));
            }

            // ── LIMITS — drawing/grid boundary for the active space ─────────────
            "LIMITS" => {
                use crate::modules::view::limits::LimitsCommand;
                let (min, max) = self.tabs[i]
                    .scene
                    .current_drawing_limits()
                    .unwrap_or((glam::DVec2::ZERO, glam::DVec2::new(12.0, 9.0)));
                let command = LimitsCommand::new(min, max);
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }
            "LIMITS ON" | "LIMITS OFF" => {
                let enabled = cmd.ends_with("ON");
                if self.tabs[i].scene.drawing_limit_check_enabled() != enabled {
                    self.push_undo_snapshot(i, "LIMITS");
                    self.tabs[i].scene.set_drawing_limit_check(enabled);
                    self.tabs[i].dirty = true;
                }
                self.command_line.push_output(if enabled {
                    "Limits checking ON."
                } else {
                    "Limits checking OFF."
                });
            }
            cmd if cmd.starts_with("LIMITS SET ") => {
                let tokens: Vec<&str> = cmd["LIMITS SET ".len()..].split_whitespace().collect();
                let values: Result<Vec<f64>, _> =
                    tokens.iter().map(|value| value.parse()).collect();
                let Ok(values) = values else {
                    self.command_line.push_error(
                        crate::t!("LIMITS: four numeric coordinates required.").as_ref(),
                    );
                    return Some(Task::none());
                };
                if tokens.len() != 4 || !values.iter().all(|value| value.is_finite()) {
                    self.command_line.push_error(
                        crate::t!("LIMITS: four finite numeric coordinates required.").as_ref(),
                    );
                } else {
                    let first = glam::DVec2::new(values[0], values[1]);
                    let opposite = glam::DVec2::new(values[2], values[3]);
                    let min = first.min(opposite);
                    let max = first.max(opposite);
                    if min.x == max.x || min.y == max.y {
                        self.command_line.push_error(
                            crate::t!("LIMITS: corners must define a non-zero area.").as_ref(),
                        );
                    } else {
                        self.push_undo_snapshot(i, "LIMITS");
                        self.tabs[i].scene.set_current_drawing_limits(min, max);
                        self.tabs[i].dirty = true;
                        self.command_line.push_output(
                            crate::tf!(
                                "Drawing limits: {:.4},{:.4} to {:.4},{:.4}.",
                                min.x,
                                min.y,
                                max.x,
                                max.y
                            )
                            .as_ref(),
                        );
                    }
                }
            }

            // ── PROPERTIES — toggle Properties panel visibility ──────────────────
            "PROPERTIES" | "PROPS" => {
                return Some(Task::done(Message::ToggleProperties));
            }

            // ── FILETAB — toggle file/document tabs ──────────────────────────────
            "FILETAB" => {
                return Some(Task::done(Message::ToggleFileTabs));
            }

            // ── LAYOUTTAB — toggle layout/paper-space tabs ───────────────────────
            "LAYOUTTAB" => {
                return Some(Task::done(Message::ToggleLayoutTabs));
            }

            // ── REDRAW / REGEN ──────────────────────────────────────────────
            // REDRAW — force a full re-rasterize of the current viewport next
            // frame, WITHOUT touching the DB (never bumps geometry_epoch /
            // block_epoch, never pushes undo). Scope: Active. This arm does not
            // itself clear previews or cancel commands (the normal non-transparent
            // dispatch teardown, commands/mod.rs:90-96, governs that separately);
            // it only queues a per-viewport cache invalidation.
            "REDRAW" => {
                use crate::scene::ViewportRefreshScope;
                self.tabs[i]
                    .scene
                    .request_refresh(ViewportRefreshScope::Active);
                self.command_line.push_output("REDRAW: viewport refreshed.");
                return Some(Task::none());
            }
            // REDRAWALL — force re-rasterize of every generated viewport.
            "REDRAWALL" => {
                use crate::scene::ViewportRefreshScope;
                self.tabs[i]
                    .scene
                    .request_refresh(ViewportRefreshScope::All);
                self.command_line
                    .push_output("REDRAWALL: viewports refreshed.");
                return Some(Task::none());
            }
            // REGEN — full model regeneration (bump_geometry: geometry_epoch AND
            // block_epoch; C4). No undo, no DB mutation, so do NOT touch
            // self.tabs[i].dirty — a newly opened drawing must not become
            // "modified" merely because tessellation caches were invalidated (C7).
            // REGENALL is functionally identical (C5).
            "REGEN" | "REGENALL" => {
                self.tabs[i].scene.bump_geometry();
                self.command_line.push_output("REGEN: regenerated model.");
                return Some(Task::none());
            }

            // ── Drafting aids — same toggles the status-bar pills drive, also
            //    reachable by name from the command line. ─────────────────────────
            // GRID — show / hide the reference grid.
            "GRID" => {
                return Some(Task::done(Message::ToggleGrid));
            }
            // SNAP — toggle cursor snapping to the grid.
            "SNAP" => {
                return Some(Task::done(Message::ToggleGridSnap));
            }
            // ISOPLANE — cycle the isometric drafting axis pair (F5).
            "ISOPLANE" => {
                return Some(Task::done(Message::CycleIsoPlane));
            }
            cmd if cmd.starts_with("ISOPLANE ") => {
                let plane = match cmd.trim_start_matches("ISOPLANE").trim() {
                    "LEFT" | "L" => Some(crate::app::settings::IsoPlane::Left),
                    "TOP" | "T" => Some(crate::app::settings::IsoPlane::Top),
                    "RIGHT" | "R" => Some(crate::app::settings::IsoPlane::Right),
                    _ => None,
                };
                if let Some(plane) = plane {
                    return Some(Task::done(Message::SetIsoPlane(plane)));
                }
                self.command_line
                    .push_error(crate::t!("ISOPLANE: expected Left, Top, or Right.").as_ref());
            }
            // ISODRAFT — enable or disable isometric drafting.
            "ISODRAFT" => {
                return Some(Task::done(Message::ToggleIsometricDrafting));
            }
            cmd if cmd.starts_with("ISODRAFT ") => {
                let requested = match cmd.trim_start_matches("ISODRAFT").trim() {
                    "1" | "ON" => Some(true),
                    "0" | "OFF" => Some(false),
                    _ => None,
                };
                match requested {
                    Some(value) if value != self.isometric_drafting => {
                        return Some(Task::done(Message::ToggleIsometricDrafting));
                    }
                    Some(_) => {}
                    None => self
                        .command_line
                        .push_error(crate::t!("ISODRAFT: expected On or Off.").as_ref()),
                }
            }
            // POLAR — toggle polar tracking.
            "POLAR" => {
                return Some(Task::done(Message::TogglePolar));
            }
            // DSETTINGS / OSNAP — open the drafting-settings popup, which is OCS's
            // settings surface (the persisted DYN/ORTHO/POLAR/OSNAP prefs).
            "DSETTINGS" | "OSNAP" => {
                return Some(Task::done(Message::ToggleSnapPopup));
            }
            // UNITS — length and angle formats, plus the insertion unit. The
            // status-bar button covers the length format alone; everything else
            // about how this drawing writes numbers is here.
            "UNITS" | "DDUNITS" => {
                return Some(Task::done(Message::OpenDrawingUnits));
            }

            // ── CLEANSCREEN — collapse the surrounding panels for a full canvas ──
            "CLEANSCREEN" => {
                return Some(Task::done(Message::ToggleCleanScreen));
            }
            // ── QUICKPROPERTIES — toggle the floating quick-properties readout ───
            "QUICKPROPERTIES" => {
                return Some(Task::done(Message::ToggleQuickProperties));
            }

            // ── TOOLPALETTES — toggle the docked tool palette panel ──────────────
            "TOOLPALETTES" | "TOOLPALETTE" => {
                self.show_tool_palettes = !self.show_tool_palettes;
                if self.show_tool_palettes && self.tool_palettes.palettes.is_empty() {
                    self.tool_palettes.palettes =
                        crate::ui::window::tool_palettes::default_palettes();
                }
                let message = if self.show_tool_palettes {
                    "Tool Palettes: shown."
                } else {
                    "Tool Palettes: hidden."
                };
                self.command_line.push_output(crate::t!(message).as_ref());
            }

            // ── SHEETSET — toggle the docked Sheet Set Manager ───────────────────
            "SHEETSET" | "SSM" => {
                self.show_sheetset = !self.show_sheetset;
                if self.show_sheetset && self.sheetset.set.is_none() {
                    self.sheetset.set = Some(crate::ui::window::sheetset::SheetSet {
                        name: "Sheet Set".into(),
                        sheets: Vec::new(),
                    });
                }
                let message = if self.show_sheetset {
                    "Sheet Set Manager: shown."
                } else {
                    "Sheet Set Manager: hidden."
                };
                self.command_line.push_output(crate::t!(message).as_ref());
            }

            // ── XDATA — read/write extended entity data ──────────────────────────
            // XDATA LIST             — show all xdata records on selected entities
            // XDATA SET <app> <str>  — append a string xdata value for <app>
            // XDATA CLEAR            — remove all xdata from selected entities
            // XDATA CLEAR <app>      — remove xdata for a specific application
            "XDATA" => {
                use crate::command::SelectThenKeywordCommand;
                let has_sel = !self.tabs[i].scene.selected_entities().is_empty();
                let c = SelectThenKeywordCommand::new(
                    "XDATA",
                    "XDATA  [List / Clear]  (SET <app> <value> by typing):",
                    vec![("List", "LIST", None), ("Clear", "CLEAR", None)],
                    has_sel,
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("XDATA ") => {
                use acadrust::xdata::{ExtendedDataRecord, XDataValue};
                let rest = cmd.trim_start_matches("XDATA").trim();
                let parts: Vec<&str> = rest.splitn(3, char::is_whitespace).collect();
                let sub = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
                let selected_handles: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(h, _)| *h)
                    .collect();
                if selected_handles.is_empty() {
                    self.command_line
                        .push_error(crate::t!("XDATA: select entities first.").as_ref());
                } else {
                    match sub.as_str() {
                        "LIST" | "" => {
                            for sh in &selected_handles {
                                if let Some(entity) = self.tabs[i].scene.document.get_entity(*sh) {
                                    let xd = &entity.common().extended_data;
                                    if xd.is_empty() {
                                        self.command_line.push_output(
                                            crate::tf!("  {:x}: no xdata.", sh.value()).as_ref(),
                                        );
                                    } else {
                                        for rec in xd.records() {
                                            self.command_line.push_output(
                                                crate::tf!(
                                                    "  {:x} [{}]: {} value(s)",
                                                    sh.value(),
                                                    rec.application_name,
                                                    rec.values.len()
                                                )
                                                .as_ref(),
                                            );
                                            for v in &rec.values {
                                                self.command_line.push_output(
                                                    crate::tf!("    {:?}", v).as_ref(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "SET" => {
                            let app = parts.get(1).copied().unwrap_or("OpenCADStudio");
                            let val = parts.get(2).copied().unwrap_or("");
                            self.push_undo_snapshot(i, "XDATA SET");
                            for sh in &selected_handles {
                                if let Some(entity) =
                                    self.tabs[i].scene.document.get_entity_mut(*sh)
                                {
                                    let mut rec = ExtendedDataRecord::new(app);
                                    rec.add_value(XDataValue::String(val.to_string()));
                                    entity.common_mut().extended_data.add_record(rec);
                                }
                            }
                            self.tabs[i].dirty = true;
                            self.command_line.push_output(
                                crate::tf!(
                                    "XDATA: set [{app}] = \"{val}\" on {} entity/entities.",
                                    selected_handles.len()
                                )
                                .as_ref(),
                            );
                        }
                        "CLEAR" => {
                            let app_filter = parts.get(1).copied();
                            self.push_undo_snapshot(i, "XDATA CLEAR");
                            for sh in &selected_handles {
                                if let Some(entity) =
                                    self.tabs[i].scene.document.get_entity_mut(*sh)
                                {
                                    let xd = &mut entity.common_mut().extended_data;
                                    if let Some(app) = app_filter {
                                        // Rebuild without the matching app.
                                        let kept: Vec<_> = xd
                                            .records()
                                            .iter()
                                            .filter(|r| r.application_name != app)
                                            .cloned()
                                            .collect();
                                        xd.clear();
                                        for r in kept {
                                            xd.add_record(r);
                                        }
                                    } else {
                                        xd.clear();
                                    }
                                }
                            }
                            self.tabs[i].dirty = true;
                            self.command_line
                                .push_output(crate::t!("XDATA: cleared.").as_ref());
                        }
                        _ => {
                            self.command_line.push_info(
                                crate::t!("Usage: XDATA LIST | SET <app> <value> | CLEAR [app]")
                                    .as_ref(),
                            );
                        }
                    }
                }
            }

            // BOX / SPHERE / CYLINDER / CONE / WEDGE / TORUS are handled by the
            // Model-tab primitive command above (with the kernel boolean caching).

            // ── EXTRUDE ────────────────────────────────────────────────────
            // PRESSPULL on a closed boundary creates a solid by extruding it to a
            // height — the same operation as EXTRUDE. THICKEN turns a closed planar
            // profile into a solid of the given thickness, which is also an extrude.
            "EXTRUDE" | "PRESSPULL" | "THICKEN" => {
                use crate::modules::insert::solid3d_cmds::ExtrudeCommand;
                // If a single entity is already selected, skip the pick step.
                let selected: Vec<_> = self.tabs[i].scene.selected_entities().into_iter().collect();
                let color = self.tabs[i].scene.layer_color(&self.tabs[i].active_layer);
                if selected.len() == 1 {
                    let handle = selected[0].0;
                    let mut cmd = ExtrudeCommand::new(color);
                    cmd.on_entity_pick(handle, glam::DVec3::ZERO);
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let cmd = ExtrudeCommand::new(color);
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                }
            }

            // ── REVOLVE ────────────────────────────────────────────────────
            "REVOLVE" => {
                use crate::modules::insert::solid3d_cmds::RevolveCommand;
                let color = self.tabs[i].scene.layer_color(&self.tabs[i].active_layer);
                let cmd = RevolveCommand::new(color);
                self.command_line.push_info(&cmd.prompt());
                self.tabs[i].active_cmd = Some(Box::new(cmd));
            }

            // ── SWEEP ──────────────────────────────────────────────────────
            "SWEEP" => {
                use crate::modules::insert::solid3d_cmds::SweepCommand;
                let color = self.tabs[i].scene.layer_color(&self.tabs[i].active_layer);
                let cmd = SweepCommand::new(color);
                self.command_line.push_info(&cmd.prompt());
                self.tabs[i].active_cmd = Some(Box::new(cmd));
            }

            // ── LOFT ───────────────────────────────────────────────────────
            "LOFT" => {
                use crate::modules::insert::solid3d_cmds::LoftCommand;
                let color = self.tabs[i].scene.layer_color(&self.tabs[i].active_layer);
                let cmd = LoftCommand::new(color);
                self.command_line.push_info(&cmd.prompt());
                self.tabs[i].active_cmd = Some(Box::new(cmd));
            }

            // ── OBJ import ───────────────────────────────────────────────
            "IMPORTOBJ" | "OBJIMPORT" => {
                return Some(Task::done(Message::ObjImport));
            }

            // ── STL export ────────────────────────────────────────────────
            "STLOUT" | "EXPORTSTL" => {
                return Some(Task::done(Message::StlExport));
            }

            // STEPOUT — export 3D meshes to STEP AP203 format
            "STEPOUT" | "EXPORTSTEP" | "STPOUT" => {
                return Some(Task::done(Message::StepExport));
            }

            // ── Plot Style Editor GUI ─────────────────────────────────────
            "PLOTSTYLEPANEL" | "PLOTSTYLEEDITOR" | "STYLESMANAGER" => {
                return Some(Task::done(Message::PlotStylePanelOpen));
            }

            // ── Plot / Page Setup ──────────────────────────────────────────
            // PLOT / PRINT open the full plot dialog (printer, paper, scale,
            // options); EXPORT / EXPORTPDF stay a direct PDF export.
            "PLOT" | "PRINT" => {
                return Some(Task::done(Message::PlotDialogOpen));
            }
            "PRINTALL" => {
                return Some(Task::done(Message::PrintAllOpen));
            }
            "EXPORT" | "EXPORTPDF" => {
                return Some(Task::done(Message::PlotExport));
            }
            // PLOTSTYLE — load or clear CTB/STB plot style table
            cmd if cmd == "PLOTSTYLE" || cmd.starts_with("PLOTSTYLE ") => {
                let sub = cmd
                    .split_once(' ')
                    .map(|(_, r)| r.trim().to_uppercase())
                    .unwrap_or_default();
                match sub.as_str() {
                    "CLEAR" | "NONE" => {
                        return Some(Task::done(Message::PlotStyleClear));
                    }
                    "" | "LOAD" => {
                        let active = self
                            .active_plot_style
                            .as_ref()
                            .map(|t| format!("Active: {}", t.name))
                            .unwrap_or_else(|| "No plot style loaded.".into());
                        self.command_line.push_info(&active);
                        return Some(Task::done(Message::PlotStyleLoad));
                    }
                    "?" | "STATUS" => {
                        let msg = self
                            .active_plot_style
                            .as_ref()
                            .map(|t| {
                                format!(
                                    "Plot style: {}  ({} color overrides)",
                                    t.name,
                                    t.aci_entries.iter().filter(|e| e.color.is_some()).count()
                                )
                            })
                            .unwrap_or_else(|| "No plot style table loaded.".into());
                        self.command_line.push_output(&msg);
                    }
                    _ => {
                        self.command_line.push_error(
                            crate::t!("Usage: PLOTSTYLE [LOAD | CLEAR | STATUS]").as_ref(),
                        );
                    }
                }
            }
            // UNDERLAY — edit properties of selected PDF/DWF/DGN underlay entities.
            // Usage:
            //   UNDERLAY FADE <0-80>
            //   UNDERLAY CONTRAST <0-100>
            //   UNDERLAY ON | OFF
            //   UNDERLAY CLIP ON | OFF
            //   UNDERLAY MONO ON | OFF
            "UNDERLAY" => {
                use crate::command::SelectThenKeywordCommand;
                let has_sel = !self.tabs[i].scene.selected_entities().is_empty();
                let c = SelectThenKeywordCommand::new(
                    "UNDERLAY",
                    "UNDERLAY  [Fade / Contrast / On / Off / Mono / Clip]:",
                    vec![
                        ("Fade", "FADE", Some("UNDERLAY  fade 0-100:")),
                        ("Contrast", "CONTRAST", Some("UNDERLAY  contrast 0-100:")),
                        ("On", "ON", None),
                        ("Off", "OFF", None),
                        ("Mono", "MONO", Some("UNDERLAY MONO  [On / Off]:")),
                        ("Clip", "CLIP", Some("UNDERLAY CLIP  [On / Off]:")),
                    ],
                    has_sel,
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("UNDERLAY ") => {
                let sub = cmd
                    .split_once(' ')
                    .map(|(_, r)| r.trim().to_uppercase())
                    .unwrap_or_default();
                let handles: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(h, _)| *h)
                    .collect();
                if handles.is_empty() {
                    self.command_line.push_error(
                        crate::t!("UNDERLAY: select underlay entities first.").as_ref(),
                    );
                } else {
                    let parts: Vec<&str> = sub.splitn(2, char::is_whitespace).collect();
                    let action = parts.first().copied().unwrap_or("");
                    let arg = parts.get(1).copied().unwrap_or("").trim();
                    let mut changed = 0usize;
                    self.push_undo_snapshot(i, "UNDERLAY");
                    for h in &handles {
                        if let Some(acadrust::EntityType::Underlay(ul)) = self.tabs[i]
                            .scene
                            .document
                            .entities_mut()
                            .find(|e| e.common().handle == *h)
                        {
                            match action {
                                "FADE" => {
                                    if let Ok(v) = arg.parse::<u8>() {
                                        ul.set_fade(v);
                                        changed += 1;
                                    }
                                }
                                "CONTRAST" => {
                                    if let Ok(v) = arg.parse::<u8>() {
                                        ul.set_contrast(v);
                                        changed += 1;
                                    }
                                }
                                "ON" => {
                                    ul.set_on(true);
                                    changed += 1;
                                }
                                "OFF" => {
                                    ul.set_on(false);
                                    changed += 1;
                                }
                                "CLIP" => match arg {
                                    "ON" => {
                                        ul.flags |=
                                            acadrust::entities::UnderlayDisplayFlags::CLIPPING;
                                        changed += 1;
                                    }
                                    "OFF" => {
                                        ul.clear_clip();
                                        changed += 1;
                                    }
                                    _ => {}
                                },
                                "MONO" => match arg {
                                    "ON" => {
                                        ul.set_monochrome(true);
                                        changed += 1;
                                    }
                                    "OFF" => {
                                        ul.set_monochrome(false);
                                        changed += 1;
                                    }
                                    _ => {}
                                },
                                _ => {
                                    // No sub-command: print status.
                                    self.command_line.push_output(crate::tf!(
                                        "Underlay {:x}: fade={}, contrast={}, on={}, clip={}, mono={}",
                                        h.value(),
                                        ul.fade,
                                        ul.contrast,
                                        ul.is_on(),
                                        ul.is_clipping(),
                                        ul.is_monochrome(),
                                    ).as_ref());
                                }
                            }
                        }
                    }
                    if changed > 0 {
                        self.tabs[i].dirty = true;
                        self.command_line
                            .push_info(crate::tf!("Updated {changed} underlay(s).").as_ref());
                    } else if !action.is_empty() {
                        self.command_line.push_error(
                            crate::t!("Usage: UNDERLAY [FADE <n>|CONTRAST <n>|ON|OFF|CLIP ON|OFF|MONO ON|OFF]").as_ref()
                        );
                    }
                }
            }

            // PAGESETUP is folded into the unified plot dialog.
            "PAGESETUP" => {
                return Some(Task::done(Message::PlotDialogOpen));
            }

            // ── Recognized commands whose full implementation is pending ─────────
            // These verbs are surfaced by the ribbon / menus but their feature is
            // still being built. Acknowledge them with an honest status so the
            // button responds instead of reporting an unknown command; each is
            // replaced by its real handler as the feature lands.
            // OBJECTSCALE ADD — add the active scale representation to every
            // selected object that supports per-scale context data.
            "OBJECTSCALE ADD" => {
                let handles: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(h, _)| *h)
                    .collect();
                if handles.is_empty() {
                    self.command_line
                        .push_error(crate::t!("OBJECTSCALE: select objects first.").as_ref());
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "OBJECTSCALE");
                let Some(scale) = self.tabs[i].scene.creation_annotation_scale_handle() else {
                    self.command_line.push_error(
                        crate::t!("OBJECTSCALE: the active annotation scale is unavailable.")
                            .as_ref(),
                    );
                    return Some(Task::none());
                };
                let mut n = 0usize;
                for h in &handles {
                    if crate::scene::annotative::create_annotation_context(
                        &mut self.tabs[i].scene.document,
                        *h,
                        scale,
                    ) {
                        crate::scene::annotative::set_entity_annotative(
                            &mut self.tabs[i].scene.document,
                            *h,
                            true,
                        );
                        n += 1;
                    }
                }
                let changes: Vec<_> = handles
                    .into_iter()
                    .map(|handle| (handle, crate::scene::ChangeKind::Modified))
                    .collect();
                self.tabs[i].scene.bump_entities(&changes);
                self.tabs[i].dirty = true;
                self.command_line.push_output(
                    crate::tf!("OBJECTSCALE: added the active scale to {n} object(s).").as_ref(),
                );
                return Some(Task::none());
            }

            // HYPERLINK <url> — attach a hyperlink to the selected objects, stored
            // in the standard PE_URL XData record so it round-trips in the file.
            "HYPERLINK" => {
                use crate::command::SelectThenValueCommand;
                let has_sel = !self.tabs[i].scene.selected_entities().is_empty();
                let c =
                    SelectThenValueCommand::new("HYPERLINK", "HYPERLINK  URL to attach:", has_sel);
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("HYPERLINK ") => {
                use acadrust::xdata::{ExtendedDataRecord, XDataValue};
                let url = cmd
                    .strip_prefix("HYPERLINK")
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if url.is_empty() {
                    self.command_line.push_info(
                        crate::t!("Usage: HYPERLINK <url>   (select objects first)").as_ref(),
                    );
                    return Some(Task::none());
                }
                let handles: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(h, _)| *h)
                    .collect();
                if handles.is_empty() {
                    self.command_line
                        .push_error(crate::t!("HYPERLINK: select objects first.").as_ref());
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "HYPERLINK");
                let mut n = 0usize;
                for h in &handles {
                    if let Some(e) = self.tabs[i].scene.document.get_entity_mut(*h) {
                        let xd = &mut e.common_mut().extended_data;
                        let mut rec = ExtendedDataRecord::new("PE_URL");
                        rec.add_value(XDataValue::String(url.clone()));
                        xd.add_record(rec);
                        n += 1;
                    }
                }
                self.tabs[i].dirty = true;
                self.command_line
                    .push_output(crate::tf!("HYPERLINK: attached to {n} object(s).").as_ref());
                return Some(Task::none());
            }

            // ADJUST — set brightness / contrast / fade on selected raster images
            //   ADJUST BRIGHTNESS|CONTRAST|FADE <0-100>
            "ADJUST" => {
                use crate::command::SelectThenKeywordCommand;
                let has_sel = !self.tabs[i].scene.selected_entities().is_empty();
                let c = SelectThenKeywordCommand::new(
                    "ADJUST",
                    "ADJUST  [Brightness / Contrast / Fade]:",
                    vec![
                        (
                            "Brightness",
                            "BRIGHTNESS",
                            Some("ADJUST  brightness 0-100:"),
                        ),
                        ("Contrast", "CONTRAST", Some("ADJUST  contrast 0-100:")),
                        ("Fade", "FADE", Some("ADJUST  fade 0-100:")),
                    ],
                    has_sel,
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("ADJUST ") => {
                let rest = cmd.trim_start_matches("ADJUST").trim();
                let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
                let action = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
                let arg = parts.get(1).copied().unwrap_or("").trim();
                let handles: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(h, _)| *h)
                    .collect();
                if handles.is_empty() {
                    self.command_line
                        .push_error(crate::t!("ADJUST: select raster image(s) first.").as_ref());
                } else if action.is_empty() {
                    self.command_line.push_info(
                        crate::t!("Usage: ADJUST BRIGHTNESS|CONTRAST|FADE <0-100>").as_ref(),
                    );
                } else if let Ok(v) = arg.parse::<u8>() {
                    let v = v.min(100);
                    self.push_undo_snapshot(i, "ADJUST");
                    let mut changed = 0usize;
                    let mut changed_handles = Vec::new();
                    for h in &handles {
                        if let Some(acadrust::EntityType::RasterImage(img)) = self.tabs[i]
                            .scene
                            .document
                            .entities_mut()
                            .find(|e| e.common().handle == *h)
                        {
                            match action.as_str() {
                                "BRIGHTNESS" => {
                                    img.brightness = v;
                                    changed += 1;
                                    changed_handles.push(*h);
                                }
                                "CONTRAST" => {
                                    img.contrast = v;
                                    changed += 1;
                                    changed_handles.push(*h);
                                }
                                "FADE" => {
                                    img.fade = v;
                                    changed += 1;
                                    changed_handles.push(*h);
                                }
                                _ => {}
                            }
                        }
                    }
                    if changed > 0 {
                        self.tabs[i].dirty = true;
                        for &handle in &changed_handles {
                            self.tabs[i].scene.reseed_derived_caches(handle);
                        }
                        let changes: Vec<_> = changed_handles
                            .into_iter()
                            .map(|handle| (handle, crate::scene::ChangeKind::Modified))
                            .collect();
                        self.tabs[i].scene.bump_entities(&changes);
                        self.command_line.push_output(
                            crate::tf!("ADJUST: {action} = {v} on {changed} image(s).").as_ref(),
                        );
                    } else {
                        self.command_line.push_error(
                            "ADJUST: no raster images selected, or unknown property (use BRIGHTNESS|CONTRAST|FADE).",
                        );
                    }
                } else {
                    self.command_line
                        .push_error(crate::t!("ADJUST: value must be 0-100.").as_ref());
                }
            }

            // ANNOSCALE / CANNOSCALE <ratio> — set the current annotation scale
            // (e.g. 1:50, 2:1, or a plain factor). Drives annotative-object size
            // in model space and is written to the drawing header.
            "ANNOSCALE" | "CANNOSCALE" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new(
                    "ANNOSCALE",
                    "ANNOSCALE  new annotation scale  (e.g. 1:50, 2:1, or a factor):",
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            "ANNOALLVISIBLE" => {
                use crate::command::ValuePromptCommand;
                let c =
                    ValuePromptCommand::new("ANNOALLVISIBLE", "ANNOALLVISIBLE  new value [0/1]:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("ANNOALLVISIBLE ") => {
                let value = cmd.split_whitespace().nth(1).unwrap_or("");
                match value {
                    "0" | "OFF" | "FALSE" => {
                        self.tabs[i].scene.set_annotation_all_visible(false);
                        self.tabs[i].dirty = true;
                    }
                    "1" | "ON" | "TRUE" => {
                        self.tabs[i].scene.set_annotation_all_visible(true);
                        self.tabs[i].dirty = true;
                    }
                    _ => self
                        .command_line
                        .push_error(crate::t!("ANNOALLVISIBLE: enter 0 or 1.").as_ref()),
                }
            }
            "ANNOAUTOSCALE" => {
                use crate::command::ValuePromptCommand;
                let c =
                    ValuePromptCommand::new("ANNOAUTOSCALE", "ANNOAUTOSCALE  new value [-4..4]:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("ANNOAUTOSCALE ") => {
                let value = cmd.split_whitespace().nth(1).unwrap_or("");
                match value.parse::<i8>() {
                    Ok(mode @ -4..=4) => self.annotation_auto_scale = mode,
                    _ => self
                        .command_line
                        .push_error("ANNOAUTOSCALE: enter an integer from -4 through 4."),
                }
            }
            "ANNOUPDATE" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(handle, _)| *handle)
                    .collect();
                if handles.is_empty() {
                    self.command_line.push_error(
                        crate::t!("ANNOUPDATE: select annotation objects first.").as_ref(),
                    );
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "ANNOUPDATE");
                let scale = self.tabs[i].scene.creation_annotation_scale_handle();
                let mut updated = 0usize;
                for handle in &handles {
                    if crate::scene::annotative::update_entity_from_annotation_style(
                        &mut self.tabs[i].scene.document,
                        *handle,
                        scale,
                    ) {
                        updated += 1;
                    }
                }
                if updated > 0 {
                    let changes: Vec<_> = handles
                        .into_iter()
                        .map(|handle| (handle, crate::scene::ChangeKind::Modified))
                        .collect();
                    self.tabs[i].scene.bump_entities(&changes);
                    self.tabs[i].dirty = true;
                }
                self.command_line
                    .push_output(crate::tf!("ANNOUPDATE: updated {updated} object(s).").as_ref());
                return Some(Task::none());
            }
            cmd if cmd.starts_with("ANNOSCALE ") || cmd.starts_with("CANNOSCALE ") => {
                let arg = cmd
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if arg.is_empty() {
                    let name = self.tabs[i]
                        .scene
                        .document
                        .header
                        .current_annotation_scale
                        .clone();
                    self.command_line
                        .push_output(crate::tf!("Current annotation scale: {name}").as_ref());
                    return Some(Task::none());
                }
                let previous = self.tabs[i].scene.displayed_annotation_scale_handle();
                match self.tabs[i].scene.set_annotation_scale_named(&arg) {
                    Some(handle) => {
                        if self.annotation_auto_scale > 0 {
                            self.tabs[i].scene.add_annotation_scale_to_objects(
                                handle,
                                previous,
                                self.annotation_auto_scale as u8,
                            );
                        }
                        self.tabs[i].dirty = true;
                        self.command_line
                            .push_output(crate::tf!("Annotation scale: {arg}").as_ref());
                    }
                    None => self.command_line.push_error(
                        crate::t!("Usage: ANNOSCALE <ratio>  e.g. 1:50, 2:1, or a factor").as_ref(),
                    ),
                }
            }

            // SCALELISTEDIT — list / add / delete the drawing's annotation scales.
            //   SCALELISTEDIT              list
            //   SCALELISTEDIT ADD 1:50     add (name is a paper:drawing ratio)
            //   SCALELISTEDIT DELETE 1:50  remove (not the current scale)
            "SCALELISTEDIT" => {
                use crate::command::KeywordCommand;
                let c = KeywordCommand::new(
                    "SCALELISTEDIT",
                    "SCALELISTEDIT  [Add / Delete]:",
                    vec![
                        (
                            "Add",
                            "ADD",
                            Some("SCALELISTEDIT ADD  new scale ratio (e.g. 1:50):"),
                        ),
                        (
                            "Delete",
                            "DELETE",
                            Some("SCALELISTEDIT DELETE  scale ratio to remove:"),
                        ),
                    ],
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("SCALELISTEDIT ") => {
                let rest = cmd.trim_start_matches("SCALELISTEDIT").trim();
                let mut parts = rest.splitn(2, char::is_whitespace);
                let sub = parts.next().unwrap_or("").to_uppercase();
                let arg = parts.next().unwrap_or("").trim();
                match sub.as_str() {
                    "ADD" => match arg.split_once(':') {
                        Some((p, d)) => match (p.trim().parse::<f64>(), d.trim().parse::<f64>()) {
                            (Ok(paper), Ok(drawing)) if paper > 0.0 && drawing > 0.0 => {
                                self.push_undo_snapshot(i, "SCALELISTEDIT");
                                if self.tabs[i].scene.add_scale(arg, paper, drawing) {
                                    self.tabs[i].dirty = true;
                                    self.command_line.push_output(
                                        crate::tf!("Added annotation scale {arg}.").as_ref(),
                                    );
                                } else {
                                    self.command_line.push_info(
                                        crate::tf!("Scale {arg} already exists.").as_ref(),
                                    );
                                }
                            }
                            _ => self.command_line.push_error(
                                crate::t!("SCALELISTEDIT ADD: use a ratio like 1:50.").as_ref(),
                            ),
                        },
                        None => self.command_line.push_error(
                            crate::t!("SCALELISTEDIT ADD: use a ratio like 1:50.").as_ref(),
                        ),
                    },
                    "DELETE" | "REMOVE" => {
                        let current = self.tabs[i]
                            .scene
                            .document
                            .header
                            .current_annotation_scale
                            .clone();
                        if arg.is_empty() {
                            self.command_line.push_info(
                                crate::t!("Usage: SCALELISTEDIT DELETE <name>").as_ref(),
                            );
                        } else if arg.eq_ignore_ascii_case(&current) {
                            self.command_line.push_error(
                                crate::tf!("Cannot delete the current annotation scale ({arg}).")
                                    .as_ref(),
                            );
                        } else {
                            self.push_undo_snapshot(i, "SCALELISTEDIT");
                            if self.tabs[i].scene.remove_scale(arg) {
                                self.tabs[i].dirty = true;
                                self.command_line.push_output(
                                    crate::tf!("Removed annotation scale {arg}.").as_ref(),
                                );
                            } else {
                                self.command_line.push_info(
                                    crate::tf!("No annotation scale named {arg}.").as_ref(),
                                );
                            }
                        }
                    }
                    "" => {
                        let names: Vec<String> = self.tabs[i]
                            .scene
                            .scale_list()
                            .into_iter()
                            .map(|(n, _, _)| n)
                            .collect();
                        if names.is_empty() {
                            self.command_line
                                .push_info(crate::t!("No annotation scales defined.").as_ref());
                        } else {
                            self.command_line.push_output(
                                crate::tf!("Annotation scales: {}", names.join(", ")).as_ref(),
                            );
                        }
                    }
                    _ => self.command_line.push_info(
                        crate::t!("Usage: SCALELISTEDIT [ADD 1:50 | DELETE 1:50]").as_ref(),
                    ),
                }
            }

            // OBJECTSCALE — open the Annotation Object Scale dialog for the
            // selected object (add / remove its per-object scale representations).
            // Reachable now that the immediate "add current scale" quick action
            // moved to the explicit `OBJECTSCALE ADD` keyword above.
            "OBJECTSCALE" => {
                return Some(Task::done(Message::AnnoObjectScaleOpen));
            }

            // DATALINK <path.csv> — import a CSV file into a table placed at the
            // origin (one-time import; a live re-reading link is future work).
            "DATALINK" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new("DATALINK", "DATALINK  path to the .csv file:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("DATALINK ") => {
                let path = cmd.trim_start_matches("DATALINK").trim();
                if path.is_empty() {
                    self.command_line.push_info(
                        "Usage: DATALINK <path-to-.csv>  — imports the CSV into a table at the origin.",
                    );
                    return Some(Task::none());
                }
                match std::fs::read_to_string(path) {
                    Ok(text) => {
                        let rows_data: Vec<Vec<String>> = text
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(|line| line.split(',').map(|s| s.trim().to_string()).collect())
                            .collect();
                        let nrows = rows_data.len();
                        let ncols = rows_data.iter().map(|r| r.len()).max().unwrap_or(0);
                        if nrows == 0 || ncols == 0 {
                            self.command_line
                                .push_error(crate::t!("DATALINK: the CSV file is empty.").as_ref());
                            return Some(Task::none());
                        }
                        use acadrust::entities::TableBuilder;
                        use acadrust::types::Vector3;
                        let mut table = TableBuilder::new(nrows, ncols)
                            .at(Vector3::new(0.0, 0.0, 0.0))
                            .row_height(0.5)
                            .column_width(2.0)
                            .build();
                        for (r, row) in rows_data.iter().enumerate() {
                            for (c, cell) in row.iter().enumerate() {
                                table.set_cell_text(r, c, cell);
                            }
                        }
                        self.push_undo_snapshot(i, "DATALINK");
                        self.tabs[i]
                            .scene
                            .add_entity_clone(acadrust::EntityType::Table(table));
                        self.tabs[i].dirty = true;
                        self.command_line.push_output(
                            crate::tf!(
                            "DATALINK: imported {nrows}×{ncols} cells into a table at the origin."
                        )
                            .as_ref(),
                        );
                    }
                    Err(e) => {
                        self.command_line.push_error(
                            crate::tf!("DATALINK: cannot read \"{path}\": {e}").as_ref(),
                        );
                    }
                }
            }

            // LANDXMLIMPORT <path> — import survey points (LandXML <CgPoint>
            // elements) as Point objects. Reads the coordinate text content
            // (northing easting elevation) → Point at (easting, northing, elev).
            "LANDXMLIMPORT" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new(
                    "LANDXMLIMPORT",
                    "LANDXMLIMPORT  path to the .xml file:",
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("LANDXMLIMPORT ") => {
                let path = cmd.trim_start_matches("LANDXMLIMPORT").trim();
                if path.is_empty() {
                    self.command_line.push_info(
                        "Usage: LANDXMLIMPORT <path-to-.xml>  (imports CgPoint survey points)",
                    );
                    return Some(Task::none());
                }
                match std::fs::read_to_string(path) {
                    Ok(xml) => {
                        let pts = parse_landxml_cgpoints(&xml);
                        if pts.is_empty() {
                            self.command_line.push_info(
                                crate::t!("LANDXMLIMPORT: no <CgPoint> survey points found.")
                                    .as_ref(),
                            );
                            return Some(Task::none());
                        }
                        self.push_undo_snapshot(i, "LANDXMLIMPORT");
                        for [x, y, z] in &pts {
                            let mut p = acadrust::entities::Point::new();
                            p.location = acadrust::types::Vector3::new(*x, *y, *z);
                            self.tabs[i]
                                .scene
                                .add_entity_clone(acadrust::EntityType::Point(p));
                        }
                        self.tabs[i].dirty = true;
                        self.command_line.push_output(
                            crate::tf!(
                            "LANDXMLIMPORT: imported {} survey point(s). Use ZOOM EXTENTS to view.",
                            pts.len()
                        )
                            .as_ref(),
                        );
                    }
                    Err(e) => self.command_line.push_error(
                        crate::tf!("LANDXMLIMPORT: cannot read \"{path}\": {e}").as_ref(),
                    ),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDCONTOUR") => {
                let arguments = cmd.trim_start_matches("POINTCLOUDCONTOUR").trim();
                let interval = arguments
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(1.0);
                self.generate_point_cloud_contours(i, interval);
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDNOISE") => {
                let arguments = cmd.trim_start_matches("POINTCLOUDNOISE").trim();
                let mut fields = arguments.split_whitespace();
                let radius = fields.next().and_then(|v| v.parse::<f64>().ok());
                let min_neighbors = fields.next().and_then(|v| v.parse::<usize>().ok());
                let noise_class = fields
                    .next()
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(7);
                match (radius, min_neighbors) {
                    (Some(radius), Some(min_neighbors)) if radius > 0.0 && min_neighbors > 0 => {
                        self.classify_point_cloud_noise(i, radius, min_neighbors, noise_class);
                    }
                    _ => {
                        self.command_line.push_error(
                            "POINTCLOUDNOISE: usage POINTCLOUDNOISE <radius> <min-neighbors> [class=7].",
                        );
                    }
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDGROUND") => {
                let arguments = cmd.trim_start_matches("POINTCLOUDGROUND").trim();
                let mut fields = arguments.split_whitespace();
                let cell = fields.next().and_then(|v| v.parse::<f64>().ok());
                let distance = fields.next().and_then(|v| v.parse::<f64>().ok());
                let angle = fields.next().and_then(|v| v.parse::<f64>().ok());
                let mut options = ocs_pointcloud::GroundOptions::default();
                if let Some(cell) = cell {
                    if cell <= 0.0 {
                        self.command_line
                            .push_error("POINTCLOUDGROUND: cell size must be positive.");
                        return Some(Task::none());
                    }
                    options.cell_size = cell;
                }
                if let Some(distance) = distance {
                    options.max_distance = distance;
                }
                if let Some(angle) = angle {
                    options.max_angle_degrees = angle;
                }
                self.classify_point_cloud_ground(i, options);
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDRULE ") => {
                // POINTCLOUDRULE <field> <op> <a> [b] <class>
                let arguments = cmd.trim_start_matches("POINTCLOUDRULE").trim();
                let fields: Vec<&str> = arguments.split_whitespace().collect();
                if fields.len() < 5 {
                    self.command_line.push_error(
                        "POINTCLOUDRULE: usage POINTCLOUDRULE <ELEVATION|INTENSITY|RETURN|SOURCE> <LT|GT|BETWEEN|EQ> <a> [b] <class>.",
                    );
                    return Some(Task::none());
                }
                let field = match fields[0] {
                    "ELEVATION" | "Z" => Some(ocs_pointcloud::RuleField::Elevation),
                    "INTENSITY" => Some(ocs_pointcloud::RuleField::Intensity),
                    "RETURN" => Some(ocs_pointcloud::RuleField::ReturnNumber),
                    "SOURCE" => Some(ocs_pointcloud::RuleField::PointSource),
                    _ => None,
                };
                let op = match fields[1] {
                    "LT" | "<" => Some(ocs_pointcloud::RuleOp::Less),
                    "GT" | ">" => Some(ocs_pointcloud::RuleOp::Greater),
                    "BETWEEN" | "BW" => Some(ocs_pointcloud::RuleOp::Between),
                    "EQ" | "=" => Some(ocs_pointcloud::RuleOp::Equals),
                    _ => None,
                };
                let (Some(field), Some(op)) = (field, op) else {
                    self.command_line
                        .push_error("POINTCLOUDRULE: unknown field or operation.");
                    return Some(Task::none());
                };
                let parsed = if fields.len() == 5 {
                    match (
                        fields[2].parse::<f64>(),
                        fields[3].parse::<f64>(),
                        fields[4].parse::<u8>(),
                    ) {
                        (Ok(a), Ok(b), Ok(class)) => Some(([a, b], class)),
                        _ => None,
                    }
                } else if fields.len() == 4 {
                    match (fields[2].parse::<f64>(), fields[3].parse::<u8>()) {
                        (Ok(a), Ok(class)) => Some(([a, a], class)),
                        _ => None,
                    }
                } else {
                    None
                };
                match parsed {
                    Some((values, class)) => {
                        self.classify_point_cloud_rule(
                            i,
                            ocs_pointcloud::ClassifyRule {
                                field,
                                op,
                                values,
                                target_class: class,
                                from_classes: Vec::new(),
                            },
                        );
                    }
                    None => {
                        self.command_line.push_error(
                            "POINTCLOUDRULE: could not parse thresholds and target class.",
                        );
                    }
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "SCRIPT" => {
                return Some(Task::done(Message::ScriptPick));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("SCRIPT ") => {
                let path = std::path::PathBuf::from(cmd.trim_start_matches("SCRIPT").trim());
                return Some(self.start_script(path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDATTACH" | "RECAP" => {
                return Some(Task::done(Message::PointCloudAttach));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDATTACHFOLDER" | "POINTCLOUDATTACHDIR" => {
                return Some(Task::done(Message::PointCloudFolderAttach));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDEXPORTALL") => {
                let argument = cmd.trim_start_matches("POINTCLOUDEXPORTALL").trim();
                if argument.is_empty() {
                    return Some(Task::done(Message::PointCloudExportAll));
                }
                return Some(self.start_point_cloud_export_all(std::path::PathBuf::from(argument)));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDATTACHFOLDER ") => {
                let folder = std::path::PathBuf::from(
                    cmd.trim_start_matches("POINTCLOUDATTACHFOLDER").trim(),
                );
                return Some(self.start_point_cloud_folder_load(folder));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDMANAGER" => {
                self.active_modal = Some(crate::app::ModalKind::PointCloudManager);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDRESTORE" => {
                return Some(self.start_point_cloud_restore(i));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDINFO" => {
                self.point_cloud_info(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDINDEX" => {
                return Some(self.start_point_cloud_index(i));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDINDEXCANCEL" => {
                self.cancel_point_cloud_index(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDCLASSIFY" => {
                use crate::command::ValuePromptCommand;
                let command = ValuePromptCommand::new(
                    "POINTCLOUDCLASSIFY",
                    "POINTCLOUDCLASSIFY  Enter class and source indices (example: 2 10-25,40):",
                );
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDCLASSIFY ") => {
                let arguments = cmd.trim_start_matches("POINTCLOUDCLASSIFY").trim();
                let mut fields = arguments.splitn(2, char::is_whitespace);
                let classification = fields.next().and_then(|value| value.parse::<u8>().ok());
                let source_indices = fields.next().unwrap_or("").trim();
                match classification {
                    Some(classification) if !source_indices.is_empty() => {
                        self.reclassify_point_cloud(i, classification, source_indices);
                    }
                    _ => self.command_line.push_error(
                        "Usage: POINTCLOUDCLASSIFY <0-255 class> <indices/ranges>; example: POINTCLOUDCLASSIFY 2 10-25,40",
                    ),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDUNDO" => {
                self.undo_point_cloud_edit(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSTATS" => {
                self.point_cloud_statistics(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDCRS" => {
                self.point_cloud_crs_info(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDREPROJECT" => {
                use crate::command::ValuePromptCommand;
                let command = ValuePromptCommand::new(
                    "POINTCLOUDREPROJECT",
                    "POINTCLOUDREPROJECT  Enter target horizontal EPSG code (XY transforms; Z is preserved):",
                );
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDREPROJECT ") => {
                match cmd
                    .trim_start_matches("POINTCLOUDREPROJECT")
                    .trim()
                    .parse::<u16>()
                {
                    Ok(code) if code > 0 => {
                        return Some(Task::done(Message::PointCloudReproject(code)))
                    }
                    _ => self
                        .command_line
                        .push_error("Usage: POINTCLOUDREPROJECT <target EPSG code>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDCLASSADD" => {
                self.add_point_cloud_class(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTBOX" => {
                if self.active_modal == Some(crate::app::ModalKind::PointCloudManager) {
                    self.active_modal = None;
                    self.reset_modal_geometry();
                }
                let command = crate::app::point_cloud::PointCloudScreenRectangleCommand::new();
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTFENCE" => {
                if self.active_modal == Some(crate::app::ModalKind::PointCloudManager) {
                    self.active_modal = None;
                    self.reset_modal_geometry();
                }
                let command = crate::app::point_cloud::PointCloudScreenFenceCommand::new();
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTBRUSH" => {
                if self.active_modal == Some(crate::app::ModalKind::PointCloudManager) {
                    self.active_modal = None;
                    self.reset_modal_geometry();
                }
                let command = crate::app::point_cloud::PointCloudScreenBrushCommand;
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDMEASURE" => {
                if self.active_modal == Some(crate::app::ModalKind::PointCloudManager) {
                    self.active_modal = None;
                    self.reset_modal_geometry();
                }
                let command = crate::app::point_cloud::PointCloudMeasureCommand::new();
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSCREENMEASURE ") => {
                let values = cmd
                    .trim_start_matches("POINTCLOUDSCREENMEASURE")
                    .split_whitespace()
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>();
                match values {
                    Ok(values) if values.len() == 7 => self.point_cloud_measure_screen(
                        i,
                        glam::dvec3(values[0], values[1], values[2]),
                        glam::dvec3(values[3], values[4], values[5]),
                        values[6] as f32,
                    ),
                    _ => self
                        .command_line
                        .push_error("POINTCLOUDSCREENMEASURE: invalid gesture."),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTPOINT" => {
                if self.active_modal == Some(crate::app::ModalKind::PointCloudManager) {
                    self.active_modal = None;
                    self.reset_modal_geometry();
                }
                let command = crate::app::point_cloud::PointCloudScreenPointCommand;
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSCREENPOINT ") => {
                let values = cmd
                    .trim_start_matches("POINTCLOUDSCREENPOINT")
                    .split_whitespace()
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>();
                match values {
                    Ok(values) if values.len() == 4 => self.point_cloud_select_screen_point(
                        i,
                        glam::dvec3(values[0], values[1], values[2]),
                        values[3] as f32,
                    ),
                    _ => self
                        .command_line
                        .push_error("POINTCLOUDSCREENPOINT: invalid gesture."),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSCREENRECT ") => {
                let values = cmd
                    .trim_start_matches("POINTCLOUDSCREENRECT")
                    .split_whitespace()
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>();
                match values {
                    Ok(values) if values.len() == 6 => self.point_cloud_select_screen_rectangle(
                        i,
                        glam::dvec3(values[0], values[1], values[2]),
                        glam::dvec3(values[3], values[4], values[5]),
                    ),
                    _ => self
                        .command_line
                        .push_error("POINTCLOUDSCREENRECT: invalid gesture."),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSCREENFENCE ") => {
                let values = cmd
                    .trim_start_matches("POINTCLOUDSCREENFENCE")
                    .split_whitespace()
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>();
                match values {
                    Ok(values) if values.len() >= 9 && values.len() % 3 == 0 => {
                        let vertices: Vec<_> = values
                            .chunks_exact(3)
                            .map(|point| glam::dvec3(point[0], point[1], point[2]))
                            .collect();
                        self.point_cloud_select_screen_fence(i, &vertices);
                    }
                    _ => self
                        .command_line
                        .push_error("POINTCLOUDSCREENFENCE: invalid gesture."),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSCREENBRUSH ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDSCREENBRUSH")
                    .split_whitespace()
                    .collect();
                let (classification, offset) = match values.first().copied() {
                    Some("SELECT") => (Some(None), 1),
                    Some("CLASS") => (
                        values
                            .get(1)
                            .and_then(|value| value.parse::<u8>().ok())
                            .map(Some),
                        2,
                    ),
                    _ => (None, 0),
                };
                let coordinates = values
                    .get(offset..)
                    .unwrap_or_default()
                    .iter()
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>();
                match (classification, coordinates) {
                    (Some(classification), Ok(values)) if values.len() == 4 => {
                        self.point_cloud_select_screen_brush(
                            i,
                            glam::dvec3(values[0], values[1], values[2]),
                            values[3] as f32,
                            classification,
                        );
                        let command: Box<dyn crate::command::CadCommand> = match classification {
                            Some(class) => Box::new(
                                crate::app::point_cloud::PointCloudBrushClassifyCommand::new(class),
                            ),
                            None => Box::new(crate::app::point_cloud::PointCloudScreenBrushCommand),
                        };
                        self.command_line.push_info(&command.prompt());
                        self.tabs[i].active_cmd = Some(command);
                    }
                    _ => self
                        .command_line
                        .push_error("POINTCLOUDSCREENBRUSH: invalid gesture."),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSECTIONCLEAR" => {
                self.clear_point_cloud_section(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSECTIONVIEW" => {
                self.point_cloud_section_view(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSECTIONMOVE ") => {
                match cmd
                    .trim_start_matches("POINTCLOUDSECTIONMOVE")
                    .trim()
                    .parse::<f64>()
                {
                    Ok(delta) => self.move_point_cloud_section(i, delta),
                    Err(_) => self
                        .command_line
                        .push_error("Usage: POINTCLOUDSECTIONMOVE <distance>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSECTIONWIDTH ") => {
                match cmd
                    .trim_start_matches("POINTCLOUDSECTIONWIDTH")
                    .trim()
                    .parse::<f64>()
                {
                    Ok(width) => self.set_point_cloud_section_width(i, width),
                    Err(_) => self
                        .command_line
                        .push_error("Usage: POINTCLOUDSECTIONWIDTH <half-width>"),
                }
            }

            // POINTCLOUDSECTION x0 y0 x1 y1 [width] — set a vertical section
            // directly (used by the tool palette and scripts; the interactive
            // fence flow below feeds the same handler).
            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSECTION ") => {
                let parsed: Result<Vec<f64>, _> = cmd
                    .trim_start_matches("POINTCLOUDSECTION")
                    .split_whitespace()
                    .map(str::parse::<f64>)
                    .collect();
                match parsed {
                    Ok(values) => match values.as_slice() {
                        [x0, y0, x1, y1] => self.set_point_cloud_section(
                            i,
                            [*x0, *y0],
                            [*x1, *y1],
                            1.0,
                            crate::scene::model::point_cloud_model::SectionMode::Dim,
                        ),
                        [x0, y0, x1, y1, width] => self.set_point_cloud_section(
                            i,
                            [*x0, *y0],
                            [*x1, *y1],
                            *width,
                            crate::scene::model::point_cloud_model::SectionMode::Dim,
                        ),
                        _ => self.command_line.push_error(
                            "Usage: POINTCLOUDSECTION <x0> <y0> <x1> <y1> [half-width]",
                        ),
                    },
                    Err(_) => self
                        .command_line
                        .push_error("Usage: POINTCLOUDSECTION <x0> <y0> <x1> <y1> [half-width]"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSECTION" => {
                // Interactive: click two points in the active viewport to draw
                // the section line. Reuses the two-corner screen command.
                let command = crate::app::point_cloud::PointCloudSectionCommand::new();
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDCOLOR ") => {
                let value = cmd.trim_start_matches("POINTCLOUDCOLOR").trim();
                let mode = match value {
                    "CLASS" | "CLASSIFICATION" => Some(ocs_pointcloud::ColorMode::Classification),
                    "RGB" | "COLOR" => Some(ocs_pointcloud::ColorMode::Rgb),
                    "INTENSITY" => Some(ocs_pointcloud::ColorMode::Intensity),
                    "ELEVATION" | "HEIGHT" | "Z" => Some(ocs_pointcloud::ColorMode::Elevation),
                    "RETURN" | "RETURNS" => Some(ocs_pointcloud::ColorMode::ReturnNumber),
                    "SOURCE" | "POINTSOURCE" => Some(ocs_pointcloud::ColorMode::PointSource),
                    _ => None,
                };
                if let Some(mode) = mode {
                    self.set_point_cloud_color_mode(i, mode);
                } else {
                    self.command_line.push_error(
                        "Usage: POINTCLOUDCOLOR <CLASS|RGB|INTENSITY|ELEVATION|RETURN|SOURCE>",
                    );
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDPOINTSIZE ") => {
                match cmd
                    .trim_start_matches("POINTCLOUDPOINTSIZE")
                    .trim()
                    .parse::<f32>()
                {
                    Ok(size) => self.set_point_cloud_point_size(i, size),
                    Err(_) => self
                        .command_line
                        .push_error("Usage: POINTCLOUDPOINTSIZE <1-32 pixels>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDCLASSVISIBLE ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDCLASSVISIBLE")
                    .split_whitespace()
                    .collect();
                let class = values.first().and_then(|value| value.parse::<u8>().ok());
                let visible = values.get(1).and_then(|value| match *value {
                    "ON" | "TRUE" | "1" | "SHOW" => Some(true),
                    "OFF" | "FALSE" | "0" | "HIDE" => Some(false),
                    _ => None,
                });
                match (class, visible) {
                    (Some(class), Some(visible)) => {
                        self.set_point_cloud_class_visible(i, class, visible)
                    }
                    _ => self
                        .command_line
                        .push_error("Usage: POINTCLOUDCLASSVISIBLE <0-255 class> <ON|OFF>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSELECTBOX ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDSELECTBOX")
                    .split_whitespace()
                    .filter_map(|value| value.parse::<f64>().ok())
                    .collect();
                if values.len() == 6 {
                    self.point_cloud_select_box(
                        i,
                        [values[0], values[1], values[2]],
                        [values[3], values[4], values[5]],
                    );
                } else {
                    self.command_line
                        .push_error("Usage: POINTCLOUDSELECTBOX <minX minY minZ maxX maxY maxZ>");
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSELECTBRUSH ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDSELECTBRUSH")
                    .split_whitespace()
                    .filter_map(|value| value.parse::<f64>().ok())
                    .collect();
                if values.len() == 4 {
                    self.point_cloud_select_brush(i, [values[0], values[1], values[2]], values[3]);
                } else {
                    self.command_line.push_error(
                        "Usage: POINTCLOUDSELECTBRUSH <centerX centerY centerZ radius>",
                    );
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSELECTPOINT ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDSELECTPOINT")
                    .split_whitespace()
                    .filter_map(|value| value.parse::<f64>().ok())
                    .collect();
                if values.len() == 4 {
                    self.point_cloud_select_nearest(
                        i,
                        [values[0], values[1], values[2]],
                        values[3],
                    );
                } else {
                    self.command_line
                        .push_error("Usage: POINTCLOUDSELECTPOINT <X Y Z search-radius>");
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTSLICE" => {
                use crate::command::ValuePromptCommand;
                let command = ValuePromptCommand::new(
                    "POINTCLOUDSELECTSLICE",
                    "POINTCLOUDSELECTSLICE  Enter minimum-Z maximum-Z:",
                );
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSELECTSLICE ") => {
                let values = cmd
                    .trim_start_matches("POINTCLOUDSELECTSLICE")
                    .split_whitespace()
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>();
                match values {
                    Ok(values)
                        if values.len() == 2 && values[0].is_finite() && values[1].is_finite() =>
                    {
                        self.point_cloud_select_elevation_slice(i, values[0], values[1]);
                    }
                    _ => self
                        .command_line
                        .push_error("Usage: POINTCLOUDSELECTSLICE <minimum-Z maximum-Z>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTFILTER" => {
                use crate::command::ValuePromptCommand;
                let command = ValuePromptCommand::new(
                    "POINTCLOUDSELECTFILTER",
                    "POINTCLOUDSELECTFILTER  Enter CLEAR, CLASS/RETURN/SOURCE list, ELEVATION low high, or flag ON/OFF/ANY:",
                );
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDSELECTCLEAR" => {
                self.clear_point_cloud_selections(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDSELECTFILTER ") => {
                let arguments = cmd.trim_start_matches("POINTCLOUDSELECTFILTER").trim();
                let mut fields = arguments.split_whitespace();
                let field = fields.next().unwrap_or("");
                if self.tabs[i].point_cloud.is_empty() {
                    self.command_line
                        .push_error("POINTCLOUDSELECTFILTER: attach a LAS/LAZ cloud first.");
                    return Some(Task::none());
                }
                let mut filter = self.tabs[i].point_cloud.selection_filter.clone();
                let values: Vec<_> = fields.collect();
                let parse_u8_list = |value: &str| {
                    value
                        .split(',')
                        .map(str::parse::<u8>)
                        .collect::<Result<Vec<_>, _>>()
                };
                let parse_u16_list = |value: &str| {
                    value
                        .split(',')
                        .map(str::parse::<u16>)
                        .collect::<Result<Vec<_>, _>>()
                };
                let flag_value = |value: Option<&&str>| match value.copied() {
                    Some("ON" | "TRUE" | "1") => Some(Ok(Some(true))),
                    Some("OFF" | "FALSE" | "0") => Some(Ok(Some(false))),
                    Some("ANY" | "CLEAR") => Some(Ok(None)),
                    Some(_) => Some(Err(())),
                    None => None,
                };
                let result: Result<(), ()> = match field {
                    "CLEAR" if values.is_empty() => {
                        filter = ocs_pointcloud::PointFilter::default();
                        Ok(())
                    }
                    "CLASS" if values.len() == 1 => {
                        if matches!(values[0], "ANY" | "CLEAR") {
                            filter.classes.clear();
                            Ok(())
                        } else {
                            parse_u8_list(values[0])
                                .map(|mut parsed| {
                                    parsed.sort_unstable();
                                    parsed.dedup();
                                    filter.classes = parsed;
                                })
                                .map_err(|_| ())
                        }
                    }
                    "RETURN" if values.len() == 1 => {
                        if matches!(values[0], "ANY" | "CLEAR") {
                            filter.returns.clear();
                            Ok(())
                        } else {
                            parse_u8_list(values[0])
                                .map(|mut parsed| {
                                    parsed.sort_unstable();
                                    parsed.dedup();
                                    filter.returns = parsed;
                                })
                                .map_err(|_| ())
                        }
                    }
                    "SOURCE" if values.len() == 1 => {
                        if matches!(values[0], "ANY" | "CLEAR") {
                            filter.sources.clear();
                            Ok(())
                        } else {
                            parse_u16_list(values[0])
                                .map(|mut parsed| {
                                    parsed.sort_unstable();
                                    parsed.dedup();
                                    filter.sources = parsed;
                                })
                                .map_err(|_| ())
                        }
                    }
                    "ELEVATION" if values.len() == 1 && matches!(values[0], "ANY" | "CLEAR") => {
                        filter.elevation = None;
                        Ok(())
                    }
                    "ELEVATION" if values.len() == 2 => {
                        match (values[0].parse::<f64>(), values[1].parse::<f64>()) {
                            (Ok(low), Ok(high)) if low.is_finite() && high.is_finite() => {
                                filter.elevation = Some([low.min(high), low.max(high)]);
                                Ok(())
                            }
                            _ => Err(()),
                        }
                    }
                    "WITHHELD" if values.len() == 1 => flag_value(values.first())
                        .unwrap_or(Err(()))
                        .map(|value| filter.withheld = value),
                    "OVERLAP" if values.len() == 1 => flag_value(values.first())
                        .unwrap_or(Err(()))
                        .map(|value| filter.overlap = value),
                    "KEY" | "KEYPOINT" if values.len() == 1 => flag_value(values.first())
                        .unwrap_or(Err(()))
                        .map(|value| filter.key_point = value),
                    "SYNTHETIC" if values.len() == 1 => flag_value(values.first())
                        .unwrap_or(Err(()))
                        .map(|value| filter.synthetic = value),
                    _ => Err(()),
                };
                if result.is_ok() {
                    self.set_point_cloud_selection_filter(i, filter);
                } else {
                    self.command_line.push_error(
                        "Usage: POINTCLOUDSELECTFILTER <CLEAR|CLASS list|RETURN list|SOURCE list|ELEVATION low high|WITHHELD/OVERLAP/KEY/SYNTHETIC ON/OFF/ANY>",
                    );
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDCLASSIFYSELECTION ") => {
                match cmd
                    .trim_start_matches("POINTCLOUDCLASSIFYSELECTION")
                    .trim()
                    .parse::<u8>()
                {
                    Ok(class) => self.patch_point_cloud_selection(
                        i,
                        &format!("Assign class {class}"),
                        ocs_pointcloud::PointPatch::classification(class),
                    ),
                    Err(_) => self
                        .command_line
                        .push_error("Usage: POINTCLOUDCLASSIFYSELECTION <0-255 class>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDCLASSIFYSELECTION" => {
                use crate::command::ValuePromptCommand;
                let command = ValuePromptCommand::new(
                    "POINTCLOUDCLASSIFYSELECTION",
                    "POINTCLOUDCLASSIFYSELECTION  Enter class 0-255:",
                );
                self.command_line.push_info(&command.prompt());
                self.tabs[i].active_cmd = Some(Box::new(command));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDBRUSHCLASSIFY ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDBRUSHCLASSIFY")
                    .split_whitespace()
                    .collect();
                if values.len() == 1 {
                    match values[0].parse::<u8>() {
                        Ok(classification) => {
                            let command = crate::app::point_cloud::PointCloudBrushClassifyCommand::new(
                                classification,
                            );
                            self.command_line.push_info(&command.prompt());
                            self.tabs[i].active_cmd = Some(Box::new(command));
                        }
                        Err(_) => self.command_line.push_error(
                            "Usage: POINTCLOUDBRUSHCLASSIFY <class> <centerX centerY centerZ radius>",
                        ),
                    }
                } else if values.len() == 5 {
                    let classification = values[0].parse::<u8>();
                    let brush = values[1..]
                        .iter()
                        .map(|value| value.parse::<f64>())
                        .collect::<Result<Vec<_>, _>>();
                    match (classification, brush) {
                        (Ok(classification), Ok(brush)) if brush[3].is_finite() => {
                            self.point_cloud_select_brush(
                                i,
                                [brush[0], brush[1], brush[2]],
                                brush[3],
                            );
                            self.patch_point_cloud_selection(
                                i,
                                &format!("Brush assign class {classification}"),
                                ocs_pointcloud::PointPatch::classification(classification),
                            );
                        }
                        _ => self.command_line.push_error(
                            "Usage: POINTCLOUDBRUSHCLASSIFY <class> <centerX centerY centerZ radius>",
                        ),
                    }
                } else {
                    self.command_line.push_error(
                        "Usage: POINTCLOUDBRUSHCLASSIFY <class> <centerX centerY centerZ radius>",
                    );
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDABOVELINE")
                || cmd.starts_with("POINTCLOUDBELOWLINE") =>
            {
                self.command_line.push_info(
                    "This imported profile-view key-in is preserved, but above/below-screen-line classification is not connected yet. Use 3D fence/brush selection for this build.",
                );
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDFLAGSELECTION ") => {
                let values: Vec<_> = cmd
                    .trim_start_matches("POINTCLOUDFLAGSELECTION")
                    .split_whitespace()
                    .collect();
                let enabled = values.get(1).and_then(|value| match *value {
                    "ON" | "TRUE" | "1" => Some(true),
                    "OFF" | "FALSE" | "0" => Some(false),
                    _ => None,
                });
                let patch = values.first().zip(enabled).and_then(|(flag, enabled)| {
                    let mut patch = ocs_pointcloud::PointPatch::default();
                    match *flag {
                        "WITHHELD" => patch.withheld = Some(enabled),
                        "OVERLAP" => patch.overlap = Some(enabled),
                        "KEY" | "KEYPOINT" => patch.key_point = Some(enabled),
                        "SYNTHETIC" => patch.synthetic = Some(enabled),
                        _ => return None,
                    }
                    Some(patch)
                });
                if let Some(patch) = patch {
                    self.patch_point_cloud_selection(i, "Change point flag", patch);
                } else {
                    self.command_line.push_error(
                        "Usage: POINTCLOUDFLAGSELECTION <WITHHELD|OVERLAP|KEY|SYNTHETIC> <ON|OFF>",
                    );
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDELEVATIONSELECTION ") => {
                match cmd
                    .trim_start_matches("POINTCLOUDELEVATIONSELECTION")
                    .trim()
                    .parse::<f64>()
                {
                    Ok(elevation) if elevation.is_finite() => self.patch_point_cloud_selection(
                        i,
                        &format!("Set elevation {elevation}"),
                        ocs_pointcloud::PointPatch {
                            elevation: Some(elevation),
                            ..Default::default()
                        },
                    ),
                    _ => self
                        .command_line
                        .push_error("Usage: POINTCLOUDELEVATIONSELECTION <survey elevation>"),
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDPTCIMPORT ") => {
                let path = cmd.trim_start_matches("POINTCLOUDPTCIMPORT").trim();
                self.import_point_cloud_ptc(i, std::path::PathBuf::from(path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDPTCIMPORT" => {
                return Some(Task::done(Message::PointCloudPtcImport));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDPTCEXPORT ") => {
                let path = cmd.trim_start_matches("POINTCLOUDPTCEXPORT").trim();
                self.export_point_cloud_ptc(i, std::path::PathBuf::from(path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDPTCEXPORT" => {
                return Some(Task::done(Message::PointCloudPtcExport));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("MNUIMPORT ") => {
                let path = cmd.trim_start_matches("MNUIMPORT").trim();
                self.import_function_key_mnu(std::path::PathBuf::from(path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "MNUIMPORT" => {
                return Some(Task::done(Message::MnuImport));
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("MNUEXPORT ") => {
                let path = cmd.trim_start_matches("MNUEXPORT").trim();
                self.export_function_key_mnu(std::path::PathBuf::from(path));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "MNUEXPORT" => {
                return Some(Task::done(Message::MnuExport));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDDETACH" => {
                self.detach_point_cloud(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDDENSITY" => {
                let desc = match self.tabs[i].point_cloud.display.density {
                    ocs_pointcloud::Density::Auto => "Auto".to_string(),
                    ocs_pointcloud::Density::EveryNth(n) => format!("1-in-{n}"),
                    ocs_pointcloud::Density::Full => "Full".to_string(),
                };
                self.command_line.push_info(
                    format!(
                        "POINTCLOUDDENSITY: current density is {desc}. Usage: POINTCLOUDDENSITY <AUTO|N|FULL>"
                    )
                    .as_str(),
                );
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDDENSITY ") => {
                let arg = cmd.trim_start_matches("POINTCLOUDDENSITY").trim();
                let density = match arg.to_ascii_uppercase().as_str() {
                    "AUTO" | "DEFAULT" => Some(ocs_pointcloud::Density::Auto),
                    "FULL" | "ALL" => Some(ocs_pointcloud::Density::Full),
                    _ => arg
                        .parse::<u64>()
                        .ok()
                        .filter(|n| *n >= 1)
                        .map(ocs_pointcloud::Density::EveryNth),
                };
                if let Some(density) = density {
                    return Some(self.set_point_cloud_density(i, density));
                }
                self.command_line
                    .push_error("Usage: POINTCLOUDDENSITY <AUTO|N|FULL>");
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDEXPORT" => {
                return Some(Task::done(Message::PointCloudExport));
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDEXPORTSTATUS" => {
                self.point_cloud_export_status(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            "POINTCLOUDEXPORTCANCEL" => {
                self.cancel_point_cloud_export(i);
            }

            #[cfg(not(target_arch = "wasm32"))]
            cmd if cmd.starts_with("POINTCLOUDEXPORT ") => {
                let path = cmd.trim_start_matches("POINTCLOUDEXPORT").trim();
                if path.is_empty() {
                    return Some(Task::done(Message::PointCloudExport));
                }
                return Some(self.start_point_cloud_export(std::path::PathBuf::from(path)));
            }

            "SYNCPVIEWPORTS" | "UNDERLAYLAYERS" | "UOSNAP" => {
                self.command_line
                    .push_info(crate::tf!("{cmd}: not yet implemented.").as_ref());
            }

            _ => return None,
        }
        Some(self.finish_dispatch(cmd))
    }
}

// Scan LandXML text for <CgPoint> survey points. Each element's text content is
// "northing easting elevation"; returned as [easting, northing, elevation] so it
// maps to a Point at (X=easting, Y=northing, Z=elevation). Tolerant manual scan
// (no XML dependency); handles the standard text-content form.
// (landxml cgpoint scan)
fn parse_landxml_cgpoints(xml: &str) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<CgPoint") {
        let after = &rest[open + "<CgPoint".len()..];
        // Skip the container element "<CgPoints>".
        if !matches!(
            after.chars().next(),
            Some(' ') | Some('>') | Some('\t') | Some('\n') | Some('\r')
        ) {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let body = &after[gt + 1..];
        let Some(close) = body.find("</CgPoint>") else {
            break;
        };
        let text = &body[..close];
        let nums: Vec<f64> = text
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() >= 3 {
            out.push([nums[1], nums[0], nums[2]]);
        }
        rest = &body[close + "</CgPoint>".len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::app::OpenCADStudio;

    fn fresh_app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app
    }

    #[test]
    fn redraw_requests_and_leaves_geometry_untouched() {
        let mut app = fresh_app();
        let i = app.active_tab;
        let geom_before = app.tabs[i].scene.geometry_epoch;
        let block_before = app.tabs[i].scene.block_epoch;
        let _ = app.run_command_line("REDRAW");
        assert!(
            app.tabs[i].scene.refresh_pending_any(),
            "REDRAW must leave a pending force request"
        );
        assert_eq!(
            app.tabs[i].scene.geometry_epoch, geom_before,
            "REDRAW must not regen"
        );
        assert_eq!(
            app.tabs[i].scene.block_epoch, block_before,
            "REDRAW must not regen blocks"
        );
    }

    #[test]
    fn aliases_route_like_full_verbs() {
        let mut full = fresh_app();
        let mut short = fresh_app();
        let _ = full.run_command_line("REDRAW");
        let _ = short.run_command_line("R");
        let i = full.active_tab;
        assert!(
            short.tabs[i].scene.refresh_pending_any(),
            "'R' must trigger REDRAW"
        );
        assert_eq!(
            short.tabs[i].scene.refresh_pending_any(),
            full.tabs[i].scene.refresh_pending_any(),
            "'R' and 'REDRAW' must leave the same refresh state"
        );
    }

    #[test]
    fn redrawall_marks_all_tiles() {
        let mut app = fresh_app();
        let i = app.active_tab;
        let _ = app.run_command_line("REDRAWALL");
        assert!(
            app.tabs[i].scene.refresh_pending_any(),
            "REDRAWALL must leave a force request"
        );
    }

    #[test]
    fn regen_rebuilds_but_does_not_dirty_document() {
        let mut app = fresh_app();
        let i = app.active_tab;
        let geom_before = app.tabs[i].scene.geometry_epoch;
        let block_before = app.tabs[i].scene.block_epoch;
        app.tabs[i].dirty = false;
        let _ = app.run_command_line("REGEN");
        assert_ne!(
            app.tabs[i].scene.geometry_epoch, geom_before,
            "REGEN must regenerate geometry"
        );
        assert_ne!(
            app.tabs[i].scene.block_epoch, block_before,
            "REGEN must regenerate block epoch"
        );
        assert!(
            !app.tabs[i].dirty,
            "REGEN must NOT mark the document as modified (no DB change)"
        );
        let _ = app.run_command_line("REGENALL");
        assert!(
            !app.tabs[i].dirty,
            "REGENALL must not dirty the document either"
        );
    }
}
