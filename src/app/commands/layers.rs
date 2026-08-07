use super::*;

impl OpenCADStudio {
    pub(super) fn dispatch_layers(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        match cmd {
            // ── Layer object commands ──────────────────────────────────────
            "LAYOFF" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                if handles.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let cmd = SelectObjectsCommand::new("LAYOFF");
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let layers: rustc_hash::FxHashSet<String> = self.tabs[i]
                        .scene
                        .selected_entities()
                        .into_iter()
                        .map(|(_, e)| e.common().layer.clone())
                        .collect();
                    let names: Vec<String> = layers.iter().cloned().collect();
                    let undo = self.begin_layer_undo(i, "LAYOFF", &names);
                    for name in &layers {
                        if name == "0" {
                            continue;
                        }
                        if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(name) {
                            dl.turn_off();
                        }
                    }
                    self.tabs[i].scene.invalidate_layer_dependencies(&names);
                    self.tabs[i].dirty = true;
                    self.commit_layer_undo(i, undo);
                    self.refresh_layer_panel();
                    self.command_line.push_info(crate::t!("Layer(s) turned off.").as_ref());
                }
            }

            "LAYFRZ" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                if handles.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let cmd = SelectObjectsCommand::new("LAYFRZ");
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let layers: rustc_hash::FxHashSet<String> = self.tabs[i]
                        .scene
                        .selected_entities()
                        .into_iter()
                        .map(|(_, e)| e.common().layer.clone())
                        .collect();
                    let names: Vec<String> = layers.iter().cloned().collect();
                    let undo = self.begin_layer_undo(i, "LAYFRZ", &names);
                    for name in &layers {
                        if name == "0" {
                            continue;
                        }
                        if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(name) {
                            dl.freeze();
                        }
                    }
                    self.tabs[i].scene.invalidate_layer_dependencies(&names);
                    self.tabs[i].dirty = true;
                    self.commit_layer_undo(i, undo);
                    self.refresh_layer_panel();
                    self.command_line.push_info(crate::t!("Layer(s) frozen.").as_ref());
                }
            }

            // LAYTRANS — translate this drawing's layers onto a set taken from
            // another drawing. Bare opens the mapping dialog; with a file it
            // maps every name the two drawings share and translates at once,
            // which is what most drawings coming from outside need. (#624)
            "LAYTRANS" => {
                return Some(self.open_layer_translator(None));
            }
            cmd if cmd.starts_with("LAYTRANS ") => {
                let path = std::path::PathBuf::from(cmd.trim_start_matches("LAYTRANS").trim());
                return Some(self.layer_translate_same(&path));
            }

            // LAYDEL <name> — delete a layer and erase the objects on it.
            "LAYDEL" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new("LAYDEL", "LAYDEL  layer to delete:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("LAYDEL ") => {
                let name = cmd.trim_start_matches("LAYDEL").trim();
                if name.is_empty() {
                    self.command_line.push_info(crate::t!("Usage: LAYDEL <layer name>").as_ref());
                    return Some(Task::none());
                }
                let resolved = self.tabs[i]
                    .scene
                    .document
                    .layers
                    .names()
                    .find(|k| k.eq_ignore_ascii_case(name))
                    .map(|s| s.to_string());
                let Some(layer) = resolved else {
                    self.command_line
                        .push_error(crate::tf!("LAYDEL: no layer named \"{name}\".").as_ref());
                    return Some(Task::none());
                };
                if layer == "0" {
                    self.command_line
                        .push_error(crate::t!("LAYDEL: layer \"0\" cannot be deleted.").as_ref());
                    return Some(Task::none());
                }
                if layer.eq_ignore_ascii_case(&self.tabs[i].active_layer) {
                    self.command_line.push_error(
                        "LAYDEL: cannot delete the current layer. Make another layer current first.",
                    );
                    return Some(Task::none());
                }
                let handles: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .document
                    .entities()
                    .filter(|e| e.common().layer == layer)
                    .map(|e| e.common().handle)
                    .collect();
                self.push_undo_snapshot(i, "LAYDEL");
                let n = handles.len();
                if !handles.is_empty() {
                    self.tabs[i].scene.erase_entities(&handles);
                }
                self.tabs[i].scene.document.layers.remove(&layer);
                self.tabs[i].scene.bump_geometry();
                self.tabs[i].dirty = true;
                self.refresh_layer_panel();
                self.command_line.push_output(crate::tf!(
                    "LAYDEL: deleted layer \"{layer}\" and {n} object(s)."
                ).as_ref());
            }

            // LAYMRG <source> <target> — move every object from <source> onto
            // <target>, then delete the emptied <source> layer.
            "LAYMRG" => {
                use crate::command::TwoValuePromptCommand;
                let c = TwoValuePromptCommand::new(
                    "LAYMRG",
                    "LAYMRG  source layer (merged away):",
                    "LAYMRG  target layer (kept):",
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("LAYMRG ") => {
                let rest = cmd.trim_start_matches("LAYMRG").trim();
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() != 2 {
                    self.command_line
                        .push_info(crate::t!("Usage: LAYMRG <source layer> <target layer>").as_ref());
                    return Some(Task::none());
                }
                let keys: Vec<String> = self.tabs[i]
                    .scene
                    .document
                    .layers
                    .names()
                    .map(|s| s.to_string())
                    .collect();
                let src = keys
                    .iter()
                    .find(|k| k.eq_ignore_ascii_case(parts[0]))
                    .cloned();
                let dst = keys
                    .iter()
                    .find(|k| k.eq_ignore_ascii_case(parts[1]))
                    .cloned();
                let (Some(src), Some(dst)) = (src, dst) else {
                    self.command_line
                        .push_error(crate::t!("LAYMRG: source and target layers must both exist.").as_ref());
                    return Some(Task::none());
                };
                if src == dst {
                    self.command_line
                        .push_error(crate::t!("LAYMRG: source and target are the same layer.").as_ref());
                    return Some(Task::none());
                }
                if src == "0" {
                    self.command_line
                        .push_error(crate::t!("LAYMRG: layer \"0\" cannot be merged away.").as_ref());
                    return Some(Task::none());
                }
                if src.eq_ignore_ascii_case(&self.tabs[i].active_layer) {
                    self.command_line.push_error(
                        "LAYMRG: cannot merge the current layer. Make another layer current first.",
                    );
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "LAYMRG");
                // The same move a translation makes for each of its pairs.
                let moved = crate::modules::draw::layers::laytrans::merge_layer(
                    &mut self.tabs[i].scene,
                    &src,
                    &dst,
                    false,
                );
                self.tabs[i].scene.invalidate_dependency_index();
                self.tabs[i]
                    .scene
                    .invalidate_layer_dependencies(std::slice::from_ref(&dst));
                self.tabs[i].dirty = true;
                self.refresh_layer_panel();
                self.command_line.push_output(crate::tf!(
                    "LAYMRG: merged \"{src}\" into \"{dst}\" ({moved} object(s))."
                ).as_ref());
            }

            // LAYERSTATE — save / restore named snapshots of all layer states
            // in the drawing's native ACAD_LAYERSTATES dictionary.
            // LAYERSTATE SAVE <name> | RESTORE <name> | DELETE <name> | ? (list)
            "LAYERSTATE" | "LAS" | "LMAN" => {
                return Some(Task::done(Message::LayerStateManagerOpen));
            }
            cmd if cmd.starts_with("LAYERSTATE ")
                || cmd.starts_with("LAS ")
                || cmd.starts_with("LMAN ") =>
            {
                let rest = cmd
                    .trim_start_matches("LAYERSTATE")
                    .trim_start_matches("LMAN")
                    .trim_start_matches("LAS")
                    .trim();
                let mut parts = rest.splitn(2, char::is_whitespace);
                let sub = parts.next().unwrap_or("").to_uppercase();
                let arg = parts.next().unwrap_or("").trim();
                match sub.as_str() {
                    "" | "?" | "LIST" => {
                        let states = self.tabs[i].scene.document.layer_states();
                        if states.is_empty() {
                            self.command_line.push_info(
                                "LAYERSTATE: no saved states. Use LAYERSTATE SAVE <name>.",
                            );
                        } else {
                            let mut names: Vec<&str> =
                                states.iter().map(|state| state.name.as_str()).collect();
                            names.sort_unstable();
                            self.command_line
                                .push_output(crate::tf!("Saved layer states: {}", names.join(", ")).as_ref());
                        }
                    }
                    "SAVE" | "S" => {
                        if arg.is_empty() {
                            self.command_line.push_info(crate::t!("Usage: LAYERSTATE SAVE <name>").as_ref());
                        } else {
                            let description = self.tabs[i]
                                .scene
                                .document
                                .layer_state(arg)
                                .map(|state| state.description)
                                .unwrap_or_default();
                            self.push_undo_snapshot(i, "LAYERSTATE SAVE");
                            self.tabs[i]
                                .scene
                                .document
                                .capture_layer_state(arg, description);
                            self.tabs[i].dirty = true;
                            self.command_line
                                .push_output(crate::tf!("LAYERSTATE: saved \"{arg}\".").as_ref());
                        }
                    }
                    "RESTORE" | "R" => {
                        if arg.is_empty() {
                            self.command_line
                                .push_info(crate::t!("Usage: LAYERSTATE RESTORE <name>").as_ref());
                        } else if self.tabs[i].scene.document.layer_state(arg).is_none() {
                            self.command_line.push_error(crate::tf!(
                                "LAYERSTATE: no saved state named \"{arg}\"."
                            ).as_ref());
                        } else {
                            let names: Vec<String> = self.tabs[i]
                                .scene
                                .document
                                .layers
                                .iter()
                                .map(|layer| layer.name.clone())
                                .collect();
                            self.push_undo_snapshot(i, "LAYERSTATE RESTORE");
                            let n = self.tabs[i]
                                .scene
                                .document
                                .restore_layer_state(arg)
                                .unwrap_or(0);
                            self.tabs[i].active_layer = self.tabs[i]
                                .scene
                                .document
                                .header
                                .current_layer_name
                                .clone();
                            self.tabs[i].scene.invalidate_layer_dependencies(&names);
                            self.tabs[i].dirty = true;
                            self.refresh_layer_panel();
                            self.command_line.push_output(crate::tf!(
                                "LAYERSTATE: restored \"{arg}\" ({n} layer(s))."
                            ).as_ref());
                        }
                    }
                    "DELETE" | "D" => {
                        if arg.is_empty() {
                            self.command_line
                                .push_info(crate::t!("Usage: LAYERSTATE DELETE <name>").as_ref());
                        } else if self.tabs[i].scene.document.layer_state(arg).is_none() {
                            self.command_line.push_error(crate::tf!(
                                "LAYERSTATE: no saved state named \"{arg}\"."
                            ).as_ref());
                        } else {
                            self.push_undo_snapshot(i, "LAYERSTATE DELETE");
                            self.tabs[i].scene.document.delete_layer_state(arg);
                            self.tabs[i].dirty = true;
                            self.command_line
                                .push_output(crate::tf!("LAYERSTATE: deleted \"{arg}\".").as_ref());
                        }
                    }
                    _ => {
                        self.command_line
                            .push_info(crate::t!("Usage: LAYERSTATE SAVE|RESTORE|DELETE <name> | ? (list)").as_ref());
                    }
                }
            }

            "LAYLCK" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                if handles.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let cmd = SelectObjectsCommand::new("LAYLCK");
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let layers: rustc_hash::FxHashSet<String> = self.tabs[i]
                        .scene
                        .selected_entities()
                        .into_iter()
                        .map(|(_, e)| e.common().layer.clone())
                        .collect();
                    let names: Vec<String> = layers.iter().cloned().collect();
                    let undo = self.begin_layer_undo(i, "LAYLCK", &names);
                    for name in &layers {
                        if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(name) {
                            dl.lock();
                        }
                    }
                    // Layer locking changes editability only.
                    self.tabs[i].dirty = true;
                    self.commit_layer_undo(i, undo);
                    self.refresh_layer_panel();
                    self.command_line.push_info(crate::t!("Layer(s) locked.").as_ref());
                }
            }

            "LAYMCUR" => {
                let entities = self.tabs[i].scene.selected_entities();
                if entities.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    // LAYMCUR acts on one object's layer; apply on the first pick.
                    let cmd = SelectObjectsCommand::instant("LAYMCUR");
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let layer = entities[0].1.common().layer.clone();
                    // Keep the document header (CLAYER) in sync, not just the
                    // per-tab default, so a later no-selection ribbon refresh
                    // (e.g. after Esc) doesn't snap back to the stale header
                    // layer. See #93.
                    let handle = self.tabs[i]
                        .scene
                        .document
                        .layers
                        .get(&layer)
                        .map(|l| l.handle)
                        .unwrap_or(acadrust::types::Handle::NULL);
                    self.tabs[i].scene.document.header.current_layer_name = layer.clone();
                    self.tabs[i].scene.document.header.current_layer_handle = handle;
                    self.tabs[i].active_layer = layer.clone();
                    self.ribbon.active_layer = layer.clone();
                    self.tabs[i].layers.current_layer = layer.clone();
                    self.tabs[i].dirty = true;
                    self.command_line
                        .push_info(crate::tf!("Current layer set to \"{layer}\".").as_ref());
                    self.refresh_layer_panel();
                }
            }

            "LAYON" => {
                let names = self.tabs[i]
                    .scene
                    .document
                    .layers
                    .iter()
                    .map(|l| l.name.clone())
                    .collect::<Vec<_>>();
                let undo = self.begin_layer_undo(i, "LAYON", &names);
                for name in &names {
                    if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(&name) {
                        dl.turn_on();
                    }
                }
                self.tabs[i].scene.invalidate_layer_dependencies(&names);
                self.tabs[i].dirty = true;
                self.commit_layer_undo(i, undo);
                self.refresh_layer_panel();
                self.command_line.push_info(crate::t!("All layers turned on.").as_ref());
            }

            "LAYTHW" => {
                let names = self.tabs[i]
                    .scene
                    .document
                    .layers
                    .iter()
                    .map(|l| l.name.clone())
                    .collect::<Vec<_>>();
                let undo = self.begin_layer_undo(i, "LAYTHW", &names);
                for name in &names {
                    if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(&name) {
                        dl.thaw();
                    }
                }
                self.tabs[i].scene.invalidate_layer_dependencies(&names);
                self.tabs[i].dirty = true;
                self.commit_layer_undo(i, undo);
                self.refresh_layer_panel();
                self.command_line.push_info(crate::t!("All layers thawed.").as_ref());
            }

            "LAYULK" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                if handles.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let cmd = SelectObjectsCommand::new("LAYULK");
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let layers: rustc_hash::FxHashSet<String> = self.tabs[i]
                        .scene
                        .selected_entities()
                        .into_iter()
                        .map(|(_, e)| e.common().layer.clone())
                        .collect();
                    let names: Vec<String> = layers.iter().cloned().collect();
                    let undo = self.begin_layer_undo(i, "LAYULK", &names);
                    for name in &layers {
                        if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(name) {
                            dl.unlock();
                        }
                    }
                    // Layer unlocking changes editability only.
                    self.tabs[i].dirty = true;
                    self.commit_layer_undo(i, undo);
                    self.refresh_layer_panel();
                    self.command_line.push_info(crate::t!("Layer(s) unlocked.").as_ref());
                }
            }

            // LAYISO — turn off all layers except those used by selected entities
            "LAYISO" => {
                let sel_layers: rustc_hash::FxHashSet<String> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(_, e)| e.common().layer.clone())
                    .collect();
                if sel_layers.is_empty() {
                    self.command_line
                        .push_error(crate::t!("LAYISO: select entities on the layers to isolate first.").as_ref());
                } else {
                    let names: Vec<String> = self.tabs[i]
                        .scene
                        .document
                        .layers
                        .iter()
                        .map(|l| l.name.clone())
                        .collect();
                    let undo = self.begin_layer_undo(i, "LAYISO", &names);
                    for name in &names {
                        if !sel_layers.contains(name) {
                            if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(name) {
                                dl.turn_off();
                            }
                        }
                    }
                    self.tabs[i].scene.invalidate_layer_dependencies(&names);
                    self.tabs[i].dirty = true;
                    self.commit_layer_undo(i, undo);
                    self.refresh_layer_panel();
                    self.command_line
                        .push_info(crate::tf!("LAYISO: isolated {} layer(s).", sel_layers.len()).as_ref());
                }
            }

            // ISOLATEOBJECTS — hide every object except the current selection
            "ISOLATEOBJECTS" => {
                if self.tabs[i].scene.selected.is_empty() {
                    self.command_line
                        .push_error(crate::t!("ISOLATEOBJECTS: select the objects to isolate first.").as_ref());
                } else {
                    let n = self.tabs[i].scene.selected.len();
                    let before = self.tabs[i].scene.object_isolation.clone();
                    let selected_before =
                        self.tabs[i].scene.selected.iter().copied().collect();
                    self.tabs[i].scene.isolate_selected();
                    self.push_object_visibility_history(
                        i,
                        "ISOLATEOBJECTS",
                        before,
                        selected_before,
                    );
                    self.command_line.push_info(crate::tf!(
                        "Isolated {n} object(s). UNISOLATEOBJECTS to restore."
                    ).as_ref());
                }
            }

            // HIDEOBJECTS — hide the current selection
            "HIDEOBJECTS" => {
                if self.tabs[i].scene.selected.is_empty() {
                    self.command_line
                        .push_error(crate::t!("HIDEOBJECTS: select the objects to hide first.").as_ref());
                } else {
                    let n = self.tabs[i].scene.selected.len();
                    let before = self.tabs[i].scene.object_isolation.clone();
                    let selected_before =
                        self.tabs[i].scene.selected.iter().copied().collect();
                    self.tabs[i].scene.hide_selected();
                    self.push_object_visibility_history(
                        i,
                        "HIDEOBJECTS",
                        before,
                        selected_before,
                    );
                    self.refresh_properties();
                    self.command_line
                        .push_info(crate::tf!("Hid {n} object(s). UNISOLATEOBJECTS to restore.").as_ref());
                }
            }

            // UNISOLATEOBJECTS — bring back everything hidden by Isolate / Hide
            "UNISOLATEOBJECTS" => {
                if self.tabs[i].scene.is_isolation_active() {
                    let before = self.tabs[i].scene.object_isolation.clone();
                    let selected_before =
                        self.tabs[i].scene.selected.iter().copied().collect();
                    self.tabs[i].scene.end_isolation();
                    self.push_object_visibility_history(
                        i,
                        "UNISOLATEOBJECTS",
                        before,
                        selected_before,
                    );
                    self.command_line
                        .push_info(crate::t!("Isolation ended — all objects shown.").as_ref());
                } else {
                    self.command_line.push_info(crate::t!("No hidden objects.").as_ref());
                }
            }

            // LAYUNISO — restore all layers that were turned off by LAYISO (turn all on)
            "LAYUNISO" => {
                let names: Vec<String> = self.tabs[i]
                    .scene
                    .document
                    .layers
                    .iter()
                    .map(|l| l.name.clone())
                    .collect();
                let undo = self.begin_layer_undo(i, "LAYUNISO", &names);
                for name in &names {
                    if let Some(dl) = self.tabs[i].scene.document.layers.get_mut(&name) {
                        dl.turn_on();
                    }
                }
                self.tabs[i].scene.invalidate_layer_dependencies(&names);
                self.tabs[i].dirty = true;
                self.commit_layer_undo(i, undo);
                self.refresh_layer_panel();
                self.command_line
                    .push_info(crate::t!("LAYUNISO: all layers restored.").as_ref());
            }

            "LAYMATCH" | "LAYMCH" => {
                use crate::modules::draw::layers::match_layer::LayMatchCommand;
                let dest: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cmd = LayMatchCommand::new(dest);
                self.command_line.push_info(&cmd.prompt());
                self.tabs[i].active_cmd = Some(Box::new(cmd));
            }

            "MATCHPROP" => {
                use crate::modules::draw::properties::match_prop::MatchPropCommand;
                self.tabs[i].scene.deselect_all();
                let cmd = MatchPropCommand::new();
                self.command_line.push_info(&cmd.prompt());
                self.tabs[i].active_cmd = Some(Box::new(cmd));
            }

            "GROUP" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                if handles.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let cmd = SelectObjectsCommand::new("GROUP");
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let auto_name =
                        super::super::helpers::next_group_auto_name(&self.tabs[i].scene);
                    use crate::modules::draw::groups::group::GroupCommand;
                    let cmd = GroupCommand::new(handles, auto_name);
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                }
            }

            "UNGROUP" => {
                let handles: Vec<_> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                if handles.is_empty() {
                    use crate::modules::draw::groups::ungroup::UngroupCommand;
                    let cmd = UngroupCommand::new();
                    self.command_line.push_info(&cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(cmd));
                } else {
                    let undo = self.begin_group_undo(i, "UNGROUP");
                    let count = self.tabs[i].scene.delete_groups_containing(&handles);
                    self.tabs[i].dirty = true;
                    self.commit_group_undo(i, undo);
                    if count > 0 {
                        self.command_line
                            .push_info(crate::tf!("{} group(s) dissolved.", count).as_ref());
                    } else {
                        self.command_line
                            .push_info(crate::t!("No groups found for selected objects.").as_ref());
                    }
                }
            }

            _ => return None,
        }
        Some(self.finish_dispatch(cmd))
    }
}

