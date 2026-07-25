// Grippable + PropertyEditable for Solid3D, Region, Body.
//
// Geometry lives in ACIS data — we cannot edit it via the properties panel.
// We expose the point_of_reference as a translate grip and show ACIS size
// as read-only info.  Grip translate also updates wire points so the wire
// fallback stays in sync; the caller (scene/mod.rs apply_grip) translates
// the MeshModel vertices to match.

use acadrust::entities::{Body, Region, Solid3D, Surface};
use glam::Vec3;

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

fn translate_wires(wires: &mut Vec<acadrust::entities::Wire>, d: Vec3) {
    for wire in wires.iter_mut() {
        for pt in wire.points.iter_mut() {
            pt.x += d.x as f64;
            pt.y += d.y as f64;
            pt.z += d.z as f64;
        }
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
            self.point_of_reference.x += d.x as f64;
            self.point_of_reference.y += d.y as f64;
            self.point_of_reference.z += d.z as f64;
            translate_wires(&mut self.wires, d.as_vec3());
        }
    }
}

impl PropertyEditable for Solid3D {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let has_history = matches!(self.history_handle, Some(h) if !h.is_null());
        let history = if has_history {
            format!("{:X}", self.history_handle.unwrap().value())
        } else {
            "None".to_string()
        };
        vec![
            PropSection {
                title: "Solid History".into(),
                props: vec![
                    ro("History", "s3d_history", history),
                    ro(
                        "Show History",
                        "s3d_show_history",
                        if has_history { "Yes" } else { "No" },
                    ),
                ],
            },
            PropSection {
                title: "Geometry".into(),
                props: vec![
                    edit("Position X", "s3d_px", self.point_of_reference.x),
                    edit("Position Y", "s3d_py", self.point_of_reference.y),
                    edit("Position Z", "s3d_pz", self.point_of_reference.z),
                ],
            },
        ]
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        let Some(v) = parse_f64(value) else {
            return;
        };
        match field {
            "s3d_px" => self.point_of_reference.x = v,
            "s3d_py" => self.point_of_reference.y = v,
            "s3d_pz" => self.point_of_reference.z = v,
            _ => {}
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
            self.point_of_reference.x += d.x as f64;
            self.point_of_reference.y += d.y as f64;
            self.point_of_reference.z += d.z as f64;
            translate_wires(&mut self.wires, d.as_vec3());
        }
    }
}

impl PropertyEditable for Region {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let (area, perimeter) = region_area_perimeter(&self.wires);
        vec![PropSection {
            title: "Geometry".into(),
            props: vec![
                ro("Area", "rgn_area", format!("{area:.4}")),
                ro("Perimeter", "rgn_perimeter", format!("{perimeter:.4}")),
            ],
        }]
    }

    fn apply_geom_prop(&mut self, _field: &str, _value: &str) {}
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
            self.point_of_reference.x += d.x as f64;
            self.point_of_reference.y += d.y as f64;
            self.point_of_reference.z += d.z as f64;
            translate_wires(&mut self.wires, d.as_vec3());
        }
    }
}

impl PropertyEditable for Body {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let has_history = matches!(self.history_handle, Some(h) if !h.is_null());
        let history = if has_history {
            format!("{:X}", self.history_handle.unwrap().value())
        } else {
            "None".to_string()
        };
        vec![PropSection {
            title: "Solid History".into(),
            props: vec![
                ro("History", "bdy_history", history),
                ro(
                    "Show History",
                    "bdy_show_history",
                    if has_history { "Yes" } else { "No" },
                ),
            ],
        }]
    }

    fn apply_geom_prop(&mut self, _field: &str, _value: &str) {}
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
                )
            })
        })
}

/// Run the appropriate `solid3d_tess::tessellate_*` for the entity,
/// returning `None` for non-volume entities or when the kernel fails.
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
        _ => None,
    }
}
