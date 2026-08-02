use super::helpers::{entity_type_key, entity_type_label, title_case_word};
use super::{OpenCADStudio, VARIES_LABEL};
use crate::io::linetypes;
use crate::scene::view::dispatch;
use crate::ui;
use acadrust::{EntityType, Handle};

/// Above this many selected objects the Properties panel skips per-entity
/// property aggregation (which is O(n) per row, plus an O(n²) group filter) and
/// shows a count-only summary instead. Bulk edits still go through the ribbon.
const MAX_PROP_AGGREGATE: usize = 2_000;

fn material_color_text(color: &acadrust::objects::MaterialColor) -> String {
    let rgb = color
        .rgb
        .map(|value| format!("#{:06X}", value as u32 & 0x00ff_ffff))
        .unwrap_or_else(|| "Object".to_string());
    format!("{rgb}, factor {:.3}", color.factor)
}

fn material_map_text(map: &acadrust::objects::MaterialMap) -> String {
    let source = match map.source {
        1 => {
            if map.file_name.trim().is_empty() {
                "File".to_string()
            } else {
                map.file_name.clone()
            }
        }
        2 => format!(
            "Procedural {}",
            map.texture.as_ref().map_or(0, |texture| texture.mode)
        ),
        _ => "None".to_string(),
    };
    format!(
        "{source}; blend {:.3}; projection {}; tiling {}; auto {}",
        map.blend_factor, map.projection, map.tiling, map.auto_transform
    )
}

