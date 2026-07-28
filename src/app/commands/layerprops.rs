use super::*;

impl OpenCADStudio {
    pub(super) fn dispatch_layerprops(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        match cmd {
            // ── LAYER management ─────────────────────────────────────────
            cmd if cmd == "LAYER" || cmd.starts_with("LAYER ") || cmd.starts_with("LA ") => {
                use acadrust::tables::Layer;
                let raw_rest = if cmd.starts_with("LAYER ") {
                    cmd.trim_start_matches("LAYER ").trim()
                } else if cmd.starts_with("LA ") {
                    cmd.trim_start_matches("LA ").trim()
                } else {
                    ""
                };
                let parts: Vec<&str> = raw_rest.split_whitespace().collect();
                let sub = parts.get(0).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "LIST" | "?" => {
                        let info: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .layers
                            .iter()
                            .map(|l| {
                                let state = if l.flags.frozen {
                                    "frozen"
                                } else if l.flags.off {
                                    "off"
                                } else if l.flags.locked {
                                    "locked"
                                } else {
                                    "on"
                                };
                                format!("{}({})", l.name, state)
                            })
                            .collect();
                        self.command_line
                            .push_output(&format!("Layers: {}", info.join(", ")));
                    }
                    "NEW" | "N" => {
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: LAYER NEW <name>");
                        } else if self.tabs[i].scene.document.layers.contains(&name) {
                            self.command_line
                                .push_error(&format!("LAYER: '{}' already exists.", name));
                        } else {
                            let mut layer = Layer::new(&name);
                            // Allocate a unique handle so the layer survives a
                            // DWG save (handle-based format; issue #67).
                            layer.handle = self.tabs[i].scene.document.allocate_handle();
                            let _ = self.tabs[i].scene.document.layers.add(layer);
                            self.push_undo_snapshot(i, "LAYER NEW");
                            self.tabs[i].dirty = true;
                            self.command_line
                                .push_output(&format!("LAYER: '{}' created.", name));
                        }
                    }
                    "ON" => {
                        for name in &parts[1..] {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(name) {
                                l.flags.off = false;
                                l.flags.frozen = false;
                            }
                        }
                        self.push_undo_snapshot(i, "LAYER ON");
                        self.tabs[i].dirty = true;
                        self.command_line.push_output("LAYER: layers turned on.");
                    }
                    "OFF" => {
                        for name in &parts[1..] {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(name) {
                                l.flags.off = true;
                            }
                        }
                        self.push_undo_snapshot(i, "LAYER OFF");
                        self.tabs[i].dirty = true;
                        self.command_line.push_output("LAYER: layers turned off.");
                    }
                    "FREEZE" | "FR" => {
                        for name in &parts[1..] {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(name) {
                                l.flags.frozen = true;
                            }
                        }
                        self.push_undo_snapshot(i, "LAYER FREEZE");
                        self.tabs[i].dirty = true;
                        self.command_line.push_output("LAYER: layers frozen.");
                    }
                    "THAW" | "TH" => {
                        for name in &parts[1..] {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(name) {
                                l.flags.frozen = false;
                            }
                        }
                        self.push_undo_snapshot(i, "LAYER THAW");
                        self.tabs[i].dirty = true;
                        self.command_line.push_output("LAYER: layers thawed.");
                    }
                    "LOCK" | "LO" => {
                        for name in &parts[1..] {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(name) {
                                l.flags.locked = true;
                            }
                        }
                        self.push_undo_snapshot(i, "LAYER LOCK");
                        self.tabs[i].dirty = true;
                        self.command_line.push_output("LAYER: layers locked.");
                    }
                    "UNLOCK" | "UL" => {
                        for name in &parts[1..] {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(name) {
                                l.flags.locked = false;
                            }
                        }
                        self.push_undo_snapshot(i, "LAYER UNLOCK");
                        self.tabs[i].dirty = true;
                        self.command_line.push_output("LAYER: layers unlocked.");
                    }
                    "COLOR" | "C" => {
                        // LAYER COLOR <name> <aci_index>
                        let layer_name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        let color_str = parts.get(2).map(|s| s.trim()).unwrap_or("");
                        if let Ok(idx) = color_str.parse::<i16>() {
                            if let Some(l) = self.tabs[i].scene.document.layers.get_mut(&layer_name)
                            {
                                l.color = acadrust::types::Color::from_index(idx);
                                self.push_undo_snapshot(i, "LAYER COLOR");
                                self.tabs[i].dirty = true;
                                // By-layer colour is baked into every wire on
                                // this layer. The dependency index maps block
                                // children back to their top-level INSERTs, so
                                // unrelated entities stay warm.
                                self.tabs[i]
                                    .scene
                                    .invalidate_layer_dependencies(std::slice::from_ref(
                                        &layer_name,
                                    ));
                                self.command_line.push_output(&format!(
                                    "LAYER: '{}' color set to ACI {}.",
                                    layer_name, idx
                                ));
                            } else {
                                self.command_line
                                    .push_error(&format!("LAYER: '{}' not found.", layer_name));
                            }
                        } else {
                            self.command_line
                                .push_error("Usage: LAYER COLOR <name> <aci_index>");
                        }
                    }
                    "SET" | "S" | "CURRENT" => {
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        if self.tabs[i].scene.document.layers.contains(&name) {
                            self.tabs[i].layers.current_layer = name.clone();
                            self.command_line
                                .push_output(&format!("LAYER: current layer set to '{}'.", name));
                        } else {
                            self.command_line
                                .push_error(&format!("LAYER: '{}' not found.", name));
                        }
                    }
                    _ => {
                        self.command_line.push_info(
                            "Usage: LAYER LIST | NEW <name> | ON/OFF/FREEZE/THAW/LOCK/UNLOCK <name> | COLOR <name> <aci> | SET <name>"
                        );
                    }
                }
            }

