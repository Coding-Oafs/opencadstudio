// Grippable + PropertyEditable for Solid3D, Region, Body.
//
// Geometry lives in ACIS data — we cannot edit it via the properties panel.
// We expose the point_of_reference as a translate grip and show ACIS size
// as read-only info.  Grip translate also updates wire points so the wire
// fallback stays in sync; the caller (scene/mod.rs apply_grip) translates
// the MeshModel vertices to match.

use acadrust::entities::{Body, Region, Solid3D, Surface};
use crate::command::EntityTransform;
use crate::entities::common::{center_grip, edit_prop as edit, parse_f64, ro_prop as ro};
use crate::entities::traits::{Grippable, PropertyEditable, Transformable};
use crate::scene::model::object::{GripApply, GripDef, PropSection};

/// Shared transform for the ACIS volume entities. Translate / rotate / scale
/// delegate to acadrust (which composes the move into the solid's ACIS
/// placement), and mirror delegates via a reflection transform. Without this
/// the entity dispatcher treated solids as non-transformable, so a moved or
/// pasted solid stayed at its original ACIS placement.
macro_rules! impl_acis_transformable {
    ($ty:ty) => {
        impl Transformable for $ty {
            fn apply_transform(&mut self, t: &EntityTransform) {
                crate::scene::view::transform::apply_standard_entity_transform(self, t, |e, p1, p2| {
                    let m = crate::scene::view::transform::reflection_about_xy_line(p1, p2);
                    acadrust::Entity::apply_transform(e, &m);
                });
            }
        }
    };
}
impl_acis_transformable!(Solid3D);
impl_acis_transformable!(Region);
impl_acis_transformable!(Body);
impl_acis_transformable!(Surface);

// ── shared helpers ────────────────────────────────────────────────────────────

fn dvec3(v: &acadrust::types::Vector3) -> glam::DVec3 {
    glam::DVec3::new(v.x, v.y, v.z)
}

fn translate_acis_entity<T: acadrust::Entity>(entity: &mut T, d: glam::DVec3) {
    acadrust::Entity::translate(
        entity,
        acadrust::types::Vector3::new(d.x, d.y, d.z),
    );
}

fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

fn handle_text(handle: Option<acadrust::Handle>) -> String {
    handle
        .filter(|handle| handle.is_valid())
        .map(|handle| format!("{:X}", handle.value()))
        .unwrap_or_else(|| "None".to_string())
}

