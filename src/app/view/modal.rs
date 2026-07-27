use super::super::{Message, OpenCADStudio};
use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{Background, Border, Color, Element, Fill, Theme};

impl OpenCADStudio {
    /// Title shown in the active modal's title bar, left of the move/close
    /// buttons. Keep in sync with the [`Self::modal_content`] dispatch.
    pub(super) fn modal_title(&self) -> &'static str {
        use super::super::ModalKind as K;
        match self.active_modal {
            Some(K::About) => "About",
            Some(K::Shortcuts) => "Keyboard Shortcuts",
            Some(K::Aliases) => "Command Aliases",
            Some(K::PluginManager) => "Plugin Manager",
            Some(K::UpdateNotice) => "Update Available",
            Some(K::Layers) => "Layer Manager",
            Some(K::Plot) => "Plot",
            Some(K::LayoutManager) => "Layout Manager",
            Some(K::ScaleManager) => "Scale Manager",
            Some(K::AnnoObjectScale) => "Annotation Object Scale",
            Some(K::Plotstyle) => "Plot Style Editor",
            Some(K::TextStyle) => "Text Style Manager",
            Some(K::MlStyle) => "Multiline Style Manager",
            Some(K::TableStyle) => "Table Style Manager",
            Some(K::MLeaderStyle) => "Multileader Style Manager",
            Some(K::DimStyle) => "Dimension Style Manager",
            Some(K::AssocPrompt) => "Default Application",
            Some(K::AecDropWarning) => "Save Warning",
            #[cfg(not(target_arch = "wasm32"))]
            Some(K::FileInUse) => "Unable to Save Drawing",
            #[cfg(not(target_arch = "wasm32"))]
            Some(K::ExternalChange) => "Drawing Changed on Disk",
            Some(K::LayerDeleteWarning) => "Delete Layer",
            Some(K::Unsaved) => "Unsaved Changes",
            Some(K::PointStyle) => "Point Style",
            Some(K::AttributeEditor) => "Attribute Editor",
            Some(K::SaveDialog) => "Save Drawing As",
            None => "",
        }
    }

    /// Build the currently-open modal dialog's content (Plan B), or `None`.
    /// Each former pop-up window is constructed here and given a bounded size
    /// (About shrinks to its content). Rendered as an overlay by `view_main`.
    pub(super) fn modal_content<'s>(&'s self) -> Option<Element<'s, Message>> {
        // Grow every dialog by the shared corner-resize delta (grow-only, so the
        // natural size is the floor). The delta resets to zero whenever a modal
        // opens/closes, so each dialog starts at its own size.
        let ex = self.modal_resize;
        let sized = |e: Element<'s, Message>, w: u16, h: u16| -> Element<'s, Message> {
            iced::widget::container(e)
                .width(iced::Length::Fixed(w as f32 + ex.x))
                .height(iced::Length::Fixed(h as f32 + ex.y))
                .into()
        };
        Some(match self.active_modal? {
            super::super::ModalKind::About => crate::ui::window::about::view_window(),
            super::super::ModalKind::Shortcuts => {
                sized(crate::ui::window::shortcuts::view_window(&self.shortcut_overrides), 720, 520)
            }
            super::super::ModalKind::Aliases => {
                sized(crate::ui::window::alias_editor::view_window(&self.alias_editor_rows), 480, 520)
            }
            super::super::ModalKind::PluginManager => sized(
                crate::ui::window::plugin_manager::view_window(
                    &self.disabled_plugins,
                    &self.external_plugins,
                    &self.loaded_plugin_ids,
                    crate::ui::window::plugin_manager::MarketView {
                        registry: &self.plugin_registry,
                        input: &self.plugin_repo_input,
                        repos: &self.plugin_repos,
                        release_tags: &self.repo_release_tags,
                        selected_tag: &self.repo_selected_tag,
                        status: &self.marketplace_status,
                    },
                ),
                520,
                460,
            ),
            super::super::ModalKind::UpdateNotice => {
                let latest = self.update_notice_version.as_deref().unwrap_or("?");
                let body = self.update_notice_body.as_deref().unwrap_or("");
                sized(crate::ui::window::update_notice::view_window(latest, body), 560, 460)
            }
            super::super::ModalKind::Layers => {
                let tab = &self.tabs[self.active_tab];
                sized(tab.layers.view_window(self.layer_name_col_w), 900, 360)
            }
            super::super::ModalKind::Plot => {
                sized(crate::ui::window::plot::view_window(&self.plot_dialog), 760, 540)
            }
            super::super::ModalKind::LayoutManager => {
                let i = self.active_tab;
                let layouts = self.tabs[i].scene.layout_names();
                let current = self.tabs[i].scene.current_layout.clone();
                sized(
                    crate::ui::window::layout_manager::view_window(
                        layouts,
                        &self.layout_manager_selected,
                        &self.layout_manager_rename_buf,
                        current,
                    ),
                    640,
                    320,
                )
            }
            super::super::ModalKind::ScaleManager => {
                let tab = &self.tabs[self.active_tab];
                let scales: Vec<(String, String)> = tab
                    .scene
                    .scale_list()
                    .into_iter()
                    .map(|(name, _, _)| {
                        let ratio = tab
                            .scene
                            .scale_paper_drawing(&name)
                            .map(|(p, d)| format!("{p}:{d}"))
                            .unwrap_or_default();
                        (name, ratio)
                    })
                    .collect();
                let current = tab.scene.document.header.current_annotation_scale.clone();
                sized(
                    crate::ui::style::scale_manager::view_window(
                        &scales,
                        &self.scale_manager_selected,
                        &current,
                        self.scale_rename.as_deref(),
                        &self.scale_rename_buf,
                        &self.scale_manager_paper_buf,
                        &self.scale_manager_drawing_buf,
                    ),
                    520,
                    360,
                )
            }
            super::super::ModalKind::AnnoObjectScale => {
                let tab = &self.tabs[self.active_tab];
                let entity = self.anno_object_scale_target;
                // Which scales the object currently has a representation for.
                let members: Vec<acadrust::types::Handle> = entity
                    .map(|h| {
                        crate::scene::annotative::object_scale_memberships(
                            &tab.scene.document,
                            h,
                        )
                        .into_iter()
                        .map(|(_, sh)| sh)
                        .collect()
                    })
                    .unwrap_or_default();
                let label = entity
                    .and_then(|h| tab.scene.document.get_entity(h))
                    .map(|e| match e {
                        acadrust::EntityType::Text(_) => "TEXT",
                        acadrust::EntityType::MText(_) => "MTEXT",
                        acadrust::EntityType::Insert(_) => "BLOCK",
                        acadrust::EntityType::MultiLeader(_) => "MULTILEADER",
                        _ => "OBJECT",
                    })
                    .unwrap_or("—");
                let scales: Vec<(String, String, bool)> = tab
                    .scene
                    .scale_list()
                    .into_iter()
                    .map(|(name, _, _)| {
                        let sh = tab.scene.scale_object_handle(&name);
                        let ratio = tab
                            .scene
                            .scale_paper_drawing(&name)
                            .map(|(p, d)| format!("{p}:{d}"))
                            .unwrap_or_default();
                        let is_member = sh.map(|h| members.contains(&h)).unwrap_or(false);
                        (name, ratio, is_member)
                    })
                    .collect();
                sized(
                    crate::ui::style::anno_object_scale::view_window(&label, &scales),
                    360,
                    420,
                )
            }
            super::super::ModalKind::Plotstyle => sized(
                crate::ui::style::plotstyle::view_window(
                    self.active_plot_style.as_ref(),
                    self.plotstyle_panel_aci,
                    &self.ps_color_buf,
                    &self.ps_lineweight_buf,
                    &self.ps_screening_buf,
                ),
                780,
                540,
            ),
            super::super::ModalKind::TextStyle => {
                let tab = &self.tabs[self.active_tab];
                let styles: Vec<String> = tab
                    .scene
                    .document
                    .text_styles
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                let (backward, upside_down, annotative) = tab
                    .scene
                    .document
                    .text_styles
                    .get(&self.textstyle_selected)
                    .map(|s| (s.flags.backward, s.flags.upside_down, s.annotative))
                    .unwrap_or((false, false, false));
                sized(
                    crate::ui::style::textstyle::view_window(crate::ui::style::textstyle::TextStyleView {
                        styles,
                        selected: &self.textstyle_selected,
                        current: &tab.scene.document.header.current_text_style_name,
                        font_buf: &self.textstyle_font,
                        width_buf: &self.textstyle_width,
                        oblique_buf: &self.textstyle_oblique,
                        height_buf: &self.textstyle_height,
                        bigfont_buf: &self.textstyle_bigfont,
                        ttf_buf: &self.textstyle_ttf,
                        backward,
                        upside_down,
                        annotative,
                        rename_active: self.style_rename.as_deref(),
                        rename_buf: &self.style_rename_buf,
                    }),
                    // Wider than the old 620 window: the TTF system-font panel
                    // (Plan B / web fonts) added a column.
                    860,
                    480,
                )
            }
            super::super::ModalKind::MlStyle => {
                use acadrust::objects::ObjectType;
                let tab = &self.tabs[self.active_tab];
                let styles: Vec<String> = tab
                    .scene
                    .document
                    .objects
                    .values()
                    .filter_map(|o| match o {
                        ObjectType::MLineStyle(s) => Some(s.name.clone()),
                        _ => None,
                    })
                    .collect();
                let selected_style = tab.scene.document.objects.values().find_map(|o| match o {
                    ObjectType::MLineStyle(s) if s.name == self.mlstyle_selected => Some(s),
                    _ => None,
                });
                sized(
                    crate::ui::style::mlstyle::view_window(
                        styles,
                        &self.mlstyle_selected,
                        selected_style,
                        tab.scene.document.header.multiline_style.clone(),
                        self.style_rename.as_deref(),
                        &self.style_rename_buf,
                    ),
                    620,
                    420,
                )
            }
            super::super::ModalKind::TableStyle => {
                use acadrust::objects::ObjectType;
                let tab = &self.tabs[self.active_tab];
                let styles: Vec<String> = tab
                    .scene
                    .document
                    .objects
                    .values()
                    .filter_map(|o| match o {
                        ObjectType::TableStyle(s) => Some(s.name.clone()),
                        _ => None,
                    })
                    .collect();
                let selected_style = tab.scene.document.objects.values().find_map(|o| match o {
                    ObjectType::TableStyle(s) if s.name == self.tablestyle_selected => Some(s),
                    _ => None,
                });
                sized(
                    crate::ui::style::tablestyle::view_window(
                        styles,
                        &self.tablestyle_selected,
                        &self.ribbon.active_table_style,
                        selected_style,
                        &self.ts_hmargin,
                        &self.ts_vmargin,
                        &self.ts_description,
                        &self.ts_cell_textstyle,
                        &self.ts_cell_height,
                        &self.ts_cell_textcolor,
                        &self.ts_cell_fillcolor,
                        &self.ts_cell_datatype,
                        &self.ts_cell_unittype,
                        &self.ts_cell_format,
                        &self.ts_border_lw,
                        &self.ts_border_color,
                        &self.ts_border_spacing,
                        self.style_rename.as_deref(),
                        &self.style_rename_buf,
                        self.ts_color_open,
                    ),
                    620,
                    420,
                )
            }
            super::super::ModalKind::MLeaderStyle => {
                use acadrust::objects::ObjectType;
                let tab = &self.tabs[self.active_tab];
                let styles: Vec<String> = tab
                    .scene
                    .document
                    .objects
                    .values()
                    .filter_map(|o| match o {
                        ObjectType::MultiLeaderStyle(s) => Some(s.name.clone()),
                        _ => None,
                    })
                    .collect();
                let selected_style = tab.scene.document.objects.values().find_map(|o| match o {
                    ObjectType::MultiLeaderStyle(s) if s.name == self.mleaderstyle_selected => {
                        Some(s)
                    }
                    _ => None,
                });
                let doc = &tab.scene.document;
                let mut block_opts: Vec<String> = vec!["None".to_string()];
                block_opts.extend(doc.block_records.iter().map(|b| b.name.clone()));
                let mut lt_opts: Vec<String> = vec!["None".to_string()];
                lt_opts.extend(doc.line_types.iter().map(|lt| lt.name.clone()));
                let mut textstyle_opts: Vec<String> = vec!["None".to_string()];
                textstyle_opts.extend(doc.text_styles.iter().map(|t| t.name.clone()));
                let opt_block = |h: Option<acadrust::types::Handle>| -> String {
                    match h {
                        Some(h) => doc
                            .block_records
                            .iter()
                            .find(|b| b.handle == h)
                            .map(|b| b.name.clone())
                            .unwrap_or_else(|| "None".to_string()),
                        None => "None".to_string(),
                    }
                };
                let opt_lt = |h: Option<acadrust::types::Handle>| -> String {
                    match h {
                        Some(h) => doc
                            .line_types
                            .iter()
                            .find(|lt| lt.handle == h)
                            .map(|lt| lt.name.clone())
                            .unwrap_or_else(|| "None".to_string()),
                        None => "None".to_string(),
                    }
                };
                let opt_ts = |h: Option<acadrust::types::Handle>| -> String {
                    match h {
                        Some(h) => doc
                            .text_styles
                            .iter()
                            .find(|t| t.handle == h)
                            .map(|t| t.name.clone())
                            .unwrap_or_else(|| "None".to_string()),
                        None => "None".to_string(),
                    }
                };
                let (line_type_name, arrowhead_name, text_style_name, block_content_name) =
                    match selected_style {
                        Some(s) => (
                            opt_lt(s.line_type_handle),
                            opt_block(s.arrowhead_handle),
                            opt_ts(s.text_style_handle),
                            opt_block(s.block_content_handle),
                        ),
                        None => Default::default(),
                    };
                sized(
                    crate::ui::style::mleaderstyle::view_window(crate::ui::style::mleaderstyle::MLeaderStyleView {
                        styles,
                        selected: &self.mleaderstyle_selected,
                        style: selected_style,
                        current: tab.active_mleader_style.clone(),
                        landing_distance: &self.mls_landing_distance,
                        landing_gap: &self.mls_landing_gap,
                        arrowhead_size: &self.mls_arrowhead_size,
                        text_height: &self.mls_text_height,
                        scale_factor: &self.mls_scale_factor,
                        break_gap: &self.mls_break_gap,
                        first_seg_angle: &self.mls_first_seg_angle,
                        second_seg_angle: &self.mls_second_seg_angle,
                        max_points: &self.mls_max_points,
                        default_text: &self.mls_default_text,
                        line_color: &self.mls_line_color,
                        text_color: &self.mls_text_color,
                        description: &self.mls_description,
                        align_space: &self.mls_align_space,
                        block_color: &self.mls_block_color,
                        block_rotation: &self.mls_block_rotation,
                        block_scale_x: &self.mls_block_scale_x,
                        block_scale_y: &self.mls_block_scale_y,
                        block_scale_z: &self.mls_block_scale_z,
                        block_opts,
                        lt_opts,
                        textstyle_opts,
                        line_type_name,
                        arrowhead_name,
                        text_style_name,
                        block_content_name,
                        rename_active: self.style_rename.as_deref(),
                        rename_buf: &self.style_rename_buf,
                        color_open: self.mls_color_open,
                    }),
                    560,
                    560,
                )
            }
            super::super::ModalKind::DimStyle => {

            let tab = &self.tabs[self.active_tab];
            let styles: Vec<String> = tab
                .scene
                .document
                .dim_styles
                .iter()
                .map(|s| s.name.clone())
                .collect();
            let doc = &tab.scene.document;
            // Dropdown options (names must match the records exactly so the
            // selection can be resolved back to a handle on the update side).
            let mut block_opts: Vec<String> = vec!["Default".to_string()];
            block_opts.extend(doc.block_records.iter().map(|b| b.name.clone()));
            let mut lt_opts: Vec<String> = vec!["ByBlock".to_string()];
            lt_opts.extend(doc.line_types.iter().map(|lt| lt.name.clone()));
            let blk_name = |h: acadrust::types::Handle| -> String {
                if h.is_null() {
                    "Default".to_string()
                } else {
                    doc.block_records
                        .iter()
                        .find(|b| b.handle == h)
                        .map(|b| b.name.clone())
                        .unwrap_or_else(|| "Default".to_string())
                }
            };
            let lt_name = |h: acadrust::types::Handle| -> String {
                if h.is_null() {
                    "ByBlock".to_string()
                } else {
                    doc.line_types
                        .iter()
                        .find(|lt| lt.handle == h)
                        .map(|lt| lt.name.clone())
                        .unwrap_or_else(|| "ByBlock".to_string())
                }
            };
            let ds_sel = doc.dim_styles.get(&self.dimstyle_selected);
            let (
                dimblk_name,
                dimblk1_name,
                dimblk2_name,
                dimldrblk_name,
                dimltex_name,
                dimltex1_name,
                dimltex2_name,
            ) = match ds_sel {
                Some(d) => (
                    blk_name(d.dimblk),
                    blk_name(d.dimblk1),
                    blk_name(d.dimblk2),
                    blk_name(d.dimldrblk),
                    lt_name(d.dimltex_handle),
                    lt_name(d.dimltex1_handle),
                    lt_name(d.dimltex2_handle),
                ),
                None => Default::default(),
            };
            sized(crate::ui::style::dimstyle::view_window(
                styles,
                &self.dimstyle_selected,
                &self.tabs[self.active_tab]
                    .scene
                    .document
                    .header
                    .current_dimstyle_name,
                self.dimstyle_tab,
                crate::ui::style::dimstyle::DimStyleValues {
                    dimdle: &self.ds_dimdle,
                    dimdli: &self.ds_dimdli,
                    dimgap: &self.ds_dimgap,
                    dimexe: &self.ds_dimexe,
                    dimexo: &self.ds_dimexo,
                    dimsd1: self.ds_dimsd1,
                    dimsd2: self.ds_dimsd2,
                    dimse1: self.ds_dimse1,
                    dimse2: self.ds_dimse2,
                    dimasz: &self.ds_dimasz,
                    dimcen: &self.ds_dimcen,
                    dimtsz: &self.ds_dimtsz,
                    dimtxt: &self.ds_dimtxt,
                    dimtxsty: &self.ds_dimtxsty,
                    dimtad: &self.ds_dimtad,
                    dimtih: self.ds_dimtih,
                    dimtoh: self.ds_dimtoh,
                    dimscale: &self.ds_dimscale,
                    dimlfac: &self.ds_dimlfac,
                    dimlunit: &self.ds_dimlunit,
                    dimdec: &self.ds_dimdec,
                    dimpost: &self.ds_dimpost,
                    dimtol: self.ds_dimtol,
                    dimlim: self.ds_dimlim,
                    dimtp: &self.ds_dimtp,
                    dimtm: &self.ds_dimtm,
                    dimtdec: &self.ds_dimtdec,
                    dimtfac: &self.ds_dimtfac,
                    annotative: self.ds_annotative,
                    dimclrd: &self.ds_dimclrd,
                    dimlwd: &self.ds_dimlwd,
                    dimclre: &self.ds_dimclre,
                    dimlwe: &self.ds_dimlwe,
                    dimfxl: &self.ds_dimfxl,
                    dimfxlon: self.ds_dimfxlon,
                    dimsah: self.ds_dimsah,
                    dimarcsym: &self.ds_dimarcsym,
                    dimjogang: &self.ds_dimjogang,
                    dimclrt: &self.ds_dimclrt,
                    dimjust: &self.ds_dimjust,
                    dimtvp: &self.ds_dimtvp,
                    dimtfill: &self.ds_dimtfill,
                    dimtfillclr: &self.ds_dimtfillclr,
                    dimtxtdirection: self.ds_dimtxtdirection,
                    dimatfit: &self.ds_dimatfit,
                    dimtix: self.ds_dimtix,
                    dimsoxd: self.ds_dimsoxd,
                    dimtmove: &self.ds_dimtmove,
                    dimupt: self.ds_dimupt,
                    dimtofl: self.ds_dimtofl,
                    dimfit: &self.ds_dimfit,
                    dimdsep: &self.ds_dimdsep,
                    dimrnd: &self.ds_dimrnd,
                    dimzin: &self.ds_dimzin,
                    dimfrac: &self.ds_dimfrac,
                    dimaunit: &self.ds_dimaunit,
                    dimadec: &self.ds_dimadec,
                    dimunit: &self.ds_dimunit,
                    dimazin: &self.ds_dimazin,
                    dimalt: self.ds_dimalt,
                    dimaltf: &self.ds_dimaltf,
                    dimaltd: &self.ds_dimaltd,
                    dimaltu: &self.ds_dimaltu,
                    dimalttd: &self.ds_dimalttd,
                    dimaltrnd: &self.ds_dimaltrnd,
                    dimapost: &self.ds_dimapost,
                    dimaltz: &self.ds_dimaltz,
                    dimalttz: &self.ds_dimalttz,
                    dimtolj: &self.ds_dimtolj,
                    dimtzin: &self.ds_dimtzin,
                    dimblk_name,
                    dimblk1_name,
                    dimblk2_name,
                    dimldrblk_name,
                    dimltex_name,
                    dimltex1_name,
                    dimltex2_name,
                    block_opts,
                    lt_opts,
                    color_open: self.ds_color_open.clone(),
                },
                self.style_rename.as_deref(),
                &self.style_rename_buf,
            ), 720, 560)
            }
            super::super::ModalKind::AssocPrompt => sized(default_assoc_dialog_window(), 440, 210),
            super::super::ModalKind::AecDropWarning => {
                let src_label = self
                    .tabs
                    .get(self.active_tab)
                    .map(|t| crate::io::format_for_version(t.scene.document.version, false))
                    .unwrap_or_else(|| "DWG".to_string());
                sized(
                    aec_drop_dialog_window(
                        self.aec_drop_count,
                        &self.save_dialog_format,
                        &src_label,
                    ),
                    480,
                    230,
                )
            }
            #[cfg(not(target_arch = "wasm32"))]
            super::super::ModalKind::FileInUse => {
                let (path, error) = self
                    .pending_save_failure
                    .as_ref()
                    .map(|failure| {
                        (
                            failure.path.display().to_string(),
                            failure.error.clone(),
                        )
                    })
                    .unwrap_or_default();
                sized(file_in_use_dialog_window(&path, &error), 560, 250)
            }
            #[cfg(not(target_arch = "wasm32"))]
            super::super::ModalKind::ExternalChange => {
                let path = self
                    .pending_external_change
                    .as_ref()
                    .map(|conflict| conflict.path.display().to_string())
                    .unwrap_or_default();
                sized(external_change_dialog_window(&path), 620, 250)
            }
            super::super::ModalKind::LayerDeleteWarning => {
                let (names, count) = self
                    .layer_delete_pending
                    .clone()
                    .unwrap_or_else(|| (Vec::new(), 0));
                sized(layer_delete_warning_window(&names, count), 440, 200)
            }
            super::super::ModalKind::Unsaved => {
                let tab_name = match &self.pending_close {
                    Some(super::super::PendingClose::Tab(idx)) => self
                        .tabs
                        .get(*idx)
                        .map(|t| t.tab_display_name())
                        .unwrap_or_default(),
                    Some(super::super::PendingClose::Quit) => self
                        .tabs
                        .iter()
                        .find(|t| t.dirty)
                        .map(|t| t.tab_display_name())
                        .unwrap_or_default(),
                    None => String::new(),
                };
                sized(unsaved_changes_dialog_window(&tab_name), 420, 160)
            }
            super::super::ModalKind::PointStyle => sized(
                crate::ui::style::point_style::view_window(
                    self.tabs[self.active_tab].scene.document.header.point_display_mode,
                    self.point_size_relative,
                    &self.point_size_buf,
                ),
                360,
                470,
            ),
            super::super::ModalKind::AttributeEditor => {
                let doc = &self.tabs[self.active_tab].scene.document;
                let layers: Vec<String> = doc.layers.iter().map(|l| l.name.clone()).collect();
                let mut linetypes: Vec<String> = vec!["ByLayer".to_string()];
                linetypes.extend(
                    doc.line_types
                        .iter()
                        .map(|lt| lt.name.clone())
                        .filter(|n| !n.is_empty() && n != "ByLayer"),
                );
                let styles: Vec<String> = doc
                    .text_styles
                    .iter()
                    .map(|s| s.name.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .collect();
                sized(
                    crate::ui::window::attribute_editor::view_window(
                        &self.attr_editor_block,
                        &self.attr_editor_rows,
                        self.attr_editor_selected,
                        self.attr_editor_tab,
                        layers,
                        linetypes,
                        styles,
                    ),
                    640,
                    500,
                )
            }
            super::super::ModalKind::SaveDialog => {
                // Native omits the file-name field (the OS dialog collects it),
                // so it stands shorter than the web build.
                #[cfg(target_arch = "wasm32")]
                let h = 200;
                #[cfg(not(target_arch = "wasm32"))]
                let h = 150;
                sized(
                    save_as_dialog_window(&self.save_dialog_filename, &self.save_dialog_format),
                    420,
                    h,
                )
            }
        })
    }
}