impl OpenCADStudio {
    /// Translate every layer the two drawings name alike, in one step.
    ///
    /// The common case for a drawing arriving from outside: the names already
    /// line up and only the properties are wrong, so there is nothing to sit in
    /// a dialog and decide.
    pub(in crate::app) fn layer_translate_same(&mut self, path: &std::path::Path) -> Task<Message> {
        use crate::modules::draw::layers::laytrans;
        let i = self.active_tab;
        let targets = match laytrans::load_targets(path) {
            Ok(targets) => targets,
            Err(why) => {
                self.command_line
                    .push_error(crate::tf!("LAYTRANS: {why}.").as_ref());
                return Task::none();
            }
        };
        let current = self.tabs[i].active_layer.clone();
        let sources = laytrans::source_layers(&self.tabs[i].scene, &current);
        let mappings = laytrans::map_same(&sources, &targets);
        if mappings.is_empty() {
            self.command_line.push_info(
                crate::t!("LAYTRANS: no layer names in common — use LAYTRANS with no file to map them.")
                    .as_ref(),
            );
            return Task::none();
        }
        self.push_undo_snapshot(i, "LAYTRANS");
        let report = laytrans::translate(
            &mut self.tabs[i].scene,
            &mappings,
            &targets,
            &current,
            laytrans::Options::default(),
        );
        self.finish_layer_translation(i, report)
    }

