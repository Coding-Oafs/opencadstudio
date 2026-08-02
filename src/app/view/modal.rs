use super::super::{Message, OpenCADStudio};
use iced::widget::{button, column, container, row, text, Space};
use iced::{Background, Element, Fill, Fit, Theme};

impl OpenCADStudio {
    /// Title shown in the active modal's title bar. Keep in sync with the
    /// [`Self::modal_content`] dispatch.
    pub(super) fn modal_title(&self) -> String {
        use super::super::ModalKind as K;
        match self.active_modal {
            Some(K::About) => crate::tr!("modal-about"),
            Some(K::Shortcuts) => crate::tr!("modal-keyboard-shortcuts"),
            Some(K::Aliases) => crate::tr!("modal-command-aliases"),
            Some(K::Options) => crate::tr!("action-options"),
            Some(K::FindReplace) => crate::tr!("modal-find-replace"),
            Some(K::PluginManager) => crate::tr!("modal-plugin-manager"),
            Some(K::UpdateNotice) => crate::tr!("modal-update-available"),
            Some(K::Layers) => crate::tr!("modal-layer-manager"),
            Some(K::LayerStateManager) => crate::tr!("modal-layer-state-manager"),
            Some(K::LayerStateEditor) => crate::tr!("modal-edit-layer-state"),
            Some(K::Plot) => crate::tr!("modal-plot"),
            Some(K::LayoutManager) => crate::tr!("modal-layout-manager"),
            Some(K::ScaleManager) => crate::tr!("modal-scale-manager"),
            Some(K::AnnoObjectScale) => crate::tr!("modal-annotation-object-scale"),
            Some(K::Plotstyle) => crate::tr!("modal-plot-style-editor"),
            Some(K::TextStyle) => crate::tr!("modal-text-style-manager"),
            Some(K::MlStyle) => crate::tr!("modal-multiline-style-manager"),
            Some(K::TableStyle) => crate::tr!("modal-table-style-manager"),
            Some(K::MLeaderStyle) => crate::tr!("modal-multileader-style-manager"),
            Some(K::DimStyle) => crate::tr!("modal-dimension-style-manager"),
            Some(K::AssocPrompt) => crate::tr!("modal-default-application"),
            Some(K::AecDropWarning) => crate::tr!("modal-save-warning"),
            #[cfg(not(target_arch = "wasm32"))]
            Some(K::FileInUse) => crate::tr!("modal-unable-save"),
            #[cfg(not(target_arch = "wasm32"))]
            Some(K::ExternalChange) => crate::tr!("modal-drawing-changed"),
            Some(K::LayerDeleteWarning) => crate::tr!("modal-delete-layer"),
            Some(K::Unsaved) => crate::tr!("modal-unsaved-changes"),
            Some(K::PointStyle) => crate::tr!("modal-point-style"),
            Some(K::AttributeEditor) => crate::tr!("modal-attribute-editor"),
            Some(K::SaveDialog) => crate::tr!("modal-save-drawing-as"),
            None => String::new(),
        }
    }

