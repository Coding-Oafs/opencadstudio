use acadrust::entities::Circle;
use crate::t;

use crate::command::EntityTransform;
use crate::entities::common::{
    center_grip, edit_prop as edit, parse_f64, ro_prop as ro, square_grip,
};
use crate::entities::traits::TruckConvertible;
use crate::scene::convert::acad_to_truck::{extrusion_wall_tris, TruckEntity, TruckObject};
use crate::scene::model::object::{GripApply, GripDef, PropSection};
use crate::scene::model::wire_model::{SnapHint, TangentGeom};

fn to_truck(circle: &Circle) -> TruckEntity {
    let cx = circle.center.x;
    let cy = circle.center.y;
    let cz = circle.center.z;
    let r = circle.radius;
    let normal = (circle.normal.x, circle.normal.y, circle.normal.z);

    let (ax, ay) = crate::scene::view::transform::ocs_axes(normal);
    let (cwx, cwy, cwz) = crate::scene::view::transform::ocs_point_to_wcs((cx, cy, cz), normal);

    let cv = glam::DVec3::new(cwx, cwy, cwz);
    let rf = r as f32;
    let q = |d: (f64, f64, f64)| {
        glam::DVec3::new(cwx + r * d.0, cwy + r * d.1, cwz + r * d.2)
    };
    let snap_pts = vec![
        (cv, SnapHint::Center),
        (q(ax), SnapHint::Quadrant),
        (q(ay), SnapHint::Quadrant),
        (q((-ax.0, -ax.1, -ax.2)), SnapHint::Quadrant),
        (q((-ay.0, -ay.1, -ay.2)), SnapHint::Quadrant),
    ];
    let tangent = TangentGeom::Circle {
        center: [cwx as f32, cwy as f32, cwz as f32],
        radius: rf,
    };

    if circle.thickness.abs() > 1e-10 {
        let t = circle.thickness;
        let (nx, ny, nz) = normal;
        let n = 64usize;
        let tau = std::f64::consts::TAU;
        let circ_pt = |a: f64| -> (f64, f64, f64) {
            let (c, s) = (a.cos(), a.sin());
            (
                cwx + r * (c * ax.0 + s * ay.0),
                cwy + r * (c * ax.1 + s * ay.1),
                cwz + r * (c * ax.2 + s * ay.2),
            )
        };
        let base: Vec<[f64; 3]> = (0..=n)
            .map(|i| {
                let (x, y, z) = circ_pt(i as f64 * tau / n as f64);
                [x, y, z]
            })
            .collect();
        let mut pts: Vec<[f64; 3]> = Vec::with_capacity((n + 1) * 2 + 4 * 3);
        pts.extend_from_slice(&base);
        pts.push([f64::NAN; 3]);
        for &[x, y, z] in &base {
            pts.push([x + t * nx, y + t * ny, z + t * nz]);
        }
        pts.push([f64::NAN; 3]);
        for i in 0..4usize {
            let (x, y, z) = circ_pt(i as f64 * std::f64::consts::FRAC_PI_2);
            pts.push([x, y, z]);
            pts.push([x + t * nx, y + t * ny, z + t * nz]);
            if i < 3 {
                pts.push([f64::NAN; 3]);
            }
        }
        return TruckEntity {
            pick_tris: extrusion_wall_tris(&base, [t * nx, t * ny, t * nz]),
            object: TruckObject::Lines(pts),
            snap_pts,
            tangent_geoms: vec![tangent],
            key_vertices: vec![],
            fill_tris: vec![],
        };
    }

    // Tessellate directly as a cos/sin polyline rather than a truck
    // `circle_arc` (arc-through-three-points). At large WCS coordinates
    // (e.g. −1.2M UTM) the three-point fit cancels catastrophically — the
    // circle comes back with a ~3% radius wobble and uneven segment lengths,
    // which then throws off a dashed linetype's dash spacing. Direct
    // evaluation only adds a small ±r term to the centre, so it stays precise;
    // the `Lines` path RTE-splits the absolute-f64 points into the
    // double-single the shader reconstructs.
    let tol = crate::scene::convert::truck_tess::current_curve_tol();
    // Chord-height tolerance → segment count: sag = r·(1 − cos(π/N)).
    let n = if r > tol {
        (std::f64::consts::PI / (1.0 - tol / r).clamp(-1.0, 1.0).acos())
            .ceil()
            .clamp(16.0, 4096.0) as usize
    } else {
        16
    };
    let tau = std::f64::consts::TAU;
    let mut pts: Vec<[f64; 3]> = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let a = i as f64 * tau / n as f64;
        let (s, c) = a.sin_cos();
        pts.push([
            cwx + r * (c * ax.0 + s * ay.0),
            cwy + r * (c * ax.1 + s * ay.1),
            cwz + r * (c * ax.2 + s * ay.2),
        ]);
    }
    TruckEntity {
        pick_tris: Vec::new(),
        object: TruckObject::Lines(pts),
        snap_pts,
        tangent_geoms: vec![tangent],
        key_vertices: vec![],
        fill_tris: vec![],
    }
}