const SAVE_FORMAT_OPTIONS: &[&str] = &[
    "DWG 2018", "DWG 2013", "DWG 2010", "DWG 2007", "DWG 2004", "DWG 2000", "DWG R14", "DXF 2018",
    "DXF 2013", "DXF 2010", "DXF 2007", "DXF 2004", "DXF 2000", "DXF R14",
];

/// Compact Save-As options dialog: pick the format/version and a default file
/// name. The destination folder and overwrite confirmation come from the
/// native OS save dialog (native) or the browser download (web) that follows.
fn save_as_dialog_window<'a>(filename: &'a str, format: &'a str) -> Element<'a, Message> {
    const BG: Color = Color {
        r: 0.15,
        g: 0.15,
        b: 0.17,
        a: 1.0,
    };
    const BORDER: Color = Color {
        r: 0.32,
        g: 0.32,
        b: 0.36,
        a: 1.0,
    };
    const TEXT: Color = Color {
        r: 0.90,
        g: 0.90,
        b: 0.90,
        a: 1.0,
    };
    const DIM: Color = Color {
        r: 0.58,
        g: 0.58,
        b: 0.62,
        a: 1.0,
    };
    const BTN_OK: Color = Color {
        r: 0.20,
        g: 0.46,
        b: 0.80,
        a: 1.0,
    };
    const BTN_HOV: Color = Color {
        r: 0.26,
        g: 0.55,
        b: 0.92,
        a: 1.0,
    };
    const BTN_GREY: Color = Color {
        r: 0.26,
        g: 0.26,
        b: 0.29,
        a: 1.0,
    };
    const BTN_GHOV: Color = Color {
        r: 0.34,
        g: 0.34,
        b: 0.38,
        a: 1.0,
    };

    let btn = |lbl: &'static str, msg: Message, base: Color, hov: Color| {
        button(text(lbl).size(12).color(TEXT))
            .on_press(msg)
            .style(move |_: &Theme, st| button::Style {
                background: Some(Background::Color(
                    if matches!(st, button::Status::Hovered | button::Status::Pressed) {
                        hov
                    } else {
                        base
                    },
                )),
                text_color: TEXT,
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
            .padding([4, 12])
    };

    let sel_fmt = SAVE_FORMAT_OPTIONS.iter().copied().find(|&s| s == format);
    let label = |s: &'static str| text(s).size(11).color(DIM);

    let mut items: Vec<Element<'a, Message>> = Vec::new();
    items.push(text("Save Drawing As").size(14).color(TEXT).into());
    items.push(Space::new().height(12).into());

    // Web has no native file dialog, so the file name is typed here. On native
    // the OS save dialog collects the name, so this field is omitted.
    #[cfg(target_arch = "wasm32")]
    {
        const INPUT_BG: Color = Color {
            r: 0.10,
            g: 0.10,
            b: 0.12,
            a: 1.0,
        };
        let input_sty =
            |_: &Theme, _: iced::widget::text_input::Status| iced::widget::text_input::Style {
                background: Background::Color(INPUT_BG),
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                icon: TEXT,
                placeholder: DIM,
                value: TEXT,
                selection: Color {
                    r: 0.20,
                    g: 0.46,
                    b: 0.80,
                    a: 0.45,
                },
            };
        items.push(
            row![
                label("File name:").width(70),
                iced::widget::text_input("drawing.dwg", filename)
                    .on_input(Message::SaveDialogFilenameChanged)
                    .style(input_sty)
                    .size(13)
                    .padding([5, 8])
                    .width(Fill),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(6)
            .into(),
        );
        items.push(Space::new().height(8).into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = filename;

    items.push(
        row![
            label("Format:").width(70),
            pick_list(SAVE_FORMAT_OPTIONS, sel_fmt, |s: &str| {
                Message::SaveDialogFormatChanged(s.to_string())
            })
            .width(Fill),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(6)
        .into(),
    );
    items.push(Space::new().height(16).into());
    items.push(
        row![
            Space::new().width(Fill),
            btn("Save as...", Message::SaveDialogConfirm, BTN_OK, BTN_HOV),
            Space::new().width(8),
            btn("Cancel", Message::SaveDialogCancel, BTN_GREY, BTN_GHOV),
        ]
        .into(),
    );

    let body = column(items).spacing(0);

    container(body)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BG)),
            ..Default::default()
        })
        .padding([14, 16])
        .width(Fill)
        .height(Fill)
        .into()
}

fn unsaved_changes_dialog_window(name: &str) -> Element<'static, Message> {
    const BG: Color = Color {
        r: 0.18,
        g: 0.18,
        b: 0.20,
        a: 1.0,
    };
    const BORDER_COL: Color = Color {
        r: 0.38,
        g: 0.38,
        b: 0.42,
        a: 1.0,
    };
    const TEXT_COL: Color = Color {
        r: 0.90,
        g: 0.90,
        b: 0.90,
        a: 1.0,
    };
    const BTN_SAVE: Color = Color {
        r: 0.20,
        g: 0.46,
        b: 0.80,
        a: 1.0,
    };
    const BTN_HOVER: Color = Color {
        r: 0.26,
        g: 0.55,
        b: 0.92,
        a: 1.0,
    };
    const BTN_DISC: Color = Color {
        r: 0.28,
        g: 0.28,
        b: 0.30,
        a: 1.0,
    };
    const BTN_DHOV: Color = Color {
        r: 0.36,
        g: 0.36,
        b: 0.40,
        a: 1.0,
    };

    let body_text = format!("Do you want to save changes to \"{}\"?", name);

    let btn = |label: &'static str, msg: Message, base: Color, hov: Color| {
        button(text(label).size(13).color(TEXT_COL))
            .on_press(msg)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hov,
                    _ => base,
                })),
                text_color: TEXT_COL,
                border: Border {
                    color: BORDER_COL,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                shadow: iced::Shadow::default(),
                snap: false,
            })
            .padding([6, 18])
    };

    container(
        column![
            text(body_text).size(13).color(TEXT_COL),
            iced::widget::Space::new().height(20),
            row![
                btn("Save", Message::UnsavedDialogSave, BTN_SAVE, BTN_HOVER),
                iced::widget::Space::new().width(8),
                btn("Discard", Message::UnsavedDialogDiscard, BTN_DISC, BTN_DHOV),
                iced::widget::Space::new().width(8),
                btn("Cancel", Message::UnsavedDialogCancel, BTN_DISC, BTN_DHOV),
            ],
        ]
        .spacing(0),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    })
    .center(Fill)
    .padding([24, 28])
    .into()
}