    /// Build the currently-open modal dialog's content (Plan B), or `None`.
    /// Iced 0.15 measures the content first, so dialogs start at their natural
    /// size and overflowing regions become scrollable where a cap is supplied.
    pub(super) fn modal_content<'s>(&'s self) -> Option<Element<'s, Message>> {
        let ex = self.modal_resize;
        Some(match self.active_modal? {
            super::super::ModalKind::About => automatic_flow(ex, |flow| {
                container(crate::ui::window::about::view_window())
                    .width(flow.width)
                    .height(flow.height)
                    .into()
            }),
            super::super::ModalKind::Shortcuts => {
                sized_flow(
                    ex,
                    720,
                    520,
                    |flow| {
                        crate::ui::window::shortcuts::view_window(
                            &self.shortcut_overrides,
                            flow,
                        )
                    },
                )
            }
            super::super::ModalKind::Aliases => {
                sized_flow(
                    ex,
                    480,
                    520,
                    |flow| {
                        crate::ui::window::alias_editor::view_window(
                            &self.alias_editor_rows,
                            flow,
                        )
                    },
                )
            }
            super::super::ModalKind::Options => sized_flow(
                ex,
                520,
                500,
                |flow| {
                    crate::ui::window::options::view_window(
                        &self.default_save_format,
                        &self.ui_theme,
                        &self.theme_color_inputs,
                        self.language,
                        flow,
                    )
                },
            ),
            super::super::ModalKind::FindReplace => automatic_flow(ex, |flow| {
                crate::ui::window::find_replace::view_window(
                    &self.find_replace.search,
                    &self.find_replace.replacement,
                    &self.find_replace.status,
                    flow,
                )
            }),
            super::super::ModalKind::PluginManager => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    sized_flow(
                        ex,
                        940,
                        600,
                        |flow| {
                            crate::ui::window::plugin_manager::view_window(
                                &self.disabled_plugins,
                                &self.external_plugins,
                                &self.loaded_plugin_ids,
                                &self.plugin_load_errors,
                                crate::ui::window::plugin_manager::MarketView {
                                    registry: &self.plugin_registry,
                                    registry_loading: self.plugin_registry_loading,
                                    registry_error: self.plugin_registry_error.as_deref(),
                                    registry_error_details_open: self
                                        .plugin_registry_error_details_open,
                                    input: &self.plugin_repo_input,
                                    search: &self.plugin_search_input,
                                    repos: &self.plugin_repos,
                                    release_tags: &self.repo_release_tags,
                                    selected_tag: &self.repo_selected_tag,
                                    selected_repo: self.selected_plugin_repo.as_deref(),
                                    readmes: &self.plugin_readmes,
                                    readme_loading: &self.plugin_readme_loading,
                                    status: &self.marketplace_status,
                                },
                                &self.active_theme,
                                flow,
                            )
                        },
                    )
                }
                #[cfg(target_arch = "wasm32")]
                {
                    automatic_flow(ex, |flow| {
                        container(crate::ui::window::plugin_manager::view_web_notice())
                            .width(flow.width)
                            .height(flow.height)
                            .into()
                    })
                }
            }
            super::super::ModalKind::UpdateNotice => {
                let latest = self.update_notice_version.as_deref().unwrap_or("?");
                let body = self.update_notice_body.as_deref().unwrap_or("");
                sized_flow(
                    ex,
                    560,
                    460,
                    |flow| crate::ui::window::update_notice::view_window(latest, body, flow),
                )
            }
            super::super::ModalKind::Layers => {
                let tab = &self.tabs[self.active_tab];
                sized_flow(
                    ex,
                    900,
                    360,
                    |flow| tab.layers.view_window(self.layer_name_col_w, flow),
                )
            }
            super::super::ModalKind::LayerStateManager => {
                let states = self.tabs[self.active_tab].scene.document.layer_states();
                sized_flow(
                    ex,
                    720,
                    420,
                    |flow| {
                        crate::ui::window::layer_state_manager::view_window(
                            states.clone(),
                            self.layer_state_selected.as_deref(),
                            &self.layer_state_name_buf,
                            &self.layer_state_description_buf,
                            &self.layer_state_filter,
                            flow,
                        )
                    },
                )
            }
            super::super::ModalKind::LayerStateEditor => {
                let tab = &self.tabs[self.active_tab];
                if let Some(state) = self.layer_state_edit_draft.as_ref() {
                    let mut linetypes: Vec<String> = tab
                        .scene
                        .document
                        .line_types
                        .iter()
                        .map(|line_type| line_type.name.clone())
                        .collect();
                    for layer in &state.layers {
                        if !layer.line_type.is_empty()
                            && !linetypes
                                .iter()
                                .any(|name| name.eq_ignore_ascii_case(&layer.line_type))
                        {
                            linetypes.push(layer.line_type.clone());
                        }
                    }
                    linetypes.sort_by_key(|name| name.to_lowercase());
                    sized_flow(
                        ex,
                        1180,
                        560,
                        |flow| {
                            crate::ui::window::layer_state_manager::view_editor(
                                state,
                                &self.layer_state_edit_filter,
                                self.layer_state_edit_color_open,
                                linetypes.clone(),
                                flow,
                            )
                        },
                    )
                } else {
                    automatic_flow(ex, |flow| {
                        container(text("The selected layer state is no longer available."))
                            .padding(16)
                            .width(flow.width)
                            .height(flow.height)
                            .into()
                    })
                }
            }
            super::super::ModalKind::Plot => {
                sized_flow(
                    ex,
                    760,
                    540,
                    |flow| crate::ui::window::plot::view_window(&self.plot_dialog, flow),
                )
            }
            super::super::ModalKind::LayoutManager => {
                let i = self.active_tab;
                let layouts = self.tabs[i].scene.layout_names();
                let current = self.tabs[i].scene.current_layout.clone();
                sized_flow(
                    ex,
                    640,
                    320,
                    |flow| {
                        crate::ui::window::layout_manager::view_window(
                            layouts.clone(),
                            &self.layout_manager_selected,
                            &self.layout_manager_rename_buf,
                            current.clone(),
                            flow,
                        )
                    },
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
                sized_flow(
                    ex,
                    520,
                    360,
                    |flow| {
                        crate::ui::style::scale_manager::view_window(
                            &scales,
                            &self.scale_manager_selected,
                            &current,
                            self.scale_rename.as_deref(),
                            &self.scale_rename_buf,
                            &self.scale_manager_paper_buf,
                            &self.scale_manager_drawing_buf,
                            flow,
                        )
                    },
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
                sized_flow(
                    ex,
                    360,
                    420,
                    |flow| {
                        crate::ui::style::anno_object_scale::view_window(
                            &label,
                            &scales,
                            flow,
                        )
                    },
                )
            }
            super::super::ModalKind::Plotstyle => sized_flow(
                ex,
                780,
                540,
                |flow| {
                    crate::ui::style::plotstyle::view_window(
                        self.active_plot_style.as_ref(),
                        self.plotstyle_panel_aci,
                        &self.ps_color_buf,
                        &self.ps_lineweight_buf,
                        &self.ps_screening_buf,
                        flow,
                    )
                },
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
                sized_flow(
                    ex,
                    860,
                    480,
                    |flow| {
                        crate::ui::style::textstyle::view_window(
                            crate::ui::style::textstyle::TextStyleView {
                                styles: styles.clone(),
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
                            },
                            flow,
                        )
                    },
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
                sized_flow(
                    ex,
                    620,
                    420,
                    |flow| {
                        crate::ui::style::mlstyle::view_window(
                            styles.clone(),
                            &self.mlstyle_selected,
                            selected_style,
                            tab.scene.document.header.multiline_style.clone(),
                            self.style_rename.as_deref(),
                            &self.style_rename_buf,
                            flow,
                        )
                    },
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
                sized_flow(
                    ex,
                    620,
                    420,
                    |flow| {
                        crate::ui::style::tablestyle::view_window(
                            styles.clone(),
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
                            flow,
                        )
                    },
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
                sized_flow(
                    ex,
                    560,
                    560,
                    |flow| {
                        crate::ui::style::mleaderstyle::view_window(
                            crate::ui::style::mleaderstyle::MLeaderStyleView {
                                styles: styles.clone(),
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
                                block_opts: block_opts.clone(),
                                lt_opts: lt_opts.clone(),
                                textstyle_opts: textstyle_opts.clone(),
                                line_type_name: line_type_name.clone(),
                                arrowhead_name: arrowhead_name.clone(),
                                text_style_name: text_style_name.clone(),
                                block_content_name: block_content_name.clone(),
                                rename_active: self.style_rename.as_deref(),
                                rename_buf: &self.style_rename_buf,
                                color_open: self.mls_color_open,
                            },
                            flow,
                        )
                    },
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
            block_opts.extend(
                doc.block_records
                    .iter()
                    .filter(|b| {
                        !b.is_layout()
                            && !b.is_model_space()
                            && !b.is_paper_space()
                            && !b.flags.is_xref
                            && !b.flags.is_xref_overlay
                            && !b.flags.is_external
                            && !b.name.starts_with('*')
                    })
                    .map(|b| b.name.clone()),
            );
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
            sized_flow(ex, 720, 560, |flow| {
                crate::ui::style::dimstyle::view_window(
                styles.clone(),
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
                    dimblk_name: dimblk_name.clone(),
                    dimblk1_name: dimblk1_name.clone(),
                    dimblk2_name: dimblk2_name.clone(),
                    dimldrblk_name: dimldrblk_name.clone(),
                    dimltex_name: dimltex_name.clone(),
                    dimltex1_name: dimltex1_name.clone(),
                    dimltex2_name: dimltex2_name.clone(),
                    block_opts: block_opts.clone(),
                    lt_opts: lt_opts.clone(),
                    color_open: self.ds_color_open.clone(),
                },
                self.style_rename.as_deref(),
                &self.style_rename_buf,
                flow,
                )
            })
            }
            super::super::ModalKind::AssocPrompt => {
                automatic_flow(ex, default_assoc_dialog_window)
            }
            super::super::ModalKind::AecDropWarning => {
                let src_label = self
                    .tabs
                    .get(self.active_tab)
                    .map(|t| {
                        let is_dxf = t
                            .current_path
                            .as_ref()
                            .and_then(|path| path.extension())
                            .and_then(|extension| extension.to_str())
                            .map(|extension| extension.eq_ignore_ascii_case("dxf"))
                            .unwrap_or(false);
                        let version = if is_dxf {
                            t.scene.document.version
                        } else {
                            t.scene
                                .document
                                .dwg_source_version
                                .unwrap_or(t.scene.document.version)
                        };
                        crate::io::format_for_version(version, is_dxf)
                    })
                    .unwrap_or_else(|| "DWG".to_string());
                automatic_flow(ex, |flow| {
                    aec_drop_dialog_window(
                        self.aec_drop_count,
                        &self.save_dialog_format,
                        &src_label,
                        flow,
                    )
                })
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
                automatic_flow(ex, |flow| file_in_use_dialog_window(&path, &error, flow))
            }
            #[cfg(not(target_arch = "wasm32"))]
            super::super::ModalKind::ExternalChange => {
                let path = self
                    .pending_external_change
                    .as_ref()
                    .map(|conflict| conflict.path.display().to_string())
                    .unwrap_or_default();
                automatic_flow(ex, |flow| external_change_dialog_window(&path, flow))
            }
            super::super::ModalKind::LayerDeleteWarning => {
                let (names, count) = self
                    .layer_delete_pending
                    .clone()
                    .unwrap_or_else(|| (Vec::new(), 0));
                automatic_flow(ex, |flow| layer_delete_warning_window(&names, count, flow))
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
                automatic_flow(ex, |flow| unsaved_changes_dialog_window(&tab_name, flow))
            }
            super::super::ModalKind::PointStyle => sized_flow(
                ex,
                360,
                470,
                |flow| {
                    crate::ui::style::point_style::view_window(
                        self.tabs[self.active_tab]
                            .scene
                            .document
                            .header
                            .point_display_mode,
                        self.point_size_relative,
                        &self.point_size_buf,
                        flow,
                    )
                },
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
                sized_flow(
                    ex,
                    640,
                    500,
                    |flow| {
                        crate::ui::window::attribute_editor::view_window(
                            &self.attr_editor_block,
                            &self.attr_editor_rows,
                            self.attr_editor_selected,
                            self.attr_editor_tab,
                            layers.clone(),
                            linetypes.clone(),
                            styles.clone(),
                            flow,
                        )
                    },
                )
            }
            super::super::ModalKind::SaveDialog => {
                automatic_flow(ex, |flow| {
                    save_as_dialog_window(
                        &self.save_dialog_filename,
                        &self.save_dialog_format,
                        flow,
                    )
                })
            }
        })
    }
}

fn sized_flow<'a>(
    extra: iced::Vector,
    max_width: u16,
    max_height: u16,
    mut build: impl FnMut(crate::ui::modal::ModalSizing) -> Element<'a, Message>,
) -> Element<'a, Message> {
    crate::ui::modal::intrinsic(
        build(crate::ui::modal::ModalSizing::INTRINSIC),
        build(crate::ui::modal::ModalSizing::FILL),
        iced::Size::new(max_width as f32, max_height as f32),
        extra,
    )
}