    /// Open the translator, optionally with a standard already loaded.
    pub(in crate::app) fn open_layer_translator(
        &mut self,
        preload: Option<std::path::PathBuf>,
    ) -> Task<Message> {
        self.layer_translator
            .get_or_insert_with(Default::default);
        self.active_modal = Some(crate::app::ModalKind::LayerTranslator);
        match preload {
            Some(path) => Task::done(Message::LayerTranslatorLoaded(path)),
            None => Task::none(),
        }
    }

    /// Read or write the mapping list as plain `from -> to` lines.
    ///
    /// A drawing standards file is where AutoCAD keeps these, but writing them
    /// there would mean inventing a place inside the DWG for something no other
    /// application reads back. A small text file beside the drawing carries the
    /// same information without touching the drawing's own structure.
    pub(in crate::app) fn layer_translator_mappings_file(&mut self, path: &std::path::Path, save: bool) {
        use crate::modules::draw::layers::laytrans::Mapping;
        if save {
            let Some(state) = self.layer_translator.as_ref() else {
                return;
            };
            let body: String = state
                .mappings
                .iter()
                .map(|m| format!("{} -> {}\n", m.from, m.to))
                .collect();
            match std::fs::write(path, body) {
                Ok(()) => self.command_line.push_output(
                    crate::tf!("LAYTRANS: saved {n} mapping(s).", n = state.mappings.len())
                        .as_ref(),
                ),
                Err(why) => self
                    .command_line
                    .push_error(crate::tf!("LAYTRANS: could not save — {why}.").as_ref()),
            }
            return;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(why) => {
                self.command_line
                    .push_error(crate::tf!("LAYTRANS: could not read — {why}.").as_ref());
                return;
            }
        };
        let parsed: Vec<Mapping> = text
            .lines()
            .filter_map(|line| {
                let (from, to) = line.split_once("->")?;
                let (from, to) = (from.trim(), to.trim());
                (!from.is_empty() && !to.is_empty()).then(|| Mapping {
                    from: from.to_string(),
                    to: to.to_string(),
                })
            })
            .collect();
        let count = parsed.len();
        if let Some(state) = self.layer_translator.as_mut() {
            state.mappings = parsed;
        }
        self.command_line
            .push_output(crate::tf!("LAYTRANS: loaded {count} mapping(s).").as_ref());
    }