fn acis_sections(
    acis: &acadrust::entities::AcisData,
    wires: &[acadrust::entities::Wire],
    silhouettes: &[acadrust::entities::Silhouette],
    history: Option<acadrust::Handle>,
) -> Vec<PropSection> {
    let mut wire_types = [0usize; 5];
    let mut transformed = 0usize;
    let mut points = 0usize;
    for wire in wires {
        let index = match wire.wire_type {
            acadrust::entities::WireType::Silhouette => 1,
            acadrust::entities::WireType::VisibleEdge => 2,
            acadrust::entities::WireType::HiddenEdge => 3,
            acadrust::entities::WireType::Isoline => 4,
            _ => 0,
        };
        wire_types[index] += 1;
        transformed += wire.has_transform as usize;
        points += wire.points.len();
    }
    let revision = if acis.revision.has_guid {
        format!(
            "{}-{}-{}-{:02X?}; end {}",
            acis.revision.major,
            acis.revision.minor1,
            acis.revision.minor2,
            acis.revision.bytes,
            acis.revision.end_marker
        )
    } else {
        "None".to_string()
    };
    let bindings = if acis.materials.is_empty() {
        "None".to_string()
    } else {
        acis.materials
            .iter()
            .map(|binding| {
                format!(
                    "{}:{}→{}",
                    binding.array_index,
                    binding.absolute_reference,
                    handle_text(binding.material_handle)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let extra = acis.extra_acis_data.as_ref().map_or_else(
        || "None".to_string(),
        |data| {
            format!(
                "{}; {} bytes",
                if data.is_binary { "SAB" } else { "SAT" },
                data.size()
            )
        },
    );
    vec![
        PropSection {
            title: "Modeler Geometry".into(),
            props: vec![
                ro(
                    "Format",
                    "acis_format",
                    if acis.is_binary { "SAB" } else { "SAT" },
                ),
                ro("Version", "acis_version", format!("{:?}", acis.version)),
                ro("Data Size", "acis_size", format!("{} bytes", acis.size())),
                ro("Revision", "acis_revision", revision),
                ro("Material Bindings", "acis_materials", bindings),
                ro("Extra Modeler Data", "acis_extra", extra),
                ro("History", "acis_history", handle_text(history)),
            ],
        },
        PropSection {
            title: "Modeler Display Data".into(),
            props: vec![
                ro(
                    "Wireframe Cache",
                    "acis_wireframe",
                    yes_no(acis.wireframe_data_present),
                ),
                ro(
                    "Reference Point",
                    "acis_wireframe_point",
                    yes_no(acis.wireframe_point_present),
                ),
                ro(
                    "Isoline List",
                    "acis_wireframe_isolines",
                    yes_no(acis.wireframe_isoline_present),
                ),
                ro(
                    "Isolines",
                    "acis_isolines",
                    acis.wireframe_isolines.to_string(),
                ),
                ro(
                    "Wires",
                    "acis_wires",
                    format!(
                        "{} (unknown {}, silhouette {}, visible {}, hidden {}, isoline {}; {} points; {} transformed)",
                        wires.len(),
                        wire_types[0],
                        wire_types[1],
                        wire_types[2],
                        wire_types[3],
                        wire_types[4],
                        points,
                        transformed
                    ),
                ),
                ro(
                    "View Silhouettes",
                    "acis_silhouettes",
                    format!(
                        "{} views; {} wires",
                        silhouettes.len(),
                        silhouettes.iter().map(|silhouette| silhouette.wires.len()).sum::<usize>()
                    ),
                ),
            ],
        },
    ]
}

fn position_section(prefix: &str, p: &acadrust::types::Vector3) -> PropSection {
    let fields = match prefix {
        "rgn" => ["rgn_px", "rgn_py", "rgn_pz"],
        "bdy" => ["bdy_px", "bdy_py", "bdy_pz"],
        "srf" => ["srf_px", "srf_py", "srf_pz"],
        _ => ["s3d_px", "s3d_py", "s3d_pz"],
    };
    PropSection {
        title: "Geometry".into(),
        props: vec![
            edit("Position X", fields[0], p.x),
            edit("Position Y", fields[1], p.y),
            edit("Position Z", fields[2], p.z),
        ],
    }
}

/// Approximate a region's enclosed area and boundary perimeter from its
/// wireframe loops. Perimeter is the total edge length across every wire.
/// Area accumulates the Newell area vector of each loop (opposite-wound
/// holes subtract) and halves its magnitude — exact for a single planar
/// loop, approximate for multi-loop or curved regions. Returns zeros when
/// there is nothing to measure.
fn region_area_perimeter(wires: &[acadrust::entities::Wire]) -> (f64, f64) {
    let mut perimeter = 0.0;
    let (mut nx, mut ny, mut nz) = (0.0, 0.0, 0.0);
    for wire in wires {
        let pts = &wire.points;
        if pts.len() < 2 {
            continue;
        }
        for seg in pts.windows(2) {
            let dx = seg[1].x - seg[0].x;
            let dy = seg[1].y - seg[0].y;
            let dz = seg[1].z - seg[0].z;
            perimeter += (dx * dx + dy * dy + dz * dz).sqrt();
        }
        let n = pts.len();
        for i in 0..n {
            let a = &pts[i];
            let b = &pts[(i + 1) % n];
            nx += (a.y - b.y) * (a.z + b.z);
            ny += (a.z - b.z) * (a.x + b.x);
            nz += (a.x - b.x) * (a.y + b.y);
        }
    }
    let area = 0.5 * (nx * nx + ny * ny + nz * nz).sqrt();
    (area, perimeter)
}

// ── Solid3D ───────────────────────────────────────────────────────────────────

impl Grippable for Solid3D {
    fn grips(&self) -> Vec<GripDef> {
        vec![center_grip(0, dvec3(&self.point_of_reference))]
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        if grip_id != 0 {
            return;
        }
        if let GripApply::Translate(d) = apply {
            translate_acis_entity(self, d);
        }
    }
}

impl PropertyEditable for Solid3D {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let mut sections =
            acis_sections(&self.acis_data, &self.wires, &self.silhouettes, self.history_handle);
        sections[0]
            .props
            .insert(0, ro("UID", "s3d_uid", self.uid.clone()));
        sections.push(position_section("s3d", &self.point_of_reference));
        sections
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        let Some(v) = parse_f64(value) else {
            return;
        };
        let delta = match field {
            "s3d_px" => acadrust::types::Vector3::new(v - self.point_of_reference.x, 0.0, 0.0),
            "s3d_py" => acadrust::types::Vector3::new(0.0, v - self.point_of_reference.y, 0.0),
            "s3d_pz" => acadrust::types::Vector3::new(0.0, 0.0, v - self.point_of_reference.z),
            _ => return,
        };
        if delta != acadrust::types::Vector3::ZERO {
            acadrust::Entity::translate(self, delta);
        }
    }
}

// ── Region ────────────────────────────────────────────────────────────────────

impl Grippable for Region {
    fn grips(&self) -> Vec<GripDef> {
        vec![center_grip(0, dvec3(&self.point_of_reference))]
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        if grip_id != 0 {
            return;
        }
        if let GripApply::Translate(d) = apply {
            translate_acis_entity(self, d);
        }
    }
}

impl PropertyEditable for Region {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let (area, perimeter) = region_area_perimeter(&self.wires);
        let mut sections =
            acis_sections(&self.acis_data, &self.wires, &self.silhouettes, self.history_handle);
        sections[0]
            .props
            .insert(0, ro("UID", "rgn_uid", self.uid.clone()));
        let mut geometry = position_section("rgn", &self.point_of_reference);
        geometry
            .props
            .push(ro("Area", "rgn_area", format!("{area:.4}")));
        geometry
            .props
            .push(ro("Perimeter", "rgn_perimeter", format!("{perimeter:.4}")));
        sections.push(geometry);
        sections
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        let Some(v) = parse_f64(value) else {
            return;
        };
        let delta = match field {
            "rgn_px" => acadrust::types::Vector3::new(v - self.point_of_reference.x, 0.0, 0.0),
            "rgn_py" => acadrust::types::Vector3::new(0.0, v - self.point_of_reference.y, 0.0),
            "rgn_pz" => acadrust::types::Vector3::new(0.0, 0.0, v - self.point_of_reference.z),
            _ => return,
        };
        if delta != acadrust::types::Vector3::ZERO {
            acadrust::Entity::translate(self, delta);
        }
    }
}

// ── Body ──────────────────────────────────────────────────────────────────────

impl Grippable for Body {
    fn grips(&self) -> Vec<GripDef> {
        vec![center_grip(0, dvec3(&self.point_of_reference))]
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        if grip_id != 0 {
            return;
        }
        if let GripApply::Translate(d) = apply {
            translate_acis_entity(self, d);
        }
    }
}

impl PropertyEditable for Body {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let mut sections =
            acis_sections(&self.acis_data, &self.wires, &self.silhouettes, self.history_handle);
        sections[0]
            .props
            .insert(0, ro("UID", "bdy_uid", self.uid.clone()));
        sections.push(position_section("bdy", &self.point_of_reference));
        sections
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        let Some(v) = parse_f64(value) else {
            return;
        };
        let delta = match field {
            "bdy_px" => acadrust::types::Vector3::new(v - self.point_of_reference.x, 0.0, 0.0),
            "bdy_py" => acadrust::types::Vector3::new(0.0, v - self.point_of_reference.y, 0.0),
            "bdy_pz" => acadrust::types::Vector3::new(0.0, 0.0, v - self.point_of_reference.z),
            _ => return,
        };
        if delta != acadrust::types::Vector3::ZERO {
            acadrust::Entity::translate(self, delta);
        }
    }
}

fn embedded_name(entity: Option<&acadrust::entities::EmbeddedEntity>) -> &'static str {
    match entity {
        Some(acadrust::entities::EmbeddedEntity::Point(_)) => "Point",
        Some(acadrust::entities::EmbeddedEntity::Line(_)) => "Line",
        Some(acadrust::entities::EmbeddedEntity::Arc(_)) => "Arc",
        Some(acadrust::entities::EmbeddedEntity::Circle(_)) => "Circle",
        Some(acadrust::entities::EmbeddedEntity::Ellipse(_)) => "Ellipse",
        Some(acadrust::entities::EmbeddedEntity::Spline(_)) => "Spline",
        Some(acadrust::entities::EmbeddedEntity::LwPolyline(_)) => "Polyline",
        Some(acadrust::entities::EmbeddedEntity::Ray(_)) => "Ray",
        Some(acadrust::entities::EmbeddedEntity::XLine(_)) => "XLine",
        Some(acadrust::entities::EmbeddedEntity::Unknown { .. }) => "Unknown",
        None => "None",
    }
}