#[cfg(not(target_arch = "wasm32"))]
fn file_in_use_dialog_window(path: &str, error: &str) -> Element<'static, Message> {
    const BG: Color = Color {
        r: 0.18,
        g: 0.18,
        b: 0.20,
        a: 1.0,
    };
    const BORDER_COL: Color = Color {
        r: 0.38,
        g: 0.38,
        b: 0.42,
        a: 1.0,
    };
    const TEXT_COL: Color = Color {
        r: 0.90,
        g: 0.90,
        b: 0.90,
        a: 1.0,
    };
    const DIM_COL: Color = Color {
        r: 0.62,
        g: 0.62,
        b: 0.66,
        a: 1.0,
    };
    const BTN_PRIMARY: Color = Color {
        r: 0.20,
        g: 0.46,
        b: 0.80,
        a: 1.0,
    };
    const BTN_PRIMARY_HOVER: Color = Color {
        r: 0.26,
        g: 0.55,
        b: 0.92,
        a: 1.0,
    };
    const BTN_SECONDARY: Color = Color {
        r: 0.28,
        g: 0.28,
        b: 0.30,
        a: 1.0,
    };
    const BTN_SECONDARY_HOVER: Color = Color {
        r: 0.36,
        g: 0.36,
        b: 0.40,
        a: 1.0,
    };

    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Drawing".to_string());
    let heading = format!("\"{file_name}\" could not be saved.");
    let path_line = format!("Path: {path}");
    let details = format!("Details: {error}");
    let btn = |label: &'static str, msg: Message, base: Color, hover: Color| {
        button(text(label).size(13).color(TEXT_COL))
            .on_press(msg)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hover,
                    _ => base,
                })),
                text_color: TEXT_COL,
                border: Border {
                    color: BORDER_COL,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
            .padding([6, 18])
    };

    container(
        column![
            text(heading).size(14).color(TEXT_COL),
            Space::new().height(8),
            text(
                "The file is open or being used by another application. \
                 Close it there and retry, or save this drawing under a different name."
            )
            .size(13)
            .color(TEXT_COL)
            .width(Fill),
            Space::new().height(12),
            text(path_line).size(11).color(DIM_COL).width(Fill),
            Space::new().height(4),
            text(details).size(11).color(DIM_COL).width(Fill),
            Space::new().height(18),
            row![
                btn(
                    "Retry",
                    Message::SaveFileInUseRetry,
                    BTN_PRIMARY,
                    BTN_PRIMARY_HOVER
                ),
                Space::new().width(8),
                btn(
                    "Save As",
                    Message::SaveFileInUseSaveAs,
                    BTN_SECONDARY,
                    BTN_SECONDARY_HOVER
                ),
                Space::new().width(8),
                btn(
                    "Cancel",
                    Message::SaveFileInUseCancel,
                    BTN_SECONDARY,
                    BTN_SECONDARY_HOVER
                ),
            ],
        ]
        .spacing(0),
    )
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    })
    .padding([18, 20])
    .width(Fill)
    .height(Fill)
    .into()
}

