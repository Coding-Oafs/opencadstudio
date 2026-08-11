use acadrust::entities::{EmbeddedEntity, Solid3D};
use acadrust::objects::{
    SolidHistoryBox, SolidHistoryBrep, SolidHistoryCylinder, SolidHistoryLoft,
    SolidHistoryNodeBase, SolidHistoryOperation, SolidHistoryPyramid,
    SolidHistoryRevolve, SolidHistorySphere, SolidHistorySweep, SolidHistoryTorus,
};
use acadrust::types::{Vector2, Vector3};
use acadrust::EntityType;
use cadkernel::brep::Body;

use crate::scene::model::object::{GripApply, GripDef, GripShape};
use crate::command::EntityTransform;

pub const GRIP_LENGTH: usize = 10_001;
pub const GRIP_WIDTH: usize = 10_002;
pub const GRIP_HEIGHT: usize = 10_003;
pub const GRIP_RADIUS: usize = 10_004;
pub const GRIP_MAJOR_RADIUS: usize = 10_005;
pub const GRIP_MINOR_RADIUS: usize = 10_006;
pub const GRIP_SIDES: usize = 10_007;

fn matrix(transform: [f64; 16]) -> Option<glam::DMat4> {
    let matrix = glam::DMat4::from_cols_array(&transform);
    (matrix.is_finite() && matrix.determinant().abs() > 1e-12).then_some(matrix)
}

fn codec_matrix(transform: &acadrust::types::Transform) -> glam::DMat4 {
    let matrix = transform.matrix.m;
    glam::DMat4::from_cols_array(&[
        matrix[0][0], matrix[1][0], matrix[2][0], matrix[3][0],
        matrix[0][1], matrix[1][1], matrix[2][1], matrix[3][1],
        matrix[0][2], matrix[1][2], matrix[2][2], matrix[3][2],
        matrix[0][3], matrix[1][3], matrix[2][3], matrix[3][3],
    ])
}

fn transform_matrix(transform: &EntityTransform) -> Option<glam::DMat4> {
    Some(match transform {
        EntityTransform::Translate(delta) => glam::DMat4::from_translation(*delta),
        EntityTransform::Rotate {
            center,
            axis,
            angle_rad,
        } => {
            let axis = axis.normalize_or_zero();
            if axis.length_squared() <= 1e-12 {
                return None;
            }
            glam::DMat4::from_translation(*center)
                * glam::DMat4::from_axis_angle(axis, *angle_rad)
                * glam::DMat4::from_translation(-*center)
        }
        EntityTransform::Scale { center, factor } => {
            glam::DMat4::from_translation(*center)
                * glam::DMat4::from_scale(glam::DVec3::splat(*factor))
                * glam::DMat4::from_translation(-*center)
        }
        EntityTransform::Mirror {
            p1,
            p2,
            working_normal,
        } => codec_matrix(&crate::scene::view::transform::reflection_about_working_line(
            *p1,
            *p2,
            *working_normal,
        )),
        EntityTransform::Affine(value) => codec_matrix(value),
    })
}

pub fn transform_operation(
    operation: &mut SolidHistoryOperation,
    transform: &EntityTransform,
) -> bool {
    let Some(base) = operation.base_mut() else {
        return false;
    };
    let Some(current) = matrix(base.transform) else {
        return false;
    };
    let Some(by) = transform_matrix(transform) else {
        return false;
    };
    let transformed = by * current;
    if !transformed.is_finite() || transformed.determinant().abs() <= 1e-12 {
        return false;
    }
    base.transform = transformed.to_cols_array();
    true
}

fn world_point(transform: [f64; 16], point: [f64; 3]) -> Option<glam::DVec3> {
    Some(matrix(transform)?.transform_point3(glam::DVec3::from_array(point)))
}

fn local_point(transform: [f64; 16], point: glam::DVec3) -> Option<glam::DVec3> {
    Some(matrix(transform)?.inverse().transform_point3(point))
}

fn grip(id: usize, world: glam::DVec3, shape: GripShape) -> GripDef {
    GripDef {
        id,
        world,
        is_midpoint: false,
        shape,
        dir: None,
    }
}