fn grips(circle: &Circle) -> Vec<GripDef> {
    let ctr = glam::DVec3::new(circle.center.x, circle.center.y, circle.center.z);
    let r = circle.radius;
    vec![
        center_grip(0, ctr),
        square_grip(1, ctr + glam::DVec3::new(r, 0.0, 0.0)),
        square_grip(2, ctr + glam::DVec3::new(0.0, r, 0.0)),
        square_grip(3, ctr - glam::DVec3::new(r, 0.0, 0.0)),
        square_grip(4, ctr - glam::DVec3::new(0.0, r, 0.0)),
    ]
}

fn properties(circle: &Circle) -> Vec<PropSection> {
    use std::f64::consts::PI;
    let r = circle.radius;
    vec![PropSection {
        title: t!("Geometry").into_owned(),
        props: vec![
            edit(t!("Center X").as_ref(), "center_x", circle.center.x),
            edit(t!("Center Y").as_ref(), "center_y", circle.center.y),
            edit(t!("Center Z").as_ref(), "center_z", circle.center.z),
            edit(t!("Radius").as_ref(), "radius", r),
            edit(t!("Diameter").as_ref(), "diameter", r * 2.0),
            edit(t!("Circumference").as_ref(), "circumference", 2.0 * PI * r),
            edit(t!("Area").as_ref(), "area", PI * r * r),
            ro(t!("Normal X").as_ref(), "normal_x", format!("{:.4}", circle.normal.x)),
            ro(t!("Normal Y").as_ref(), "normal_y", format!("{:.4}", circle.normal.y)),
            ro(t!("Normal Z").as_ref(), "normal_z", format!("{:.4}", circle.normal.z)),
        ],
    }]
}

fn apply_geom_prop(circle: &mut Circle, field: &str, value: &str) {
    use std::f64::consts::PI;
    let Some(v) = parse_f64(value) else {
        return;
    };
    match field {
        "center_x" => circle.center.x = v,
        "center_y" => circle.center.y = v,
        "center_z" => circle.center.z = v,
        "radius" if v > 0.0 => circle.radius = v,
        "diameter" if v > 0.0 => circle.radius = v / 2.0,
        "circumference" if v > 0.0 => circle.radius = v / (2.0 * PI),
        "area" if v > 0.0 => circle.radius = (v / PI).sqrt(),
        _ => {}
    }
}

fn apply_grip(circle: &mut Circle, grip_id: usize, apply: GripApply) {
    match (grip_id, apply) {
        (0, GripApply::Absolute(p)) => {
            circle.center.x = p.x as f64;
            circle.center.y = p.y as f64;
            circle.center.z = p.z as f64;
        }
        (0, GripApply::Translate(d)) => {
            circle.center.x += d.x as f64;
            circle.center.y += d.y as f64;
            circle.center.z += d.z as f64;
        }
        (1..=4, GripApply::Absolute(p)) => {
            let dx = p.x - circle.center.x;
            let dy = p.y - circle.center.y;
            circle.radius = (dx * dx + dy * dy).sqrt();
        }
        _ => {}
    }
}

fn apply_transform(circle: &mut Circle, t: &EntityTransform) {
    crate::scene::view::transform::apply_standard_entity_transform(circle, t, |entity, p1, p2| {
        crate::scene::view::transform::reflect_xy_point(
            &mut entity.center.x,
            &mut entity.center.y,
            p1,
            p2,
        );
    });
}

impl TruckConvertible for Circle {
    fn to_truck(&self, _document: &acadrust::CadDocument) -> Option<TruckEntity> {
        Some(to_truck(self))
    }
}

crate::impl_entity_basics!(Circle);

impl crate::entities::traits::MassPropsCalc for Circle {
    fn mass_props(&self) -> crate::entities::traits::MassProps {
        use std::f64::consts::{PI, TAU};
        let r = self.radius;
        crate::entities::traits::MassProps {
            area: PI * r * r,
            perimeter: TAU * r,
            cx: self.center.x,
            cy: self.center.y,
        }
    }
}