            // Bare UCS → interactive front-end (option then value as steps), so
            // `UCS Z 90` is typable in the command line and works headlessly.
            // The front-end delegates back to the inline handler below. (#169)
            "UCS" => {
                use crate::modules::view::ucs_cmd::UcsCommand;
                let cmd_obj = UcsCommand::new();
                self.command_line.push_info(&cmd_obj.prompt());
                self.tabs[i].active_cmd = Some(Box::new(cmd_obj));
            }

            // ── UCS management (inline `UCS <option> …`) ─────────────────────
            cmd if cmd.starts_with("UCS ") => {
                use super::super::helpers::{ucs_rotated_z, ucs_to_wcs, ucs_z_axis};
                use acadrust::tables::Ucs;
                use acadrust::types::Vector3;
                let parts: Vec<&str> = cmd.splitn(4, ' ').collect();
                let sub = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "LIST" | "?" => {
                        let active_name = self.tabs[i]
                            .active_ucs
                            .as_ref()
                            .map(|u| u.name.clone())
                            .unwrap_or_else(|| "WCS".into());
                        let names: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .ucss
                            .iter()
                            .map(|u| u.name.clone())
                            .collect();
                        if names.is_empty() {
                            self.command_line.push_output(&format!(
                                "Active UCS: {}  |  No named UCSs defined.",
                                active_name
                            ));
                        } else {
                            self.command_line.push_output(&format!(
                                "Active UCS: {}  |  Named: {}",
                                active_name,
                                names.join(", ")
                            ));
                        }
                    }
                    "SAVE" | "S" => {
                        let name = parts.get(2).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: UCS SAVE <name>");
                        } else {
                            // Save the current active UCS under this name.
                            let ucs = match &self.tabs[i].active_ucs {
                                Some(u) => {
                                    let mut saved = u.clone();
                                    saved.name = name.clone();
                                    saved
                                }
                                None => Ucs::new(&name), // save WCS (identity)
                            };
                            self.tabs[i].scene.document.ucss.add_or_replace(ucs);
                            self.tabs[i].dirty = true;
                            self.command_line
                                .push_output(&format!("UCS '{}' saved.", name));
                        }
                    }
                    "DELETE" | "DEL" | "D" => {
                        let name = parts.get(2).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: UCS DELETE <name>");
                        } else if self.tabs[i].scene.document.ucss.remove(&name).is_some() {
                            self.tabs[i].dirty = true;
                            self.command_line
                                .push_output(&format!("UCS '{}' deleted.", name));
                        } else {
                            self.command_line
                                .push_error(&format!("UCS '{}' not found.", name));
                        }
                    }
                    "W" | "WORLD" => {
                        self.tabs[i].active_ucs = None;
                        self.command_line
                            .push_output("UCS reset to World Coordinate System.");
                    }
                    // UCS ORIGIN x,y,z  — shift the active UCS origin, keep axes
                    "ORIGIN" | "O" => {
                        let coord_str = parts.get(2).copied().unwrap_or("");
                        if let Some((pt, _)) = super::super::helpers::parse_coord(coord_str) {
                            // `pt` is in current UCS space; convert to WCS.
                            // The @/# relative-coordinate prefix is ignored
                            // here — a UCS origin is always absolute.
                            let wcs_origin = if let Some(ref ucs) = self.tabs[i].active_ucs {
                                ucs_to_wcs(pt, ucs)
                            } else {
                                pt
                            };
                            let ucs = self.tabs[i]
                                .active_ucs
                                .get_or_insert_with(|| Ucs::new("*ACTIVE*"));
                            ucs.origin = Vector3::new(
                                wcs_origin.x as f64,
                                wcs_origin.y as f64,
                                wcs_origin.z as f64,
                            );
                            self.command_line.push_output(&format!(
                                "UCS origin set to ({:.4}, {:.4}, {:.4}).",
                                wcs_origin.x, wcs_origin.y, wcs_origin.z
                            ));
                        } else {
                            self.command_line.push_error("Usage: UCS ORIGIN x,y,z");
                        }
                    }
                    // UCS Z angle  — rotate active UCS around its Z axis by degrees
                    "Z" => {
                        let deg: Option<f32> = parts.get(2).and_then(|s| s.trim().parse().ok());
                        if let Some(angle_deg) = deg {
                            let rad = angle_deg.to_radians();
                            let current = self.tabs[i].active_ucs.as_ref();
                            let origin = current
                                .map(|u| {
                                    glam::DVec3::new(u.origin.x, u.origin.y, u.origin.z)
                                })
                                .unwrap_or(glam::DVec3::ZERO);
                            let mut new_ucs = ucs_rotated_z(origin, rad);
                            // If already had axes, compose rotation on top
                            if let Some(ref ucs) = self.tabs[i].active_ucs {
                                let old_x = glam::Vec3::new(
                                    ucs.x_axis.x as f32,
                                    ucs.x_axis.y as f32,
                                    ucs.x_axis.z as f32,
                                );
                                let old_y = glam::Vec3::new(
                                    ucs.y_axis.x as f32,
                                    ucs.y_axis.y as f32,
                                    ucs.y_axis.z as f32,
                                );
                                let z_ax = ucs_z_axis(ucs).as_vec3();
                                let rot = glam::Quat::from_axis_angle(z_ax, rad);
                                let nx = rot * old_x;
                                let ny = rot * old_y;
                                new_ucs.x_axis =
                                    Vector3::new(nx.x as f64, nx.y as f64, nx.z as f64);
                                new_ucs.y_axis =
                                    Vector3::new(ny.x as f64, ny.y as f64, ny.z as f64);
                            }
                            self.tabs[i].active_ucs = Some(new_ucs);
                            self.command_line
                                .push_output(&format!("UCS rotated {:.2}° around Z.", angle_deg));
                        } else {
                            self.command_line.push_error("Usage: UCS Z <angle_degrees>");
                        }
                    }
                    // UCS X angle  — rotate around current UCS X axis
                    "X" => {
                        let deg: Option<f32> = parts.get(2).and_then(|s| s.trim().parse().ok());
                        if let Some(angle_deg) = deg {
                            let rad = angle_deg.to_radians();
                            let ucs = self.tabs[i]
                                .active_ucs
                                .get_or_insert_with(|| Ucs::new("*ACTIVE*"));
                            let x_ax = glam::Vec3::new(
                                ucs.x_axis.x as f32,
                                ucs.x_axis.y as f32,
                                ucs.x_axis.z as f32,
                            );
                            let old_y = glam::Vec3::new(
                                ucs.y_axis.x as f32,
                                ucs.y_axis.y as f32,
                                ucs.y_axis.z as f32,
                            );
                            let rot = glam::Quat::from_axis_angle(x_ax, rad);
                            let ny = rot * old_y;
                            ucs.y_axis = Vector3::new(ny.x as f64, ny.y as f64, ny.z as f64);
                            self.command_line
                                .push_output(&format!("UCS rotated {:.2}° around X.", angle_deg));
                        } else {
                            self.command_line.push_error("Usage: UCS X <angle_degrees>");
                        }
                    }
                    // UCS Y angle  — rotate around current UCS Y axis
                    "Y" => {
                        let deg: Option<f32> = parts.get(2).and_then(|s| s.trim().parse().ok());
                        if let Some(angle_deg) = deg {
                            let rad = angle_deg.to_radians();
                            let ucs = self.tabs[i]
                                .active_ucs
                                .get_or_insert_with(|| Ucs::new("*ACTIVE*"));
                            let y_ax = glam::Vec3::new(
                                ucs.y_axis.x as f32,
                                ucs.y_axis.y as f32,
                                ucs.y_axis.z as f32,
                            );
                            let old_x = glam::Vec3::new(
                                ucs.x_axis.x as f32,
                                ucs.x_axis.y as f32,
                                ucs.x_axis.z as f32,
                            );
                            let rot = glam::Quat::from_axis_angle(y_ax, rad);
                            let nx = rot * old_x;
                            ucs.x_axis = Vector3::new(nx.x as f64, nx.y as f64, nx.z as f64);
                            self.command_line
                                .push_output(&format!("UCS rotated {:.2}° around Y.", angle_deg));
                        } else {
                            self.command_line.push_error("Usage: UCS Y <angle_degrees>");
                        }
                    }
                    _ => {
                        // UCS <name> — activate a named UCS
                        let name = sub.clone();
                        if let Some(named) = self.tabs[i].scene.document.ucss.get(&name).cloned() {
                            self.tabs[i].active_ucs = Some(named);
                            self.command_line
                                .push_output(&format!("UCS '{}' activated.", name));
                        } else {
                            self.command_line.push_error(&format!(
                                "UCS '{}' not found.  Usage: UCS LIST | SAVE <name> | DELETE <name> | W | ORIGIN x,y,z | X/Y/Z <angle>",
                                name
                            ));
                        }
                    }
                }
                // Keep the scene's ViewCube UCS in lock-step with active_ucs and
                // persist it to the active pane (per-viewport UCS inside a
                // viewport, header model UCS in the Model tab) so it round-trips.
                self.tabs[i].sync_ucs_to_scene();
                self.tabs[i].persist_active_ucs();
            }

            // ── Named Views (VIEW command) ────────────────────────────────
            // PLAN — look straight down at the drawing (top view). The optional
            // World/Ucs/Current keyword is accepted; all map to the world top
            // view for now.
            // PLAN — plan view of a UCS (#326). Shows its options right on
            // activation instead of flipping the view like a cube click; the
            // view only changes once an option (or the Current default) runs.
            "PLAN" => {
                use crate::command::KeywordCommand;
                let c = KeywordCommand::new(
                    "PLAN",
                    "PLAN  [Current ucs / Ucs / World] <Current>:",
                    vec![
                        ("Current ucs", "CURRENT", None),
                        ("Ucs", "UCS", Some("PLAN UCS  ucs name:")),
                        ("World", "WORLD", None),
                    ],
                )
                .with_default("CURRENT");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("PLAN ") => {
                let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
                let sub = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "CURRENT" | "C" => {
                        let r = self.tabs[i].scene.viewcube_ucs_mat();
                        self.plan_snap(i, r);
                        self.command_line.push_output("PLAN: current UCS plan view.");
                    }
                    "WORLD" | "W" => {
                        self.plan_snap(i, glam::Mat4::IDENTITY);
                        self.command_line.push_output("PLAN: world plan view.");
                    }
                    "UCS" | "U" => {
                        let name = parts.get(2).map(|s| s.trim()).unwrap_or("");
                        if name.is_empty() {
                            self.command_line.push_info("Usage: PLAN UCS <name>");
                        } else {
                            let ucs = self.tabs[i]
                                .scene
                                .document
                                .ucss
                                .iter()
                                .find(|u| u.name.eq_ignore_ascii_case(name))
                                .cloned();
                            match ucs {
                                Some(u) => {
                                    let r = crate::app::helpers::UcsXform::from_ucs(&u)
                                        .rotation_mat();
                                    self.plan_snap(i, r);
                                    self.command_line.push_output(&format!(
                                        "PLAN: plan view of UCS '{}'.",
                                        u.name
                                    ));
                                }
                                None => self
                                    .command_line
                                    .push_error(&format!("UCS '{name}' not found.")),
                            }
                        }
                    }
                    _ => self
                        .command_line
                        .push_info("Usage: PLAN [CURRENT / UCS <name> / WORLD]"),
                }
            }

            "VIEW" => {
                use crate::command::KeywordCommand;
                let c = KeywordCommand::new(
                    "VIEW",
                    "VIEW  [List / Save / Restore / Delete]:",
                    vec![
                        ("List", "LIST", None),
                        ("Save", "SAVE", Some("VIEW SAVE  new view name:")),
                        ("Restore", "RESTORE", Some("VIEW RESTORE  view name:")),
                        ("Delete", "DELETE", Some("VIEW DELETE  view name:")),
                    ],
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("VIEW ") => {
                let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
                let sub = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "LIST" | "?" => {
                        let views: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .views
                            .iter()
                            .map(|v| v.name.clone())
                            .collect();
                        if views.is_empty() {
                            self.command_line.push_output("No named views saved.");
                        } else {
                            self.command_line
                                .push_output(&format!("Named views: {}", views.join(", ")));
                        }
                    }
                    "SAVE" | "S" => {
                        let name = parts.get(2).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: VIEW SAVE <name>");
                        } else {
                            let new_view = self.tabs[i].scene.current_as_named_view(&name);
                            self.tabs[i].scene.document.views.add_or_replace(new_view);
                            self.command_line
                                .push_output(&format!("View '{}' saved.", name));
                        }
                    }
                    "DELETE" | "DEL" | "D" => {
                        let name = parts.get(2).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: VIEW DELETE <name>");
                        } else {
                            if self.tabs[i].scene.document.views.remove(&name).is_some() {
                                self.command_line
                                    .push_output(&format!("View '{}' deleted.", name));
                            } else {
                                self.command_line
                                    .push_error(&format!("View '{}' not found.", name));
                            }
                        }
                    }
                    "RESTORE" | "R" => {
                        let name = parts.get(2).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: VIEW RESTORE <name>");
                        } else {
                            let found = self.tabs[i].scene.document.views.get(&name).cloned();
                            if let Some(v) = found {
                                self.tabs[i].scene.restore_named_view(&v);
                                self.command_line
                                    .push_output(&format!("View '{}' restored.", v.name));
                            } else {
                                self.command_line
                                    .push_error(&format!("View '{}' not found.", name));
                            }
                        }
                    }
                    // Standard orientation presets — snap the camera to a world
                    // axis view (these names take precedence over a same-named
                    // saved view, matching the standard orientation behaviour).
                    "TOP" | "FRONT" | "BACK" | "LEFT" | "RIGHT" | "BOTTOM" => {
                        use crate::scene::pipeline::viewcube::{
                            FACE_BACK, FACE_BOTTOM, FACE_FRONT, FACE_LEFT, FACE_RIGHT, FACE_TOP,
                        };
                        let face = match sub.as_str() {
                            "TOP" => FACE_TOP,
                            "BOTTOM" => FACE_BOTTOM,
                            "FRONT" => FACE_FRONT,
                            "BACK" => FACE_BACK,
                            "RIGHT" => FACE_RIGHT,
                            _ => FACE_LEFT,
                        };
                        return Some(Task::done(Message::ViewCubeSnapWorld(
                            crate::scene::CubeRegion::Face(face),
                        )));
                    }
                    "ISO" | "ISOMETRIC" | "SWISO" => {
                        return Some(Task::done(Message::ViewCubeHome));
                    }
                    // VIEW <name> shortcut for restore
                    _ => {
                        let name = sub.clone();
                        let found = self.tabs[i].scene.document.views.get(&name).cloned();
                        if let Some(v) = found {
                            self.tabs[i].scene.restore_named_view(&v);
                            self.command_line
                                .push_output(&format!("View '{}' restored.", v.name));
                        } else {
                            self.command_line.push_error(
                                "Usage: VIEW LIST | VIEW SAVE <name> | VIEW RESTORE <name> | VIEW DELETE <name>"
                            );
                        }
                    }
                }
            }

            // ── DimStyle management ───────────────────────────────────────
            // TABLESTYLE — Table Style Manager.
            cmd if cmd == "TABLESTYLE" || cmd == "TS" || cmd.starts_with("TABLESTYLE ") => {
                use acadrust::objects::{ObjectType, TableStyle};
                let raw_rest = cmd.split_once(' ').map(|(_, r)| r.trim()).unwrap_or("");
                let parts: Vec<&str> = raw_rest.split_whitespace().collect();
                let sub = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "DIALOG" | "UI" => {
                        return Some(Task::done(Message::TableStyleDialogOpen));
                    }
                    "LIST" | "?" => {
                        let doc = &self.tabs[i].scene.document;
                        let styles: Vec<String> = doc
                            .objects
                            .values()
                            .filter_map(|o| {
                                if let ObjectType::TableStyle(s) = o {
                                    Some(s)
                                } else {
                                    None
                                }
                            })
                            .map(|s| {
                                format!(
                                    "{}  (h_margin:{:.2} v_margin:{:.2})",
                                    s.name, s.horizontal_margin, s.vertical_margin
                                )
                            })
                            .collect();
                        if styles.is_empty() {
                            self.command_line.push_output("No table styles.");
                        } else {
                            self.command_line
                                .push_output(&format!("TableStyles:\n  {}", styles.join("\n  ")));
                        }
                    }
                    "NEW" | "N" => {
                        let name = parts.get(1).copied().unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: TABLESTYLE NEW <name>");
                        } else {
                            let doc = &self.tabs[i].scene.document;
                            let exists = doc.objects.values().any(|o| {
                                matches!(o, ObjectType::TableStyle(s) if s.name.eq_ignore_ascii_case(&name))
                            });
                            if exists {
                                self.command_line
                                    .push_error(&format!("TABLESTYLE: '{}' already exists.", name));
                            } else {
                                self.push_undo_snapshot(i, "TABLESTYLE NEW");
                                let mut style = TableStyle::standard();
                                style.name = name.clone();
                                let nh = acadrust::Handle::new(
                                    self.tabs[i].scene.document.next_handle(),
                                );
                                style.handle = nh;
                                self.tabs[i]
                                    .scene
                                    .document
                                    .objects
                                    .insert(nh, ObjectType::TableStyle(style));
                                self.tabs[i].dirty = true;
                                self.command_line
                                    .push_output(&format!("TABLESTYLE: '{}' created.", name));
                            }
                        }
                    }
                    _ => {
                        self.command_line
                            .push_error("Usage: TABLESTYLE [LIST|NEW <name>]");
                    }
                }
            }

            // MLSTYLE — Multiline Style Manager.
            // Usage:
            //   MLSTYLE                — open dialog
            //   MLSTYLE LIST / ?       — list all multiline styles
            //   MLSTYLE NEW <name>     — create a new style
            //   MLSTYLE SET <name>     — set current multiline style
            //   MLSTYLE DEL <name>     — delete a style (not Standard)
            cmd if cmd == "MLSTYLE" || cmd.starts_with("MLSTYLE ") => {
                use acadrust::objects::{MLineStyle, ObjectType};
                let raw_rest = cmd.split_once(' ').map(|(_, r)| r.trim()).unwrap_or("");
                let parts: Vec<&str> = raw_rest.split_whitespace().collect();
                let sub = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "DIALOG" | "UI" => {
                        return Some(Task::done(Message::MlStyleDialogOpen));
                    }
                    "LIST" | "?" => {
                        let doc = &self.tabs[i].scene.document;
                        let current = &doc.header.multiline_style;
                        let styles: Vec<String> = doc
                            .objects
                            .values()
                            .filter_map(|o| {
                                if let ObjectType::MLineStyle(s) = o {
                                    Some(s)
                                } else {
                                    None
                                }
                            })
                            .map(|s| {
                                let cur = if &s.name == current { " (current)" } else { "" };
                                format!("{}  [{}]{}", s.name, s.elements.len(), cur)
                            })
                            .collect();
                        if styles.is_empty() {
                            self.command_line.push_output("No multiline styles.");
                        } else {
                            self.command_line
                                .push_output(&format!("MLineStyles:\n  {}", styles.join("\n  ")));
                        }
                    }
                    "NEW" | "N" => {
                        let name = parts.get(1).copied().unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: MLSTYLE NEW <name>");
                        } else {
                            let doc = &self.tabs[i].scene.document;
                            let exists = doc.objects.values().any(|o| {
                                matches!(o, ObjectType::MLineStyle(s) if s.name.eq_ignore_ascii_case(&name))
                            });
                            if exists {
                                self.command_line
                                    .push_error(&format!("MLSTYLE: '{}' already exists.", name));
                            } else {
                                self.push_undo_snapshot(i, "MLSTYLE NEW");
                                let mut style = MLineStyle::standard();
                                style.name = name.clone();
                                let nh = acadrust::Handle::new(
                                    self.tabs[i].scene.document.next_handle(),
                                );
                                style.handle = nh;
                                self.tabs[i]
                                    .scene
                                    .document
                                    .objects
                                    .insert(nh, ObjectType::MLineStyle(style));
                                self.tabs[i].dirty = true;
                                self.command_line
                                    .push_output(&format!("MLSTYLE: '{}' created.", name));
                            }
                        }
                    }
                    "SET" | "S" => {
                        let name = parts.get(1).copied().unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: MLSTYLE SET <name>");
                        } else {
                            let doc = &self.tabs[i].scene.document;
                            let exists = doc.objects.values().any(|o| {
                                matches!(o, ObjectType::MLineStyle(s) if s.name.eq_ignore_ascii_case(&name))
                            });
                            if exists {
                                self.push_undo_snapshot(i, "MLSTYLE SET");
                                self.tabs[i].scene.document.header.multiline_style = name.clone();
                                self.tabs[i].dirty = true;
                                self.command_line.push_output(&format!(
                                    "MLSTYLE: current style set to '{}'.",
                                    name
                                ));
                            } else {
                                self.command_line
                                    .push_error(&format!("MLSTYLE: '{}' not found.", name));
                            }
                        }
                    }
                    "DEL" | "DELETE" => {
                        let name = parts.get(1).copied().unwrap_or("").to_string();
                        if name.is_empty() || name.eq_ignore_ascii_case("Standard") {
                            self.command_line
                                .push_error("Cannot delete the Standard style.");
                        } else {
                            let doc = &self.tabs[i].scene.document;
                            let handle = doc.objects.iter().find_map(|(&h, o)| {
                                if let ObjectType::MLineStyle(s) = o {
                                    if s.name.eq_ignore_ascii_case(&name) {
                                        Some(h)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            });
                            if let Some(h) = handle {
                                self.push_undo_snapshot(i, "MLSTYLE DEL");
                                self.tabs[i].scene.document.objects.remove(&h);
                                self.tabs[i].dirty = true;
                                self.command_line
                                    .push_output(&format!("MLSTYLE: '{}' deleted.", name));
                            } else {
                                self.command_line
                                    .push_error(&format!("MLSTYLE: '{}' not found.", name));
                            }
                        }
                    }
                    _ => {
                        self.command_line
                            .push_error("Usage: MLSTYLE [LIST|NEW <name>|SET <name>|DEL <name>]");
                    }
                }
            }

            cmd if cmd == "DIMSTYLE"
                || cmd == "DDIM"
                || cmd.starts_with("DIMSTYLE ")
                || cmd.starts_with("DDIM ") =>
            {
                use acadrust::tables::DimStyle;
                let raw_rest = cmd.split_once(' ').map(|(_, r)| r.trim()).unwrap_or("");
                let parts: Vec<&str> = raw_rest.split_whitespace().collect();
                let sub = parts.get(0).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    // No sub-command or "DIALOG" → open the DimStyle Manager dialog
                    "" | "DIALOG" | "UI" => {
                        return Some(Task::done(Message::DimStyleDialogOpen));
                    }
                    "LIST" | "?" => {
                        let styles: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .dim_styles
                            .iter()
                            .map(|s| format!("{}(txt:{:.2} asz:{:.2})", s.name, s.dimtxt, s.dimasz))
                            .collect();
                        if styles.is_empty() {
                            self.command_line.push_output("No dim styles defined.");
                        } else {
                            self.command_line
                                .push_output(&format!("DimStyles: {}", styles.join(", ")));
                        }
                    }
                    "NEW" | "N" => {
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_error("Usage: DIMSTYLE NEW <name>");
                        } else if self.tabs[i].scene.document.dim_styles.contains(&name) {
                            self.command_line
                                .push_error(&format!("DIMSTYLE: '{}' already exists.", name));
                        } else {
                            let undo = self.begin_dim_style_undo(
                                i,
                                "DIMSTYLE NEW",
                                std::slice::from_ref(&name),
                            );
                            let style = DimStyle::new(&name);
                            let _ = self.tabs[i].scene.document.dim_styles.add(style);
                            self.tabs[i].dirty = true;
                            self.commit_dim_style_undo(i, undo);
                            self.command_line
                                .push_output(&format!("DIMSTYLE: '{}' created.", name));
                        }
                    }
                    "SET" | "S" => {
                        // DIMSTYLE SET <name> <property> <value>
                        // e.g. DIMSTYLE SET Standard dimtxt 2.5
                        let style_name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        let prop = parts.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
                        let val_str = parts.get(3).map(|s| s.trim()).unwrap_or("");
                        if let Ok(val) = val_str.parse::<f64>() {
                            let undo = self.begin_dim_style_undo(
                                i,
                                "DIMSTYLE SET",
                                std::slice::from_ref(&style_name),
                            );
                            if let Some(ds) =
                                self.tabs[i].scene.document.dim_styles.get_mut(&style_name)
                            {
                                match prop.as_str() {
                                    "dimtxt" => {
                                        ds.dimtxt = val;
                                    }
                                    "dimasz" => {
                                        ds.dimasz = val;
                                    }
                                    "dimdli" => {
                                        ds.dimdli = val;
                                    }
                                    "dimexo" => {
                                        ds.dimexo = val;
                                    }
                                    "dimexe" => {
                                        ds.dimexe = val;
                                    }
                                    "dimgap" => {
                                        ds.dimgap = val;
                                    }
                                    "dimscale" => {
                                        ds.dimscale = val;
                                    }
                                    "dimlfac" => {
                                        ds.dimlfac = val;
                                    }
                                    "dimdle" => {
                                        ds.dimdle = val;
                                    }
                                    "dimtvp" => {
                                        ds.dimtvp = val;
                                    }
                                    "dimcen" => {
                                        ds.dimcen = val;
                                    }
                                    "dimtsz" => {
                                        ds.dimtsz = val;
                                    }
                                    "dimfxl" => {
                                        ds.dimfxl = val;
                                    }
                                    _ => {
                                        self.command_line.push_error(&format!(
                                            "DIMSTYLE: unknown property '{}'. Try: dimtxt dimasz dimdli dimexo dimexe dimgap dimscale dimlfac dimdle dimcen dimtsz", prop
                                        ));
                                        return Some(Task::none());
                                    }
                                }
                                self.tabs[i].dirty = true;
                                self.tabs[i].scene
                                    .invalidate_dim_style_dependencies(&style_name);
                                self.commit_dim_style_undo(i, undo);
                                self.command_line.push_output(&format!(
                                    "DIMSTYLE: '{style_name}'.{prop} = {val:.3}"
                                ));
                            } else {
                                self.command_line
                                    .push_error(&format!("DIMSTYLE: '{}' not found.", style_name));
                            }
                        } else {
                            self.command_line
                                .push_error("Usage: DIMSTYLE SET <name> <property> <value>");
                        }
                    }
                    _ => {
                        self.command_line.push_info(
                            "Usage: DIMSTYLE LIST | NEW <name> | SET <name> <prop> <val>",
                        );
                    }
                }
            }

            // ── MLeader Style management ──────────────────────────────────
            cmd if cmd == "MLEADERSTYLE" || cmd.starts_with("MLEADERSTYLE ") => {
                use acadrust::objects::{MultiLeaderStyle, ObjectType};
                let raw_rest = cmd.trim_start_matches("MLEADERSTYLE").trim();
                let parts: Vec<&str> = raw_rest.split_whitespace().collect();
                let sub = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "DIALOG" | "UI" => {
                        return Some(Task::done(Message::MLeaderStyleDialogOpen));
                    }
                    "LIST" | "?" => {
                        let styles: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .objects
                            .values()
                            .filter_map(|o| {
                                if let ObjectType::MultiLeaderStyle(s) = o {
                                    Some(format!(
                                        "{}(txt:{:.2} asz:{:.2})",
                                        s.name, s.text_height, s.arrowhead_size
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let current = &self.tabs[i].active_mleader_style;
                        if styles.is_empty() {
                            self.command_line
                                .push_output(&format!("MLeader styles: (none)  active: {current}"));
                        } else {
                            self.command_line.push_output(&format!(
                                "MLeader styles: {}  active: {current}",
                                styles.join(", ")
                            ));
                        }
                    }
                    "NEW" | "N" => {
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line
                                .push_error("Usage: MLEADERSTYLE NEW <name>");
                        } else {
                            let already_exists = self.tabs[i].scene.document.objects.values().any(
                                |o| matches!(o, ObjectType::MultiLeaderStyle(s) if s.name == name),
                            );
                            if already_exists {
                                self.command_line.push_error(&format!(
                                    "MLEADERSTYLE: '{}' already exists.",
                                    name
                                ));
                            } else {
                                let handle = self.tabs[i].scene.document.allocate_handle();
                                let mut style = MultiLeaderStyle::new(&name);
                                style.handle = handle;
                                self.tabs[i]
                                    .scene
                                    .document
                                    .objects
                                    .insert(handle, ObjectType::MultiLeaderStyle(style));
                                self.push_undo_snapshot(i, "MLEADERSTYLE NEW");
                                self.tabs[i].dirty = true;
                                self.command_line
                                    .push_output(&format!("MLEADERSTYLE: '{}' created.", name));
                            }
                        }
                    }
                    "SET" | "S" => {
                        // MLEADERSTYLE SET <name> <property> <value>
                        // Properties: text_height arrowhead_size landing_distance landing_gap
                        let style_name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        let prop = parts.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
                        let val_str = parts.get(3).map(|s| s.trim()).unwrap_or("");
                        if let Ok(val) = val_str.parse::<f64>() {
                            let style_entry = self.tabs[i]
                                .scene
                                .document
                                .objects
                                .values_mut()
                                .find_map(|o| {
                                    if let ObjectType::MultiLeaderStyle(s) = o {
                                        if s.name == style_name {
                                            Some(s)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                });
                            if let Some(s) = style_entry {
                                match prop.as_str() {
                                    "text_height" | "textheight" | "txth" => {
                                        s.text_height = val;
                                    }
                                    "arrowhead_size" | "arrowsize" | "asz" => {
                                        s.arrowhead_size = val;
                                    }
                                    "landing_distance" | "landing" | "dogleg" => {
                                        s.landing_distance = val;
                                    }
                                    "landing_gap" | "gap" => {
                                        s.landing_gap = val;
                                    }
                                    _ => {
                                        self.command_line.push_error(&format!(
                                            "MLEADERSTYLE: unknown property '{}'. Try: text_height arrowhead_size landing_distance landing_gap", prop
                                        ));
                                        return Some(Task::none());
                                    }
                                }
                                self.push_undo_snapshot(i, "MLEADERSTYLE SET");
                                self.tabs[i].dirty = true;
                                self.command_line.push_output(&format!(
                                    "MLEADERSTYLE: '{style_name}'.{prop} = {val:.3}"
                                ));
                            } else {
                                self.command_line.push_error(&format!(
                                    "MLEADERSTYLE: '{}' not found.",
                                    style_name
                                ));
                            }
                        } else {
                            self.command_line
                                .push_error("Usage: MLEADERSTYLE SET <name> <property> <value>");
                        }
                    }
                    "CURRENT" | "C" | "ACTIVE" => {
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line.push_output(&format!(
                                "Current MLeader style: {}",
                                self.tabs[i].active_mleader_style
                            ));
                        } else {
                            let exists = name == "Standard" || self.tabs[i].scene.document.objects.values()
                                .any(|o| matches!(o, ObjectType::MultiLeaderStyle(s) if s.name == name));
                            if exists {
                                self.tabs[i].active_mleader_style = name.clone();
                                self.command_line.push_output(&format!(
                                    "MLEADERSTYLE: current style set to '{name}'."
                                ));
                            } else {
                                self.command_line
                                    .push_error(&format!("MLEADERSTYLE: '{}' not found.", name));
                            }
                        }
                    }
                    _ => {
                        self.command_line.push_info(
                            "Usage: MLEADERSTYLE LIST | NEW <name> | SET <name> <prop> <val> | CURRENT [<name>]"
                        );
                    }
                }
            }

            // ── TextStyle / Style management ──────────────────────────────
            cmd if cmd == "STYLE"
                || cmd == "TEXTSTYLE"
                || cmd.starts_with("STYLE ")
                || cmd.starts_with("TEXTSTYLE ") =>
            {
                let (prefix, rest) = if cmd.starts_with("TEXTSTYLE") {
                    ("TEXTSTYLE", cmd.trim_start_matches("TEXTSTYLE").trim())
                } else {
                    ("STYLE", cmd.trim_start_matches("STYLE").trim())
                };
                let parts: Vec<&str> = rest.splitn(3, ' ').collect();
                let sub = parts.get(0).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "DIALOG" | "UI" => {
                        return Some(Task::done(Message::TextStyleDialogOpen));
                    }
                    "LIST" | "?" => {
                        let styles: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .text_styles
                            .iter()
                            .map(|s| {
                                format!(
                                    "{} (font: {}, w: {:.2}, oblique: {:.1}°)",
                                    s.name,
                                    s.font_file,
                                    s.width_factor,
                                    s.oblique_angle.to_degrees()
                                )
                            })
                            .collect();
                        if styles.is_empty() {
                            self.command_line.push_output("No text styles defined.");
                        } else {
                            self.command_line
                                .push_output(&format!("Text styles: {}", styles.join(" | ")));
                        }
                    }
                    "SET" | "S" => {
                        // STYLE SET <name> — set active text style (for future text commands)
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("");
                        if self.tabs[i].scene.document.text_styles.get(name).is_some() {
                            self.command_line
                                .push_output(&format!("{prefix}: active style set to '{name}'."));
                        } else {
                            self.command_line
                                .push_error(&format!("{prefix}: style '{name}' not found."));
                        }
                    }
                    "NEW" | "N" => {
                        let name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        if name.is_empty() {
                            self.command_line
                                .push_error(&format!("Usage: {prefix} NEW <name>"));
                        } else if self.tabs[i].scene.document.text_styles.contains(&name) {
                            self.command_line
                                .push_error(&format!("{prefix}: style '{name}' already exists."));
                        } else {
                            let undo = self.begin_text_style_undo(
                                i,
                                "STYLE NEW",
                                std::slice::from_ref(&name),
                            );
                            let style = acadrust::tables::TextStyle::new(&name);
                            let _ = self.tabs[i].scene.document.text_styles.add(style);
                            self.tabs[i].dirty = true;
                            self.commit_text_style_undo(i, undo);
                            self.command_line
                                .push_output(&format!("{prefix}: style '{name}' created."));
                        }
                    }
                    "FONT" | "F" => {
                        // STYLE FONT <name> <font_file>
                        let style_name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        let font = parts.get(2).map(|s| s.trim()).unwrap_or("").to_string();
                        if style_name.is_empty() || font.is_empty() {
                            self.command_line
                                .push_error(&format!("Usage: {prefix} FONT <style> <font_file>"));
                        } else {
                            let undo = self.begin_text_style_undo(
                                i,
                                "STYLE FONT",
                                std::slice::from_ref(&style_name),
                            );
                            if let Some(style) =
                            self.tabs[i].scene.document.text_styles.get_mut(&style_name)
                        {
                                style.font_file = font.clone();
                            self.tabs[i].dirty = true;
                                self.tabs[i]
                                    .scene
                                    .invalidate_text_style_dependencies(&style_name);
                                self.commit_text_style_undo(i, undo);
                            self.command_line.push_output(&format!(
                                "{prefix}: '{style_name}' font set to '{font}'."
                            ));
                        } else {
                            self.command_line
                                .push_error(&format!("{prefix}: style '{style_name}' not found."));
                            }
                        }
                    }
                    "WIDTH" | "W" => {
                        // STYLE WIDTH <name> <factor>
                        let style_name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        let factor_str = parts.get(2).map(|s| s.trim()).unwrap_or("");
                        if let Ok(factor) = factor_str.parse::<f64>() {
                            let undo = self.begin_text_style_undo(
                                i,
                                "STYLE WIDTH",
                                std::slice::from_ref(&style_name),
                            );
                            if let Some(style) =
                                self.tabs[i].scene.document.text_styles.get_mut(&style_name)
                            {
                                style.width_factor = factor;
                                self.tabs[i].dirty = true;
                                self.tabs[i]
                                    .scene
                                    .invalidate_text_style_dependencies(&style_name);
                                self.commit_text_style_undo(i, undo);
                                self.command_line.push_output(&format!(
                                    "{prefix}: '{style_name}' width factor set to {factor:.3}."
                                ));
                            } else {
                                self.command_line.push_error(&format!(
                                    "{prefix}: style '{style_name}' not found."
                                ));
                            }
                        } else {
                            self.command_line
                                .push_error(&format!("Usage: {prefix} WIDTH <style> <factor>"));
                        }
                    }
                    "OBLIQUE" => {
                        // STYLE OBLIQUE <name> <angle_degrees>
                        let style_name = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
                        let angle_str = parts.get(2).map(|s| s.trim()).unwrap_or("");
                        if let Ok(deg) = angle_str.parse::<f64>() {
                            let undo = self.begin_text_style_undo(
                                i,
                                "STYLE OBLIQUE",
                                std::slice::from_ref(&style_name),
                            );
                            if let Some(style) =
                                self.tabs[i].scene.document.text_styles.get_mut(&style_name)
                            {
                                style.oblique_angle = deg.to_radians();
                                self.tabs[i].dirty = true;
                                self.tabs[i]
                                    .scene
                                    .invalidate_text_style_dependencies(&style_name);
                                self.commit_text_style_undo(i, undo);
                                self.command_line.push_output(&format!(
                                    "{prefix}: '{style_name}' oblique angle set to {deg:.1}°."
                                ));
                            } else {
                                self.command_line.push_error(&format!(
                                    "{prefix}: style '{style_name}' not found."
                                ));
                            }
                        } else {
                            self.command_line.push_error(&format!(
                                "Usage: {prefix} OBLIQUE <style> <angle_degrees>"
                            ));
                        }
                    }
                    _ => {
                        self.command_line.push_info(&format!(
                            "Usage: {prefix} LIST | NEW <name> | FONT <style> <file> | WIDTH <style> <factor> | OBLIQUE <style> <angle>"
                        ));
                    }
                }
            }

            _ => return None,
        }
        Some(self.finish_dispatch(cmd))
    }

    /// Snap to the plan view of the UCS whose rotation is `r_ucs`, straight —
    /// no "already there → flip to the opposite face" cube behaviour (#326).
    fn plan_snap(&mut self, i: usize, r_ucs: glam::Mat4) {
        let eye_dir = r_ucs.transform_vector3(
            crate::scene::CubeRegion::Face(crate::scene::pipeline::viewcube::FACE_TOP)
                .snap_direction(),
        );
        if self.tabs[i].scene.active_viewport.is_some() {
            self.tabs[i]
                .scene
                .mutate_active_viewport_camera(|c| c.snap_to_face(eye_dir, r_ucs));
        } else {
            self.tabs[i]
                .scene
                .camera
                .borrow_mut()
                .snap_to_face(eye_dir, r_ucs);
        }
        self.tabs[i].scene.camera_generation += 1;
    }
}
