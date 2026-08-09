// EXTRUDE / REVOLVE: turning a drawn profile into a solid.
//
// The profile comes from `entities::curve`, which is where every entity's
// geometry is already defined once, so a circle, an arc-bulged polyline and a
// closed spline all arrive as the same thing: a plane plus a chain of curves
// in that plane's coordinates. The kernel sweeps that chain into analytic
// surfaces — a straight run into a plane, an arc into a cylinder, a profile
// turned about an axis into a cone, a sphere or a torus — so the result can
// be written back out as ACIS rather than as facets.
//
// A spline profile has no analytic side wall, so it is refused rather than
// approximated into a surface nobody asked for. That is the kernel's answer
// and this module passes it on.

use acadrust::kernel::brep::{self, Body};
use acadrust::kernel::geom2d::Curve;
use acadrust::kernel::space::{Plane, PlanarCurve};
use acadrust::EntityType;

use crate::entities::curve::entity_curve;

/// A drawn profile, as the kernel wants it: the plane it lies in and the
/// chain of pieces closing a loop in that plane.
pub struct Profile {
    pub plane: Plane,
    pub pieces: Vec<Curve>,
}

/// The closed profile an entity describes, or `None` when it does not
/// describe one.
///
/// A single closed curve — a circle, an ellipse, a closed polyline — is what
/// a sweep needs. The kernel wants it as a chain of at least three pieces,
/// which a polyline already is and which a circle is not, so a round profile
/// is handed over as the arcs it is made of rather than as one curve with
/// nowhere for the sweep to start.
pub fn profile_of(entity: &EntityType) -> Option<Profile> {
    let planar: PlanarCurve = entity_curve(entity)?;
    if !planar.curve.is_closed() {
        return None;
    }
    let pieces = match &planar.curve {
        // A polyline is already a chain. Its own segments carry the bulges,
        // so an arc stays an arc rather than becoming a run of chords.
        Curve::Polyline(_) => planar.curve.segments(),
        // Everything else closed on itself is cut into quarters, which is
        // both the fewest pieces a chain may have and the fewest that leave
        // each one unambiguous about which way round it goes.
        Curve::Circle(circle) => quarters(circle.centre, circle.radius),
        other => split_evenly(other, 4),
    };
    (pieces.len() >= 3).then_some(Profile {
        plane: planar.plane,
        pieces,
    })
}

/// A circle as four arcs.
fn quarters(centre: [f64; 2], radius: f64) -> Vec<Curve> {
    use acadrust::kernel::geom2d::Arc;
    use std::f64::consts::FRAC_PI_2;
    (0..4)
        .map(|quarter| {
            let start = FRAC_PI_2 * quarter as f64;
            Curve::Arc(Arc {
                centre,
                radius,
                start_angle: start,
                end_angle: start + FRAC_PI_2,
            })
        })
        .collect()
}

/// Any other closed curve as `count` straight pieces between points on it.
///
/// The honest fallback: an ellipse or a spline has no analytic sweep, so the
/// kernel would refuse the exact form anyway. Chords at least say plainly
/// what they are.
fn split_evenly(curve: &Curve, count: usize) -> Vec<Curve> {
    use acadrust::kernel::geom2d::Line;
    (0..count)
        .map(|step| Curve::Line(Line {
            start: curve.point_at(step as f64 / count as f64),
            end: curve.point_at((step + 1) as f64 / count as f64),
        }))
        .collect()
}

/// EXTRUDE: drag the profile `height` along its own plane's normal.
///
/// `None` for a profile that does not close, encloses nothing, or holds a
/// piece with no analytic side wall.
pub fn extruded(entity: &EntityType, height: f64) -> Option<Body> {
    let profile = profile_of(entity)?;
    let normal = profile.plane.normal()?;
    brep::extrude(
        profile.plane,
        &profile.pieces,
        [normal[0] * height, normal[1] * height, normal[2] * height],
    )
}

/// REVOLVE: turn the profile about the axis from `from` to `to` by `angle`
/// radians.
///
/// The axis has to lie in the profile's plane — a profile and an axis that do
/// not share one sweep into surfaces with no analytic form, and the kernel
/// refuses rather than approximating them.
pub fn revolved(
    entity: &EntityType,
    from: [f64; 3],
    to: [f64; 3],
    angle: f64,
) -> Option<Body> {
    let profile = profile_of(entity)?;
    brep::revolve(
        profile.plane,
        &profile.pieces,
        from,
        [to[0] - from[0], to[1] - from[1], to[2] - from[2]],
        angle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::{Circle, LwPolyline, LwVertex};
    use acadrust::types::{Vector2, Vector3};

    fn square(size: f64) -> EntityType {
        let mut polyline = LwPolyline::new();
        for corner in [[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]] {
            polyline.add_vertex(LwVertex::new(Vector2::new(corner[0], corner[1])));
        }
        polyline.is_closed = true;
        EntityType::LwPolyline(polyline)
    }

    fn volume(body: &Body) -> f64 {
        crate::scene::model::solid_model::volume(body)
    }

    #[test]
    fn a_closed_polyline_extrudes_into_a_prism() {
        let solid = extruded(&square(10.0), 4.0).expect("a prism");
        assert!((volume(&solid) - 400.0).abs() < 1e-6, "{}", volume(&solid));
    }

    #[test]
    fn a_circle_extrudes_into_a_cylinder() {
        // Cut into quarter arcs on the way, so the wall is four cylinder
        // patches rather than a run of flat chords — the volume says which.
        let mut circle = Circle::new();
        circle.center = Vector3::new(0.0, 0.0, 0.0);
        circle.radius = 5.0;
        let solid = extruded(&EntityType::Circle(circle), 3.0).expect("a cylinder");
        let expected = std::f64::consts::PI * 25.0 * 3.0;
        let got = volume(&solid);
        assert!(got > 0.98 * expected, "{got} vs {expected}");
        assert!(got <= expected * 1.000_001, "{got} vs {expected}");
    }

    #[test]
    fn a_square_revolved_about_its_own_edge_is_a_tube() {
        // The axis runs up the square's left side, so what it sweeps is a
        // solid cylinder of radius ten and height ten.
        let solid = revolved(
            &square(10.0),
            [0.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            std::f64::consts::TAU,
        );
        // The profile is in the XY plane and the axis lies in it, so this is
        // a revolution about the y axis: radius ten, height ten.
        let solid = solid.expect("a cylinder");
        let expected = std::f64::consts::PI * 100.0 * 10.0;
        let got = volume(&solid);
        assert!(got > 0.98 * expected, "{got} vs {expected}");
    }

    #[test]
    fn an_open_profile_is_refused() {
        let mut polyline = LwPolyline::new();
        for corner in [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]] {
            polyline.add_vertex(LwVertex::new(Vector2::new(corner[0], corner[1])));
        }
        polyline.is_closed = false;
        assert!(extruded(&EntityType::LwPolyline(polyline), 4.0).is_none());
    }

    #[test]
    fn an_axis_off_the_profiles_plane_is_refused() {
        assert!(revolved(
            &square(10.0),
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 1.0],
            std::f64::consts::TAU
        )
        .is_none());
    }
}