#[cfg(not(target_arch = "wasm32"))]
fn external_change_dialog_window(path: &str) -> Element<'static, Message> {
    const BG: Color = Color {
        r: 0.18,
        g: 0.18,
        b: 0.20,
        a: 1.0,
    };
    const BORDER_COL: Color = Color {
        r: 0.38,
        g: 0.38,
        b: 0.42,
        a: 1.0,
    };
    const TEXT_COL: Color = Color {
        r: 0.90,
        g: 0.90,
        b: 0.90,
        a: 1.0,
    };
    const DIM_COL: Color = Color {
        r: 0.62,
        g: 0.62,
        b: 0.66,
        a: 1.0,
    };
    const BTN_PRIMARY: Color = Color {
        r: 0.20,
        g: 0.46,
        b: 0.80,
        a: 1.0,
    };
    const BTN_PRIMARY_HOVER: Color = Color {
        r: 0.26,
        g: 0.55,
        b: 0.92,
        a: 1.0,
    };
    const BTN_SECONDARY: Color = Color {
        r: 0.28,
        g: 0.28,
        b: 0.30,
        a: 1.0,
    };
    const BTN_SECONDARY_HOVER: Color = Color {
        r: 0.36,
        g: 0.36,
        b: 0.40,
        a: 1.0,
    };
    const BTN_DANGER: Color = Color {
        r: 0.62,
        g: 0.31,
        b: 0.14,
        a: 1.0,
    };
    const BTN_DANGER_HOVER: Color = Color {
        r: 0.78,
        g: 0.39,
        b: 0.17,
        a: 1.0,
    };

    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Drawing".to_string());
    let heading = format!("\"{file_name}\" was changed by another application.");
    let path_line = format!("Path: {path}");
    let btn = |label: &'static str, msg: Message, base: Color, hover: Color| {
        button(text(label).size(13).color(TEXT_COL))
            .on_press(msg)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hover,
                    _ => base,
                })),
                text_color: TEXT_COL,
                border: Border {
                    color: BORDER_COL,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
            .padding([6, 14])
    };

    container(
        column![
            text(heading).size(14).color(TEXT_COL),
            Space::new().height(8),
            text(
                "Saving now could destroy those external changes. Reload the disk copy, \
                 save your local work elsewhere, or explicitly overwrite it."
            )
            .size(13)
            .color(TEXT_COL)
            .width(Fill),
            Space::new().height(12),
            text(path_line).size(11).color(DIM_COL).width(Fill),
            Space::new().height(18),
            row![
                btn(
                    "Reload from Disk",
                    Message::ExternalChangeReload,
                    BTN_PRIMARY,
                    BTN_PRIMARY_HOVER
                ),
                Space::new().width(8),
                btn(
                    "Save As",
                    Message::ExternalChangeSaveAs,
                    BTN_SECONDARY,
                    BTN_SECONDARY_HOVER
                ),
                Space::new().width(8),
                btn(
                    "Overwrite",
                    Message::ExternalChangeOverwrite,
                    BTN_DANGER,
                    BTN_DANGER_HOVER
                ),
                Space::new().width(8),
                btn(
                    "Cancel",
                    Message::ExternalChangeCancel,
                    BTN_SECONDARY,
                    BTN_SECONDARY_HOVER
                ),
            ],
        ]
        .spacing(0),
    )
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    })
    .padding([18, 20])
    .width(Fill)
    .height(Fill)
    .into()
}