fn visual_style_properties_text(style: &acadrust::objects::VisualStyle) -> String {
    style
        .properties
        .iter()
        .enumerate()
        .map(|(index, property)| {
            format!(
                "{}: {:?} (enabled {})",
                index, property.value, property.enabled
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl OpenCADStudio {
    /// Rebuild the PropertiesPanel from the current entity selection.
    /// Preserves UI state (open pickers, edit buffer) across refreshes.
    pub(super) fn refresh_properties(&mut self) {
        let i = self.active_tab;
        if !crate::entities::object_data::cache_is_prepared(
            &self.tabs[i].scene.object_data_cache,
        ) {
            self.tabs[i].scene.object_data_cache =
                crate::entities::object_data::build_cache(&self.tabs[i].scene.document);
        }
        // Note: the color-picker dropdown is intentionally NOT carried over — a
        // rebuild means the selection (or a property) changed, so the dropdown
        // closes, matching the deselect / reselect / click-away expectation.
        let color_palette_open = self.tabs[i].properties.color_palette_open;
        let edit_buf = std::mem::take(&mut self.tabs[i].properties.edit_buf);
        // Expanded coordinate groups persist across rebuilds AND selection
        // changes — it's a per-user view preference, not per-entity state.
        let expanded_groups = std::mem::take(&mut self.tabs[i].properties.expanded_groups);
        // Which entities the previous panel was built for — an uncommitted
        // edit buffer only survives a rebuild for the *same* selection.
        let prev_handles = std::mem::take(&mut self.tabs[i].properties.source_handles);
        let selected_group = self.tabs[i].properties.selected_group.clone();

        // Seed the per-thread unit context from the document header so the
        // entity property builders (which only see f64 values) can format
        // lengths/angles per LUNITS / LUPREC / AUNITS / AUPREC.
        {
            let h = &self.tabs[i].scene.document.header;
            crate::entities::common::set_unit_context(crate::entities::common::UnitContext {
                lunits: h.linear_unit_format,
                luprec: h.linear_unit_precision,
                aunits: h.angular_unit_format,
                auprec: h.angular_unit_precision,
            });
        }

        let layer_names: Vec<String> = self.tabs[i]
            .scene
            .document
            .layers
            .iter()
            .map(|l| l.name.clone())
            .collect();
        let linetype_items: Vec<ui::properties::LinetypeItem> = self.tabs[i]
            .scene
            .document
            .line_types
            .iter()
            .map(|lt| {
                let name = if lt.name.is_empty() {
                    "ByLayer".to_string()
                } else {
                    lt.name.clone()
                };
                let art = linetypes::extract_pattern(&lt.description);
                ui::properties::LinetypeItem { name, art }
            })
            .collect();
        let text_style_names: Vec<String> = self.tabs[i]
            .scene
            .document
            .text_styles
            .iter()
            .map(|style| style.name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();

        // Current-Vertex focus survives only while the same object stays
        // selected; a changed selection resets to the first vertex. Seed the
        // per-thread focus so the polyline property builder / editor targets it.
        let cur_handles: Vec<acadrust::Handle> = self.tabs[i]
            .scene
            .selected_entities()
            .iter()
            .map(|(h, _)| *h)
            .collect();
        let prop_vertex = if cur_handles == prev_handles {
            self.tabs[i].properties.prop_vertex
        } else {
            0
        };
        let prop_vertex = if cur_handles.len() == 1 {
            let vertex_count = self.tabs[i]
                .scene
                .document
                .get_entity(cur_handles[0])
                .and_then(|entity| match entity {
                    acadrust::EntityType::LwPolyline(polyline) => Some(polyline.vertices.len()),
                    acadrust::EntityType::Polyline2D(polyline) => Some(polyline.vertices.len()),
                    acadrust::EntityType::Leader(leader) => Some(leader.vertices.len()),
                    _ => None,
                });
            vertex_count.map_or(prop_vertex, |count| prop_vertex.min(count.saturating_sub(1)))
        } else {
            prop_vertex
        };
        let prop_vertex_indicator_active = if cur_handles == prev_handles {
            self.tabs[i].properties.prop_vertex_indicator_active
        } else {
            false
        };
        crate::scene::view::dispatch::set_prop_current_vertex(prop_vertex);

        let annotation_scale_handle = self.tabs[i].scene.displayed_annotation_scale_handle();
        let new_panel = {
            let selected = self.tabs[i].scene.selected_entities();
            let mut panel = match selected.len() {
                0 => {
                    use crate::scene::model::object::{PropSection, PropValue, Property};

                    let tab = &self.tabs[i];
                    let scene = &tab.scene;
                    let doc = &scene.document;
                    let header = &doc.header;
                    let camera = scene.camera.borrow();
                    let (viewport_width, viewport_height) = scene.selection.borrow().vp_size;
                    let aspect = if viewport_height > 0.0 {
                        viewport_width as f64 / viewport_height as f64
                    } else {
                        1.0
                    };
                    let view_height = camera.ortho_size() as f64 * 2.0;
                    let view_width = view_height * aspect;
                    let format_length = crate::entities::common::format_length;
                    let read_only = |label: &str, value: String| Property {
                        label: label.to_string(),
                        field: "drawing_property",
                        value: PropValue::ReadOnly(value),
                    };
                    let current_layer = if header.current_layer_name.is_empty() {
                        tab.active_layer.clone()
                    } else {
                        header.current_layer_name.clone()
                    };
                    let current_linetype = if !header.current_linetype_name.is_empty() {
                        header.current_linetype_name.clone()
                    } else if !header.current_linetype_handle.is_null() {
                        doc.line_types
                            .iter()
                            .find(|line_type| {
                                line_type.handle == header.current_linetype_handle
                            })
                            .map(|line_type| line_type.name.clone())
                            .unwrap_or_else(|| "ByLayer".to_string())
                    } else {
                        "ByLayer".to_string()
                    };
                    let material = if header.current_material_handle.is_null() {
                        "ByLayer".to_string()
                    } else {
                        doc.objects
                            .get(&header.current_material_handle)
                            .and_then(|object| match object {
                                acadrust::objects::ObjectType::Material(material) => {
                                    Some(material.name.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| "ByLayer".to_string())
                    };
                    let plot_style = match header.current_plotstyle_type {
                        1 => "ByBlock",
                        2 => "ByColor",
                        3 => "ByObject",
                        _ => "ByLayer",
                    };
                    let layout_plot_table = doc.objects.values().find_map(|object| {
                        let acadrust::objects::ObjectType::Layout(layout) = object else {
                            return None;
                        };
                        (layout.name == scene.current_layout
                            && !layout.plot_style_sheet.trim().is_empty())
                        .then(|| layout.plot_style_sheet.clone())
                    });
                    let plot_table = layout_plot_table
                        .or_else(|| {
                            (!header.stylesheet.trim().is_empty())
                                .then(|| header.stylesheet.clone())
                        })
                        .unwrap_or_else(|| "None".to_string());
                    let ucs_per_viewport = scene
                        .active_viewport
                        .and_then(|handle| doc.get_entity(handle))
                        .and_then(|entity| match entity {
                            acadrust::EntityType::Viewport(viewport) => {
                                Some(viewport.ucs_per_viewport)
                            }
                            _ => None,
                        })
                        .unwrap_or(false);
                    let ucs_name = tab
                        .active_ucs
                        .as_ref()
                        .map(|ucs| ucs.name.trim())
                        .filter(|name| !name.is_empty())
                        .unwrap_or("World")
                        .to_string();
                    let annotation_scale = if header.current_annotation_scale.trim().is_empty() {
                        "1:1".to_string()
                    } else {
                        header.current_annotation_scale.clone()
                    };
                    let sections = vec![
                        PropSection {
                            title: "General".to_string(),
                            props: vec![
                                read_only("AC Version", doc.version.as_str().to_string()),
                                Property {
                                    label: "Color".to_string(),
                                    field: "color",
                                    value: PropValue::ColorChoice(header.current_entity_color),
                                },
                                Property {
                                    label: "Layer".to_string(),
                                    field: "layer",
                                    value: PropValue::LayerChoice(current_layer),
                                },
                                Property {
                                    label: "Linetype".to_string(),
                                    field: "linetype",
                                    value: PropValue::LinetypeChoice(current_linetype),
                                },
                                read_only(
                                    "Linetype scale",
                                    format_length(header.current_entity_linetype_scale),
                                ),
                                Property {
                                    label: "Lineweight".to_string(),
                                    field: "line_weight",
                                    value: PropValue::LwChoice(
                                        acadrust::types::LineWeight::from_value(
                                            header.current_line_weight,
                                        ),
                                    ),
                                },
                                read_only("Transparency", "ByLayer".to_string()),
                                read_only("Thickness", format_length(header.thickness)),
                            ],
                        },
                        PropSection {
                            title: "3D Visualization".to_string(),
                            props: vec![read_only("Material", material)],
                        },
                        PropSection {
                            title: "Plot style".to_string(),
                            props: vec![
                                read_only("Plot style", plot_style.to_string()),
                                read_only("Plot style table", plot_table.clone()),
                                read_only(
                                    "Plot table attached to",
                                    if plot_table == "None" {
                                        "None".to_string()
                                    } else {
                                        scene.current_layout.clone()
                                    },
                                ),
                                read_only(
                                    "Plot table type",
                                    if header.plotstyle_mode {
                                        "Named plot styles".to_string()
                                    } else {
                                        "Color-dependent plot styles".to_string()
                                    },
                                ),
                            ],
                        },
                        PropSection {
                            title: "View".to_string(),
                            props: vec![
                                read_only("Center X", format_length(camera.target.x)),
                                read_only("Center Y", format_length(camera.target.y)),
                                read_only("Center Z", format_length(camera.target.z)),
                                read_only("Height", format_length(view_height)),
                                read_only("Width", format_length(view_width)),
                            ],
                        },
                        PropSection {
                            title: "Misc".to_string(),
                            props: vec![
                                read_only("Annotation scale", annotation_scale),
                                read_only(
                                    "UCS icon On",
                                    if self.show_ucs_icon { "Yes" } else { "No" }.to_string(),
                                ),
                                read_only(
                                    "UCS icon at origin",
                                    if self.ucs_icon_at_origin { "Yes" } else { "No" }
                                        .to_string(),
                                ),
                                read_only(
                                    "UCS per viewport",
                                    if ucs_per_viewport { "Yes" } else { "No" }.to_string(),
                                ),
                                read_only("UCS Name", ucs_name),
                                read_only("Visual Style", tab.visual_style.clone()),
                            ],
                        },
                    ];
                    ui::PropertiesPanel {
                        title: "No selection".to_string(),
                        sections,
                        layer_combo: iced::widget::combo_box::State::new(layer_names.clone()),
                        linetype_combo: iced::widget::combo_box::State::new(
                            linetype_items.clone(),
                        ),
                        lineweight_combo: iced::widget::combo_box::State::new(
                            ui::properties::lw_options(),
                        ),
                        linetype_items,
                        ..Default::default()
                    }
                }
                1 => {
                    let (handle, source_entity) = selected[0];
                    let contextual = crate::scene::annotative::entity_for_annotation_context(
                        &self.tabs[i].scene.document,
                        source_entity,
                        annotation_scale_handle,
                    );
                    let entity = contextual.as_ref();
                    let group_names = self.tabs[i].scene.group_names_for_entity(handle);
                    let mut sections =
                        dispatch::properties_sectioned(handle, entity, &text_style_names);

                    // Turn the Material row into an editable picker: the source
                    // options (ByLayer / ByBlock) plus every named material the
                    // drawing defines. A custom flag (3) shows the stored handle's
                    // material name as the selection.
                    {
                        let doc = &self.tabs[i].scene.document;
                        let common = entity.common();
                        let mat_names: Vec<String> = doc
                            .objects
                            .iter()
                            .filter_map(|(_, o)| match o {
                                acadrust::objects::ObjectType::Material(m) => Some(m.name.clone()),
                                _ => None,
                            })
                            .collect();
                        let selected = match common.material_flags {
                            0 => "ByLayer".to_string(),
                            1 => "ByBlock".to_string(),
                            _ => common
                                .material_handle
                                .and_then(|mh| {
                                    doc.objects.iter().find_map(|(h, o)| match o {
                                        acadrust::objects::ObjectType::Material(m) if *h == mh => {
                                            Some(m.name.clone())
                                        }
                                        _ => None,
                                    })
                                })
                                .unwrap_or_else(|| "ByLayer".to_string()),
                        };
                        let mut options = vec!["ByLayer".to_string(), "ByBlock".to_string()];
                        options.extend(mat_names);
                        for section in sections.iter_mut() {
                            if let Some(row) =
                                section.props.iter_mut().find(|p| p.field == "material")
                            {
                                row.value = crate::scene::model::object::PropValue::Choice {
                                    selected: selected.clone(),
                                    options: options.clone(),
                                };
                            }
                        }
                        let effective_handle = match common.material_flags {
                            3 => common.material_handle,
                            0 => doc
                                .layers
                                .get(&common.layer)
                                .map(|layer| layer.material)
                                .filter(|handle| handle.is_valid()),
                            _ => None,
                        };
                        if let Some(acadrust::objects::ObjectType::Material(material)) =
                            effective_handle.and_then(|handle| doc.objects.get(&handle))
                        {
                            use crate::entities::common::ro_prop;
                            sections.push(crate::scene::model::object::PropSection {
                                title: "Material Details".to_string(),
                                props: vec![
                                    ro_prop("Name", "mat_name", material.name.clone()),
                                    ro_prop(
                                        "Description",
                                        "mat_description",
                                        material.description.clone(),
                                    ),
                                    ro_prop(
                                        "Ambient",
                                        "mat_ambient",
                                        material_color_text(&material.ambient_color),
                                    ),
                                    ro_prop(
                                        "Diffuse",
                                        "mat_diffuse",
                                        material_color_text(&material.diffuse_color),
                                    ),
                                    ro_prop(
                                        "Specular",
                                        "mat_specular",
                                        material_color_text(&material.specular_color),
                                    ),
                                    ro_prop(
                                        "Gloss",
                                        "mat_gloss",
                                        format!("{:.3}", material.specular_gloss_factor),
                                    ),
                                    ro_prop(
                                        "Opacity",
                                        "mat_opacity",
                                        format!("{:.3}", material.opacity_percent),
                                    ),
                                    ro_prop(
                                        "Reflectivity",
                                        "mat_reflectivity",
                                        format!("{:.3}", material.reflectivity),
                                    ),
                                    ro_prop(
                                        "Translucence",
                                        "mat_translucence",
                                        format!("{:.3}", material.translucence),
                                    ),
                                    ro_prop(
                                        "Refraction",
                                        "mat_refraction",
                                        format!("{:.3}", material.refraction_index),
                                    ),
                                    ro_prop(
                                        "Self Illumination",
                                        "mat_self_illumination",
                                        format!("{:.3}", material.self_illumination),
                                    ),
                                    ro_prop(
                                        "Luminance",
                                        "mat_luminance",
                                        format!(
                                            "{:.3} (mode {})",
                                            material.luminance, material.luminance_mode
                                        ),
                                    ),
                                    ro_prop(
                                        "Diffuse Map",
                                        "mat_diffuse_map",
                                        material_map_text(&material.diffuse_map),
                                    ),
                                    ro_prop(
                                        "Specular Map",
                                        "mat_specular_map",
                                        material_map_text(&material.specular_map),
                                    ),
                                    ro_prop(
                                        "Reflection Map",
                                        "mat_reflection_map",
                                        material_map_text(&material.reflection_map),
                                    ),
                                    ro_prop(
                                        "Opacity Map",
                                        "mat_opacity_map",
                                        material_map_text(&material.opacity_map),
                                    ),
                                    ro_prop(
                                        "Bump Map",
                                        "mat_bump_map",
                                        material_map_text(&material.bump_map),
                                    ),
                                    ro_prop(
                                        "Refraction Map",
                                        "mat_refraction_map",
                                        material_map_text(&material.refraction_map),
                                    ),
                                    ro_prop(
                                        "Normal Map",
                                        "mat_normal_map",
                                        material_map_text(&material.normal_map),
                                    ),
                                    ro_prop(
                                        "Render Flags",
                                        "mat_render_flags",
                                        format!(
                                            "illumination {}; channels {}; mode {}; two-sided {}",
                                            material.illumination_model,
                                            material.channel_flags,
                                            material.mode,
                                            material.two_sided_material
                                        ),
                                    ),
                                    ro_prop(
                                        "Advanced",
                                        "mat_advanced",
                                        format!(
                                            "normal {:.3}; bump {:.3}; reflect {:.3}; transmit {:.3}",
                                            material.normal_map_strength,
                                            material.indirect_bump_scale,
                                            material.reflectance_scale,
                                            material.transmittance_scale
                                        ),
                                    ),
                                ],
                            });
                        }
                    }

                    {
                        let doc = &self.tabs[i].scene.document;
                        let common = entity.common();
                        if let Some(acadrust::objects::ObjectType::BookColor(book)) = common
                            .color_book_handle
                            .filter(|handle| handle.is_valid())
                            .and_then(|handle| doc.objects.get(&handle))
                        {
                            for section in sections.iter_mut() {
                                if let Some(row) =
                                    section.props.iter_mut().find(|row| row.field == "color")
                                {
                                    row.value =
                                        crate::scene::model::object::PropValue::ColorChoice(
                                            book.color,
                                        );
                                }
                            }
                            use crate::entities::common::ro_prop;
                            sections.push(crate::scene::model::object::PropSection {
                                title: "Color Book".to_string(),
                                props: vec![
                                    ro_prop("Book", "book_color_book", book.book_name.clone()),
                                    ro_prop(
                                        "Color Name",
                                        "book_color_name",
                                        book.color_name.clone(),
                                    ),
                                    ro_prop(
                                        "Color",
                                        "book_color_value",
                                        format!("{:?}", book.color),
                                    ),
                                ],
                            });
                        }

                        let style_handles = [
                            ("Full", common.full_visual_style_handle),
                            ("Face", common.face_visual_style_handle),
                            ("Edge", common.edge_visual_style_handle),
                        ];
                        for (scope, handle) in style_handles {
                            let Some((handle, style)) = handle
                                .filter(|handle| handle.is_valid())
                                .and_then(|handle| {
                                    doc.objects.get(&handle).and_then(|object| match object {
                                        acadrust::objects::ObjectType::VisualStyle(style) => {
                                            Some((handle, style))
                                        }
                                        _ => None,
                                    })
                                })
                            else {
                                continue;
                            };
                            use crate::entities::common::ro_prop;
                            sections.push(crate::scene::model::object::PropSection {
                                title: format!("{scope} Visual Style"),
                                props: vec![
                                    ro_prop(
                                        "Handle",
                                        "vs_handle",
                                        format!("{:X}", handle.value()),
                                    ),
                                    ro_prop(
                                        "Description",
                                        "vs_description",
                                        style.description.clone(),
                                    ),
                                    ro_prop(
                                        "Type",
                                        "vs_type",
                                        style.style_type.to_string(),
                                    ),
                                    ro_prop(
                                        "Face",
                                        "vs_face",
                                        format!(
                                            "lighting {}; quality {}; color {}; modifiers {}",
                                            style.face_lighting_model,
                                            style.face_lighting_quality,
                                            style.face_color_mode,
                                            style.face_modifier
                                        ),
                                    ),
                                    ro_prop(
                                        "Edges",
                                        "vs_edges",
                                        format!(
                                            "model {}; style {}",
                                            style.edge_model, style.edge_style
                                        ),
                                    ),
                                    ro_prop(
                                        "Extended Lighting",
                                        "vs_extended_lighting",
                                        style.extended_lighting_model.to_string(),
                                    ),
                                    ro_prop(
                                        "Internal",
                                        "vs_internal",
                                        style.internal_use_only.to_string(),
                                    ),
                                    ro_prop(
                                        "Property Bag",
                                        "vs_properties",
                                        visual_style_properties_text(style),
                                    ),
                                ],
                            });
                        }
                    }

                    if matches!(
                        entity,
                        acadrust::EntityType::Solid3D(_)
                            | acadrust::EntityType::Region(_)
                            | acadrust::EntityType::Body(_)
                            | acadrust::EntityType::Surface(_)
                            | acadrust::EntityType::Mesh(_)
                            | acadrust::EntityType::PolygonMesh(_)
                            | acadrust::EntityType::PolyfaceMesh(_)
                    ) {
                        if let Some(mesh) = self.tabs[i]
                            .scene
                            .meshes
                            .get(&handle)
                            .or_else(|| self.tabs[i].scene.block_meshes.get(&handle))
                        {
                            use crate::entities::common::ro_prop;
                            let metrics = mesh.metrics;
                            let mut props = vec![
                                ro_prop(
                                    "Vertices",
                                    "mesh_vertices",
                                    metrics.vertices.to_string(),
                                ),
                                ro_prop(
                                    "Triangles",
                                    "mesh_triangles",
                                    metrics.triangles.to_string(),
                                ),
                                ro_prop(
                                    "Surface Area",
                                    "mesh_surface_area",
                                    format!("{:.6}", metrics.surface_area),
                                ),
                                ro_prop(
                                    "Centroid",
                                    "mesh_centroid",
                                    format!(
                                        "{:.6}, {:.6}, {:.6}",
                                        metrics.centroid[0],
                                        metrics.centroid[1],
                                        metrics.centroid[2]
                                    ),
                                ),
                                ro_prop(
                                    "Tessellation",
                                    "mesh_complete",
                                    if mesh.complete { "Complete" } else { "Partial" },
                                ),
                            ];
                            if mesh.complete
                                && matches!(
                                    entity,
                                    acadrust::EntityType::Solid3D(_)
                                        | acadrust::EntityType::Region(_)
                                        | acadrust::EntityType::Body(_)
                                )
                            {
                                props.insert(
                                    3,
                                    ro_prop(
                                        "Volume",
                                        "mesh_volume",
                                        format!("{:.6}", metrics.volume),
                                    ),
                                );
                            }
                            sections.push(crate::scene::model::object::PropSection {
                                title: "Mass Properties".to_string(),
                                props,
                            });
                        }
                    }

                    sections.extend(crate::entities::object_data::sections(
                        &self.tabs[i].scene.document,
                        &self.tabs[i].scene.object_data_cache,
                        handle,
                        entity,
                    ));

                    // Uniform-scale checkbox for block references (#427):
                    // while the three scale factors are equal (and the user
                    // hasn't opted into per-axis editing) the panel shows one
                    // Scale row plus a checked "Uniform scale" box; unchecking
                    // expands the familiar Scale X/Y/Z rows.
                    if let acadrust::EntityType::Insert(ins) = entity {
                        let eq = (ins.x_scale() - ins.y_scale()).abs() < 1e-12
                            && (ins.x_scale() - ins.z_scale()).abs() < 1e-12;
                        let uniform =
                            eq && !self.props_asym_scale.contains(&handle.value());
                        for section in sections.iter_mut() {
                            let Some(xi) =
                                section.props.iter().position(|p| p.field == "x_scale")
                            else {
                                continue;
                            };
                            let toggle = crate::scene::model::object::Property {
                                label: "Uniform scale".into(),
                                field: "ins_uniform",
                                value: crate::scene::model::object::PropValue::BoolToggle {
                                    field: "ins_uniform",
                                    value: uniform,
                                },
                            };
                            if uniform {
                                section
                                    .props
                                    .retain(|p| p.field != "y_scale" && p.field != "z_scale");
                                let xi = section
                                    .props
                                    .iter()
                                    .position(|p| p.field == "x_scale")
                                    .unwrap_or(xi.min(section.props.len()));
                                section.props[xi] = crate::entities::common::edit_prop(
                                    "Scale",
                                    "u_scale",
                                    ins.x_scale(),
                                );
                                section.props.insert(xi, toggle);
                            } else {
                                section.props.insert(xi, toggle);
                            }
                        }
                    }

                    // In named plot-style mode (PSTYLEMODE=1) the Plot style row
                    // picks a named style from the drawing's plot style table. In
                    // the color-dependent mode it stays read-only (the object's
                    // color drives the plot style), so it is left untouched.
                    if self.tabs[i].scene.document.header.plotstyle_mode {
                        let doc = &self.tabs[i].scene.document;
                        let dict_h = doc.header.acad_plotstylename_dict_handle;
                        if let Some(dict) = crate::scene::annotative::as_dict(doc, dict_h) {
                            let common = entity.common();
                            let mut options = vec!["ByLayer".to_string(), "ByBlock".to_string()];
                            options.extend(dict.entries.iter().map(|(n, _)| n.clone()));
                            let selected = match common.plotstyle_flags {
                                0 => "ByLayer".to_string(),
                                1 => "ByBlock".to_string(),
                                _ => common
                                    .plotstyle_handle
                                    .and_then(|ph| {
                                        dict.entries
                                            .iter()
                                            .find(|(_, h)| *h == ph)
                                            .map(|(n, _)| n.clone())
                                    })
                                    .unwrap_or_else(|| "ByLayer".to_string()),
                            };
                            for section in sections.iter_mut() {
                                if let Some(row) =
                                    section.props.iter_mut().find(|p| p.field == "plot_style")
                                {
                                    row.value = crate::scene::model::object::PropValue::Choice {
                                        selected: selected.clone(),
                                        options: options.clone(),
                                    };
                                }
                            }
                        }
                    }

                    // Turn the MLEADER handle-backed rows (multileader style, text
                    // style, arrowhead block, leader linetype) into editable name
                    // pickers. The current handle resolves to a display name; the
                    // options list every candidate the drawing offers. Applying a
                    // pick resolves the name back to a handle in the update loop.
                    if let acadrust::EntityType::MultiLeader(ml) = entity {
                        let doc = &self.tabs[i].scene.document;
                        // Option lists.
                        let mleader_styles: Vec<String> = doc
                            .objects
                            .iter()
                            .filter_map(|(_, o)| match o {
                                acadrust::objects::ObjectType::MultiLeaderStyle(s) => {
                                    Some(s.name.clone())
                                }
                                _ => None,
                            })
                            .collect();
                        // A leader with no linetype handle draws ByBlock; expose that
                        // as the first option so the default is selectable.
                        let ltype_names: Vec<String> = std::iter::once("ByBlock".to_string())
                            .chain(
                                doc.line_types
                                    .iter()
                                    .map(|l| l.name.clone())
                                    .filter(|n| !n.is_empty()),
                            )
                            .collect();
                        // Arrowheads are blocks; the closed-filled default has no
                        // block, so seed the list with it and add the arrowhead
                        // blocks (leading underscore) present in the drawing.
                        let arrow_names: Vec<String> = std::iter::once("Closed filled".to_string())
                            .chain(
                                doc.block_records
                                    .iter()
                                    .filter(|b| b.name.starts_with('_'))
                                    .map(|b| b.name.clone()),
                            )
                            .collect();
                        let tstyle_names = text_style_names.clone();
                        // Currently selected names.
                        let cur_style = ml
                            .style_handle
                            .and_then(|h| {
                                doc.objects.iter().find_map(|(oh, o)| match o {
                                    acadrust::objects::ObjectType::MultiLeaderStyle(s)
                                        if *oh == h =>
                                    {
                                        Some(s.name.clone())
                                    }
                                    _ => None,
                                })
                            })
                            .unwrap_or_else(|| "Standard".to_string());
                        let cur_tstyle = ml
                            .text_style_handle
                            .and_then(|h| {
                                doc.text_styles.iter().find(|s| s.handle == h).map(|s| s.name.clone())
                            })
                            .unwrap_or_else(|| "Standard".to_string());
                        let cur_arrow = ml
                            .arrowhead_handle
                            .and_then(|h| {
                                doc.block_records.iter().find(|b| b.handle == h).map(|b| b.name.clone())
                            })
                            .unwrap_or_else(|| "Closed filled".to_string());
                        let cur_ltype = ml
                            .line_type_handle
                            .and_then(|h| {
                                doc.line_types.iter().find(|l| l.handle == h).map(|l| l.name.clone())
                            })
                            .unwrap_or_else(|| "ByBlock".to_string());
                        let mut set_choice =
                            |field: &str, selected: String, options: Vec<String>| {
                                for section in sections.iter_mut() {
                                    if let Some(row) =
                                        section.props.iter_mut().find(|p| p.field == field)
                                    {
                                        row.value =
                                            crate::scene::model::object::PropValue::Choice {
                                                selected: selected.clone(),
                                                options: options.clone(),
                                            };
                                    }
                                }
                            };
                        set_choice("mleader_style", cur_style, mleader_styles);
                        set_choice("text_style_handle", cur_tstyle, tstyle_names);
                        set_choice("arrowhead_handle", cur_arrow, arrow_names);
                        set_choice("line_type_handle", cur_ltype, ltype_names);
                    }

                    // Inject viewport-only properties that require doc access.
                    if let acadrust::EntityType::Viewport(vp) = entity {
                        let frozen_names: Vec<String> = vp
                            .frozen_layers
                            .iter()
                            .filter_map(|&h| {
                                self.tabs[i]
                                    .scene
                                    .document
                                    .layers
                                    .iter()
                                    .find(|l| l.handle == h)
                                    .map(|l| l.name.clone())
                            })
                            .collect();

                        // Collect available UCS names for the name picker.
                        let ucs_names: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .ucss
                            .iter()
                            .map(|u| u.name.clone())
                            .filter(|n| !n.is_empty())
                            .collect();

                        // Current UCS name (resolved from vp.ucs_handle).
                        let current_ucs = self.tabs[i]
                            .scene
                            .document
                            .ucss
                            .iter()
                            .find(|u| u.handle == vp.ucs_handle)
                            .map(|u| u.name.clone())
                            .unwrap_or_default();

                        // Collect available named view names.
                        let view_names: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .views
                            .iter()
                            .map(|v| v.name.clone())
                            .filter(|n| !n.is_empty())
                            .collect();

                        if let Some(geom) = sections.last_mut() {
                            geom.props.push(crate::scene::model::object::Property {
                                label: "Frozen Layers".to_string(),
                                field: "frozen_layers",
                                value: crate::scene::model::object::PropValue::EditText(
                                    frozen_names.join(", "),
                                ),
                            });
                            if !ucs_names.is_empty() {
                                geom.props.push(crate::scene::model::object::Property {
                                    label: "UCS Name".to_string(),
                                    field: "vp_ucs_name",
                                    value: crate::scene::model::object::PropValue::Choice {
                                        selected: current_ucs,
                                        options: ucs_names,
                                    },
                                });
                            }
                            if !view_names.is_empty() {
                                geom.props.push(crate::scene::model::object::Property {
                                    label: "Named View".to_string(),
                                    field: "vp_named_view",
                                    value: crate::scene::model::object::PropValue::Choice {
                                        selected: String::new(),
                                        options: view_names,
                                    },
                                });
                            }
                        }

                        // Drive the viewport scale picker from the drawing's
                        // own scale list instead of a built-in set.
                        let file_scales = self.tabs[i].scene.scale_list();
                        if !file_scales.is_empty() {
                            let eff = crate::scene::vp_effective_scale(
                                vp.custom_scale,
                                vp.view_height,
                                vp.height,
                            );
                            let selected = file_scales
                                .iter()
                                .find(|(_, _, f)| (f - eff).abs() < 0.001 * f.max(0.001))
                                .map(|(n, _, _)| n.clone())
                                .unwrap_or_default();
                            let options: Vec<String> =
                                file_scales.iter().map(|(n, _, _)| n.clone()).collect();
                            if let Some(geom) = sections.last_mut() {
                                if let Some(prop) =
                                    geom.props.iter_mut().find(|p| p.field == "vscale_std")
                                {
                                    prop.value = crate::scene::model::object::PropValue::Choice {
                                        selected,
                                        options,
                                    };
                                }
                            }
                        }
                    }

                    // Inject DimStyle picker + style-derived groups for Dimensions.
                    if let acadrust::EntityType::Dimension(d) = entity {
                        let dim_style_names: Vec<String> = self.tabs[i]
                            .scene
                            .document
                            .dim_styles
                            .iter()
                            .map(|s| s.name.clone())
                            .filter(|n| !n.is_empty())
                            .collect();
                        if !dim_style_names.is_empty() {
                            // Current style is already shown as EditText in the geom section;
                            // replace/upgrade it to a Choice if we have a list.
                            if let Some(geom) = sections.last_mut() {
                                // Find and replace the style_name EditText with a Choice.
                                if let Some(prop) =
                                    geom.props.iter_mut().find(|p| p.field == "style_name")
                                {
                                    let current = match &prop.value {
                                        crate::scene::model::object::PropValue::EditText(s) => s.clone(),
                                        _ => String::new(),
                                    };
                                    prop.value = crate::scene::model::object::PropValue::Choice {
                                        selected: current,
                                        options: dim_style_names,
                                    };
                                }
                            }
                        }

                        // Append the resolved dimension style's groups
                        // (Lines & Arrows, Text, Fit, Units, Tolerances).
                        let style_name = d.base().style_name.clone();
                        if let Some(style) =
                            self.tabs[i].scene.document.dim_styles.iter().find(|s| {
                                s.name.eq_ignore_ascii_case(&style_name)
                                    || (style_name.trim().is_empty()
                                        && s.name.eq_ignore_ascii_case("Standard"))
                            })
                        {
                            sections.extend(crate::entities::dimension::style_sections(style));

                            // The style groups are a read-only mirror, but the
                            // dim-line colour is an editable per-object override:
                            // prefer the ACAD_DSTYLE code-176 (ACI) override, else
                            // the style's DIMCLRD.
                            use crate::entities::dim_override as dov;
                            use crate::scene::model::object::PropValue;
                            let dim_c = dov::color(&d.base().common.extended_data, dov::DIMCLRD)
                                .unwrap_or_else(|| acadrust::types::Color::from_index(style.dimclrd));
                            set_row_value(
                                &mut sections,
                                "dim_line_color",
                                PropValue::ColorChoice(dim_c),
                            );
                        }
                    }

                    // ── Doc-dependent property rows ──────────────────────────
                    // Rows whose value lives on another object (a block record,
                    // an underlay definition, a dimension / multileader style)
                    // are left empty by the entity builders and resolved here,
                    // where the document is reachable.
                    let doc = &self.tabs[i].scene.document;
                    match entity {
                        // Block reference: the referenced block's units and the
                        // unit-scale factor against the drawing's INSUNITS.
                        acadrust::EntityType::Insert(ins) => {
                            let host = doc.header.insertion_units;
                            let src = doc
                                .block_records
                                .get(&ins.block_name)
                                .map(|br| br.units)
                                .unwrap_or(0);
                            set_row(&mut sections, "block_unit", insunits_name(src).to_string());
                            let host_mm = if host == 0 { 1.0 } else { insunits_to_mm(host) };
                            let src_mm = if src == 0 { 1.0 } else { insunits_to_mm(src) };
                            let factor = if host_mm.abs() > 1e-12 { src_mm / host_mm } else { 1.0 };
                            set_row(&mut sections, "unit_factor", format!("{factor:.4}"));

                            // Name row: editable for regular blocks — pick an
                            // existing definition to re-point this reference, or
                            // type a new name to rename the definition (every
                            // insert of it follows). Anonymous (*) and
                            // xref(-dependent) blocks keep the read-only row.
                            let regular = |br: &acadrust::tables::BlockRecord| {
                                !br.is_anonymous()
                                    && !br.flags.is_xref
                                    && !br.name.contains('|')
                            };
                            let editable = doc
                                .block_records
                                .get(&ins.block_name)
                                .map(&regular)
                                .unwrap_or(false);
                            if editable {
                                let mut options: Vec<String> = doc
                                    .block_records
                                    .iter()
                                    .filter(|br| regular(br))
                                    .map(|br| br.name.clone())
                                    .collect();
                                options.sort_by(|a, b| {
                                    a.to_lowercase().cmp(&b.to_lowercase())
                                });
                                set_row_value(
                                    &mut sections,
                                    "block",
                                    crate::scene::model::object::PropValue::EditChoice {
                                        value: ins.block_name.clone(),
                                        options,
                                    },
                                );
                            }
                        }
                        // Underlay: name + path from the referenced definition.
                        acadrust::EntityType::Underlay(ul) => {
                            if let Some((name, path)) =
                                doc.objects.iter().find_map(|(h, o)| match o {
                                    acadrust::objects::ObjectType::UnderlayDefinition(def)
                                        if *h == ul.definition_handle =>
                                    {
                                        let nm = if !def.name.is_empty() {
                                            def.name.clone()
                                        } else {
                                            def.page_name.clone()
                                        };
                                        Some((nm, def.file_path.clone()))
                                    }
                                    _ => None,
                                })
                            {
                                set_row(&mut sections, "ul_name", name);
                                set_row(&mut sections, "ul_path", path);
                            }
                        }
                        // Leader: text style / vertical text placement / overall
                        // scale come from its dimension style.
                        acadrust::EntityType::Leader(ld) => {
                            // Dim style row → dropdown of the drawing's dim styles.
                            let names: Vec<String> = doc
                                .dim_styles
                                .iter()
                                .map(|s| s.name.clone())
                                .filter(|n| !n.is_empty())
                                .collect();
                            if !names.is_empty() {
                                for section in sections.iter_mut() {
                                    if let Some(p) = section
                                        .props
                                        .iter_mut()
                                        .find(|p| p.field == "dimension_style")
                                    {
                                        let cur = match &p.value {
                                            crate::scene::model::object::PropValue::EditText(s) => {
                                                s.clone()
                                            }
                                            _ => ld.dimension_style.clone(),
                                        };
                                        p.value = crate::scene::model::object::PropValue::Choice {
                                            selected: cur,
                                            options: names.clone(),
                                        };
                                    }
                                }
                            }
                            // Lines & Arrows / Text / Fit default to the assigned
                            // dimension style (the same source the tessellator
                            // uses); most rows are editable per-object overrides
                            // stored in ACAD_DSTYLE. Arrow size / block / overall
                            // scale and dim-line lineweight drive the render; text
                            // offset / vertical position round-trip to file but
                            // don't change the leader glyph here (its annotation
                            // is a separate entity). Dim-line colour is read-only.
                            if let Some(ds) = find_dim_style(doc, &ld.dimension_style) {
                                use crate::entities::dim_override as dov;
                                use crate::scene::model::object::PropValue;
                                let xd = &ld.common.extended_data;

                                // Arrow block (DIMLDRBLK): override handle → name,
                                // else the style's arrow. Options are the arrowhead
                                // blocks the drawing carries plus the default.
                                let arrow_label = match dov::handle(xd, dov::DIMLDRBLK) {
                                    Some(h) => doc
                                        .block_records
                                        .iter()
                                        .find(|b| b.handle == h)
                                        .map(|b| arrowhead_label(&b.name))
                                        .unwrap_or_else(|| "Closed filled".to_string()),
                                    None => leader_arrow_label(doc, ds, ld.arrow_enabled),
                                };
                                let mut arrow_opts: Vec<String> =
                                    std::iter::once("Closed filled".to_string())
                                        .chain(
                                            doc.block_records
                                                .iter()
                                                .filter(|b| b.name.starts_with('_'))
                                                .map(|b| arrowhead_label(&b.name)),
                                        )
                                        .collect();
                                // Keep the current value selectable even when it
                                // isn't in the standard list (e.g. "None", or a
                                // style arrow whose block isn't underscore-named).
                                if !arrow_opts.contains(&arrow_label) {
                                    arrow_opts.insert(0, arrow_label.clone());
                                }
                                set_row_value(
                                    &mut sections,
                                    "arrow_block",
                                    PropValue::Choice {
                                        selected: arrow_label,
                                        options: arrow_opts,
                                    },
                                );

                                let asz = dov::real(xd, dov::DIMASZ).unwrap_or(ds.dimasz);
                                set_row_value(
                                    &mut sections,
                                    "arrow_size",
                                    PropValue::EditText(format!("{asz:.4}")),
                                );

                                let lwd = dov::int(xd, dov::DIMLWD).unwrap_or(ds.dimlwd);
                                let lw_sel = dim_lineweight_label(lwd);
                                let mut lw_opts = lineweight_options();
                                if !lw_opts.contains(&lw_sel) {
                                    lw_opts.insert(0, lw_sel.clone());
                                }
                                set_row_value(
                                    &mut sections,
                                    "dim_line_lw",
                                    PropValue::Choice {
                                        selected: lw_sel,
                                        options: lw_opts,
                                    },
                                );

                                // Dim-line colour: a per-object ACAD_DSTYLE
                                // override (code 176, an ACI index) wins over the
                                // style's DIMCLRD. Editable — the picked colour is
                                // written back as that override so it round-trips.
                                let dim_c = dov::color(xd, dov::DIMCLRD)
                                    .unwrap_or_else(|| acadrust::types::Color::from_index(ds.dimclrd));
                                set_row_value(
                                    &mut sections,
                                    "dim_line_color",
                                    PropValue::ColorChoice(dim_c),
                                );

                                let gap = dov::real(xd, dov::DIMGAP).unwrap_or(ds.dimgap);
                                set_row_value(
                                    &mut sections,
                                    "text_offset",
                                    PropValue::EditText(format!("{gap:.4}")),
                                );

                                let tad = dov::int(xd, dov::DIMTAD).unwrap_or(ds.dimtad);
                                set_row_value(
                                    &mut sections,
                                    "text_pos_vert",
                                    PropValue::Choice {
                                        selected: dimtad_label(tad).to_string(),
                                        options: tad_options(),
                                    },
                                );

                                let scl = dov::real(xd, dov::DIMSCALE).unwrap_or(ds.dimscale);
                                set_row_value(
                                    &mut sections,
                                    "dim_scale_overall",
                                    PropValue::EditText(format!("{scl:.4}")),
                                );
                            }
                        }
                        // Feature-control frame: FCF text style is the dimension
                        // style's DIMTXSTY.
                        acadrust::EntityType::Tolerance(tol) => {
                            if let Some(ds) = find_dim_style(doc, &tol.dimension_style_name) {
                                if !ds.dimtxsty.is_empty() {
                                    set_row(&mut sections, "tol_text_style", ds.dimtxsty.clone());
                                }
                            }
                        }
                        // MultiLeader: max points + segment-angle constraints
                        // are MLeaderStyle settings, not stored on the entity.
                        acadrust::EntityType::MultiLeader(ml) => {
                            if let Some(sh) = ml.style_handle {
                                if let Some((mx, a1, a2)) =
                                    doc.objects.iter().find_map(|(h, o)| match o {
                                        acadrust::objects::ObjectType::MultiLeaderStyle(s)
                                            if *h == sh =>
                                        {
                                            Some((
                                                s.max_leader_points,
                                                s.first_segment_angle,
                                                s.second_segment_angle,
                                            ))
                                        }
                                        _ => None,
                                    })
                                {
                                    set_row(&mut sections, "max_leader_points", mx.to_string());
                                    set_row(
                                        &mut sections,
                                        "first_segment_angle",
                                        format!("{a1:.4}"),
                                    );
                                    set_row(
                                        &mut sections,
                                        "second_segment_angle",
                                        format!("{a2:.4}"),
                                    );
                                }
                            }
                        }
                        _ => {}
                    }

                    // Annotative Yes/No + a conditional "Annotative scale" row.
                    // Read-only: annotative state comes from the entity's style
                    // (or its own flag) and the assigned annotation-scale name(s)
                    // are walked from the entity's extension dictionary — both
                    // need the document, so they are resolved here.
                    {
                        // Which entities show an Annotative row, the field it uses,
                        // and — for those that don't already carry the row
                        // (dimension / table) — the existing field to insert it
                        // after. MLeader uses its editable toggle field.
                        let anno: Option<(&str, Option<&str>)> = match entity {
                            acadrust::EntityType::Text(_)
                            | acadrust::EntityType::MText(_)
                            | acadrust::EntityType::Insert(_)
                            | acadrust::EntityType::Leader(_) => Some(("annotative", None)),
                            acadrust::EntityType::MultiLeader(_) => {
                                Some(("enable_annotation_scale", None))
                            }
                            acadrust::EntityType::Dimension(_) => {
                                Some(("annotative", Some("style_name")))
                            }
                            acadrust::EntityType::Table(_) => {
                                Some(("annotative", Some("tbl_style_handle")))
                            }
                            _ => None,
                        };
                        if let Some((anno_field, insert_after)) = anno {
                            let is_anno = crate::scene::annotative::is_annotative(doc, entity);
                            // Dimensions/tables carry no Annotative row yet — add one
                            // right after their style row.
                            if let Some(anchor) = insert_after {
                                insert_row_after(
                                    &mut sections,
                                    anchor,
                                    crate::entities::common::ro_prop(
                                        "Annotative",
                                        "annotative",
                                        "No",
                                    ),
                                );
                            }
                            // Objects that carry a per-object annotation context
                            // (MTEXT via its native flag, single-line TEXT via the
                            // context alone) get an editable toggle: turning it on
                            // synthesizes a real per-scale representation. The
                            // remaining types are style-driven and stay read-only.
                            if anno_field == "annotative" {
                                match entity {
                                    acadrust::EntityType::MText(t) => set_row_value(
                                        &mut sections,
                                        "annotative",
                                        crate::scene::model::object::PropValue::BoolToggle {
                                            field: "is_annotative",
                                            value: t.is_annotative,
                                        },
                                    ),
                                    acadrust::EntityType::Text(_)
                                    | acadrust::EntityType::Insert(_) => set_row_value(
                                        &mut sections,
                                        "annotative",
                                        crate::scene::model::object::PropValue::BoolToggle {
                                            field: "annotative_ctx",
                                            value: is_anno,
                                        },
                                    ),
                                    _ => set_row(
                                        &mut sections,
                                        "annotative",
                                        if is_anno { "Yes" } else { "No" }.to_string(),
                                    ),
                                }
                            }
                            if is_anno {
                                // The applied annotation scale follows the current
                                // annotation scale (CANNOSCALE / the status-bar
                                // scale pill), not a per-object stored value.
                                insert_row_after(
                                    &mut sections,
                                    anno_field,
                                    crate::entities::common::ro_prop(
                                        "Annotative scale",
                                        "annotative_scale",
                                        doc.header.current_annotation_scale.clone(),
                                    ),
                                );
                            }
                        }
                    }

                    if !group_names.is_empty() {
                        let label = group_names.join(", ");
                        if let Some(general) = sections.first_mut() {
                            general.props.push(crate::scene::model::object::Property {
                                label: "Group".to_string(),
                                field: "group",
                                value: crate::scene::model::object::PropValue::ReadOnly(label),
                            });
                        }
                    }
                    let title = match entity {
                        acadrust::EntityType::Insert(ins) => {
                            let is_xref = self.tabs[i]
                                .scene
                                .document
                                .block_records
                                .iter()
                                .find(|br| br.name == ins.block_name)
                                .map(|br| br.flags.is_xref || br.flags.is_xref_overlay)
                                .unwrap_or(false);
                            if is_xref {
                                "External Reference".to_string()
                            } else {
                                entity_type_label(entity)
                            }
                        }
                        _ => entity_type_label(entity),
                    };
                    ui::PropertiesPanel {
                        choice_combos: sections
                            .iter()
                            .flat_map(|section| section.props.iter())
                            .filter_map(|prop| match &prop.value {
                                crate::scene::model::object::PropValue::Choice { options, .. } => Some((
                                    prop.field.to_string(),
                                    iced::widget::combo_box::State::new(options.clone()),
                                )),
                                _ => None,
                            })
                            .collect(),
                        sections,
                        title,
                        layer_combo: iced::widget::combo_box::State::new(layer_names.clone()),
                        linetype_combo: iced::widget::combo_box::State::new(linetype_items.clone()),
                        lineweight_combo: iced::widget::combo_box::State::new(
                            ui::properties::lw_options(),
                        ),
                        linetype_items,
                        ..Default::default()
                    }
                }
                // Property aggregation is O(n) per row plus an O(n²) group filter
                // (`group.handles.contains` scans per entity), stalling the rebuild
                // for seconds at tens of thousands of objects. Above the cap show a
                // count-only panel; bulk edits still go through the ribbon.
                n if n > MAX_PROP_AGGREGATE => ui::PropertiesPanel {
                    title: format!("{} objects selected", n),
                    layer_combo: iced::widget::combo_box::State::new(layer_names.clone()),
                    linetype_combo: iced::widget::combo_box::State::new(linetype_items.clone()),
                    lineweight_combo: iced::widget::combo_box::State::new(
                        ui::properties::lw_options(),
                    ),
                    linetype_items,
                    ..Default::default()
                },
                _ => {
                    let groups = build_selection_groups(&selected);
                    let active_group = selected_group
                        .and_then(|group| groups.iter().find(|g| g.label == group.label).cloned())
                        .or_else(|| groups.first().cloned());

                    let filtered: Vec<(Handle, &EntityType)> = active_group
                        .as_ref()
                        .map(|group| {
                            selected
                                .iter()
                                .filter(|(handle, _)| group.handles.contains(handle))
                                .copied()
                                .collect()
                        })
                        .unwrap_or_default();

                    let sections = aggregate_sections(&filtered, &text_style_names);
                    ui::PropertiesPanel {
                        choice_combos: sections
                            .iter()
                            .flat_map(|section| section.props.iter())
                            .filter_map(|prop| match &prop.value {
                                crate::scene::model::object::PropValue::Choice { options, .. } => Some((
                                    prop.field.to_string(),
                                    iced::widget::combo_box::State::new(options.clone()),
                                )),
                                _ => None,
                            })
                            .collect(),
                        sections,
                        title: format!("{} objects selected", selected.len()),
                        selection_group_combo: iced::widget::combo_box::State::new(groups.clone()),
                        selection_groups: groups,
                        selected_group: active_group,
                        layer_combo: iced::widget::combo_box::State::new(layer_names.clone()),
                        linetype_combo: iced::widget::combo_box::State::new(linetype_items.clone()),
                        lineweight_combo: iced::widget::combo_box::State::new(
                            ui::properties::lw_options(),
                        ),
                        linetype_items,
                        ..Default::default()
                    }
                }
            };
            panel.color_palette_open = color_palette_open;
            let new_handles: Vec<acadrust::Handle> = selected.iter().map(|(h, _)| *h).collect();
            // Carry the in-progress edits only when the selection is unchanged
            // (a commit-triggered rebuild); a genuine selection change starts
            // with a clean buffer so no stale value leaks onto the new entity.
            panel.edit_buf = if prev_handles == new_handles {
                edit_buf
            } else {
                Default::default()
            };
            panel.expanded_groups = expanded_groups;
            panel.source_handles = new_handles;
            panel.prop_vertex = prop_vertex;
            panel.prop_vertex_indicator_active = prop_vertex_indicator_active;
            panel
        };

        self.tabs[i].properties = new_panel;
        self.refresh_selected_grips();
        self.sync_ribbon_from_selection();
    }

    /// Drive the Home-ribbon Layer / Color / Linetype / Lineweight dropdowns
    /// from the current entity selection. With no selection the ribbon falls
    /// back to the active creation defaults (per-tab active_layer + ByLayer).
    /// Mixed selections keep the prior value (we'd need a UI "*Varies*"
    /// marker to do better).
    pub(super) fn sync_ribbon_from_selection(&mut self) {
        let i = self.active_tab;
        // The Start (welcome) tab has no document — keep the ribbon's
        // current-layer chip empty rather than re-seeding it with a default.
        if self.tabs[i].is_start {
            self.ribbon.active_layer = String::new();
            return;
        }
        let selected = self.tabs[i].scene.selected_entities();
        if selected.is_empty() {
            // Creation defaults: prefer the file's saved CECOLOR / CELTYPE /
            // CELWEIGHT (and current_layer_name); fall back to ByLayer when
            // those slots are still at their factory default.
            let header = &self.tabs[i].scene.document.header;
            let layer = if header.current_layer_name.is_empty() {
                self.tabs[i].active_layer.clone()
            } else {
                header.current_layer_name.clone()
            };
            self.ribbon.active_layer = layer;
            self.ribbon.active_color = header.current_entity_color;
            // current_linetype_name may be empty when only the handle was
            // written; resolve via line_types table in that case.
            let lt = if !header.current_linetype_name.is_empty() {
                header.current_linetype_name.clone()
            } else if !header.current_linetype_handle.is_null() {
                self.tabs[i]
                    .scene
                    .document
                    .line_types
                    .iter()
                    .find(|lt| lt.handle == header.current_linetype_handle)
                    .map(|lt| lt.name.clone())
                    .unwrap_or_else(|| "ByLayer".to_string())
            } else {
                "ByLayer".to_string()
            };
            self.ribbon.active_linetype = lt;
            self.ribbon.active_lineweight =
                acadrust::types::LineWeight::from_value(header.current_line_weight);
            return;
        }

        let mut layer: Option<String> = None;
        let mut color: Option<acadrust::types::Color> = None;
        let mut linetype: Option<String> = None;
        let mut lineweight: Option<acadrust::types::LineWeight> = None;
        let mut layer_mixed = false;
        let mut color_mixed = false;
        let mut linetype_mixed = false;
        let mut lineweight_mixed = false;

        for (_h, e) in &selected {
            let c = e.common();
            let lt = if c.linetype.is_empty() {
                "ByLayer".to_string()
            } else {
                c.linetype.clone()
            };
            match &layer {
                None => layer = Some(c.layer.clone()),
                Some(prev) if prev != &c.layer => layer_mixed = true,
                _ => {}
            }
            match &color {
                None => color = Some(c.color),
                Some(prev) if prev != &c.color => color_mixed = true,
                _ => {}
            }
            match &linetype {
                None => linetype = Some(lt),
                Some(prev) if prev != &lt => linetype_mixed = true,
                _ => {}
            }
            match &lineweight {
                None => lineweight = Some(c.line_weight),
                Some(prev) if prev != &c.line_weight => lineweight_mixed = true,
                _ => {}
            }
        }
        if !layer_mixed {
            if let Some(l) = layer {
                self.ribbon.active_layer = l;
            }
        }
        if !color_mixed {
            if let Some(c) = color {
                self.ribbon.active_color = c;
            }
        }
        if !linetype_mixed {
            if let Some(l) = linetype {
                self.ribbon.active_linetype = l;
            }
        }
        if !lineweight_mixed {
            if let Some(lw) = lineweight {
                self.ribbon.active_lineweight = lw;
            }
        }
    }

    /// Rebuild the cached selected_grips from the current entity selection.
    pub(super) fn refresh_selected_grips(&mut self) {
        let i = self.active_tab;
        let is_paper = self.tabs[i].scene.current_layout != "Model";
        // Paper-space entity coordinates are NOT offset by world_offset (same rule
        // as wire tessellation in wires_for_block). Only subtract in model space.
        let wo = if is_paper {
            [0.0f64; 3]
        } else {
            [0.0_f64; 3]
        };
        let (new_handle, new_grips, new_grip_handles) = {
            let annotation_scale_handle = self.tabs[i].scene.displayed_annotation_scale_handle();
            let selected = self.tabs[i].scene.selected_entities();
            let single_handle = (selected.len() == 1).then(|| selected[0].0);
            let mut grips = Vec::new();
            let mut handles = Vec::new();
            for (handle, entity) in selected {
                let contextual = crate::scene::annotative::entity_for_annotation_context(
                    &self.tabs[i].scene.document,
                    entity,
                    annotation_scale_handle,
                );
                for mut grip in dispatch::grips(contextual.as_ref()) {
                    // Subtract in f64: at UTM magnitudes an f32 cast before
                    // the offset costs ~1 unit and draws the grip off the wire.
                    grip.world.x -= wo[0];
                    grip.world.y -= wo[1];
                    grip.world.z -= wo[2];
                    handles.push(handle);
                    grips.push(grip);
                }
            }
            (single_handle, grips, handles)
        };
        self.tabs[i].selected_handle = new_handle;
        self.tabs[i].selected_grips = new_grips;
        self.tabs[i].selected_grip_handles = new_grip_handles;
        let available: rustc_hash::FxHashSet<_> = self.tabs[i]
            .selected_grip_handles
            .iter()
            .copied()
            .zip(self.tabs[i].selected_grips.iter().map(|grip| grip.id))
            .collect();
        self.tabs[i]
            .hot_grips
            .retain(|key| available.contains(key));
        // Append the dynamic-block visibility (lookup) grip, if the lone
        // selection is a visibility-parametric block reference.
        self.refresh_visibility_grip(wo);
    }

    pub(super) fn property_target_handles(&self, i: usize) -> Vec<Handle> {
        let handles = self.tabs[i].properties.selected_handles();
        if !handles.is_empty() {
            handles
        } else {
            self.tabs[i].selected_handle.into_iter().collect()
        }
    }

    pub(super) fn invalidate_property_targets(&mut self, i: usize, handles: &[Handle]) {
        let mut context_object_changed = false;
        for &handle in handles {
            context_object_changed |= self.tabs[i]
                .scene
                .sync_displayed_annotation_context(handle);
            // Hatch / SOLID fills render from prebuilt cached models; rebuild
            // them or pattern edits (scale, background, …) stay invisible
            // (#415).
            self.tabs[i].scene.refresh_fill_model(handle);
        }
        if context_object_changed {
            self.tabs[i].scene.poison_undo_recording();
        }
        // Solid (ACIS) meshes bake their colour into the mesh, so a colour /
        // layer change needs an explicit recolour — re-tessellating wires
        // alone wouldn't update them.
        self.tabs[i].scene.recolor_meshes_for_handles(handles);
        let changes: Vec<_> = handles
            .iter()
            .map(|&handle| (handle, crate::scene::ChangeKind::Modified))
            .collect();
        self.tabs[i].scene.bump_entities(&changes);
    }

    /// Add an entity to the correct space (model or paper space layout).
    pub(super) fn commit_entity(&mut self, entity: acadrust::EntityType) {
        let _ = self.commit_entity_handle(entity);
    }

    /// Like [`commit_entity`] but returns the handle the new entity was given
    /// (or `None` if it could not be added). Lets callers follow up — e.g.
    /// open the in-place text editor on a freshly created MultiLeader.
    pub(super) fn commit_entity_handle(
        &mut self,
        mut entity: acadrust::EntityType,
    ) -> Option<Handle> {
        let i = self.active_tab;
        let tracks_draw_anchor = self.tabs[i].active_cmd.is_some()
            && matches!(
                &entity,
                acadrust::EntityType::Line(_)
                    | acadrust::EntityType::Arc(_)
                    | acadrust::EntityType::LwPolyline(_)
                    | acadrust::EntityType::Polyline(_)
                    | acadrust::EntityType::Polyline2D(_)
                    | acadrust::EntityType::Polyline3D(_)
            );
        let layer = &self.tabs[i].active_layer;
        if layer != "0" || entity.as_entity().layer().is_empty() {
            entity.as_entity_mut().set_layer(layer.clone());
        }

        // INSUNITS: when inserting a block whose BlockRecord.units differ
        // from the host's header.insertion_units, scale the new INSERT so
        // 1 source-unit equals the matching host length. When either side
        // is unitless (0) AutoCAD falls back to MEASUREMENT (0 = Imperial /
        // inches, 1 = Metric / mm); honour the same fallback.
        if let acadrust::EntityType::Insert(ref mut ins) = entity {
            let header = &self.tabs[i].scene.document.header;
            let measurement_fallback = if header.measurement == 1 { 4 } else { 1 };
            let host_raw = header.insertion_units;
            let host_units = if host_raw == 0 { measurement_fallback } else { host_raw };
            let src_raw = self.tabs[i]
                .scene
                .document
                .block_records
                .get(&ins.block_name)
                .map(|br| br.units)
                .unwrap_or(0);
            let src_units = if src_raw == 0 { measurement_fallback } else { src_raw };
            if src_units != host_units {
                let ratio = insunits_to_mm(src_units) / insunits_to_mm(host_units);
                if ratio.is_finite() && (ratio - 1.0).abs() > 1e-9 {
                    ins.set_x_scale(ratio);
                    ins.set_y_scale(ratio);
                    ins.set_z_scale(ratio);
                }
            }
        }

        crate::scene::view::dispatch::apply_color(&mut entity, self.ribbon.active_color);
        crate::scene::view::dispatch::apply_common_prop(
            &mut entity,
            "linetype",
            &self.ribbon.active_linetype.clone(),
        );
        crate::scene::view::dispatch::apply_line_weight(&mut entity, self.ribbon.active_lineweight);
        // CELTSCALE (header.current_entity_linetype_scale): new entities
        // pick up the document's saved per-entity linetype scale. The user
        // can override per entity later via the properties panel.
        let celtscale = self.tabs[i].scene.document.header.current_entity_linetype_scale;
        if (celtscale - 1.0).abs() > 1e-9 && celtscale.abs() > 1e-9 {
            entity.common_mut().linetype_scale = celtscale;
        }

        // A new dimension inherits the document's current dimension style
        // (DIMSTYLE) instead of staying at the entity "Standard" default. Only
        // fill in when still at the default so an explicitly-styled dimension
        // is preserved. See #92.
        if let acadrust::EntityType::Dimension(ref mut d) = entity {
            let cur = self.tabs[i]
                .scene
                .document
                .header
                .current_dimstyle_name
                .clone();
            let s = d.base().style_name.clone();
            if (s.is_empty() || s.eq_ignore_ascii_case("Standard")) && !cur.is_empty() {
                d.base_mut().style_name = cur;
            }
        }

        // MultiLeader / Table inherit the document's current style (#92). These
        // styles live in the objects dictionary, so resolve the current style
        // name to its object handle. Left untouched when the command already
        // assigned a style or no matching style object exists.
        match &mut entity {
            acadrust::EntityType::MultiLeader(ml) if ml.style_handle.is_none() => {
                let name = self.tabs[i]
                    .scene
                    .document
                    .header
                    .current_mleader_style_name
                    .clone();
                if !name.is_empty() {
                    let found =
                        self.tabs[i].scene.document.objects.iter().find_map(|(h, o)| match o {
                            acadrust::objects::ObjectType::MultiLeaderStyle(s)
                                if s.name.eq_ignore_ascii_case(&name) =>
                            {
                                Some((*h, s.clone()))
                            }
                            _ => None,
                        });
                    if let Some((h, s)) = found {
                        debug_assert_eq!(h, s.handle);
                        crate::scene::annotative::apply_mleader_style(ml, &s);
                    }
                }
            }
            acadrust::EntityType::Table(t) if t.table_style_handle.is_none() => {
                let name = self.tabs[i]
                    .scene
                    .document
                    .header
                    .current_table_style_name
                    .clone();
                if !name.is_empty() {
                    t.table_style_handle =
                        self.tabs[i].scene.document.objects.iter().find_map(|(h, o)| {
                            match o {
                                acadrust::objects::ObjectType::TableStyle(s)
                                    if s.name.eq_ignore_ascii_case(&name) =>
                                {
                                    Some(*h)
                                }
                                _ => None,
                            }
                        });
                }
            }
            _ => {}
        }

        let text_style_annotative = match &entity {
            acadrust::EntityType::Text(text) => {
                crate::scene::annotative::text_style_is_annotative(
                    &self.tabs[i].scene.document,
                    &text.style,
                )
            }
            acadrust::EntityType::MText(text) => {
                crate::scene::annotative::text_style_is_annotative(
                    &self.tabs[i].scene.document,
                    &text.style,
                )
            }
            acadrust::EntityType::AttributeEntity(attribute) => {
                crate::scene::annotative::text_style_is_annotative(
                    &self.tabs[i].scene.document,
                    &attribute.text_style,
                )
            }
            acadrust::EntityType::AttributeDefinition(attribute) => {
                crate::scene::annotative::text_style_is_annotative(
                    &self.tabs[i].scene.document,
                    &attribute.text_style,
                )
            }
            _ => false,
        };
        if text_style_annotative {
            match &mut entity {
                acadrust::EntityType::MText(text) => text.is_annotative = true,
                acadrust::EntityType::AttributeEntity(attribute) => {
                    attribute.flags.annotative = true
                }
                acadrust::EntityType::AttributeDefinition(attribute) => {
                    attribute.flags.annotative = true
                }
                _ => {}
            }
        }
        let needs_annotation_context = crate::scene::annotative::is_annotative(
            &self.tabs[i].scene.document,
            &entity,
        ) || crate::scene::annotative::annotation_style_is_annotative(
            &self.tabs[i].scene.document,
            &entity,
        );

        let new_handle = if matches!(&entity, acadrust::EntityType::Viewport(_))
            && self.tabs[i].scene.current_layout != "Model"
        {
            // Assign a unique viewport ID (max existing id + 1, min 2).
            if let acadrust::EntityType::Viewport(ref mut vp) = entity {
                let layout_block = self.tabs[i].scene.current_layout_block_handle_pub();
                let max_id = self.tabs[i]
                    .scene
                    .document
                    .entities()
                    .filter_map(|e| {
                        if let acadrust::EntityType::Viewport(v) = e {
                            if v.common.owner_handle == layout_block {
                                Some(v.id)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .max()
                    .unwrap_or(1);
                vp.id = (max_id + 1).max(2);
            }

            let layout = self.tabs[i].scene.current_layout.clone();
            match self.tabs[i]
                .scene
                .document
                .add_entity_to_layout(entity, &layout)
            {
                Ok(new_handle) => {
                    if self.tabs[i].scene.is_recording_undo() {
                        self.tabs[i].scene.record_undo_before(new_handle, None);
                    }
                    self.tabs[i].scene.auto_fit_viewport(new_handle);
                    // Adding a viewport straight onto the document layout
                    // bypasses Scene::add_entity; publish the exact new handle
                    // so only its border is tessellated.
                    self.tabs[i]
                        .scene
                        .bump_entities(&[(new_handle, crate::scene::ChangeKind::Added)]);
                    Some(new_handle)
                }
                Err(e) => {
                    self.command_line
                        .push_error(&format!("Viewport could not be added: {e}"));
                    None
                }
            }
        } else {
            Some(self.tabs[i].scene.add_entity(entity))
        };

        if needs_annotation_context {
            if let (Some(handle), Some(scale)) = (
                new_handle,
                self.tabs[i].scene.creation_annotation_scale_handle(),
            ) {
                crate::scene::annotative::create_annotation_context(
                    &mut self.tabs[i].scene.document,
                    handle,
                    scale,
                );
                self.tabs[i]
                    .scene
                    .bump_entities(&[(handle, crate::scene::ChangeKind::Modified)]);
            }
        }

        if tracks_draw_anchor {
            if let Some(handle) = new_handle {
                self.tabs[i].last_draw_anchor = Some(handle);
            }
        }
        new_handle
    }
}

// ── Multi-selection property aggregation ───────────────────────────────────

pub(super) fn build_selection_groups(
    selected: &[(Handle, &EntityType)],
) -> Vec<ui::properties::SelectionGroup> {
    let mut groups = vec![ui::properties::SelectionGroup {
        label: format!("All({})", selected.len()),
        handles: selected.iter().map(|(handle, _)| *handle).collect(),
    }];

    let mut by_type: std::collections::BTreeMap<String, Vec<Handle>> =
        std::collections::BTreeMap::new();
    for (handle, entity) in selected {
        by_type
            .entry(entity_type_key(entity))
            .or_default()
            .push(*handle);
    }

    for (kind, handles) in by_type {
        groups.push(ui::properties::SelectionGroup {
            label: format!("{}({})", title_case_word(&kind), handles.len()),
            handles,
        });
    }

    groups
}

pub(super) fn aggregate_sections(
    selected: &[(Handle, &EntityType)],
    text_style_names: &[String],
) -> Vec<crate::scene::model::object::PropSection> {
    if selected.is_empty() {
        return vec![];
    }

    let mut all_sections: Vec<Vec<crate::scene::model::object::PropSection>> = selected
        .iter()
        .map(|(handle, entity)| dispatch::properties_sectioned(*handle, entity, text_style_names))
        .collect();

    let mut result = all_sections.remove(0);
    for sections in all_sections {
        result = merge_sections(&result, &sections);
    }
    result
}

fn merge_sections(
    left: &[crate::scene::model::object::PropSection],
    right: &[crate::scene::model::object::PropSection],
) -> Vec<crate::scene::model::object::PropSection> {
    left.iter()
        .filter_map(|section| {
            let rhs = right
                .iter()
                .find(|candidate| candidate.title == section.title)?;
            let props: Vec<crate::scene::model::object::Property> = section
                .props
                .iter()
                .filter_map(|prop| {
                    let other = rhs
                        .props
                        .iter()
                        .find(|candidate| candidate.field == prop.field)?;
                    Some(crate::scene::model::object::Property {
                        label: prop.label.clone(),
                        field: prop.field,
                        value: merge_prop_value(&prop.value, &other.value),
                    })
                })
                .collect();
            if props.is_empty() {
                None
            } else {
                Some(crate::scene::model::object::PropSection {
                    title: section.title.clone(),
                    props,
                })
            }
        })
        .collect()
}

fn merge_prop_value(
    left: &crate::scene::model::object::PropValue,
    right: &crate::scene::model::object::PropValue,
) -> crate::scene::model::object::PropValue {
    use crate::scene::model::object::PropValue;

    if left == right {
        return left.clone();
    }

    match (left, right) {
        (PropValue::LayerChoice(_), PropValue::LayerChoice(_)) => {
            PropValue::LayerChoice(VARIES_LABEL.into())
        }
        (PropValue::ColorChoice(_), PropValue::ColorChoice(_))
        | (PropValue::ColorVaries, _)
        | (_, PropValue::ColorVaries) => PropValue::ColorVaries,
        (PropValue::LwChoice(_), PropValue::LwChoice(_))
        | (PropValue::LwVaries, _)
        | (_, PropValue::LwVaries) => PropValue::LwVaries,
        (PropValue::LinetypeChoice(_), PropValue::LinetypeChoice(_)) => {
            PropValue::LinetypeChoice(VARIES_LABEL.into())
        }
        (
            PropValue::Choice { options, .. },
            PropValue::Choice {
                options: other_options,
                ..
            },
        ) if options == other_options => PropValue::Choice {
            selected: VARIES_LABEL.into(),
            options: options.clone(),
        },
        (
            PropValue::EditChoice { options, .. },
            PropValue::EditChoice {
                options: other_options,
                ..
            },
        ) if options == other_options => PropValue::EditChoice {
            value: VARIES_LABEL.into(),
            options: options.clone(),
        },
        (PropValue::EditText(_), PropValue::EditText(_)) => {
            PropValue::EditText(VARIES_LABEL.into())
        }
        (PropValue::ReadOnly(_), PropValue::ReadOnly(_)) => {
            PropValue::ReadOnly(VARIES_LABEL.into())
        }
        (PropValue::HatchPatternChoice(_), PropValue::HatchPatternChoice(_)) => {
            PropValue::HatchPatternChoice(VARIES_LABEL.into())
        }
        (
            PropValue::BoolToggle { field, .. },
            PropValue::BoolToggle {
                field: other_field, ..
            },
        ) if field == other_field => PropValue::ReadOnly(VARIES_LABEL.into()),
        _ => left.clone(),
    }
}

/// Set the first property row matching `field` (across all sections) to a
/// read-only `value`. No-op when the field is absent. Used to fill the
/// doc-dependent placeholder rows the entity builders leave empty.
fn set_row(
    sections: &mut [crate::scene::model::object::PropSection],
    field: &str,
    value: String,
) {
    for section in sections.iter_mut() {
        if let Some(row) = section.props.iter_mut().find(|p| p.field == field) {
            row.value = crate::scene::model::object::PropValue::ReadOnly(value);
            return;
        }
    }
}

/// Replace a row's value with an arbitrary control (editable field, dropdown,
/// colour picker …) rather than plain read-only text.
fn set_row_value(
    sections: &mut [crate::scene::model::object::PropSection],
    field: &str,
    value: crate::scene::model::object::PropValue,
) {
    for section in sections.iter_mut() {
        if let Some(row) = section.props.iter_mut().find(|p| p.field == field) {
            row.value = value;
            return;
        }
    }
}

/// The lineweight dropdown options (named defaults + the standard millimetre
/// steps), matching the labels `dim_lineweight_label` produces.
pub(crate) fn lineweight_options() -> Vec<String> {
    let mut v = vec![
        "ByLayer".to_string(),
        "ByBlock".to_string(),
        "Default".to_string(),
    ];
    for lw in [
        0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158,
        200, 211,
    ] {
        v.push(format!("{:.2} mm", lw as f64 / 100.0));
    }
    v
}

/// Inverse of `dim_lineweight_label`: a lineweight label → DIMLWD enum value.
pub(crate) fn dim_lineweight_from_label(label: &str) -> i16 {
    match label.trim() {
        "ByLayer" => -1,
        "ByBlock" => -2,
        "Default" => -3,
        s => s
            .trim_end_matches("mm")
            .trim()
            .parse::<f64>()
            .map(|mm| (mm * 100.0).round() as i16)
            .unwrap_or(-3),
    }
}

/// The vertical-text-position dropdown options, matching `dimtad_label`.
pub(crate) fn tad_options() -> Vec<String> {
    ["Centered", "Above", "Outside", "JIS", "Below"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Inverse of `dimtad_label`: a vertical-position label → DIMTAD value.
pub(crate) fn dimtad_from_label(label: &str) -> i16 {
    match label.trim() {
        "Above" => 1,
        "Outside" => 2,
        "JIS" => 3,
        "Below" => 4,
        _ => 0,
    }
}

/// Resolve a dimension style by name (case-insensitive), falling back to
/// "Standard" when the name is blank.
fn find_dim_style<'a>(
    doc: &'a acadrust::CadDocument,
    name: &str,
) -> Option<&'a acadrust::tables::DimStyle> {
    doc.dim_styles.iter().find(|s| {
        s.name.eq_ignore_ascii_case(name)
            || (name.trim().is_empty() && s.name.eq_ignore_ascii_case("Standard"))
    })
}

/// Insert `row` immediately after the first property whose field matches.
fn insert_row_after(
    sections: &mut [crate::scene::model::object::PropSection],
    field: &str,
    row: crate::scene::model::object::Property,
) {
    for section in sections.iter_mut() {
        if let Some(idx) = section.props.iter().position(|p| p.field == field) {
            section.props.insert(idx + 1, row);
            return;
        }
    }
}

/// Vertical text placement (DIMTAD) label.
fn dimtad_label(dimtad: i16) -> &'static str {
    match dimtad {
        1 => "Above",
        2 => "Outside",
        3 => "JIS",
        4 => "Below",
        _ => "Centered",
    }
}

/// Friendly arrowhead name from an arrowhead block-record name (the `_CLOSED…`
/// style internal names map to their palette labels; a null/empty name is the
/// closed-filled default).
pub(crate) fn arrowhead_label(name: &str) -> String {
    let key = name.trim().trim_start_matches('_').to_ascii_uppercase();
    let label = match key.as_str() {
        "" | "CLOSEDFILLED" => "Closed filled",
        "CLOSED" => "Closed",
        "CLOSEDBLANK" => "Closed blank",
        "DOT" => "Dot",
        "DOTSMALL" => "Dot small",
        "DOTBLANK" => "Dot blank",
        "SMALLDOTBLANK" => "Dot small blank",
        "ORIGIN" => "Origin indicator",
        "ORIGIN2" => "Origin indicator 2",
        "OPEN" => "Open",
        "OPEN90" => "Right angle",
        "OPEN30" => "Open 30",
        "NONE" => "None",
        "OBLIQUE" => "Oblique",
        "ARCHTICK" => "Architectural tick",
        "BOXBLANK" => "Box",
        "BOXFILLED" => "Box filled",
        "DATUMBLANK" => "Datum triangle",
        "DATUMFILLED" => "Datum triangle filled",
        "INTEGRAL" => "Integral",
        _ => return name.to_string(),
    };
    label.to_string()
}

/// The leader's arrowhead label, resolved from the dim style's DIMLDRBLK block.
fn leader_arrow_label(
    doc: &acadrust::CadDocument,
    ds: &acadrust::tables::DimStyle,
    arrow_enabled: bool,
) -> String {
    if !arrow_enabled {
        return "None".to_string();
    }
    if ds.dimldrblk.is_null() {
        return "Closed filled".to_string();
    }
    doc.block_records
        .iter()
        .find(|b| b.handle == ds.dimldrblk)
        .map(|b| arrowhead_label(&b.name))
        .unwrap_or_else(|| "Closed filled".to_string())
}

/// DIMLWD lineweight enum → label.
fn dim_lineweight_label(dimlwd: i16) -> String {
    match dimlwd {
        -1 => "ByLayer".to_string(),
        -2 => "ByBlock".to_string(),
        -3 => "Default".to_string(),
        v if v >= 0 => format!("{:.2} mm", v as f64 / 100.0),
        _ => "Default".to_string(),
    }
}

/// Human-readable INSUNITS name (DXF group 70 unit codes).
fn insunits_name(code: i16) -> &'static str {
    match code {
        1 => "Inches",
        2 => "Feet",
        3 => "Miles",
        4 => "Millimeters",
        5 => "Centimeters",
        6 => "Meters",
        7 => "Kilometers",
        8 => "Microinches",
        9 => "Mils",
        10 => "Yards",
        11 => "Angstroms",
        12 => "Nanometers",
        13 => "Microns",
        14 => "Decimeters",
        15 => "Decameters",
        16 => "Hectometers",
        17 => "Gigameters",
        18 => "Astronomical Units",
        19 => "Light Years",
        20 => "Parsecs",
        21 => "US Survey Feet",
        _ => "Unitless",
    }
}

/// Convert INSUNITS (DXF group 70) to millimetres.
/// 0 = unitless / unknown: returns 1.0 so the caller treats it as "do not scale".
fn insunits_to_mm(code: i16) -> f64 {
    match code {
        1 => 25.4,            // Inches
        2 => 304.8,           // Feet
        3 => 1_609_344.0,     // Miles
        4 => 1.0,             // Millimeters
        5 => 10.0,            // Centimeters
        6 => 1_000.0,         // Meters
        7 => 1_000_000.0,     // Kilometers
        8 => 0.000_025_4,     // Microinches
        9 => 0.025_4,         // Mils
        10 => 914.4,          // Yards
        11 => 1.0e-7,         // Angstroms
        12 => 1.0e-6,         // Nanometers
        13 => 0.001,          // Microns
        14 => 100.0,          // Decimeters
        15 => 10_000.0,       // Decameters
        16 => 100_000.0,      // Hectometers
        17 => 1.0e12,         // Gigameters
        18 => 1.496e14,       // Astronomical Units
        19 => 9.461e18,       // Light Years
        20 => 3.086e19,       // Parsecs
        21 => 304.800_609_6,  // US Survey Feet
        _ => 1.0,
    }
}
