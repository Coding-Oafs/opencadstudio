use super::*;

impl OpenCADStudio {
    pub(super) fn dispatch_view(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        match cmd {
            "DONATE" => {
                self.command_line.push_info(crate::t!("Opening Patreon page...").as_ref());
                return Some(crate::sys::open_url(
                    "https://patreon.com/HakanSeven12",
                    self.main_window,
                ));
            }

            "WEBVERSION" => {
                self.command_line.push_info(crate::t!("Opening OCS Web...").as_ref());
                return Some(crate::sys::open_url(
                    "https://hakanseven12.github.io/OpenCADStudio/",
                    self.main_window,
                ));
            }

            "HELP" => {
                self.command_line
                    .push_info(crate::t!("Opening OCS Discussions for help and questions...").as_ref());
                return Some(crate::sys::open_url(
                    "https://github.com/HakanSeven12/OpenCADStudio/discussions",
                    self.main_window,
                ));
            }

            // ── DWGPROPS — print round-trip-only HeaderVariables ─────────
            // No UI dialog for these yet; the command surfaces them so
            // users can confirm the values that the parser populated and
            // the writer will round-trip on save.
            "DWGPROPS" | "DWGPROP" => {
                let i = self.active_tab;
                let h = &self.tabs[i].scene.document.header;
                let path_label = self.tabs[i]
                    .current_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unsaved)".to_string());
                self.command_line
                    .push_output(crate::tf!("Drawing: {}", path_label).as_ref());
                self.command_line
                    .push_output(crate::tf!("  Created (Julian):  {:.6}", h.create_date_julian).as_ref());
                self.command_line
                    .push_output(crate::tf!("  Updated (Julian):  {:.6}", h.update_date_julian).as_ref());
                self.command_line
                    .push_output(crate::tf!("  Total edit time:   {:.4}", h.total_editing_time).as_ref());
                self.command_line
                    .push_output(crate::tf!("  User elapsed:      {:.4}", h.user_elapsed_time).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Last saved by:     {}",
                    if h.last_saved_by.is_empty() {
                        "(unknown)"
                    } else {
                        &h.last_saved_by
                    }
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Fingerprint GUID:  {}",
                    if h.fingerprint_guid.is_empty() {
                        "(none)"
                    } else {
                        &h.fingerprint_guid
                    }
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Version GUID:      {}",
                    if h.version_guid.is_empty() {
                        "(none)"
                    } else {
                        &h.version_guid
                    }
                ).as_ref());
                self.command_line
                    .push_output(crate::tf!("  Code page:         {}", h.code_page).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Menu name:         {}",
                    if h.menu_name.is_empty() {
                        "(none)"
                    } else {
                        &h.menu_name
                    }
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Hyperlink base:    {}",
                    if h.hyperlink_base.is_empty() {
                        "(none)"
                    } else {
                        &h.hyperlink_base
                    }
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Project name:      {}",
                    if h.project_name.is_empty() {
                        "(none)"
                    } else {
                        &h.project_name
                    }
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Stylesheet:        {}",
                    if h.stylesheet.is_empty() {
                        "(none)"
                    } else {
                        &h.stylesheet
                    }
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Required versions: {:#018x}",
                    h.required_versions
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  Measurement:       {} ({})",
                    h.measurement,
                    if h.measurement == 1 {
                        "Metric"
                    } else {
                        "Imperial"
                    }
                ).as_ref());
                self.command_line
                    .push_output(crate::tf!("  Proxy graphics:    {}", h.proxy_graphics).as_ref());
                self.command_line
                    .push_output(crate::tf!("  Tree depth:        {}", h.tree_depth).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  User vars (int):   {} {} {} {} {}",
                    h.user_int1, h.user_int2, h.user_int3, h.user_int4, h.user_int5
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  User vars (real):  {:.6} {:.6} {:.6} {:.6} {:.6}",
                    h.user_real1, h.user_real2, h.user_real3, h.user_real4, h.user_real5
                ).as_ref());
                self.command_line.push_output(crate::tf!(
                    "  User timer:        {}",
                    if h.user_timer { "On" } else { "Off" }
                ).as_ref());
            }

            // Edit a USERI1..USERI5 / USERR1..USERR5 slot. Lets the user
            // store drawing-scoped scalars (and save them through round-trip)
            // even though we don't have a LISP / DIESEL runtime yet.
            //   USERI 1 42        → header.user_int1 = 42
            //   USERR 3 1.5e-3    → header.user_real3 = 0.0015
            "USERI" | "USERR" => {
                use crate::command::UserRegCommand;
                let name = if cmd == "USERR" { "USERR" } else { "USERI" };
                let c = UserRegCommand::new(name);
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("USERI ") || cmd.starts_with("USERR ") => {
                let is_real = cmd.starts_with("USERR");
                let rest = if is_real {
                    cmd.trim_start_matches("USERR").trim()
                } else {
                    cmd.trim_start_matches("USERI").trim()
                };
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                let slot: Option<usize> = parts.first().and_then(|s| s.parse().ok());
                let value = parts.get(1).copied().unwrap_or("").trim();
                let i = self.active_tab;
                let h = &mut self.tabs[i].scene.document.header;
                match (slot, value, is_real) {
                    (Some(n @ 1..=5), v, true) => {
                        if let Ok(val) = v.parse::<f64>() {
                            match n {
                                1 => h.user_real1 = val,
                                2 => h.user_real2 = val,
                                3 => h.user_real3 = val,
                                4 => h.user_real4 = val,
                                _ => h.user_real5 = val,
                            }
                            self.tabs[i].dirty = true;
                            self.command_line.push_output(crate::tf!("USERR{n} = {val}").as_ref());
                        } else {
                            self.command_line.push_info(crate::t!("Usage: USERR <1-5> <real>").as_ref());
                        }
                    }
                    (Some(n @ 1..=5), v, false) => {
                        if let Ok(val) = v.parse::<i16>() {
                            match n {
                                1 => h.user_int1 = val,
                                2 => h.user_int2 = val,
                                3 => h.user_int3 = val,
                                4 => h.user_int4 = val,
                                _ => h.user_int5 = val,
                            }
                            self.tabs[i].dirty = true;
                            self.command_line.push_output(crate::tf!("USERI{n} = {val}").as_ref());
                        } else {
                            self.command_line.push_info(crate::t!("Usage: USERI <1-5> <integer>").as_ref());
                        }
                    }
                    _ => self
                        .command_line
                        .push_info(crate::t!("Usage: USERI <1-5> <int> | USERR <1-5> <real>").as_ref()),
                }
            }

            "REPORT" => {
                // Pre-fill the GitHub issue body with version + platform so
                // reports arrive with the basics already filled in.
                let body = format!(
                    "<!-- Describe the issue and the steps to reproduce it. -->\n\n\n\
                     ---\n- Open CAD Studio: v{}\n- Platform: {}\n",
                    env!("CARGO_PKG_VERSION"),
                    crate::sys::platform_info(),
                );
                let url = format!(
                    "https://github.com/HakanSeven12/OpenCADStudio/issues/new?body={}",
                    crate::sys::percent_encode(&body)
                );
                self.command_line.push_info(crate::t!("Opening feedback page...").as_ref());
                return Some(crate::sys::open_url(&url, self.main_window));
            }

            "ABOUT" => {
                return Some(Task::done(Message::AboutOpen));
            }

            "PLUGINS" | "PLUGINMANAGER" => {
                return Some(Task::done(Message::PluginManagerOpen));
            }

            "CHANGELOG" => {
                self.command_line.push_info(crate::t!("Opening release notes...").as_ref());
                return Some(crate::sys::open_url(
                    "https://github.com/HakanSeven12/OpenCADStudio/releases",
                    self.main_window,
                ));
            }

            // ── ALIASEDIT — command-alias editor ───────────────────────────
            // Opens the command-alias table editor (ocad.pgp). Keyboard
            // key-bindings are edited separately via CUI (below).
            "ALIASEDIT" => {
                return Some(Task::done(Message::AliasEditorOpen));
            }

            // ── CUI — keyboard shortcut / key-binding editor ───────────────
            "CUI" => {
                return Some(Task::done(Message::ShortcutsPanelOpen));
            }

            // CUIEXPORT <path> — write the keyboard-shortcut customizations
            // (the drawing-independent CUI data) to a plain "KEY COMMAND" file.
            "CUIEXPORT" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new("CUIEXPORT", "CUIEXPORT  file to save shortcuts to:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("CUIEXPORT ") => {
                let path = cmd.trim_start_matches("CUIEXPORT").trim();
                if path.is_empty() {
                    self.command_line.push_info(
                        "Usage: CUIEXPORT <path> — save the keyboard shortcuts to a file.",
                    );
                    return Some(Task::none());
                }
                let mut keys: Vec<(&String, &String)> = self.shortcut_bindings.iter().collect();
                keys.sort_by(|a, b| a.0.cmp(b.0));
                let text: String = keys.iter().map(|(k, v)| format!("{k} {v}\n")).collect();
                let count = self.shortcut_bindings.len();
                match std::fs::write(path, text) {
                    Ok(()) => self.command_line.push_output(crate::tf!(
                        "CUIEXPORT: wrote {count} shortcut(s) to \"{path}\"."
                    ).as_ref()),
                    Err(e) => self
                        .command_line
                        .push_error(crate::tf!("CUIEXPORT: cannot write \"{path}\": {e}").as_ref()),
                }
            }

            // CUIIMPORT / CUILOAD <path> — load shortcut customizations from a
            // "KEY COMMAND" file (lines starting with # are ignored).
            "CUIIMPORT" | "CUILOAD" => {
                use crate::command::ValuePromptCommand;
                let c = ValuePromptCommand::new("CUIIMPORT", "CUIIMPORT  shortcuts file to load:");
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("CUIIMPORT ") || cmd.starts_with("CUILOAD ") => {
                let path = cmd
                    .trim_start_matches("CUIIMPORT")
                    .trim_start_matches("CUILOAD")
                    .trim();
                if path.is_empty() {
                    self.command_line.push_info(
                        "Usage: CUIIMPORT <path> — load keyboard shortcuts from a file.",
                    );
                    return Some(Task::none());
                }
                match std::fs::read_to_string(path) {
                    Ok(text) => {
                        let mut n = 0usize;
                        for line in text.lines() {
                            let line = line.trim();
                            if line.is_empty() || line.starts_with('#') {
                                continue;
                            }
                            if let Some((k, v)) = line.split_once(char::is_whitespace) {
                                let key = crate::app::shortcuts::normalize_key(k);
                                if !key.is_empty() {
                                    self.shortcut_bindings
                                        .insert(key, v.trim().to_uppercase());
                                    n += 1;
                                }
                            }
                        }
                        self.persist_settings_if_changed();
                        self.command_line.push_output(crate::tf!(
                            "CUIIMPORT: loaded {n} shortcut(s) from \"{path}\"."
                        ).as_ref());
                    }
                    Err(e) => self
                        .command_line
                        .push_error(crate::tf!("CUIIMPORT: cannot read \"{path}\": {e}").as_ref()),
                }
            }

            // ── Keyboard Shortcuts panel ──────────────────────────────────
            cmd if cmd == "SHORTCUTS" || cmd.starts_with("SHORTCUTS ") => {
                let raw_rest = cmd.trim_start_matches("SHORTCUTS").trim();
                let parts: Vec<&str> = raw_rest.splitn(3, ' ').collect();
                let sub = parts.first().map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "" | "LIST" | "?" => {
                        return Some(Task::done(Message::ShortcutsPanelOpen));
                    }
                    "SET" | "S" => {
                        // SHORTCUTS SET <key> <command>
                        // e.g. SHORTCUTS SET CTRL+D DIST
                        let key = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
                        let cmd_str = parts.get(2).map(|s| s.to_uppercase()).unwrap_or_default();
                        if key.is_empty() || cmd_str.is_empty() {
                            self.command_line.push_error(crate::t!("Usage: SHORTCUTS SET <key> <command>  e.g. SHORTCUTS SET CTRL+D DIST").as_ref());
                        } else {
                            let key = crate::app::shortcuts::normalize_key(&key);
                            if key.is_empty() {
                                self.command_line.push_error(crate::t!("Usage: SHORTCUTS SET <key> <command>  e.g. SHORTCUTS SET CTRL+D DIST").as_ref());
                            } else {
                                self.shortcut_bindings.insert(key.clone(), cmd_str.clone());
                                self.persist_settings_if_changed();
                                self.command_line
                                    .push_output(crate::tf!("Shortcut set: {key} → {cmd_str}").as_ref());
                            }
                        }
                    }
                    "CLEAR" | "DELETE" | "REMOVE" => {
                        let key = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
                        if key.is_empty() {
                            self.command_line.push_error(crate::t!("Usage: SHORTCUTS CLEAR <key>").as_ref());
                        } else if self
                            .shortcut_bindings
                            .remove(&crate::app::shortcuts::normalize_key(&key))
                            .is_some()
                        {
                            self.persist_settings_if_changed();
                            self.command_line
                                .push_output(crate::tf!("Shortcut '{key}' removed.").as_ref());
                        } else {
                            self.command_line
                                .push_error(crate::tf!("Shortcut '{key}' not found.").as_ref());
                        }
                    }
                    _ => {
                        self.command_line
                            .push_info(crate::t!("Usage: SHORTCUTS LIST | SET <key> <cmd> | CLEAR <key>").as_ref());
                    }
                }
            }

            // ── Color Scheme / Theme selector ─────────────────────────────
            cmd if cmd == "COLORSCHEME" || cmd.starts_with("COLORSCHEME ") => {
                use iced::Theme;
                let sub = cmd
                    .split_once(' ')
                    .map(|(_, r)| r.trim())
                    .unwrap_or("")
                    .to_uppercase();
                // Map name to Theme variant.
                let theme: Option<Theme> = match sub.as_str() {
                    "DARK" => Some(Theme::Dark),
                    "LIGHT" => Some(Theme::Light),
                    "DRACULA" => Some(Theme::Dracula),
                    "NORD" => Some(Theme::Nord),
                    "SOLARIZED_LIGHT" | "SOLARIZEDLIGHT" => Some(Theme::SolarizedLight),
                    "SOLARIZED_DARK" | "SOLARIZEDDARK" => Some(Theme::SolarizedDark),
                    "GRUVBOX_LIGHT" | "GRUVBOXLIGHT" => Some(Theme::GruvboxLight),
                    "GRUVBOX_DARK" | "GRUVBOXDARK" => Some(Theme::GruvboxDark),
                    "TOKYONIGHT" | "TOKYO_NIGHT" => Some(Theme::TokyoNight),
                    "TOKYONIGHTSTORM" | "TOKYO_NIGHT_STORM" => Some(Theme::TokyoNightStorm),
                    "TOKYONIGHTLIGHT" | "TOKYO_NIGHT_LIGHT" => Some(Theme::TokyoNightLight),
                    "KANAGAWAWAVE" | "KANAGAWA_WAVE" => Some(Theme::KanagawaWave),
                    "KANAGAWADRAGON" | "KANAGAWA_DRAGON" => Some(Theme::KanagawaDragon),
                    "KANAGAWALOTUS" | "KANAGAWA_LOTUS" => Some(Theme::KanagawaLotus),
                    "MOONFLY" => Some(Theme::Moonfly),
                    "NIGHTFLY" => Some(Theme::Nightfly),
                    "OXOCARBON" => Some(Theme::Oxocarbon),
                    "FERRA" => Some(Theme::Ferra),
                    "" | "LIST" | "?" => {
                        self.command_line.push_output(
                            "Available themes: DARK LIGHT DRACULA NORD SOLARIZED_LIGHT SOLARIZED_DARK \
                             GRUVBOX_LIGHT GRUVBOX_DARK TOKYONIGHT TOKYONIGHTSTORM TOKYONIGHTLIGHT \
                             KANAGAWAWAVE KANAGAWADRAGON KANAGAWALOTUS MOONFLY NIGHTFLY OXOCARBON FERRA"
                        );
                        return Some(Task::none());
                    }
                    _ => {
                        self.command_line.push_error(crate::tf!(
                            "COLORSCHEME: unknown theme '{}'. Type COLORSCHEME LIST for options.",
                            sub
                        ).as_ref());
                        return Some(Task::none());
                    }
                };
                if let Some(t) = theme {
                    let name = format!("{:?}", t);
                    self.command_line
                        .push_output(crate::tf!("Color scheme set to '{name}'.").as_ref());
                    return Some(Task::done(Message::SetTheme(t)));
                }
                return Some(Task::none());
            }

            // ── Layout Manager GUI ─────────────────────────────────────────
            "LAYOUTMANAGER" | "LAYOUTPANEL" => {
                return Some(Task::done(Message::LayoutManagerOpen));
            }

            // ── Layout / viewport ──────────────────────────────────────────
            "MVIEW" => {
                if self.tabs[i].scene.current_layout == "Model" {
                    self.command_line
                        .push_error(crate::t!("MVIEW: switch to a paper space layout first.").as_ref());
                } else {
                    use crate::modules::layout::mview::MviewCommand;
                    let scene = &self.tabs[i].scene;
                    let layout = scene.current_layout.clone();
                    let paper_bounds = scene
                        .printable_area_limits()
                        .or_else(|| scene.paper_limits())
                        .unwrap_or(((0.0, 0.0), (297.0, 210.0)));
                    let views = scene.document.views.iter().cloned().collect();
                    let new_cmd = MviewCommand::new(layout, paper_bounds, views);
                    self.command_line.push_info(&new_cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(new_cmd));
                }
            }

            // ── MSPACE / PSPACE ───────────────────────────────────────────
            "MSPACE" => {
                return Some(Task::done(Message::MspaceCommand));
            }
            "PSPACE" => {
                return Some(Task::done(Message::PspaceCommand));
            }

            // ── Viewport arrangement shortcuts ────────────────────────────
            // Tile the model viewports into preset splits. Each delegates to the
            // matching VPORTS configuration so the Model/paper handling stays in
            // one place.
            "HORIZONTAL" => return self.dispatch_view("VPORTS 2H", i),
            "VERTICAL" => return self.dispatch_view("VPORTS 2V", i),
            "VPJOIN" => return self.dispatch_view("VPORTS SINGLE", i),
            "CASCADE" => return self.dispatch_view("VPORTS 4", i),

            // ── VPORTS — list or create preset viewport configurations ────
            cmd if cmd == "VPORTS" || cmd.starts_with("VPORTS ") => {
                let sub = cmd.split_whitespace().nth(1).unwrap_or("").to_uppercase();
                let scene = &self.tabs[i].scene;
                if scene.current_layout == "Model" {
                    // Bare VPORTS → ask for the configuration interactively;
                    // the next command-line entry supplies it.
                    if sub.is_empty() {
                        self.awaiting_vports = true;
                        self.command_line
                            .push_info(crate::t!("VPORTS  Configuration [SIngle/2H/2V/4]:").as_ref());
                        return Some(self.focus_cmd_input());
                    }
                    // Model space: split the tiled viewport layout via pane_grid.
                    use iced::widget::pane_grid::{Axis, Configuration as C};
                    let split = |axis, a, b| C::Split {
                        axis,
                        ratio: 0.5,
                        a: Box::new(a),
                        b: Box::new(b),
                    };
                    let config: Option<(C<usize>, usize)> = match sub.as_str() {
                        "SINGLE" | "SI" | "1" => Some((C::Pane(0), 1)),
                        "2H" | "2" => Some((split(Axis::Horizontal, C::Pane(0), C::Pane(1)), 2)),
                        "2V" => Some((split(Axis::Vertical, C::Pane(0), C::Pane(1)), 2)),
                        "4" => Some((
                            split(
                                Axis::Vertical,
                                split(Axis::Horizontal, C::Pane(0), C::Pane(2)),
                                split(Axis::Horizontal, C::Pane(1), C::Pane(3)),
                            ),
                            4,
                        )),
                        _ => None,
                    };
                    match config {
                        Some((config, n)) => {
                            self.tabs[i].scene.set_model_panes(config);
                            self.tabs[i].scene.camera_generation += 1;
                            self.command_line
                                .push_output(crate::tf!("VPORTS: {n} viewport(s).").as_ref());
                        }
                        None => {
                            self.command_line
                                .push_error(crate::t!("VPORTS: use SINGLE | 2H | 2V | 4.").as_ref());
                        }
                    }
                } else if sub.is_empty() {
                    // ── List existing viewports ──────────────────────────
                    let layout_block = scene.current_layout_block_handle_pub();
                    let viewports: Vec<_> = scene
                        .document
                        .entities()
                        .filter_map(|e| {
                            if let acadrust::EntityType::Viewport(vp) = e {
                                if vp.id > 1 && vp.common.owner_handle == layout_block {
                                    Some((
                                        vp.id,
                                        vp.center.clone(),
                                        vp.width,
                                        vp.height,
                                        crate::scene::vp_effective_scale(
                                            vp.custom_scale,
                                            vp.view_height,
                                            vp.height,
                                        ),
                                        vp.status.is_on,
                                        vp.status.locked,
                                    ))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        })
                        .collect();
                    if viewports.is_empty() {
                        self.command_line.push_info(crate::t!("No viewports. Use MVIEW to create one, or VPORTS 2H / 2V / 4 / SINGLE.").as_ref());
                    } else {
                        self.command_line.push_output(crate::tf!(
                            "{} viewport(s) in layout \"{}\":",
                            viewports.len(),
                            scene.current_layout
                        ).as_ref());
                        for (id, center, w, h, scale, is_on, locked) in &viewports {
                            let state = match (is_on, locked) {
                                (true, true) => "On, Locked",
                                (true, false) => "On",
                                (false, _) => "Off",
                            };
                            self.command_line.push_output(crate::tf!(
                                "  VP #{id}: {w:.1}×{h:.1} @ ({:.1},{:.1})  scale={scale:.4}  [{state}]",
                                center.x, center.y
                            ).as_ref());
                        }
                    }
                } else {
                    // ── Preset viewport layout ───────────────────────────
                    // Determine paper dimensions from PlotSettings (fallback A4 landscape).
                    let layout_name = scene.current_layout.clone();
                    let (paper_w, paper_h) = {
                        use acadrust::objects::ObjectType;
                        let mut pw = 297.0_f64;
                        let mut ph = 210.0_f64;
                        for (_, obj) in &scene.document.objects {
                            if let ObjectType::PlotSettings(ps) = obj {
                                if ps.page_name == layout_name && ps.paper_width > 0.0 {
                                    pw = ps.paper_width;
                                    ph = ps.paper_height;
                                    break;
                                }
                            }
                        }
                        (pw, ph)
                    };
                    let margin = 5.0_f64; // mm margin around the usable area
                    let uw = paper_w - 2.0 * margin; // usable width
                    let uh = paper_h - 2.0 * margin; // usable height
                                                     // Collect rectangle specs: (cx, cz, w, h) in mm
                    let rects: Vec<(f64, f64, f64, f64)> = match sub.as_str() {
                        "2H" => {
                            // Two viewports side by side (horizontal split)
                            let vw = (uw - 2.0) / 2.0;
                            vec![
                                (margin + vw / 2.0, margin + uh / 2.0, vw, uh),
                                (margin + vw + 2.0 + vw / 2.0, margin + uh / 2.0, vw, uh),
                            ]
                        }
                        "2V" => {
                            // Two viewports stacked (vertical split)
                            let vh = (uh - 2.0) / 2.0;
                            vec![
                                (margin + uw / 2.0, margin + vh + 2.0 + vh / 2.0, uw, vh),
                                (margin + uw / 2.0, margin + vh / 2.0, uw, vh),
                            ]
                        }
                        "4" => {
                            // Four equal viewports (2×2 grid)
                            let vw = (uw - 2.0) / 2.0;
                            let vh = (uh - 2.0) / 2.0;
                            vec![
                                (margin + vw / 2.0, margin + vh + 2.0 + vh / 2.0, vw, vh),
                                (
                                    margin + vw + 2.0 + vw / 2.0,
                                    margin + vh + 2.0 + vh / 2.0,
                                    vw,
                                    vh,
                                ),
                                (margin + vw / 2.0, margin + vh / 2.0, vw, vh),
                                (margin + vw + 2.0 + vw / 2.0, margin + vh / 2.0, vw, vh),
                            ]
                        }
                        "SINGLE" | "1" => {
                            // Single full-page viewport
                            vec![(margin + uw / 2.0, margin + uh / 2.0, uw, uh)]
                        }
                        _ => {
                            self.command_line.push_error(
                                "VPORTS: unknown option. Use VPORTS 2H | 2V | 4 | SINGLE",
                            );
                            vec![]
                        }
                    };
                    if !rects.is_empty() {
                        // Remove existing user viewports in this layout first.
                        let layout_block = self.tabs[i].scene.current_layout_block_handle_pub();
                        let to_erase: Vec<acadrust::Handle> = self.tabs[i]
                            .scene
                            .document
                            .entities()
                            .filter_map(|e| {
                                if let acadrust::EntityType::Viewport(vp) = e {
                                    if vp.id > 1 && vp.common.owner_handle == layout_block {
                                        Some(vp.common.handle)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();
                        self.push_undo_snapshot(i, "VPORTS");
                        self.tabs[i].scene.erase_entities(&to_erase);
                        // Create new viewports.
                        for (cx, cz, w, h) in &rects {
                            let mut vp = acadrust::entities::Viewport::new();
                            vp.center = acadrust::types::Vector3::new(*cx, 0.0, *cz);
                            vp.width = *w;
                            vp.height = *h;
                            vp.id = 2; // commit_entity will assign unique IDs
                            match self.tabs[i].scene.document.add_entity_to_layout(
                                acadrust::EntityType::Viewport(vp),
                                &layout_name,
                            ) {
                                Ok(handle) => {
                                    self.tabs[i].scene.auto_fit_viewport(handle);
                                }
                                Err(e) => {
                                    self.command_line.push_error(crate::tf!("VPORTS: {e}").as_ref());
                                }
                            }
                        }
                        // Re-assign unique IDs (1 + existing max per viewport).
                        let layout_block2 = self.tabs[i].scene.current_layout_block_handle_pub();
                        let mut id_counter = 2_i16;
                        let handles: Vec<acadrust::Handle> = self.tabs[i]
                            .scene
                            .document
                            .entities()
                            .filter_map(|e| {
                                if let acadrust::EntityType::Viewport(vp) = e {
                                    if vp.id >= 2 && vp.common.owner_handle == layout_block2 {
                                        Some(vp.common.handle)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();
                        for h in handles {
                            if let Some(acadrust::EntityType::Viewport(vp)) =
                                self.tabs[i].scene.document.get_entity_mut(h)
                            {
                                vp.id = id_counter;
                                id_counter += 1;
                            }
                        }
                        self.tabs[i].dirty = true;
                        self.command_line.push_output(crate::tf!(
                            "VPORTS: created {} viewport(s) [{}].",
                            rects.len(),
                            sub
                        ).as_ref());
                    }
                }
            }

            // ── VPLAYER — per-viewport layer freeze/thaw ──────────────────
            "VPLAYER" => {
                let scene = &self.tabs[i].scene;
                if scene.current_layout == "Model" {
                    self.command_line
                        .push_error(crate::t!("VPLAYER: switch to a paper space layout first.").as_ref());
                } else if scene.active_viewport.is_none() {
                    self.command_line
                        .push_error(crate::t!("VPLAYER: enter a viewport first (double-click or MS).").as_ref());
                } else {
                    use crate::modules::layout::vplayer::VplayerCommand;
                    let vp_handle = scene.active_viewport.unwrap();
                    // Collect current frozen layer names for display.
                    let frozen_names: Vec<String> = {
                        if let Some(acadrust::EntityType::Viewport(vp)) =
                            scene.document.get_entity(vp_handle)
                        {
                            vp.frozen_layers
                                .iter()
                                .filter_map(|h| {
                                    scene
                                        .document
                                        .layers
                                        .iter()
                                        .find(|l| l.handle == *h)
                                        .map(|l| l.name.clone())
                                })
                                .collect()
                        } else {
                            vec![]
                        }
                    };
                    if frozen_names.is_empty() {
                        self.command_line
                            .push_info(crate::t!("VPLAYER: no frozen layers in active viewport.").as_ref());
                    } else {
                        self.command_line.push_info(crate::tf!(
                            "VPLAYER: frozen layers: {}",
                            frozen_names.join(", ")
                        ).as_ref());
                    }
                    let new_cmd = VplayerCommand::new(vp_handle);
                    self.command_line.push_info(&new_cmd.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(new_cmd));
                }
            }

            // ── Draw Order ────────────────────────────────────────────────
            // TEXTTOFRONT / TEXTTOBACK — move every text and dimension object to
            // the front (or back) of the draw order via the DRAWORDER machinery.
            "TEXTTOFRONT" | "TEXTTOBACK" => {
                let to_front = cmd.ends_with("FRONT");
                let handles: rustc_hash::FxHashSet<acadrust::Handle> = self.tabs[i]
                    .scene
                    .document
                    .entities()
                    .filter(|e| {
                        matches!(
                            e,
                            acadrust::EntityType::Text(_)
                                | acadrust::EntityType::MText(_)
                                | acadrust::EntityType::Dimension(_)
                        )
                    })
                    .map(|e| e.common().handle)
                    .collect();
                if handles.is_empty() {
                    self.command_line
                        .push_info(crate::tf!("{cmd}: no text or dimension objects.").as_ref());
                    return Some(Task::none());
                }
                self.tabs[i].scene.replace_selection(handles);
                return self.dispatch_view(
                    if to_front {
                        "DRAWORDER FRONT"
                    } else {
                        "DRAWORDER BACK"
                    },
                    i,
                );
            }

            "DRAWORDER" => {
                use crate::command::SelectThenKeywordCommand;
                let has_sel = !self.tabs[i].scene.selected_entities().is_empty();
                let c = SelectThenKeywordCommand::new(
                    "DRAWORDER",
                    "DRAWORDER  [Front / Back]  (Above/Under <handle> by typing):",
                    vec![("Front", "FRONT", None), ("Back", "BACK", None)],
                    has_sel,
                );
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
            }
            cmd if cmd.starts_with("DRAWORDER ") => {
                use acadrust::objects::{ObjectType, SortEntitiesTable};
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                let option = parts.get(1).unwrap_or(&"").to_uppercase();
                let i = self.active_tab;
                let selected: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .map(|(h, _)| *h)
                    .collect();
                if selected.is_empty() {
                    self.command_line
                        .push_error(crate::t!("DRAWORDER: select entities first.").as_ref());
                } else {
                    // Parse relative target handle for ABOVE/UNDER.
                    let relative_target: Option<(bool, acadrust::Handle)> = match option.as_str() {
                        "A" | "ABOVE" => {
                            let h_val = parts.get(2).and_then(|s| u64::from_str_radix(s, 16).ok());
                            h_val.map(|v| (true, acadrust::Handle::new(v)))
                        }
                        "U" | "UNDER" | "BELOW" => {
                            let h_val = parts.get(2).and_then(|s| u64::from_str_radix(s, 16).ok());
                            h_val.map(|v| (false, acadrust::Handle::new(v)))
                        }
                        _ => None,
                    };
                    let to_front_opt = match option.as_str() {
                        "F" | "FRONT" => Some(true),
                        "B" | "BACK" => Some(false),
                        _ => None,
                    };

                    if relative_target.is_some() || to_front_opt.is_some() {
                        self.push_undo_snapshot(i, "DRAWORDER");
                        let block_handle = self.tabs[i].scene.current_layout_block_handle_pub();

                        // For FRONT/BACK, anchor the new sort handle to the
                        // block's current effective draw-order range so the moved
                        // entities land strictly above/below every sibling —
                        // including ones not yet in the table, which sort by
                        // their own handle. (min_eff, max_eff) over siblings.
                        let fb_baseline: Option<(u64, u64)> = if to_front_opt.is_some() {
                            let selected_set: rustc_hash::FxHashSet<u64> =
                                selected.iter().map(|h| h.value()).collect();
                            let doc_ref = &self.tabs[i].scene.document;
                            let overrides: rustc_hash::FxHashMap<u64, u64> = doc_ref
                                .objects
                                .values()
                                .find_map(|obj| {
                                    if let ObjectType::SortEntitiesTable(t) = obj {
                                        if t.block_owner_handle == block_handle {
                                            return Some(
                                                t.entries()
                                                    .map(|e| {
                                                        (
                                                            e.entity_handle.value(),
                                                            e.sort_handle.value(),
                                                        )
                                                    })
                                                    .collect(),
                                            );
                                        }
                                    }
                                    None
                                })
                                .unwrap_or_default();
                            let mut max_eff = 0u64;
                            let mut min_eff = u64::MAX;
                            for e in doc_ref.entities() {
                                let c = e.common();
                                let hv = c.handle.value();
                                if selected_set.contains(&hv) {
                                    continue;
                                }
                                if c.owner_handle != block_handle && !c.owner_handle.is_null() {
                                    continue;
                                }
                                let eff = overrides.get(&hv).copied().unwrap_or(hv);
                                max_eff = max_eff.max(eff);
                                min_eff = min_eff.min(eff);
                            }
                            if min_eff == u64::MAX {
                                min_eff = 1;
                            }
                            Some((min_eff, max_eff))
                        } else {
                            None
                        };

                        let doc = &mut self.tabs[i].scene.document;
                        let table_handle = doc.objects.iter().find_map(|(h, obj)| {
                            if let ObjectType::SortEntitiesTable(t) = obj {
                                if t.block_owner_handle == block_handle {
                                    Some(*h)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                        let get_or_create =
                            |doc: &mut acadrust::CadDocument, block_handle| -> acadrust::Handle {
                                if let Some(th) = doc.objects.iter().find_map(|(h, obj)| {
                                    if let ObjectType::SortEntitiesTable(t) = obj {
                                        if t.block_owner_handle == block_handle {
                                            Some(*h)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                }) {
                                    th
                                } else {
                                    let nh = acadrust::Handle::new(doc.next_handle());
                                    let mut table = SortEntitiesTable::for_block(block_handle);
                                    table.handle = nh;
                                    doc.objects.insert(nh, ObjectType::SortEntitiesTable(table));
                                    nh
                                }
                            };
                        let th = table_handle.unwrap_or_else(|| {
                            let nh = acadrust::Handle::new(doc.next_handle());
                            let mut table = SortEntitiesTable::for_block(block_handle);
                            table.handle = nh;
                            doc.objects.insert(nh, ObjectType::SortEntitiesTable(table));
                            nh
                        });
                        let _ = get_or_create; // suppress unused warning
                        if let Some(ObjectType::SortEntitiesTable(table)) = doc.objects.get_mut(&th)
                        {
                            if let Some((above, target)) = relative_target {
                                // move_above/move_below read the target's sort
                                // handle from the table and no-op when it is
                                // absent. A reference object that was never
                                // reordered isn't in the table yet, so seed it
                                // with its own handle as the implicit sort key.
                                if !table.contains(target) {
                                    table.add_entry(target, target);
                                }
                                for h in &selected {
                                    if above {
                                        table.move_above(*h, target);
                                    } else {
                                        table.move_below(*h, target);
                                    }
                                }
                                let rel = if above { "above" } else { "below" };
                                self.command_line.push_info(crate::tf!(
                                    "DRAWORDER: moved {} entities {} {:x}.",
                                    selected.len(),
                                    rel,
                                    target.value()
                                ).as_ref());
                            } else if let Some(to_front) = to_front_opt {
                                let (min_eff, max_eff) = fb_baseline.unwrap_or((1, 0));
                                for (k, h) in selected.iter().enumerate() {
                                    let sort = if to_front {
                                        max_eff.saturating_add(1 + k as u64)
                                    } else {
                                        min_eff.saturating_sub(1 + k as u64).max(1)
                                    };
                                    table.add_entry(*h, acadrust::Handle::new(sort));
                                }
                                let dir = if to_front { "front" } else { "back" };
                                self.command_line.push_info(crate::tf!(
                                    "DRAWORDER: moved {} entities to {}.",
                                    selected.len(),
                                    dir
                                ).as_ref());
                            }
                        }
                        // Sort order lives in SortEntitiesTable, which the
                        // render-side `sort_cache` rebuilds per geometry epoch.
                        // Bump it so the new draw order shows immediately
                        // instead of waiting for an unrelated geometry change.
                        // Draw order changes submission order only; all
                        // per-entity tessellation remains valid.
                        self.tabs[i].scene.bump_geometry_no_blocks();
                        self.tabs[i].dirty = true;
                    } else {
                        self.command_line.push_info(
                            crate::t!("Usage: DRAWORDER F|FRONT | B|BACK | A|ABOVE <handle> | U|UNDER <handle>").as_ref()
                        );
                    }
                }
            }

            // SYNCPVIEWPORTS — copy the first selected viewport's display settings
            // (view direction/target, scale, snap/grid, frozen layers) to the rest.
            "SYNCPVIEWPORTS" | "VPSYNC" => {
                let vps: Vec<acadrust::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .iter()
                    .filter(|(_, e)| matches!(e, acadrust::EntityType::Viewport(_)))
                    .map(|(h, _)| *h)
                    .collect();
                if vps.len() < 2 {
                    self.command_line.push_error(
                        "SYNCPVIEWPORTS: select two or more viewports (the first is the master).",
                    );
                    return Some(Task::none());
                }
                let src = match self.tabs[i].scene.document.get_entity(vps[0]) {
                    Some(acadrust::EntityType::Viewport(vp)) => vp.clone(),
                    _ => {
                        self.command_line
                            .push_error(crate::t!("SYNCPVIEWPORTS: master is not a viewport.").as_ref());
                        return Some(Task::none());
                    }
                };
                self.push_undo_snapshot(i, "SYNCPVIEWPORTS");
                let mut n = 0usize;
                for h in &vps[1..] {
                    if let Some(acadrust::EntityType::Viewport(vp)) =
                        self.tabs[i].scene.document.get_entity_mut(*h)
                    {
                        vp.view_direction = src.view_direction;
                        vp.view_target = src.view_target;
                        vp.view_height = src.view_height;
                        vp.snap_base = src.snap_base;
                        vp.snap_spacing = src.snap_spacing;
                        vp.snap_angle = src.snap_angle;
                        vp.grid_spacing = src.grid_spacing;
                        vp.grid_major = src.grid_major;
                        vp.frozen_layers = src.frozen_layers.clone();
                        n += 1;
                    }
                }
                self.tabs[i].dirty = true;
                self.command_line.push_output(crate::tf!(
                    "SYNCPVIEWPORTS: synced {n} viewport(s) to the master."
                ).as_ref());
            }

            // HIDE — hidden-line view of the active viewport.
            "HIDE" => {
                return Some(Task::done(Message::SetRenderMode(
                    acadrust::entities::ViewportRenderMode::HiddenLine,
                )));
            }

            // VISUALSTYLES <name> — apply a built-in visual style to the active
            // viewport via its render mode (the style-definition manager dialog
            // is not modelled; this applies the standard styles).
            cmd if cmd == "VISUALSTYLES" || cmd.starts_with("VISUALSTYLES ") => {
                use acadrust::entities::ViewportRenderMode as VRM;
                let name = cmd.strip_prefix("VISUALSTYLES").unwrap_or("").trim().to_uppercase();
                let mode = match name.as_str() {
                    "2DWIREFRAME" | "2D" => Some(VRM::Wireframe2D),
                    "3DWIREFRAME" | "WIREFRAME" | "3D" => Some(VRM::Wireframe3D),
                    "HIDDEN" | "HIDDENLINE" => Some(VRM::HiddenLine),
                    "FLAT" | "FLATSHADED" => Some(VRM::FlatShaded),
                    "REALISTIC" | "SHADED" | "GOURAUD" => Some(VRM::GouraudShaded),
                    "CONCEPTUAL" | "SHADEDWITHEDGES" => Some(VRM::GouraudShadedWithEdges),
                    _ => None,
                };
                match mode {
                    Some(m) => return Some(Task::done(Message::SetRenderMode(m))),
                    None => self.command_line.push_info(
                        "VISUALSTYLES <2DWIREFRAME|3DWIREFRAME|HIDDEN|REALISTIC|CONCEPTUAL|SHADED>",
                    ),
                }
            }

            _ => return None,
        }
        Some(self.finish_dispatch(cmd))
    }
}

// ── Draw Order: interactive reference-object pick ──────────────────────────

/// Moves a captured selection above or below a reference object the user
/// picks in the viewport. On pick it relaunches `DRAWORDER A|U <handle>`
/// with the captured handles reinstalled as the selection, so the existing
/// command path performs the actual reorder.
pub(crate) struct DrawOrderRefCommand {
    to_move: Vec<acadrust::Handle>,
    above: bool,
}

impl DrawOrderRefCommand {
    pub(crate) fn new(to_move: Vec<acadrust::Handle>, above: bool) -> Self {
        Self { to_move, above }
    }
}

impl CadCommand for DrawOrderRefCommand {
    fn name(&self) -> &'static str {
        "DRAWORDER"
    }

    fn prompt(&self) -> String {
        if self.above {
            crate::t!("DRAWORDER  Select reference object (move selection above):").into_owned()
        } else {
            crate::t!("DRAWORDER  Select reference object (move selection under):").into_owned()
        }
    }

    fn needs_entity_pick(&self) -> bool {
        true
    }

    fn on_entity_pick(
        &mut self,
        handle: acadrust::Handle,
        _pt: glam::DVec3,
    ) -> crate::command::CmdResult {
        if handle.is_null() {
            return crate::command::CmdResult::NeedPoint;
        }
        let opt = if self.above { "A" } else { "U" };
        let cmd = format!("DRAWORDER {} {:x}", opt, handle.value());
        crate::command::CmdResult::Relaunch(cmd, std::mem::take(&mut self.to_move))
    }

    fn on_point(&mut self, _pt: glam::DVec3) -> crate::command::CmdResult {
        crate::command::CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> crate::command::CmdResult {
        crate::command::CmdResult::Cancel
    }
}