    /// Write what the last translation did beside the drawing.
    pub(in crate::app) fn write_layer_translation_log(&mut self, i: usize) {
        let Some(report) = self.last_layer_translation.as_ref() else {
            return;
        };
        let base = self.tabs[i]
            .current_path
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(&self.tabs[i].tab_title));
        let path = base.with_extension("laytrans.log");
        match std::fs::write(&path, report.to_log()) {
            Ok(()) => self
                .command_line
                .push_output(crate::tf!("LAYTRANS: log written to {}.", path.display()).as_ref()),
            Err(why) => self
                .command_line
                .push_error(crate::tf!("LAYTRANS: could not write log — {why}.").as_ref()),
        }
    }

    /// Apply a translation's result: mark the drawing, refresh the panel and
    /// say what moved. Shared by the command and the dialog so both report the
    /// same way.
    pub(in crate::app) fn finish_layer_translation(
        &mut self,
        i: usize,
        report: crate::modules::draw::layers::laytrans::Report,
    ) -> Task<Message> {
        let layers = report.translated.len();
        let objects = report.objects();
        self.tabs[i].dirty = true;
        self.refresh_layer_panel();
        self.refresh_properties();
        for (layer, reason) in &report.skipped {
            self.command_line
                .push_info(crate::tf!("LAYTRANS: skipped \"{layer}\" — {reason}.").as_ref());
        }
        self.command_line.push_output(
            crate::tf!("LAYTRANS: translated {layers} layer(s), {objects} object(s).").as_ref(),
        );
        self.last_layer_translation = Some(report);
        Task::none()
    }
}