/// Warning shown before a lossy Save-As: the drawing carries unsupported
/// (AEC / application) objects that survive only as verbatim source-version
/// bytes, so saving to a different version or to DXF would drop them. Offers to
/// save in the source version (keep them) or proceed (drop them).
fn aec_drop_dialog_window(count: usize, target: &str, src_version: &str) -> Element<'static, Message> {
    const BG: Color = Color { r: 0.18, g: 0.18, b: 0.20, a: 1.0 };
    const BORDER_COL: Color = Color { r: 0.38, g: 0.38, b: 0.42, a: 1.0 };
    const TEXT_COL: Color = Color { r: 0.90, g: 0.90, b: 0.90, a: 1.0 };
    const BTN_SAVE: Color = Color { r: 0.20, g: 0.46, b: 0.80, a: 1.0 };
    const BTN_HOVER: Color = Color { r: 0.26, g: 0.55, b: 0.92, a: 1.0 };
    const BTN_DISC: Color = Color { r: 0.28, g: 0.28, b: 0.30, a: 1.0 };
    const BTN_DHOV: Color = Color { r: 0.36, g: 0.36, b: 0.40, a: 1.0 };

    let body_text = format!(
        "This drawing contains {count} AEC/Civil objects that \"{target}\" \
         cannot store, so they will not be saved.\n\n\
         To keep them, save in the source version ({src_version})."
    );

    let btn = |label: &'static str, msg: Message, base: Color, hov: Color| {
        button(text(label).size(13).color(TEXT_COL))
            .on_press(msg)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hov,
                    _ => base,
                })),
                text_color: TEXT_COL,
                border: Border {
                    color: BORDER_COL,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                shadow: iced::Shadow::default(),
                snap: false,
            })
            .padding([6, 14])
    };

    container(
        column![
            text(body_text).size(13).color(TEXT_COL),
            iced::widget::Space::new().height(20),
            row![
                btn("Save in source version", Message::AecDropSameVersion, BTN_SAVE, BTN_HOVER),
                iced::widget::Space::new().width(8),
                btn("Save anyway", Message::AecDropProceed, BTN_DISC, BTN_DHOV),
                iced::widget::Space::new().width(8),
                btn("Back", Message::AecDropBack, BTN_DISC, BTN_DHOV),
            ],
        ]
        .spacing(0),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    })
    .center(Fill)
    .padding([24, 28])
    .into()
}