fn vector_text(vector: &acadrust::types::Vector3) -> String {
    format!("{:.6}, {:.6}, {:.6}", vector.x, vector.y, vector.z)
}

fn matrix_text(matrix: &[f64; 16]) -> String {
    matrix
        .chunks_exact(4)
        .map(|row| {
            format!(
                "[{:.4}, {:.4}, {:.4}, {:.4}]",
                row[0], row[1], row[2], row[3]
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sweep_options_text(options: &acadrust::entities::SurfaceSweepOptions) -> String {
    format!(
        "draft {:.6} ({:.6}→{:.6}); twist {:.6}; scale {:.6}; align {:.6}; solid {}; flags {}/{}; align-start {}; bank {}; base {}; sweep-xform {}; path-xform {}; ref {}",
        options.draft_angle,
        options.draft_start_distance,
        options.draft_end_distance,
        options.twist_angle,
        options.scale_factor,
        options.align_angle,
        yes_no(options.is_solid),
        options.sweep_alignment_flags,
        options.path_flags,
        yes_no(options.align_start),
        yes_no(options.bank),
        yes_no(options.base_point_set),
        yes_no(options.sweep_entity_transform_computed),
        yes_no(options.path_entity_transform_computed),
        vector_text(&options.reference_vector)
    )
}

fn surface_construction_section(surface: &Surface) -> PropSection {
    use acadrust::entities::SurfaceData;
    let mut props = vec![
        ro("Kind", "srf_kind", format!("{:?}", surface.kind)),
        ro(
            "Modeler Format",
            "srf_modeler_version",
            surface.modeler_format_version.to_string(),
        ),
        edit(
            "U Isolines",
            "srf_u_isolines",
            surface.u_isolines as f64,
        ),
        edit(
            "V Isolines",
            "srf_v_isolines",
            surface.v_isolines as f64,
        ),
    ];
    match &surface.surface_data {
        SurfaceData::Generic => {}
        SurfaceData::Plane { class_version } => {
            props.push(ro(
                "Class Version",
                "srf_class_version",
                class_version.to_string(),
            ));
        }
        SurfaceData::Extruded {
            sweep_entity,
            options,
            sweep_vector,
            sweep_transform,
        } => {
            props.extend([
                ro(
                    "Sweep Entity",
                    "srf_sweep_entity",
                    embedded_name(sweep_entity.as_ref()),
                ),
                ro("Sweep Vector", "srf_sweep_vector", vector_text(sweep_vector)),
                ro("Sweep Options", "srf_sweep_options", sweep_options_text(options)),
                ro(
                    "Sweep Transform",
                    "srf_sweep_transform",
                    matrix_text(sweep_transform),
                ),
                ro(
                    "Sweep Entity Transform",
                    "srf_sweep_entity_transform",
                    matrix_text(&options.sweep_entity_transform),
                ),
                ro(
                    "Path Entity Transform",
                    "srf_path_entity_transform",
                    matrix_text(&options.path_entity_transform),
                ),
            ]);
        }
        SurfaceData::Lofted {
            loft_transform,
            cross_section_entities,
            guide_entities,
            path_entity,
            plane_normal_lofting_type,
            start_draft_angle,
            end_draft_angle,
            start_draft_magnitude,
            end_draft_magnitude,
            arc_length_parameterization,
            no_twist,
            align_direction,
            simple_surfaces,
            closed_surfaces,
            solid,
            ruled_surface,
            virtual_guide,
            cross_sections,
            guide_curves,
            path_curve,
        } => {
            props.extend([
                ro(
                    "Embedded Curves",
                    "srf_loft_embedded",
                    format!(
                        "{} cross sections; {} guides; path {}",
                        cross_section_entities.len(),
                        guide_entities.len(),
                        embedded_name(path_entity.as_ref())
                    ),
                ),
                ro(
                    "Database Curves",
                    "srf_loft_handles",
                    format!(
                        "{} cross sections; {} guides; path {}",
                        cross_sections.len(),
                        guide_curves.len(),
                        handle_text(*path_curve)
                    ),
                ),
                ro(
                    "Draft",
                    "srf_loft_draft",
                    format!(
                        "type {}; angle {:.6}→{:.6}; magnitude {:.6}→{:.6}",
                        plane_normal_lofting_type,
                        start_draft_angle,
                        end_draft_angle,
                        start_draft_magnitude,
                        end_draft_magnitude
                    ),
                ),
                ro(
                    "Loft Flags",
                    "srf_loft_flags",
                    format!(
                        "arc-length {}; no-twist {}; align {}; simple {}; closed {}; solid {}; ruled {}; virtual-guide {}",
                        yes_no(*arc_length_parameterization),
                        yes_no(*no_twist),
                        yes_no(*align_direction),
                        yes_no(*simple_surfaces),
                        yes_no(*closed_surfaces),
                        yes_no(*solid),
                        yes_no(*ruled_surface),
                        yes_no(*virtual_guide)
                    ),
                ),
                ro(
                    "Loft Transform",
                    "srf_loft_transform",
                    matrix_text(loft_transform),
                ),
            ]);
        }
        SurfaceData::Revolved {
            revolve_entity,
            class_version,
            entity_id,
            axis_point,
            axis_vector,
            revolve_angle,
            start_angle,
            entity_transform,
            draft_angle,
            draft_start_distance,
            draft_end_distance,
            twist_angle,
            solid,
            close_to_axis,
        } => {
            props.extend([
                ro(
                    "Revolve Entity",
                    "srf_revolve_entity",
                    embedded_name(revolve_entity.as_ref()),
                ),
                ro(
                    "Class / Entity",
                    "srf_revolve_ids",
                    format!("{class_version} / {entity_id}"),
                ),
                ro("Axis Point", "srf_axis_point", vector_text(axis_point)),
                ro("Axis Vector", "srf_axis_vector", vector_text(axis_vector)),
                ro(
                    "Angles",
                    "srf_revolve_angles",
                    format!(
                        "start {:.6}; revolve {:.6}; draft {:.6}; twist {:.6}",
                        start_angle, revolve_angle, draft_angle, twist_angle
                    ),
                ),
                ro(
                    "Draft Distances",
                    "srf_revolve_draft_distances",
                    format!("{draft_start_distance:.6}→{draft_end_distance:.6}"),
                ),
                ro(
                    "Revolve Flags",
                    "srf_revolve_flags",
                    format!(
                        "solid {}; close-to-axis {}",
                        yes_no(*solid),
                        yes_no(*close_to_axis)
                    ),
                ),
                ro(
                    "Entity Transform",
                    "srf_revolve_transform",
                    matrix_text(entity_transform),
                ),
            ]);
        }
        SurfaceData::Swept {
            class_version,
            sweep_entity,
            path_entity,
            sweep_transform,
            path_transform,
            options,
        } => {
            props.extend([
                ro(
                    "Class Version",
                    "srf_class_version",
                    class_version.to_string(),
                ),
                ro(
                    "Sweep / Path",
                    "srf_swept_entities",
                    format!(
                        "{} / {}",
                        embedded_name(sweep_entity.as_ref()),
                        embedded_name(path_entity.as_ref())
                    ),
                ),
                ro("Sweep Options", "srf_sweep_options", sweep_options_text(options)),
                ro(
                    "Sweep Transform",
                    "srf_sweep_transform",
                    matrix_text(sweep_transform),
                ),
                ro(
                    "Path Transform",
                    "srf_path_transform",
                    matrix_text(path_transform),
                ),
                ro(
                    "Sweep Entity Transform",
                    "srf_sweep_entity_transform",
                    matrix_text(&options.sweep_entity_transform),
                ),
                ro(
                    "Path Entity Transform",
                    "srf_path_entity_transform",
                    matrix_text(&options.path_entity_transform),
                ),
            ]);
        }
        SurfaceData::Nurb {
            short_170,
            cv_hull_display,
            u_vector1,
            v_vector1,
            u_vector2,
            v_vector2,
        } => {
            props.extend([
                ro(
                    "NURB Version",
                    "srf_nurb_version",
                    short_170.to_string(),
                ),
                ro(
                    "CV Hull",
                    "srf_nurb_cv_hull",
                    yes_no(*cv_hull_display),
                ),
                ro("U Vector 1", "srf_nurb_u1", vector_text(u_vector1)),
                ro("V Vector 1", "srf_nurb_v1", vector_text(v_vector1)),
                ro("U Vector 2", "srf_nurb_u2", vector_text(u_vector2)),
                ro("V Vector 2", "srf_nurb_v2", vector_text(v_vector2)),
            ]);
        }
    }
    PropSection {
        title: "Surface Construction".into(),
        props,
    }
}

fn translate_surface(surface: &mut Surface, delta: acadrust::types::Vector3) {
    let before = surface.point_of_reference;
    acadrust::Entity::translate(surface, delta);
    if surface.point_of_reference == before {
        surface.point_of_reference = before + delta;
    }
}

impl Grippable for Surface {
    fn grips(&self) -> Vec<GripDef> {
        vec![center_grip(0, dvec3(&self.point_of_reference))]
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        if grip_id != 0 {
            return;
        }
        if let GripApply::Translate(delta) = apply {
            translate_surface(
                self,
                acadrust::types::Vector3::new(delta.x, delta.y, delta.z),
            );
        }
    }
}

impl PropertyEditable for Surface {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let mut sections =
            acis_sections(&self.acis_data, &self.wires, &self.silhouettes, self.history_handle);
        sections.push(surface_construction_section(self));
        sections.push(position_section("srf", &self.point_of_reference));
        sections
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        match field {
            "srf_u_isolines" => {
                if let Some(value) = parse_f64(value) {
                    self.u_isolines = (value.round() as i16).max(0);
                }
            }
            "srf_v_isolines" => {
                if let Some(value) = parse_f64(value) {
                    self.v_isolines = (value.round() as i16).max(0);
                }
            }
            _ => {
                let Some(value) = parse_f64(value) else {
                    return;
                };
                let delta = match field {
                    "srf_px" => acadrust::types::Vector3::new(
                        value - self.point_of_reference.x,
                        0.0,
                        0.0,
                    ),
                    "srf_py" => acadrust::types::Vector3::new(
                        0.0,
                        value - self.point_of_reference.y,
                        0.0,
                    ),
                    "srf_pz" => acadrust::types::Vector3::new(
                        0.0,
                        0.0,
                        value - self.point_of_reference.z,
                    ),
                    _ => return,
                };
                if delta != acadrust::types::Vector3::ZERO {
                    translate_surface(self, delta);
                }
            }
        }
    }
}

// ── Accessors for the Solid3D / Region / Body trio ─────────────────────────
//
// These three entity types share a common subset of fields (ACIS data
// + point_of_reference + wires fallback). Code that needs to treat them
// uniformly (mesh tess dispatch, fallback wires, grip translate) used
// to repeat a three-arm `match entity` block at every callsite — the
// helpers below collapse those to a single call.

use crate::scene::model::mesh_model::MeshLodSet;
use crate::scene::convert::solid3d_tess;
use acadrust::{types::Vector3, EntityType};

/// `point_of_reference` of an ACIS-backed volume entity, if applicable.
pub fn point_of_reference(e: &EntityType) -> Option<&Vector3> {
    match e {
        EntityType::Solid3D(s) => Some(&s.point_of_reference),
        EntityType::Region(r) => Some(&r.point_of_reference),
        EntityType::Body(b) => Some(&b.point_of_reference),
        EntityType::Surface(s) => Some(&s.point_of_reference),
        _ => None,
    }
}

/// Pre-stored edge-wire fallback list (used when the SAT/SAB kernel
/// can't produce a mesh — drawings authored by SOLVIEW / 3DPLOT carry
/// these explicitly).
pub fn fallback_wires(e: &EntityType) -> Option<&[acadrust::entities::Wire]> {
    match e {
        EntityType::Solid3D(s) => Some(&s.wires),
        EntityType::Region(r) => Some(&r.wires),
        EntityType::Body(b) => Some(&b.wires),
        EntityType::Surface(s) => Some(&s.wires),
        _ => None,
    }
}

pub fn wire_point(
    wire: &acadrust::entities::Wire,
    point: &acadrust::types::Vector3,
) -> acadrust::types::Vector3 {
    if !wire.has_transform {
        return *point;
    }
    let x = point.x * wire.scale.x;
    let y = point.y * wire.scale.y;
    let z = point.z * wire.scale.z;
    acadrust::types::Vector3::new(
        wire.translation.x
            + wire.x_axis.x * x
            + wire.y_axis.x * y
            + wire.z_axis.x * z,
        wire.translation.y
            + wire.x_axis.y * x
            + wire.y_axis.y * y
            + wire.z_axis.y * z,
        wire.translation.z
            + wire.x_axis.z * x
            + wire.y_axis.z * y
            + wire.z_axis.z * z,
    )
}

/// Whether every ACIS face uses a surface family the mesh pipeline can decode.
/// Unsupported or unresolved faces must keep their display-cache wires visible;
/// otherwise a parseable but incomplete shell looks like a valid solid.
pub fn acis_has_complete_surface_support(e: &EntityType) -> bool {
    let sat = match e {
        EntityType::Solid3D(s) => s.acis_data.parse(),
        EntityType::Region(r) => r.acis_data.parse(),
        EntityType::Body(b) => b.acis_data.parse(),
        EntityType::Surface(s) => s.acis_data.parse(),
        _ => None,
    };
    let Some(sat) = sat else {
        return false;
    };
    let faces = sat.faces();
    !faces.is_empty()
        && faces.iter().all(|face| {
            sat.resolve(face.surface()).is_some_and(|surface| {
                matches!(
                    surface.entity_type.as_str(),
                    "plane-surface"
                        | "cone-surface"
                        | "sphere-surface"
                        | "torus-surface"
                        | "spline-surface"
                        | "meshsurf-surface"
                        | "bs3-surface"
                )
            })
        })
}

/// Build material-aware shaded geometry for every standard 3-D solid/surface
/// and mesh family, returning `None` when decoded geometry is unusable.
pub fn tessellate_volume(
    e: &EntityType,
    color: [f32; 4],
    facet_res: f64,
    isolines: usize,
) -> Option<MeshLodSet> {
    match e {
        EntityType::Solid3D(s) => solid3d_tess::tessellate_solid3d(s, color, facet_res, isolines),
        EntityType::Region(r) => solid3d_tess::tessellate_region(r, color, facet_res, isolines),
        EntityType::Body(b) => solid3d_tess::tessellate_body(b, color, facet_res, isolines),
        EntityType::Surface(s) => solid3d_tess::tessellate_surface(s, color, facet_res, isolines),
        EntityType::Mesh(_) | EntityType::PolygonMesh(_) | EntityType::PolyfaceMesh(_) => {
            crate::entities::mesh::tessellate_shaded_mesh(e, color)
        }
        _ => None,
    }
}
