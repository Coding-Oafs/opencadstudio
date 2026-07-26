use acadrust::entities::{SectionSymbol, ViewBorder};

use crate::command::EntityTransform;
use crate::entities::common::{edit_angle_prop, edit_prop, parse_f64, ro_prop, square_grip};
use crate::entities::traits::{Grippable, PropertyEditable, Transformable};
use crate::scene::model::object::{GripApply, GripDef, PropSection};

fn handle_text(handle: acadrust::Handle) -> String {
    if handle.is_null() {
        "None".to_string()
    } else {
        format!("{:X}", handle.value())
    }
}

fn vector_text(vector: &acadrust::types::Vector3) -> String {
    format!("{:.6}, {:.6}, {:.6}", vector.x, vector.y, vector.z)
}

fn apply_entity_transform<T: acadrust::Entity>(entity: &mut T, transform: &EntityTransform) {
    crate::scene::view::transform::apply_standard_entity_transform(
        entity,
        transform,
        |entity, first, second| {
            entity.apply_transform(
                &crate::scene::view::transform::reflection_about_xy_line(first, second),
            );
        },
    );
}

impl Grippable for SectionSymbol {
    fn grips(&self) -> Vec<GripDef> {
        self.points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                square_grip(
                    index,
                    glam::DVec3::new(point.point.x, point.point.y, point.point.z),
                )
            })
            .collect()
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        let Some(point) = self.points.get_mut(grip_id) else {
            return;
        };
        match apply {
            GripApply::Translate(delta) => {
                point.point.x += delta.x;
                point.point.y += delta.y;
                point.point.z += delta.z;
            }
            GripApply::Absolute(position) => {
                point.point =
                    acadrust::types::Vector3::new(position.x, position.y, position.z);
            }
        }
        self.sync_display_fields();
    }
}

impl PropertyEditable for SectionSymbol {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let point_data = self
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                format!(
                    "{}: point [{}]; bulge {:.6}; label {:?}; offset [{}]; flag {}",
                    index + 1,
                    vector_text(&point.point),
                    point.bulge,
                    point.label,
                    vector_text(&point.label_offset),
                    point.raw_flag_280
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        vec![
            PropSection {
                title: "Section Symbol".into(),
                props: vec![
                    edit_prop("Symbol Scale", "section_scale", self.symbol_scale),
                    ro_prop(
                        "View Symbol Version",
                        "section_view_version",
                        self.view_symbol_version.to_string(),
                    ),
                    ro_prop("Version", "section_version", self.version.to_string()),
                    ro_prop(
                        "Style",
                        "section_style",
                        handle_text(self.style_handle),
                    ),
                    ro_prop(
                        "View Representation",
                        "section_view_rep",
                        handle_text(self.view_rep_handle),
                    ),
                    ro_prop(
                        "View Symbol Flags",
                        "section_view_flags",
                        self.raw_view_symbol_70.to_string(),
                    ),
                    ro_prop(
                        "Point Count",
                        "section_point_count",
                        self.points.len().to_string(),
                    ),
                    ro_prop(
                        "Stored Point Counts",
                        "section_raw_counts",
                        format!(
                            "{}, {}, flags {}",
                            self.raw_point_count_90,
                            self.raw_point_record_count,
                            self.raw_flags_90
                        ),
                    ),
                    ro_prop("Identifier", "section_label", self.label.clone()),
                ],
            },
            PropSection {
                title: "Section Points".into(),
                props: vec![ro_prop("Records", "section_points", point_data)],
            },
        ]
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        if field == "section_scale" {
            if let Some(value) = parse_f64(value) {
                self.symbol_scale = value.max(0.0);
            }
        }
    }
}

impl Transformable for SectionSymbol {
    fn apply_transform(&mut self, transform: &EntityTransform) {
        apply_entity_transform(self, transform);
        self.sync_display_fields();
    }
}

impl Grippable for ViewBorder {
    fn grips(&self) -> Vec<GripDef> {
        vec![
            square_grip(0, glam::DVec3::new(self.min[0], self.min[1], 0.0)),
            square_grip(1, glam::DVec3::new(self.max[0], self.max[1], 0.0)),
            square_grip(2, glam::DVec3::new(self.center[0], self.center[1], 0.0)),
        ]
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        let delta = match apply {
            GripApply::Translate(delta) => delta,
            GripApply::Absolute(position) => {
                let current = match grip_id {
                    0 => glam::DVec3::new(self.min[0], self.min[1], 0.0),
                    1 => glam::DVec3::new(self.max[0], self.max[1], 0.0),
                    2 => glam::DVec3::new(self.center[0], self.center[1], 0.0),
                    _ => return,
                };
                position - current
            }
        };
        match grip_id {
            0 => {
                self.min[0] += delta.x;
                self.min[1] += delta.y;
            }
            1 => {
                self.max[0] += delta.x;
                self.max[1] += delta.y;
            }
            2 => {
                self.min[0] += delta.x;
                self.min[1] += delta.y;
                self.max[0] += delta.x;
                self.max[1] += delta.y;
            }
            _ => return,
        }
        self.center = [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
        ];
    }
}

impl PropertyEditable for ViewBorder {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        vec![PropSection {
            title: "Drawing View".into(),
            props: vec![
                edit_prop("Center X", "view_border_x", self.center[0]),
                edit_prop("Center Y", "view_border_y", self.center[1]),
                ro_prop(
                    "Width",
                    "view_border_width",
                    format!("{:.6}", self.max[0] - self.min[0]),
                ),
                ro_prop(
                    "Height",
                    "view_border_height",
                    format!("{:.6}", self.max[1] - self.min[1]),
                ),
                edit_prop("Scale", "view_border_scale", self.scale),
                edit_angle_prop(
                    "Rotation",
                    "view_border_rotation",
                    self.rotation_angle.to_degrees(),
                ),
                ro_prop("Version", "view_border_version", self.version.to_string()),
                ro_prop(
                    "Active Viewport",
                    "view_border_viewport",
                    handle_text(self.active_viewport),
                ),
                ro_prop(
                    "Scale Object",
                    "view_border_scale_handle",
                    handle_text(self.scale_handle),
                ),
            ],
        }]
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        let Some(value) = parse_f64(value) else {
            return;
        };
        match field {
            "view_border_x" => {
                let delta = value - self.center[0];
                self.min[0] += delta;
                self.max[0] += delta;
                self.center[0] = value;
            }
            "view_border_y" => {
                let delta = value - self.center[1];
                self.min[1] += delta;
                self.max[1] += delta;
                self.center[1] = value;
            }
            "view_border_scale" => self.scale = value.max(0.0),
            "view_border_rotation" => self.rotation_angle = value.to_radians(),
            _ => {}
        }
    }
}

impl Transformable for ViewBorder {
    fn apply_transform(&mut self, transform: &EntityTransform) {
        apply_entity_transform(self, transform);
    }
}