/// Confirmation shown when the chosen Save-As filename already exists in the
/// target folder. "Replace" overwrites; "Cancel" returns to the Save dialog.
/// Confirm deleting layer(s) that still have objects on them. "Delete Objects"
/// erases them and removes the layers; "Cancel" leaves everything.
fn layer_delete_warning_window(names: &[String], count: usize) -> Element<'static, Message> {
    const BG: Color = Color { r: 0.18, g: 0.18, b: 0.20, a: 1.0 };
    const BORDER_COL: Color = Color { r: 0.38, g: 0.38, b: 0.42, a: 1.0 };
    const TEXT_COL: Color = Color { r: 0.90, g: 0.90, b: 0.90, a: 1.0 };
    const BTN_DEL: Color = Color { r: 0.72, g: 0.26, b: 0.24, a: 1.0 };
    const BTN_DEL_HOV: Color = Color { r: 0.84, g: 0.32, b: 0.30, a: 1.0 };
    const BTN_CANCEL: Color = Color { r: 0.28, g: 0.28, b: 0.30, a: 1.0 };
    const BTN_CANCEL_HOV: Color = Color { r: 0.36, g: 0.36, b: 0.40, a: 1.0 };

    let obj = if count == 1 { "object" } else { "objects" };
    let subject = if names.len() == 1 {
        format!("Layer \"{}\"", names[0])
    } else {
        format!("{} selected layers", names.len())
    };
    let body_text = format!(
        "{subject} still {} {count} {obj}.\n\nDeleting will also remove {} from the drawing. Continue?",
        if names.len() == 1 { "has" } else { "hold" },
        if count == 1 { "that object" } else { "those objects" }
    );

    let btn = |label: &'static str, msg: Message, base: Color, hov: Color| {
        button(text(label).size(13).color(TEXT_COL))
            .on_press(msg)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hov,
                    _ => base,
                })),
                text_color: TEXT_COL,
                border: Border { color: BORDER_COL, width: 1.0, radius: 4.0.into() },
                shadow: iced::Shadow::default(),
                snap: false,
            })
            .padding([6, 18])
    };

    container(
        column![
            text(body_text).size(13).color(TEXT_COL),
            iced::widget::Space::new().height(20),
            row![
                btn("Delete Objects", Message::LayerDeleteConfirm, BTN_DEL, BTN_DEL_HOV),
                iced::widget::Space::new().width(8),
                btn("Cancel", Message::CloseModal, BTN_CANCEL, BTN_CANCEL_HOV),
            ],
        ]
        .spacing(0),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    })
    .center(Fill)
    .padding([24, 28])
    .into()
}