pub fn primitive_grips(
    document: &acadrust::CadDocument,
    handle: acadrust::Handle,
) -> Vec<GripDef> {
    let Some(operation) = document.solid_history_operation(handle) else {
        return Vec::new();
    };
    let mut grips = Vec::new();
    let mut add = |id, transform, point, shape| {
        if let Some(world) = world_point(transform, point) {
            grips.push(grip(id, world, shape));
        }
    };
    match operation {
        SolidHistoryOperation::Box(value) | SolidHistoryOperation::Wedge(value) => {
            add(
                GRIP_LENGTH,
                value.base.transform,
                [value.length, value.width * 0.5, 0.0],
                GripShape::Square,
            );
            add(
                GRIP_WIDTH,
                value.base.transform,
                [value.length * 0.5, value.width, 0.0],
                GripShape::Square,
            );
            add(
                GRIP_HEIGHT,
                value.base.transform,
                [value.length * 0.5, value.width * 0.5, value.height],
                GripShape::Square,
            );
        }
        SolidHistoryOperation::Cylinder(value) | SolidHistoryOperation::Cone(value) => {
            add(
                GRIP_RADIUS,
                value.base.transform,
                [value.major_radius, 0.0, value.height * 0.5],
                GripShape::Square,
            );
            add(
                GRIP_HEIGHT,
                value.base.transform,
                [0.0, 0.0, value.height],
                GripShape::Square,
            );
        }
        SolidHistoryOperation::Sphere(value) => add(
            GRIP_RADIUS,
            value.base.transform,
            [value.radius, 0.0, 0.0],
            GripShape::Square,
        ),
        SolidHistoryOperation::Torus(value) => {
            add(
                GRIP_MAJOR_RADIUS,
                value.base.transform,
                [value.major_radius, 0.0, 0.0],
                GripShape::Square,
            );
            add(
                GRIP_MINOR_RADIUS,
                value.base.transform,
                [value.major_radius + value.minor_radius, 0.0, 0.0],
                GripShape::Square,
            );
        }
        SolidHistoryOperation::Pyramid(value) => {
            add(
                GRIP_RADIUS,
                value.base.transform,
                [value.radius, 0.0, 0.0],
                GripShape::Square,
            );
            add(
                GRIP_HEIGHT,
                value.base.transform,
                [0.0, 0.0, value.height],
                GripShape::Square,
            );
            let angle = (value.sides.clamp(3, 71) as f64 * 5.0).to_radians();
            add(
                GRIP_SIDES,
                value.base.transform,
                [value.radius * angle.cos(), value.radius * angle.sin(), 0.0],
                GripShape::Triangle,
            );
        }
        _ => {}
    }
    grips
}

pub fn apply_primitive_grip(
    operation: &mut SolidHistoryOperation,
    grip_id: usize,
    apply: GripApply,
) -> bool {
    let GripApply::Absolute(world) = apply else {
        return false;
    };
    let Some(transform) = operation.base().map(|base| base.transform) else {
        return false;
    };
    let Some(local) = local_point(transform, world) else {
        return false;
    };
    let positive = |value: f64| value.abs().max(1e-6);
    match operation {
        SolidHistoryOperation::Box(value) | SolidHistoryOperation::Wedge(value) => {
            match grip_id {
                GRIP_LENGTH => value.length = positive(local.x),
                GRIP_WIDTH => value.width = positive(local.y),
                GRIP_HEIGHT => value.height = positive(local.z),
                _ => return false,
            }
        }
        SolidHistoryOperation::Cylinder(value) | SolidHistoryOperation::Cone(value) => {
            match grip_id {
                GRIP_RADIUS => {
                    let radius = local.x.hypot(local.y).max(1e-6);
                    value.major_radius = radius;
                    value.minor_radius = radius;
                    value.x_radius = radius;
                }
                GRIP_HEIGHT => value.height = positive(local.z),
                _ => return false,
            }
        }
        SolidHistoryOperation::Sphere(value) if grip_id == GRIP_RADIUS => {
            value.radius = local.length().max(1e-6);
        }
        SolidHistoryOperation::Torus(value) => match grip_id {
            GRIP_MAJOR_RADIUS => {
                value.major_radius = local.x.hypot(local.y).max(1e-6)
            }
            GRIP_MINOR_RADIUS => {
                value.minor_radius = (local.x.hypot(local.y) - value.major_radius)
                    .abs()
                    .max(1e-6)
            }
            _ => return false,
        },
        SolidHistoryOperation::Pyramid(value) => match grip_id {
            GRIP_RADIUS => value.radius = local.x.hypot(local.y).max(1e-6),
            GRIP_HEIGHT => value.height = positive(local.z),
            GRIP_SIDES => {
                let angle = local.y.atan2(local.x).rem_euclid(std::f64::consts::TAU);
                value.sides = (angle.to_degrees() / 5.0).round() as i32;
                value.sides = value.sides.clamp(3, 71);
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}

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
