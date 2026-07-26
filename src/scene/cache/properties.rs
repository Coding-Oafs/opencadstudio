use acadrust::{EntityType, Handle};

use crate::scene::model::object::{PropSection, PropValue, Property};

pub fn general_section(entity: &EntityType) -> PropSection {
    let common = entity.common();
    let linetype_display = if common.linetype.is_empty() {
        "ByLayer".to_string()
    } else {
        common.linetype.clone()
    };
    // Alpha 0 is the ByLayer default (Transparency::BY_LAYER); show it by name
    // and fall back to a rounded percentage only for an explicit value.
    let transp_display = if common.transparency.alpha() == 0 {
        "ByLayer".to_string()
    } else {
        format!(
            "{}",
            (common.transparency.alpha() as f64 / 255.0 * 100.0).round() as u32
        )
    };

    // Hyperlink is stored in XDATA under the "PE_URL" application.
    let hyperlink = common
        .extended_data
        .get_record("PE_URL")
        .and_then(|r| {
            r.values.iter().find_map(|v| match v {
                acadrust::xdata::XDataValue::String(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();

    let mut section = PropSection {
        title: "General".into(),
        props: vec![
            Property {
                label: "Handle".into(),
                field: "handle",
                value: PropValue::ReadOnly(common.handle.value().to_string()),
            },
            Property {
                label: "Color".into(),
                field: "color",
                value: PropValue::ColorChoice(common.color),
            },
            Property {
                label: "Layer".into(),
                field: "layer",
                value: PropValue::LayerChoice(common.layer.clone()),
            },
            Property {
                label: "Linetype".into(),
                field: "linetype",
                value: PropValue::LinetypeChoice(linetype_display),
            },
            Property {
                label: "Linetype scale".into(),
                field: "linetype_scale",
                value: PropValue::EditText(format!("{:.4}", common.linetype_scale)),
            },
            Property {
                label: "Plot style".into(),
                field: "plot_style",
                value: PropValue::ReadOnly(
                    match common.plotstyle_flags {
                        0 => "ByLayer",
                        1 => "ByBlock",
                        _ => "ByColor",
                    }
                    .into(),
                ),
            },
            Property {
                label: "Lineweight".into(),
                field: "lineweight",
                value: PropValue::LwChoice(common.line_weight),
            },
            Property {
                label: "Transparency".into(),
                field: "transparency",
                value: PropValue::EditText(transp_display),
            },
            Property {
                label: "Hyperlink".into(),
                field: "hyperlink",
                value: PropValue::EditText(hyperlink),
            },
        ],
    };

    // Thickness (DXF 39) is a General-group property, but only the entity
    // types that carry an extrusion thickness expose it (line, circle, arc,
    // polyline, text, 2D solid, …). Show it right after Hyperlink for those.
    if let Some(t) = crate::scene::view::dispatch::entity_thickness(entity) {
        section
            .props
            .push(crate::entities::common::edit_prop("Thickness", "thickness", t));
    }

    section
}

/// The "3D Visualization" group (Material), common to every graphical object.
/// Material source is flag-based; a custom material handle is shown as "Custom"
/// (name resolution needs the doc).
pub fn visualization_section(entity: &EntityType) -> Option<PropSection> {
    if matches!(
        entity,
        EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::Seqend(_)
            | EntityType::Leader(_)
            | EntityType::Unknown(_)
            // Non-plotting drawing-view border: never rendered, no properties.
            | EntityType::ViewBorder(_)
    ) {
        return None;
    }
    let common = entity.common();
    let material = match common.material_flags {
        0 => "ByLayer",
        1 => "ByBlock",
        _ => "Custom",
    };
    let mut props = vec![Property {
        label: "Material".into(),
        field: "material",
        value: PropValue::ReadOnly(material.into()),
    }];
    let handle_text = |handle: Option<acadrust::Handle>| {
        handle
            .filter(|handle| handle.is_valid())
            .map_or_else(|| "None".to_string(), |handle| format!("{:X}", handle.value()))
    };
    props.extend([
        Property {
            label: "Visual style".into(),
            field: "visual_style",
            value: PropValue::ReadOnly(handle_text(common.full_visual_style_handle)),
        },
        Property {
            label: "Face style".into(),
            field: "face_visual_style",
            value: PropValue::ReadOnly(handle_text(common.face_visual_style_handle)),
        },
        Property {
            label: "Edge style".into(),
            field: "edge_visual_style",
            value: PropValue::ReadOnly(handle_text(common.edge_visual_style_handle)),
        },
        Property {
            label: "Shadow flags".into(),
            field: "shadow_flags",
            value: PropValue::ReadOnly(common.shadow_flags.to_string()),
        },
    ]);
    Some(PropSection {
        title: "3D Visualization".into(),
        props,
    })
}

pub fn fallback_properties(_handle: Handle, entity: &EntityType) -> PropSection {
    PropSection {
        title: "Geometry".into(),
        props: vec![Property {
            label: "Type".into(),
            field: "type",
            value: PropValue::ReadOnly(crate::entities::names::ui_name_or_class(entity).into()),
        }],
    }
}