/// First-launch prompt offering to register Open CAD Studio as the default
/// handler for .dwg / .dxf. "Yes" runs the platform association call; "Not now"
/// just dismisses. Either answer flips the persisted `default_assoc_prompted`
/// flag so the dialog never reappears.
fn default_assoc_dialog_window() -> Element<'static, Message> {
    const BG: Color = Color {
        r: 0.18,
        g: 0.18,
        b: 0.20,
        a: 1.0,
    };
    const BORDER_COL: Color = Color {
        r: 0.38,
        g: 0.38,
        b: 0.42,
        a: 1.0,
    };
    const TEXT_COL: Color = Color {
        r: 0.90,
        g: 0.90,
        b: 0.90,
        a: 1.0,
    };
    const DIM_COL: Color = Color {
        r: 0.62,
        g: 0.62,
        b: 0.66,
        a: 1.0,
    };
    const BTN_YES: Color = Color {
        r: 0.20,
        g: 0.46,
        b: 0.80,
        a: 1.0,
    };
    const BTN_YHOV: Color = Color {
        r: 0.26,
        g: 0.55,
        b: 0.92,
        a: 1.0,
    };
    const BTN_NO: Color = Color {
        r: 0.28,
        g: 0.28,
        b: 0.30,
        a: 1.0,
    };
    const BTN_NHOV: Color = Color {
        r: 0.36,
        g: 0.36,
        b: 0.40,
        a: 1.0,
    };

    let btn = |label: &'static str, msg: Message, base: Color, hov: Color| {
        button(text(label).size(13).color(TEXT_COL))
            .on_press(msg)
            .style(move |_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered | button::Status::Pressed => hov,
                    _ => base,
                })),
                text_color: TEXT_COL,
                border: Border {
                    color: BORDER_COL,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                shadow: iced::Shadow::default(),
                snap: false,
            })
            .padding([6, 18])
    };

    container(
        column![
            text("Make Open CAD Studio your default CAD app?")
                .size(15)
                .color(TEXT_COL),
            iced::widget::Space::new().height(10),
            text("Open .dwg and .dxf drawings in Open CAD Studio by default. You can change this later in your system settings.")
                .size(12)
                .color(DIM_COL),
            iced::widget::Space::new().height(22),
            row![
                iced::widget::Space::new().width(Fill),
                btn("Not now", Message::AssocPromptNo, BTN_NO, BTN_NHOV),
                iced::widget::Space::new().width(8),
                btn("Yes, set as default", Message::AssocPromptYes, BTN_YES, BTN_YHOV),
            ]
            .align_y(iced::Center),
        ]
        .spacing(0),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    })
    .center(Fill)
    .padding([24, 28])
    .into()
}