fn automatic_flow<'a>(
    extra: iced::Vector,
    mut build: impl FnMut(crate::ui::modal::ModalSizing) -> Element<'a, Message>,
) -> Element<'a, Message> {
    crate::ui::modal::intrinsic(
        build(crate::ui::modal::ModalSizing::INTRINSIC),
        build(crate::ui::modal::ModalSizing::FILL),
        iced::Size::new(f32::INFINITY, f32::INFINITY),
        extra,
    )
}

fn dialog_button(
    label: &'static str,
    message: Message,
    style: fn(&Theme, button::Status) -> button::Style,
) -> Element<'static, Message> {
    button(text(label).size(13))
        .on_press(message)
        .style(style)
        .padding([6, 18])
        .into()
}

fn dialog_body_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        background: Some(Background::Color(palette.background.base.color)),
        text_color: Some(palette.background.base.text),
        ..Default::default()
    }
}

fn dialog_muted_text_style(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.68)),
    }
}

/// Compact Save-As options dialog: pick the format/version and a default file
/// name. The destination folder and overwrite confirmation come from the
/// native OS save dialog (native) or the browser download (web) that follows.
fn save_as_dialog_window<'a>(
    filename: &'a str,
    format: &'a str,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    let sel_fmt = crate::io::SAVE_FORMAT_OPTIONS
        .iter()
        .copied()
        .find(|&s| s == format);
    let label = |s: &'static str| text(s).size(11).style(dialog_muted_text_style);
    let field_width = if matches!(sizing.width, iced::Length::Fill) {
        Fill
    } else {
        iced::Length::Shrink
    };

    let mut items: Vec<Element<'a, Message>> = Vec::new();
    items.push(text("Save Drawing As").size(14).into());
    items.push(Space::new().height(12).into());

    // Web has no native file dialog, so the file name is typed here. On native
    // the OS save dialog collects the name, so this field is omitted.
    #[cfg(target_arch = "wasm32")]
    {
        items.push(
            row![
                label("File name:").width(70),
                iced::widget::text_input("drawing.dwg", filename)
                    .on_input(Message::SaveDialogFilenameChanged)
                    .size(13)
                    .padding([5, 8])
                    .width(field_width),
            ]
            .align_y(iced::Alignment::Center)
            .spacing(6)
            .width(sizing.width)
            .into(),
        );
        items.push(Space::new().height(8).into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = filename;

    items.push(
        row![
            label("Format:").width(70),
            iced::widget::pick_list(
                sel_fmt,
                crate::io::SAVE_FORMAT_OPTIONS,
                |value| value.to_string(),
            )
            .on_select(|s: &str| Message::SaveDialogFormatChanged(s.to_string()))
            .width(field_width),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(6)
        .width(sizing.width)
        .into(),
    );
    items.push(Space::new().height(16).into());
    items.push(
        row![
            Space::new().width(Fit),
            dialog_button("Save as...", Message::SaveDialogConfirm, button::primary),
            Space::new().width(8),
            dialog_button("Cancel", Message::SaveDialogCancel, button::secondary),
        ]
        .into(),
    );

    let body = column(items)
        .spacing(0)
        .width(sizing.width)
        .height(sizing.height);

    container(body)
        .style(dialog_body_style)
        .padding([14, 16])
        .width(sizing.width)
        .height(sizing.height)
        .into()
}

fn unsaved_changes_dialog_window(
    name: &str,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
    let body_text = format!("Do you want to save changes to \"{}\"?", name);

    container(
        column![
            text(body_text).size(13),
            iced::widget::Space::new().height(20),
            row![
                dialog_button("Save", Message::UnsavedDialogSave, button::primary),
                iced::widget::Space::new().width(8),
                dialog_button("Discard", Message::UnsavedDialogDiscard, button::danger),
                iced::widget::Space::new().width(8),
                dialog_button("Cancel", Message::UnsavedDialogCancel, button::secondary),
            ],
        ]
        .spacing(0),
    )
    .style(dialog_body_style)
    .center_x(sizing.width)
    .center_y(sizing.height)
    .padding([24, 28])
    .into()
}

#[cfg(not(target_arch = "wasm32"))]
fn file_in_use_dialog_window(
    path: &str,
    error: &str,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Drawing".to_string());
    let heading = format!("\"{file_name}\" could not be saved.");
    let path_line = format!("Path: {path}");
    let details = format!("Details: {error}");

    container(
        column![
            text(heading).size(14),
            Space::new().height(8),
            text(
                "The file is open or being used by another application. \
                 Close it there and retry, or save this drawing under a different name."
            )
            .size(13)
            .width(Fit),
            Space::new().height(12),
            text(path_line).size(11).style(dialog_muted_text_style).width(Fit),
            Space::new().height(4),
            text(details).size(11).style(dialog_muted_text_style).width(Fit),
            Space::new().height(18),
            row![
                dialog_button(
                    "Retry",
                    Message::SaveFileInUseRetry,
                    button::primary
                ),
                Space::new().width(8),
                dialog_button(
                    "Save As",
                    Message::SaveFileInUseSaveAs,
                    button::secondary
                ),
                Space::new().width(8),
                dialog_button(
                    "Cancel",
                    Message::SaveFileInUseCancel,
                    button::secondary
                ),
            ],
        ]
        .spacing(0),
    )
    .style(dialog_body_style)
    .padding([18, 20])
    .width(sizing.width)
    .height(sizing.height)
    .into()
}

#[cfg(not(target_arch = "wasm32"))]
fn external_change_dialog_window(
    path: &str,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Drawing".to_string());
    let heading = format!("\"{file_name}\" was changed by another application.");
    let path_line = format!("Path: {path}");

    container(
        column![
            text(heading).size(14),
            Space::new().height(8),
            text(
                "Saving now could destroy those external changes. Reload the disk copy, \
                 save your local work elsewhere, or explicitly overwrite it."
            )
            .size(13)
            .width(Fit),
            Space::new().height(12),
            text(path_line).size(11).style(dialog_muted_text_style).width(Fit),
            Space::new().height(18),
            row![
                dialog_button(
                    "Reload from Disk",
                    Message::ExternalChangeReload,
                    button::primary
                ),
                Space::new().width(8),
                dialog_button(
                    "Save As",
                    Message::ExternalChangeSaveAs,
                    button::secondary
                ),
                Space::new().width(8),
                dialog_button(
                    "Overwrite",
                    Message::ExternalChangeOverwrite,
                    button::danger
                ),
                Space::new().width(8),
                dialog_button(
                    "Cancel",
                    Message::ExternalChangeCancel,
                    button::secondary
                ),
            ],
        ]
        .spacing(0),
    )
    .style(dialog_body_style)
    .padding([18, 20])
    .width(sizing.width)
    .height(sizing.height)
    .into()
}

/// Warning shown before a lossy Save-As: the drawing carries unsupported
/// (AEC / application) objects that survive only as verbatim source-version
/// bytes, so saving to a different version or to DXF would drop them. Offers to
/// save in the source version (keep them) or proceed (drop them).
fn aec_drop_dialog_window(
    count: usize,
    target: &str,
    src_version: &str,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
    let body_text = format!(
        "This drawing contains {count} AEC/Civil objects that \"{target}\" \
         cannot store, so they will not be saved.\n\n\
         To keep them, save in the source version ({src_version})."
    );

    container(
        column![
            text(body_text).size(13),
            iced::widget::Space::new().height(20),
            row![
                dialog_button(
                    "Save in source version",
                    Message::AecDropSameVersion,
                    button::primary
                ),
                iced::widget::Space::new().width(8),
                dialog_button("Save anyway", Message::AecDropProceed, button::warning),
                iced::widget::Space::new().width(8),
                dialog_button("Back", Message::AecDropBack, button::secondary),
            ],
        ]
        .spacing(0),
    )
    .style(dialog_body_style)
    .center_x(sizing.width)
    .center_y(sizing.height)
    .padding([24, 28])
    .into()
}

/// Confirmation shown when the chosen Save-As filename already exists in the
/// target folder. "Replace" overwrites; "Cancel" returns to the Save dialog.
/// Confirm deleting layer(s) that still have objects on them. "Delete Objects"
/// erases them and removes the layers; "Cancel" leaves everything.
fn layer_delete_warning_window(
    names: &[String],
    count: usize,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
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

    container(
        column![
            text(body_text).size(13),
            iced::widget::Space::new().height(20),
            row![
                dialog_button(
                    "Delete Objects",
                    Message::LayerDeleteConfirm,
                    button::danger
                ),
                iced::widget::Space::new().width(8),
                dialog_button("Cancel", Message::CloseModal, button::secondary),
            ],
        ]
        .spacing(0),
    )
    .style(dialog_body_style)
    .center_x(sizing.width)
    .center_y(sizing.height)
    .padding([24, 28])
    .into()
}

/// First-launch prompt offering to register Open CAD Studio as the default
/// handler for .dwg / .dxf. "Yes" runs the platform association call; "Not now"
/// just dismisses. Either answer flips the persisted `default_assoc_prompted`
/// flag so the dialog never reappears.
fn default_assoc_dialog_window(
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'static, Message> {
    container(
        column![
            text("Make Open CAD Studio your default CAD app?")
                .size(15),
            iced::widget::Space::new().height(10),
            text("Open .dwg and .dxf drawings in Open CAD Studio by default. You can change this later in your system settings.")
                .size(12)
                .style(dialog_muted_text_style),
            iced::widget::Space::new().height(22),
            row![
                iced::widget::Space::new().width(Fit),
                dialog_button("Not now", Message::AssocPromptNo, button::secondary),
                iced::widget::Space::new().width(8),
                dialog_button(
                    "Yes, set as default",
                    Message::AssocPromptYes,
                    button::primary
                ),
            ]
            .align_y(iced::Center),
        ]
        .spacing(0),
    )
    .style(dialog_body_style)
    .center_x(sizing.width)
    .center_y(sizing.height)
    .padding([24, 28])
    .into()
}
