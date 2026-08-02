use super::*;

impl OpenCADStudio {
    pub(super) fn dispatch_fileops(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        match cmd {
            "NEW" => return Some(Task::done(Message::TabNew)),
            "OPEN" => return Some(Task::done(Message::OpenFile)),
            "SAVE" | "QSAVE" => return Some(Task::done(Message::SaveFile)),
            // SAVEALL — write every open drawing that already has a file path.
            // Tabs without a path (never saved) are skipped with a note.
            "SAVEALL" => {
                if self.read_only {
                    self.command_line
                        .push_error(crate::t!("Read-only session (--read-only): saving is disabled.").as_ref());
                    return Some(Task::none());
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let mut saved = 0usize;
                    let mut skipped = 0usize;
                    for t in 0..self.tabs.len() {
                        if self.tabs[t].is_start {
                            continue;
                        }
                        if let Some(path) = self.tabs[t].current_path.clone() {
                            if self
                                .active_save_jobs
                                .contains_key(&self.tabs[t].id)
                            {
                                self.command_line.push_info(crate::tf!(
                                    "SAVEALL: {} already has a save running",
                                    path.display()
                                ).as_ref());
                                skipped += 1;
                                continue;
                            }
                            match self.save_tab_synchronously_protected(
                                t,
                                path.clone(),
                                false,
                            ) {
                                Ok(()) => {
                                    saved += 1;
                                }
                                Err(e) => self.command_line.push_error(crate::tf!(
                                    "SAVEALL: {} failed: {e}",
                                    path.display()
                                ).as_ref()),
                            }
                        } else {
                            skipped += 1;
                        }
                    }
                    self.command_line.push_output(crate::tf!(
                        "SAVEALL: saved {saved} drawing(s){}.",
                        if skipped > 0 {
                            format!("; {skipped} need SAVEAS (no file path yet)")
                        } else {
                            String::new()
                        }
                    ).as_ref());
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.command_line
                        .push_info(crate::t!("SAVEALL: save each tab individually in the web build.").as_ref());
                }
                return Some(Task::none());
            }
            "SAVEAS" => return Some(Task::done(Message::SaveAs)),
            // UNDO <n> — step back n operations at once; bare UNDO / U is one step.
            cmd if cmd.starts_with("UNDO ") => {
                let arg = cmd["UNDO ".len()..].trim();
                match arg.parse::<usize>() {
                    Ok(0) => return Some(Task::none()),
                    Ok(n) => return Some(Task::done(Message::UndoMany(n))),
                    Err(_) => {
                        self.command_line
                            .push_error(crate::t!("Usage: UNDO [number of steps]").as_ref());
                        return Some(Task::none());
                    }
                }
            }
            "UNDO" => return Some(Task::done(Message::Undo)),
            "REDO" => return Some(Task::done(Message::Redo)),
            // OOPS — restore the objects removed by the most recent ERASE,
            // without undoing any work done since.
            "OOPS" => {
                if self.oops_cache.is_empty() {
                    self.command_line.push_info(crate::t!("OOPS: nothing to restore.").as_ref());
                } else {
                    let restored = std::mem::take(&mut self.oops_cache);
                    let pending = self.begin_undo(i, "OOPS", restored.len(), true);
                    let restored = self.tabs[i].scene.restore_erased_entities(restored);
                    let n = restored.len();
                    self.tabs[i].dirty = true;
                    self.refresh_properties();
                    if let Some(pending) = pending {
                        self.commit_undo_delta(i, pending);
                    }
                    self.command_line
                        .push_output(crate::tf!("OOPS: restored {n} object(s).").as_ref());
                }
            }
            "CLEAR" | "CLR" => return Some(Task::done(Message::ClearScene)),
            "WIREFRAME" => return Some(Task::done(Message::SetWireframe(true))),
            // Visual-style commands. OCS renders either a wireframe or a shaded
            // view; the named styles map onto the closest of the two and the
            // chosen style is reported so the mapping is explicit. (`SOLID` is
            // intentionally NOT a visual-style verb — it is the 2D filled-polygon
            // draw command; the shaded ribbon button drives `SetWireframe`.)
            "VS" | "VSCURRENT" | "SHADEMODE" => {
                use crate::command::KeywordCommand;
                let c = KeywordCommand::new(
                    "VSCURRENT",
                    "VSCURRENT  visual style  [Shaded / Wireframe / Hidden / Realistic / Conceptual / X-ray]:",
                    vec![
                        ("Shaded", "SHADED", None),
                        ("Wireframe", "WIREFRAME", None),
                        ("Hidden", "HIDDEN", None),
                        ("Realistic", "REALISTIC", None),
                        ("Conceptual", "CONCEPTUAL", None),
                        ("X-Ray", "XRAY", None),
                    ],
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            // The named visual-style shortcuts still switch directly, and the
            // `<name> <style>` argument form (also what the picker dispatches).
            cmd if cmd == "HIDDENLINE"
                || cmd == "XRAY"
                || cmd == "REALISTIC"
                || cmd == "CONCEPTUAL"
                || cmd == "2DWIREFRAME"
                || cmd == "3DWIREFRAME"
                || cmd.starts_with("VSCURRENT ")
                || cmd.starts_with("SHADEMODE ")
                || cmd.starts_with("VS ") =>
            {
                let style = match cmd {
                    "VS" | "VSCURRENT" | "SHADEMODE" => String::new(),
                    s if s.starts_with("VS ")
                        || s.starts_with("VSCURRENT ")
                        || s.starts_with("SHADEMODE ") =>
                    {
                        cmd.split_whitespace().nth(1).unwrap_or("").to_uppercase()
                    }
                    other => other.to_string(),
                };
                let (wireframe, label) = match style.as_str() {
                    "" | "SHADED" | "S" | "REALISTIC" | "CONCEPTUAL" => (false, "Shaded"),
                    "2DWIREFRAME" | "3DWIREFRAME" | "WIREFRAME" | "W" => (true, "Wireframe"),
                    "HIDDENLINE" | "HIDDEN" | "H" => (false, "Hidden (shown shaded)"),
                    "XRAY" | "X" => (true, "X-Ray (shown as wireframe)"),
                    _ => {
                        self.command_line.push_error(
                            "Usage: VSCURRENT <2dwireframe|wireframe|hidden|realistic|conceptual|shaded|xray>",
                        );
                        return Some(Task::none());
                    }
                };
                self.command_line
                    .push_output(crate::tf!("Visual style: {label}.").as_ref());
                return Some(Task::done(Message::SetWireframe(wireframe)));
            }
            // CLOSE — close the active drawing tab (with the unsaved-changes
            // prompt the tab-close handler already runs).
            "CLOSE" => {
                return Some(Task::done(Message::TabClose(self.active_tab)));
            }

            // ARCHIVE / ETRANSMIT — package the drawing and its referenced files
            // (xrefs + raster images) into a sibling "<name>_archive" folder.
            "ARCHIVE" | "ETRANSMIT" => {
                use std::path::PathBuf;
                if let Some(src) = self.tabs[i].current_path.clone() {
                    let stem = src
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "drawing".into());
                    let parent = src
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."));
                    let folder = parent.join(format!("{stem}_archive"));
                    match std::fs::create_dir_all(&folder) {
                        Ok(()) => {
                            let mut copied = 0usize;
                            if let Some(fname) = src.file_name() {
                                if std::fs::copy(&src, folder.join(fname)).is_ok() {
                                    copied += 1;
                                }
                            }
                            let mut deps: Vec<String> = Vec::new();
                            for br in self.tabs[i].scene.document.block_records.iter() {
                                if !br.xref_path.trim().is_empty() {
                                    deps.push(br.xref_path.clone());
                                }
                            }
                            for e in self.tabs[i].scene.document.entities() {
                                if let acadrust::EntityType::RasterImage(img) = e {
                                    if !img.file_path.trim().is_empty() {
                                        deps.push(img.file_path.clone());
                                    }
                                }
                            }
                            for d in deps {
                                let dp = PathBuf::from(&d);
                                let resolved = if dp.is_absolute() { dp } else { parent.join(&dp) };
                                if resolved.exists() {
                                    if let Some(fname) = resolved.file_name() {
                                        if std::fs::copy(&resolved, folder.join(fname)).is_ok() {
                                            copied += 1;
                                        }
                                    }
                                }
                            }
                            self.command_line.push_output(crate::tf!(
                                "{cmd}: packaged {copied} file(s) into {}",
                                folder.display()
                            ).as_ref());
                        }
                        Err(e) => self
                            .command_line
                            .push_error(crate::tf!("{cmd}: cannot create folder ({e}).").as_ref()),
                    }
                } else {
                    self.command_line
                        .push_error(crate::t!("ARCHIVE: save the drawing first (it has no file path yet).").as_ref());
                }
            }

            "EXIT" | "QUIT" => {
                // Funnel through the OS close path so the unsaved-changes
                // dialog runs before `iced::exit()`. Falls back to a hard
                // exit if there's no main window registered yet.
                if let Some(id) = self.main_window {
                    return Some(Task::done(Message::WindowCloseRequested(id)));
                }
                return Some(self.exit_app());
            }

            // ── Frame-budget HUD (Phase 5.3) ───────────────────────────────
            // Toggle the per-rebuild wire-tessellation readout overlay.
            "PERF" => {
                self.perf_hud = !self.perf_hud;
                crate::perf::set_ui_enabled(self.perf_hud);
                self.command_line.push_info(if self.perf_hud {
                    "PERF panel on — tracing render and interaction costs"
                } else {
                    "PERF panel off"
                });
                return Some(Task::none());
            }

            // ── Background color ───────────────────────────────────────────
            // Usage:  BACKGROUND <r> <g> <b>      (0–255 each)
            //         BACKGROUND DEFAULT|BLACK|DARKGRAY|GRAY|LIGHTGRAY|WHITE  (preset)
            //         BACKGROUND DEFAULT          (restore the app default, rgb 33,40,48)
            // The chosen colour is also stored as the persisted default
            // (`default_bg_color` / `default_paper_bg_color`) so it survives
            // restarts and applies to new drawings (#188).
            // Bare BACKGROUND enters an interactive prompt for the colour, so
            // the command works both as a one-shot (`BACKGROUND BLACK`) and as a
            // type-then-choose flow (`BACKGROUND` ⏎, then `BLACK`). The prompt
            // delegates back to the inline handler below via `Dispatch`.
            "BACKGROUND" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new(
                    "BACKGROUND",
                    "BACKGROUND  colour [Default/Black/DarkGray/Gray/LightGray/White] or R G B (0–255):",
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("BACKGROUND ") => {
                let args = cmd.split_whitespace().skip(1).collect::<Vec<_>>();
                let is_paper = self.tabs[i].scene.current_layout != "Model";
                if args
                    .first()
                    .map(|s| s.eq_ignore_ascii_case("DEFAULT") || s.eq_ignore_ascii_case("RESET"))
                    .unwrap_or(false)
                {
                    if is_paper {
                        self.tabs[i].paper_bg_color = None;
                        self.tabs[i].scene.paper_bg_color = [1.0, 1.0, 1.0, 1.0];
                        self.default_paper_bg_color = None;
                    } else {
                        self.tabs[i].bg_color = None;
                        self.tabs[i].scene.bg_color = [33.0 / 255.0, 40.0 / 255.0, 48.0 / 255.0, 1.0];
                        self.default_bg_color = None;
                    }
                    // Wire colour adaptation (`adapt_to_bg`) reads the bg
                    // at tessellation time, so the cached wires need to
                    // refresh — otherwise a light→dark bg flip leaves
                    // black lines invisible against the new bg. Meshes
                    // bake colour into per-vertex GPU buffers at upload
                    // time; `recolor_meshes` rewrites the CPU side so
                    // the next epoch-driven re-upload picks up the new
                    // colour.
                    self.tabs[i].scene.recolor_meshes();
                    self.tabs[i].scene.bump_geometry();
                    self.command_line
                        .push_output(crate::t!("Background reset to default.").as_ref());
                } else if let Some(rgba) = parse_background_color(&args) {
                    if is_paper {
                        self.tabs[i].paper_bg_color = Some(rgba);
                        self.tabs[i].scene.paper_bg_color = rgba;
                        self.default_paper_bg_color = Some(rgba);
                    } else {
                        self.tabs[i].bg_color = Some(rgba);
                        self.tabs[i].scene.bg_color = rgba;
                        self.default_bg_color = Some(rgba);
                    }
                    self.tabs[i].scene.recolor_meshes();
                    self.tabs[i].scene.bump_geometry();
                    let [r, g, b, _] = rgba;
                    self.command_line.push_output(crate::tf!(
                        "Background: rgb({}, {}, {})",
                        (r * 255.0).round() as u8,
                        (g * 255.0).round() as u8,
                        (b * 255.0).round() as u8
                    ).as_ref());
                    // Persisted centrally after this message via
                    // `persist_settings_if_changed()`.
                } else {
                    self.command_line.push_info(
                        "Usage: BACKGROUND <r> <g> <b> (0–255) | DEFAULT|BLACK|DARKGRAY|GRAY|LIGHTGRAY|WHITE",
                    );
                }
            }
            // ORTHO toggles the orthogonal cursor constraint — the standard
            // drafting aid, the same state the status-bar pill drives. Camera
            // projection is a separate concern: PARALLEL / PERSP, driven by the
            // Projection ribbon group.
            "ORTHO" => return Some(Task::done(Message::ToggleOrtho)),
            "PARALLEL" => return Some(Task::done(Message::SetProjection(true))),
            "PERSP" => return Some(Task::done(Message::SetProjection(false))),
            "LAYERS" => return Some(Task::done(Message::ToggleLayers)),

            // SCRIPT <path> — run a command script: each non-blank, non-comment
            // line is fed through the same command path the `--script` startup
            // flag uses, so the behaviour matches headless automation exactly.
            "SCRIPT" | "SCR" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new("SCRIPT", "SCRIPT  path to the .scr file:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("SCRIPT ") || cmd.starts_with("SCR ") => {
                let path = cmd.split_once(' ').map(|(_, r)| r.trim().to_string());
                match path {
                    Some(p) if !p.is_empty() => match std::fs::read_to_string(&p) {
                        Ok(text) => {
                            let cmds: Vec<Task<Message>> = text
                                .lines()
                                .map(str::trim)
                                .filter(|l| {
                                    !l.is_empty() && !l.starts_with('#') && !l.starts_with(';')
                                })
                                .map(|l| Task::done(Message::Command(l.to_string())))
                                .collect();
                            self.command_line.push_output(crate::tf!(
                                "SCRIPT: running {} command(s) from {p}.",
                                cmds.len()
                            ).as_ref());
                            return Some(Task::batch(cmds));
                        }
                        Err(e) => {
                            self.command_line
                                .push_error(crate::tf!("SCRIPT: cannot read {p}: {e}").as_ref());
                        }
                    },
                    _ => {
                        self.command_line
                            .push_info(crate::t!("Usage: SCRIPT <path to .scr file>").as_ref());
                    }
                }
            }

            _ => return None,
        }
        Some(self.finish_dispatch(cmd))
    }
}

/// Parse the argument list of the `BACKGROUND` command into an `[r,g,b,a]`
/// colour (channels 0.0–1.0, `a` always 1.0). Accepts:
///   * three whitespace-separated 0–255 values: `255 255 255`
///   * a named preset: WHITE / BLACK / GRAY|GREY / DARKGRAY|DARKGREY / LTGRAY
/// Returns `None` if the arguments don't match either form.
fn parse_background_color(args: &[&str]) -> Option<[f32; 4]> {
    let to_rgba = |[r, g, b]: [u8; 3]| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
    // Single token: a named preset.
    if args.len() == 1 {
        let preset = match args[0].to_ascii_uppercase().as_str() {
            "WHITE" => [255, 255, 255],
            "BLACK" => [0, 0, 0],
            "GRAY" | "GREY" => [128, 128, 128],
            "DARKGRAY" | "DARKGREY" | "DKGRAY" => [64, 64, 64],
            "LTGRAY" | "LIGHTGRAY" | "LIGHTGREY" => [192, 192, 192],
            _ => return None,
        };
        return Some(to_rgba(preset));
    }
    // Three separate tokens: `r g b`.
    if args.len() >= 3 {
        let r = args[0].parse::<u8>().ok()?;
        let g = args[1].parse::<u8>().ok()?;
        let b = args[2].parse::<u8>().ok()?;
        return Some(to_rgba([r, g, b]));
    }
    None
}
