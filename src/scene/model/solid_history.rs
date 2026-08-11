use acadrust::entities::{EmbeddedEntity, Solid3D};
use acadrust::objects::{
    SolidHistoryBox, SolidHistoryBrep, SolidHistoryCylinder, SolidHistoryLoft,
    SolidHistoryNodeBase, SolidHistoryOperation, SolidHistoryPyramid,
    SolidHistoryRevolve, SolidHistorySphere, SolidHistorySweep, SolidHistoryTorus,
};
use acadrust::types::{Vector2, Vector3};
use acadrust::EntityType;
use cadkernel::brep::Body;

fn base(transform: [f64; 16]) -> SolidHistoryNodeBase {
    let mut base = SolidHistoryNodeBase::new(1);
    base.transform = transform;
    base
}

fn embedded(entity: &EntityType) -> Option<EmbeddedEntity> {
    Some(match entity {
        EntityType::Point(value) => EmbeddedEntity::Point(value.clone()),
        EntityType::Line(value) => EmbeddedEntity::Line(value.clone()),
        EntityType::Arc(value) => EmbeddedEntity::Arc(value.clone()),
        EntityType::Circle(value) => EmbeddedEntity::Circle(value.clone()),
        EntityType::Ellipse(value) => EmbeddedEntity::Ellipse(value.clone()),
        EntityType::Spline(value) => EmbeddedEntity::Spline(value.clone()),
        EntityType::LwPolyline(value) => EmbeddedEntity::LwPolyline(value.clone()),
        EntityType::Ray(value) => EmbeddedEntity::Ray(value.clone()),
        EntityType::XLine(value) => EmbeddedEntity::XLine(value.clone()),
        _ => return None,
    })
}

pub fn box_op(
    transform: [f64; 16],
    length: f64,
    width: f64,
    height: f64,
) -> SolidHistoryOperation {
    SolidHistoryOperation::Box(SolidHistoryBox {
        base: base(transform),
        operation_major: 1,
        length,
        width,
        height,
        ..SolidHistoryBox::default()
    })
}

pub fn wedge_op(
    transform: [f64; 16],
    length: f64,
    width: f64,
    height: f64,
) -> SolidHistoryOperation {
    SolidHistoryOperation::Wedge(SolidHistoryBox {
        base: base(transform),
        operation_major: 1,
        length,
        width,
        height,
        ..SolidHistoryBox::default()
    })
}

pub fn cylinder_op(
    transform: [f64; 16],
    radius: f64,
    height: f64,
) -> SolidHistoryOperation {
    SolidHistoryOperation::Cylinder(SolidHistoryCylinder {
        base: base(transform),
        operation_major: 1,
        height,
        major_radius: radius,
        minor_radius: radius,
        x_radius: radius,
        ..SolidHistoryCylinder::default()
    })
}

pub fn cone_op(
    transform: [f64; 16],
    radius: f64,
    height: f64,
) -> SolidHistoryOperation {
    SolidHistoryOperation::Cone(SolidHistoryCylinder {
        base: base(transform),
        operation_major: 1,
        height,
        major_radius: radius,
        minor_radius: radius,
        x_radius: radius,
        ..SolidHistoryCylinder::default()
    })
}

pub fn sphere_op(transform: [f64; 16], radius: f64) -> SolidHistoryOperation {
    SolidHistoryOperation::Sphere(SolidHistorySphere {
        base: base(transform),
        operation_major: 1,
        radius,
        ..SolidHistorySphere::default()
    })
}

pub fn torus_op(
    transform: [f64; 16],
    major_radius: f64,
    minor_radius: f64,
) -> SolidHistoryOperation {
    SolidHistoryOperation::Torus(SolidHistoryTorus {
        base: base(transform),
        operation_major: 1,
        major_radius,
        minor_radius,
        ..SolidHistoryTorus::default()
    })
}

pub fn pyramid_op(
    transform: [f64; 16],
    radius: f64,
    height: f64,
    sides: usize,
) -> SolidHistoryOperation {
    SolidHistoryOperation::Pyramid(SolidHistoryPyramid {
        base: base(transform),
        operation_major: 1,
        height,
        sides: sides as i32,
        radius,
        ..SolidHistoryPyramid::default()
    })
}

pub fn brep_op(body: &Body) -> SolidHistoryOperation {
    let acis_data = crate::scene::convert::acis_export::planar_solid_to_sat(body)
        .map(|document| {
            let mut solid = Solid3D::new();
            solid.set_sat_document(&document);
            solid.acis_data
        })
        .unwrap_or_default();
    SolidHistoryOperation::Brep(SolidHistoryBrep {
        base: base(glam::DMat4::IDENTITY.to_cols_array()),
        operation_major: 1,
        acis_data,
        ..SolidHistoryBrep::default()
    })
}

pub fn extrusion_op(profile: &EntityType, height: f64) -> SolidHistoryOperation {
    SolidHistoryOperation::Extrusion(SolidHistorySweep {
        base: base(glam::DMat4::IDENTITY.to_cols_array()),
        operation_major: 1,
        direction: Vector3::new(0.0, 0.0, height),
        sweep_entity: embedded(profile),
        scale_factor: 1.0,
        sweep_entity_transform: glam::DMat4::IDENTITY.to_cols_array(),
        path_entity_transform: glam::DMat4::IDENTITY.to_cols_array(),
        ..SolidHistorySweep::default()
    })
}

pub fn sweep_op(profile: &EntityType, path: &EntityType) -> SolidHistoryOperation {
    SolidHistoryOperation::Sweep(SolidHistorySweep {
        base: base(glam::DMat4::IDENTITY.to_cols_array()),
        operation_major: 1,
        sweep_entity: embedded(profile),
        path_entity: embedded(path),
        scale_factor: 1.0,
        sweep_entity_transform: glam::DMat4::IDENTITY.to_cols_array(),
        path_entity_transform: glam::DMat4::IDENTITY.to_cols_array(),
        ..SolidHistorySweep::default()
    })
}

pub fn loft_op(profiles: &[EntityType]) -> SolidHistoryOperation {
    SolidHistoryOperation::Loft(SolidHistoryLoft {
        base: base(glam::DMat4::IDENTITY.to_cols_array()),
        operation_major: 1,
        cross_sections: profiles.iter().filter_map(embedded).collect(),
        ..SolidHistoryLoft::default()
    })
}

pub fn revolve_op(
    profile: &EntityType,
    axis_start: [f64; 3],
    axis_end: [f64; 3],
    angle: f64,
) -> SolidHistoryOperation {
    let direction = glam::DVec3::from_array(axis_end) - glam::DVec3::from_array(axis_start);
    SolidHistoryOperation::Revolve(SolidHistoryRevolve {
        base: base(glam::DMat4::IDENTITY.to_cols_array()),
        operation_major: 1,
        axis_point: Vector3::new(axis_start[0], axis_start[1], axis_start[2]),
        direction: Vector2::new(direction.x, direction.y),
        revolve_angle: angle,
        sweep_entity: embedded(profile),
        ..SolidHistoryRevolve::default()
    })
}
