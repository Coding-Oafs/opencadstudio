use acadrust::entities::Spline;
use crate::t;
use cadkernel::space::NurbsCurve3;

use crate::command::EntityTransform;
use crate::entities::common::{
    dropdown_grip, edit_prop as edit, parse_f64, ro_prop as ro, round_grip, square_grip,
};
use crate::entities::traits::RenderConvertible;
use crate::scene::convert::acad_to_render::{RenderEntity, RenderObject};
use crate::scene::model::object::{GripApply, GripDef, PropSection, PropValue, Property};

fn to_render(spl: &Spline) -> RenderEntity {
    let n = spl.control_points.len();
    if n < 2 {
        // A fit-point spline (DWG scenario 2 / R2013+) stores only the points
        // the curve passes through — no control points or knots. Interpolate a
        // smooth Catmull-Rom curve through them and hand it back as a dense
        // polyline so it still draws.
        if spl.fit_points.len() >= 2 {
            // Drawn from the same interpolation TRIM and snap use, so that
            // what is on screen is the curve those commands cut. The two had
            // parted: this drew a C² cubic solved here (or a Catmull-Rom for a
            // closed spline), while the kernel interpolated its own — near
            // enough to look right and far enough apart that a trim landed
            // beside the line it was aimed at.
            let planar = crate::entities::curve::spline_curve(spl);
            let pts = match &planar {
                Some(curve) => crate::entities::curve::curve_points(curve),
                // A fit spline through points in space is not a planar curve,
                // so the kernel has nothing to say about it and the solve
                // here remains the only description of its shape.
                None if spl.flags.closed || spl.flags.periodic => {
                    catmull_rom_polyline(&spl.fit_points, true)
                }
                None => fit_spline_polyline(spl),
            };
            let snap = planar
                .as_ref()
                .map(crate::entities::curve::snap_from)
                .unwrap_or_default();
            // Fit points are on the curve, so they stay on offer as ends in
            // their own right — but never through `key_vertices`, whose
            // consecutive entries the snap engine joins and offers the middle
            // of. The chord between two fit points of a curvy spline does not
            // run along it.
            let mut snap_pts = snap.snap_pts;
            snap_pts.extend(spl.fit_points.iter().map(|p| {
                (
                    glam::DVec3::new(p.x, p.y, p.z),
                    crate::scene::model::wire_model::SnapHint::Endpoint,
                )
            }));
            let key_vertices = if planar.is_some() {
                Vec::new()
            } else {
                spl.fit_points.iter().map(|p| [p.x, p.y, p.z]).collect()
            };
            return RenderEntity {
                pick_tris: Vec::new(),
                object: RenderObject::Lines(pts),
                snap_pts,
                tangent_geoms: vec![],
                key_vertices,
                fill_tris: vec![],
            };
        }
        return RenderEntity {
            pick_tris: Vec::new(),
            object: RenderObject::Lines(Vec::new()),
            snap_pts: vec![],
            tangent_geoms: vec![],
            key_vertices: vec![],
            fill_tris: vec![],
        };
    }

    // Snap sources from the spline's own curve. A spline is not a chain of
    // straight segments, so nothing goes in `key_vertices`: consecutive
    // entries there are joined and their midpoints offered, and the chord
    // between two fit points of a curvy spline does not run along it.
    //
    // Fit points are on the curve and stay on offer, as ends in their own
    // right. Control points are not on the curve and are dropped — snapping
    // to one put the cursor in empty space beside the geometry.
    let curve_snap = crate::entities::curve::spline_curve(spl)
        .map(|curve| crate::entities::curve::snap_from(&curve));
    let (snap_pts, key_vertices) = match curve_snap {
        Some(snap) => {
            let mut points = snap.snap_pts;
            points.extend(spl.fit_points.iter().map(|p| {
                (
                    glam::DVec3::new(p.x, p.y, p.z),
                    crate::scene::model::wire_model::SnapHint::Endpoint,
                )
            }));
            (points, Vec::new())
        }
        // A spline through points in space is not a planar curve, so the
        // kernel has nothing to say about it. Its stored points remain the
        // only thing to offer.
        None => {
            let source = if spl.fit_points.is_empty() {
                &spl.control_points
            } else {
                &spl.fit_points
            };
            (
                Vec::new(),
                source.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
            )
        }
    };

    // Sampled through the spline's own curve — the same definition EXTRUDE
    // and REVOLVE read — and closed back onto its start when the flags say
    // it is periodic and the two ends do not already meet.
    let is_closed = spl.flags.closed || spl.flags.periodic;
    let mut points = crate::entities::curve::spline_curve(spl)
        .map(|planar| crate::entities::curve::curve_points(&planar))
        .unwrap_or_else(|| measurement_polyline(spl));
    if is_closed {
        if let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) {
            let gap = ((last[0] - first[0]).powi(2)
                + (last[1] - first[1]).powi(2)
                + (last[2] - first[2]).powi(2))
            .sqrt();
            if gap > 1e-6 {
                points.push(first);
            }
        }
    }
    let object = RenderObject::Lines(points);

    RenderEntity {
        pick_tris: Vec::new(),
        object,
        snap_pts,
        tangent_geoms: vec![],
        key_vertices,
        fill_tris: vec![],
    }
}

pub(crate) fn measurement_polyline(spl: &Spline) -> Vec<[f64; 3]> {
    let count = spl.control_points.len();
    if count < 2 {
        if spl.fit_points.len() < 2 {
            return spl.control_points.iter().map(|p| [p.x, p.y, p.z]).collect();
        }
        return if spl.flags.closed || spl.flags.periodic {
            catmull_rom_polyline(&spl.fit_points, true)
        } else {
            fit_spline_polyline(spl)
        };
    }

    let degree = spl.degree.max(0) as usize;
    if degree == 0 || degree >= count {
        return spl.control_points.iter().map(|p| [p.x, p.y, p.z]).collect();
    }
    // The kernel's space curve holds rational and polynomial curves alike.
    let controls: Vec<[f64; 3]> = spl
        .control_points
        .iter()
        .map(|point| [point.x, point.y, point.z])
        .collect();
    let weights = (spl.weights.len() == count).then(|| {
        spl.weights
            .iter()
            .map(|weight| if weight.abs() < 1e-12 { 1.0 } else { *weight })
            .collect()
    });
    match NurbsCurve3::new(degree, controls, spl.knots.clone(), weights) {
        Some(curve) => {
            curve.tessellate_angle(cadkernel::tessellation::DEFAULT_ANGLE)
        }
        None => spl.control_points.iter().map(|p| [p.x, p.y, p.z]).collect(),
    }
}

/// Sample a Catmull-Rom spline through `pts` into a dense polyline. The curve
/// passes through every input point; open ends use reflected phantom points so
/// they don't kink, closed curves wrap around.
fn catmull_rom_polyline(pts: &[acadrust::types::Vector3], closed: bool) -> Vec<[f64; 3]> {
    let n = pts.len();
    if n < 2 {
        return pts.iter().map(|p| [p.x, p.y, p.z]).collect();
    }
    let get = |i: isize| -> [f64; 3] {
        let j = if closed {
            let m = n as isize;
            (((i % m) + m) % m) as usize
        } else {
            i.clamp(0, n as isize - 1) as usize
        };
        [pts[j].x, pts[j].y, pts[j].z]
    };
    let seg_count = if closed { n } else { n - 1 };
    let mut out = Vec::new();
    for seg in 0..seg_count {
        let p1 = get(seg as isize);
        let p2 = get(seg as isize + 1);
        // Reflect at open ends so the tangent isn't pulled toward a clamped dup.
        let p0 = if !closed && seg == 0 {
            [
                2.0 * p1[0] - p2[0],
                2.0 * p1[1] - p2[1],
                2.0 * p1[2] - p2[2],
            ]
        } else {
            get(seg as isize - 1)
        };
        let p3 = if !closed && seg == seg_count - 1 {
            [
                2.0 * p2[0] - p1[0],
                2.0 * p2[1] - p1[1],
                2.0 * p2[2] - p1[2],
            ]
        } else {
            get(seg as isize + 2)
        };
        let point_at = |t: f64| {
            let (t2, t3) = (t * t, t * t * t);
            let mut q = [0.0f64; 3];
            for k in 0..3 {
                q[k] = 0.5
                    * (2.0 * p1[k]
                        + (-p0[k] + p2[k]) * t
                        + (2.0 * p0[k] - 5.0 * p1[k] + 4.0 * p2[k] - p3[k]) * t2
                        + (-p0[k] + 3.0 * p1[k] - 3.0 * p2[k] + p3[k]) * t3);
            }
            q
        };
        let tangent_at = |t: f64| {
            let mut q = [0.0; 3];
            for k in 0..3 {
                let a = -p0[k] + p2[k];
                let b = 2.0 * p0[k] - 5.0 * p1[k] + 4.0 * p2[k] - p3[k];
                let c = -p0[k] + 3.0 * p1[k] - 3.0 * p2[k] + p3[k];
                q[k] = 0.5 * (a + 2.0 * b * t + 3.0 * c * t * t);
            }
            q
        };
        let sampled = cadkernel::tessellation::sample_curve3_angle(
            point_at,
            tangent_at,
            cadkernel::tessellation::DEFAULT_ANGLE,
        );
        out.extend(sampled.into_iter().skip(usize::from(seg > 0)));
    }
    out
}

/// Interpolate an open fit-point spline into a dense polyline: the C² cubic that
/// passes through every fit point, clamped to the stored start/end tangents when
/// present (natural end otherwise), so its ends follow the specified tangents
/// instead of the local slopes Catmull-Rom would use.
fn fit_spline_polyline(spl: &Spline) -> Vec<[f64; 3]> {
    let p: Vec<[f64; 3]> = spl.fit_points.iter().map(|q| [q.x, q.y, q.z]).collect();
    let n = p.len();
    if n < 2 {
        return p;
    }

    // Parameterise the fit points (unnormalised, so a unit end tangent — how the
    // tangents are stored — is a consistent dP/dt). Match the spline's knot
    // parameterisation: 2 = uniform, 1 = centripetal (√chord), else chord.
    let dist = |a: [f64; 3], b: [f64; 3]| {
        let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        (dx * dx + dy * dy + dz * dz).sqrt()
    };
    let mut t = vec![0.0f64; n];
    for i in 1..n {
        let d = dist(p[i - 1], p[i]).max(1e-9);
        let step = match spl.knot_parameterization {
            2 => 1.0,
            1 => d.sqrt(),
            _ => d,
        };
        t[i] = t[i - 1] + step;
    }
    let h: Vec<f64> = (0..n - 1).map(|i| (t[i + 1] - t[i]).max(1e-9)).collect();

    let nonzero = |v: &acadrust::types::Vector3| v.x * v.x + v.y * v.y + v.z * v.z > 1e-18;
    let begin = nonzero(&spl.begin_tangent).then(|| {
        [
            spl.begin_tangent.x,
            spl.begin_tangent.y,
            spl.begin_tangent.z,
        ]
    });
    let end = nonzero(&spl.end_tangent)
        .then(|| [spl.end_tangent.x, spl.end_tangent.y, spl.end_tangent.z]);

    // Solve for the knot slopes m_i = dP/dt per coordinate. The tridiagonal is
    // the C² continuity system; the end rows are the clamped tangent (m fixed)
    // or the natural condition (S'' = 0) when no tangent is stored.
    let mut slopes = [vec![0.0f64; n], vec![0.0f64; n], vec![0.0f64; n]];
    for k in 0..3 {
        let seg_slope = |i: usize| (p[i + 1][k] - p[i][k]) / h[i];
        let (mut a, mut b, mut c, mut d) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        match begin {
            Some(bt) => {
                b[0] = 1.0;
                d[0] = bt[k];
            }
            None => {
                b[0] = 2.0;
                c[0] = 1.0;
                d[0] = 3.0 * seg_slope(0);
            }
        }
        for i in 1..n - 1 {
            a[i] = h[i];
            b[i] = 2.0 * (h[i - 1] + h[i]);
            c[i] = h[i - 1];
            d[i] = 3.0 * (h[i] * seg_slope(i - 1) + h[i - 1] * seg_slope(i));
        }
        match end {
            Some(et) => {
                b[n - 1] = 1.0;
                d[n - 1] = et[k];
            }
            None => {
                a[n - 1] = 1.0;
                b[n - 1] = 2.0;
                d[n - 1] = 3.0 * seg_slope(n - 2);
            }
        }
        thomas_solve(&a, &b, &c, &mut d);
        slopes[k] = d;
    }

    // Evaluate each segment as a cubic Hermite (dP/du = slope · h_i).
    let mut out = Vec::new();
    for i in 0..n - 1 {
        let point_at = |u: f64| {
            let (u2, u3) = (u * u, u * u * u);
            let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
            let h10 = u3 - 2.0 * u2 + u;
            let h01 = -2.0 * u3 + 3.0 * u2;
            let h11 = u3 - u2;
            let mut q = [0.0f64; 3];
            for k in 0..3 {
                let m0 = slopes[k][i] * h[i];
                let m1 = slopes[k][i + 1] * h[i];
                q[k] = h00 * p[i][k] + h10 * m0 + h01 * p[i + 1][k] + h11 * m1;
            }
            q
        };
        let tangent_at = |u: f64| {
            let u2 = u * u;
            let (h00, h10, h01, h11) = (
                6.0 * u2 - 6.0 * u,
                3.0 * u2 - 4.0 * u + 1.0,
                -6.0 * u2 + 6.0 * u,
                3.0 * u2 - 2.0 * u,
            );
            let mut q = [0.0; 3];
            for k in 0..3 {
                let m0 = slopes[k][i] * h[i];
                let m1 = slopes[k][i + 1] * h[i];
                q[k] = h00 * p[i][k] + h10 * m0 + h01 * p[i + 1][k] + h11 * m1;
            }
            q
        };
        let sampled = cadkernel::tessellation::sample_curve3_angle(
            point_at,
            tangent_at,
            cadkernel::tessellation::DEFAULT_ANGLE,
        );
        out.extend(sampled.into_iter().skip(usize::from(i > 0)));
    }
    out
}

/// In-place Thomas solve for a tridiagonal system (`a` sub-, `b` main-, `c`
/// super-diagonal; `d` right-hand side, overwritten with the solution).
fn thomas_solve(a: &[f64], b: &[f64], c: &[f64], d: &mut [f64]) {
    let n = d.len();
    let mut cp = vec![0.0f64; n];
    cp[0] = c[0] / b[0];
    d[0] /= b[0];
    for i in 1..n {
        let m = b[i] - a[i] * cp[i - 1];
        cp[i] = c[i] / m;
        d[i] = (d[i] - a[i] * d[i - 1]) / m;
    }
    for i in (0..n - 1).rev() {
        d[i] -= cp[i] * d[i + 1];
    }
}

pub(crate) const SPLINE_MODE_GRIP_ID: usize = usize::MAX - 1;

fn control_vertices(spline: &Spline) -> Vec<acadrust::types::Vector3> {
    if !spline.control_points.is_empty() {
        return spline.control_points.clone();
    }
    let elevation = spline.fit_points.first().map(|p| p.z).unwrap_or(0.0);
    crate::modules::draw::modify::spline_ops::spline_control_curve(spline)
        .map(|curve| {
            curve
                .control_points()
                .iter()
                .map(|p| acadrust::types::Vector3::new(p[0], p[1], elevation))
                .collect()
        })
        .unwrap_or_default()
}

fn choice_prop(
    label: &str,
    field: &'static str,
    selected: &str,
    options: &[&str],
) -> Property {
    Property {
        label: label.into(),
        field,
        value: PropValue::Choice {
            selected: selected.to_string(),
            options: options.iter().map(|option| option.to_string()).collect(),
        },
    }
}

fn index_prop(label: &str, field: &'static str, index: usize, count: usize) -> Property {
    Property {
        label: label.into(),
        field,
        value: PropValue::EditText(if count == 0 {
            "0".to_string()
        } else {
            (index + 1).to_string()
        }),
    }
}

fn convert_to_control_method(spline: &mut Spline) -> bool {
    if !spline.control_points.is_empty() && spline.fit_points.is_empty() {
        return true;
    }
    let elevation = spline.fit_points.first().map(|point| point.z).unwrap_or(0.0);
    let Some(curve) = crate::modules::draw::modify::spline_ops::spline_control_curve(spline) else {
        return false;
    };
    spline.degree = curve.degree() as i32;
    spline.knots = curve.knots().to_vec();
    spline.control_points = curve
        .control_points()
        .iter()
        .map(|point| acadrust::types::Vector3::new(point[0], point[1], elevation))
        .collect();
    spline.weights = if curve.is_rational() {
        curve.weights().to_vec()
    } else {
        Vec::new()
    };
    spline.flags.rational = curve.is_rational();
    spline.fit_points.clear();
    spline.begin_tangent = acadrust::types::Vector3::ZERO;
    spline.end_tangent = acadrust::types::Vector3::ZERO;
    true
}

fn convert_to_fit_method(spline: &mut Spline) -> bool {
    if !spline.fit_points.is_empty() {
        return true;
    }
    let degree = spline.degree.max(1) as usize;
    let controls: Vec<_> = spline
        .control_points
        .iter()
        .map(|point| [point.x, point.y, point.z])
        .collect();
    let weights = (spline.weights.len() == controls.len()).then(|| spline.weights.clone());
    let Some(curve) = NurbsCurve3::new(degree, controls, spline.knots.clone(), weights) else {
        return false;
    };
    let (start, end) = curve.domain();
    let mut parameters: Vec<f64> = curve.knots()[degree..=curve.control_points().len()]
        .iter()
        .copied()
        .filter(|parameter| *parameter >= start && *parameter <= end)
        .collect();
    parameters.dedup_by(|left, right| (*left - *right).abs() < 1e-12);
    let sample_count = if degree == 3 {
        curve.control_points().len().saturating_sub(2).max(2)
    } else {
        curve.control_points().len().max(4)
    };
    for step in 0..sample_count {
        let fraction = step as f64 / (sample_count - 1) as f64;
        parameters.push(start + (end - start) * fraction);
    }
    parameters.sort_by(f64::total_cmp);
    parameters.dedup_by(|left, right| (*left - *right).abs() < 1e-12);

    spline.fit_points = parameters
        .iter()
        .map(|parameter| {
            let point = curve.point_at_knot(*parameter);
            acadrust::types::Vector3::new(point[0], point[1], point[2])
        })
        .collect();
    let begin = curve.tangent_at_knot(start);
    let end_tangent = curve.tangent_at_knot(end);
    spline.begin_tangent = acadrust::types::Vector3::new(begin[0], begin[1], begin[2]);
    spline.end_tangent =
        acadrust::types::Vector3::new(end_tangent[0], end_tangent[1], end_tangent[2]);
    spline.degree = 3;
    spline.knots.clear();
    spline.control_points.clear();
    spline.weights.clear();
    spline.flags.rational = false;
    spline.knot_parameterization = 0;
    true
}

fn is_planar(spline: &Spline) -> bool {
    let points = if spline.fit_points.is_empty() {
        &spline.control_points
    } else {
        &spline.fit_points
    };
    if points.len() <= 3 {
        return true;
    }
    let origin = points[0];
    let Some(axis) = points.iter().skip(1).find_map(|point| {
        let vector = [point.x - origin.x, point.y - origin.y, point.z - origin.z];
        let length2 = vector.iter().map(|value| value * value).sum::<f64>();
        (length2 > 1e-18).then_some(vector)
    }) else {
        return true;
    };
    let Some(normal) = points.iter().skip(1).find_map(|point| {
        let vector = [point.x - origin.x, point.y - origin.y, point.z - origin.z];
        let cross = [
            axis[1] * vector[2] - axis[2] * vector[1],
            axis[2] * vector[0] - axis[0] * vector[2],
            axis[0] * vector[1] - axis[1] * vector[0],
        ];
        let length2 = cross.iter().map(|value| value * value).sum::<f64>();
        (length2 > 1e-18).then_some(cross)
    }) else {
        return true;
    };
    let normal_length = normal.iter().map(|value| value * value).sum::<f64>().sqrt();
    points.iter().all(|point| {
        let offset = [point.x - origin.x, point.y - origin.y, point.z - origin.z];
        let distance = normal
            .iter()
            .zip(offset)
            .map(|(component, value)| component * value)
            .sum::<f64>()
            .abs()
            / normal_length;
        distance <= 1e-8
    })
}

fn grips(spline: &Spline) -> Vec<GripDef> {
    let (source, show_control_vertices) = if spline.cv_frame_visible {
        (control_vertices(spline), true)
    } else if !spline.fit_points.is_empty() {
        (spline.fit_points.clone(), false)
    } else {
        (spline.control_points.clone(), true)
    };
    let mut grips: Vec<_> = source
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let world = glam::DVec3::new(p.x, p.y, p.z);
            if show_control_vertices {
                round_grip(i, world)
            } else {
                square_grip(i, world)
            }
        })
        .collect();
    if let Some(first) = source.first() {
        grips.push(dropdown_grip(
            SPLINE_MODE_GRIP_ID,
            glam::DVec3::new(first.x, first.y, first.z),
        ));
    }
    grips
}

fn properties(spline: &Spline) -> Vec<PropSection> {
    let fit_method = !spline.fit_points.is_empty();
    let method = if fit_method { "Fit" } else { "Control Vertices" };
    let closed = spline.flags.closed || spline.flags.periodic;
    let yes_no = |b: bool| if b { "Yes" } else { "No" };
    let knot_param = match spline.knot_parameterization {
        0 => "Chord",
        1 => "Square Root",
        2 => "Uniform",
        15 => "Custom",
        _ => "Custom",
    };
    let current = crate::scene::view::dispatch::prop_current_vertex();
    let mut data_points = if fit_method {
        let count = spline.fit_points.len();
        let index = current.min(count.saturating_sub(1));
        let point = spline.fit_points.get(index);
        vec![
            ro(
                t!("Number of fit points").as_ref(),
                "fit_pt_count",
                count.to_string(),
            ),
            index_prop(
                t!("Current Fit point").as_ref(),
                "current_fit_point",
                index,
                count,
            ),
            edit(
                t!("Fit point X").as_ref(),
                "fit_pt_x",
                point.map(|value| value.x).unwrap_or(0.0),
            ),
            edit(
                t!("Fit point Y").as_ref(),
                "fit_pt_y",
                point.map(|value| value.y).unwrap_or(0.0),
            ),
            edit(
                t!("Fit point Z").as_ref(),
                "fit_pt_z",
                point.map(|value| value.z).unwrap_or(0.0),
            ),
            choice_prop(
                t!("Knot parameterization").as_ref(),
                "knot_param",
                knot_param,
                &["Chord", "Square Root", "Uniform", "Custom"],
            ),
        ]
    } else {
        let count = spline.control_points.len();
        let index = current.min(count.saturating_sub(1));
        let point = spline.control_points.get(index);
        let weight = spline.weights.get(index).copied().unwrap_or(1.0);
        vec![
            ro(
                t!("Number of control points").as_ref(),
                "ctrl_pt_count",
                count.to_string(),
            ),
            index_prop(
                t!("Current Control point").as_ref(),
                "current_control_point",
                index,
                count,
            ),
            edit(
                t!("Control point X").as_ref(),
                "ctrl_pt_x",
                point.map(|value| value.x).unwrap_or(0.0),
            ),
            edit(
                t!("Control point Y").as_ref(),
                "ctrl_pt_y",
                point.map(|value| value.y).unwrap_or(0.0),
            ),
            edit(
                t!("Control point Z").as_ref(),
                "ctrl_pt_z",
                point.map(|value| value.z).unwrap_or(0.0),
            ),
            edit(t!("Weight").as_ref(), "weight", weight),
        ]
    };
    data_points.push(choice_prop(
        t!("CV frame").as_ref(),
        "cv_frame",
        if spline.cv_frame_visible { "Show" } else { "Hide" },
        &["Hide", "Show"],
    ));

    let mut misc = vec![
        choice_prop(
            t!("Method").as_ref(),
            "spline_method",
            method,
            &["Fit", "Control Vertices"],
        ),
        ro(t!("Degree").as_ref(), "degree", spline.degree.to_string()),
        choice_prop(
            t!("Closed").as_ref(),
            "closed",
            yes_no(closed),
            &["No", "Yes"],
        ),
        ro(
            t!("Periodic").as_ref(),
            "periodic",
            yes_no(spline.flags.periodic),
        ),
        ro(
            t!("Planar").as_ref(),
            "planar",
            yes_no(is_planar(spline)),
        ),
    ];
    if fit_method {
        misc.extend([
            edit(
                t!("Start tangent vector X").as_ref(),
                "start_tan_x",
                spline.begin_tangent.x,
            ),
            edit(
                t!("Start tangent vector Y").as_ref(),
                "start_tan_y",
                spline.begin_tangent.y,
            ),
            edit(
                t!("Start tangent vector Z").as_ref(),
                "start_tan_z",
                spline.begin_tangent.z,
            ),
            edit(
                t!("End tangent vector X").as_ref(),
                "end_tan_x",
                spline.end_tangent.x,
            ),
            edit(
                t!("End tangent vector Y").as_ref(),
                "end_tan_y",
                spline.end_tangent.y,
            ),
            edit(
                t!("End tangent vector Z").as_ref(),
                "end_tan_z",
                spline.end_tangent.z,
            ),
            edit(
                t!("Fit tolerance").as_ref(),
                "fit_tolerance",
                spline.fit_tolerance,
            ),
        ]);
    }

    vec![
        PropSection {
            title: t!("Data Points").into_owned(),
            props: data_points,
        },
        PropSection {
            title: t!("Misc").into_owned(),
            props: misc,
        },
    ]
}

fn apply_geom_prop(spline: &mut Spline, field: &str, value: &str) {
    match field {
        "spline_method" => {
            if value == "Fit" {
                convert_to_fit_method(spline);
            } else if value == "Control Vertices" {
                convert_to_control_method(spline);
            }
            return;
        }
        "knot_param" => {
            spline.knot_parameterization = match value {
                "Chord" => 0,
                "Square Root" => 1,
                "Uniform" => 2,
                "Custom" => 15,
                _ => return,
            };
            return;
        }
        "cv_frame" => {
            spline.cv_frame_visible = value == "Show";
            return;
        }
        "closed" => {
            spline.flags.closed = value == "Yes";
            if !spline.flags.closed {
                spline.flags.periodic = false;
            }
            return;
        }
        _ => {}
    }
    let Some(v) = parse_f64(value) else { return };
    let control_index = crate::scene::view::dispatch::prop_current_vertex()
        .min(spline.control_points.len().saturating_sub(1));
    let fit_index = crate::scene::view::dispatch::prop_current_vertex()
        .min(spline.fit_points.len().saturating_sub(1));
    match field {
        "ctrl_pt_x" => {
            if let Some(cp) = spline.control_points.get_mut(control_index) {
                cp.x = v;
            }
        }
        "ctrl_pt_y" => {
            if let Some(cp) = spline.control_points.get_mut(control_index) {
                cp.y = v;
            }
        }
        "ctrl_pt_z" => {
            if let Some(cp) = spline.control_points.get_mut(control_index) {
                cp.z = v;
            }
        }
        "weight" => {
            if v > 0.0 && !spline.control_points.is_empty() {
                if spline.weights.len() != spline.control_points.len() {
                    spline.weights = vec![1.0; spline.control_points.len()];
                }
                spline.weights[control_index] = v;
                spline.flags.rational = spline
                    .weights
                    .iter()
                    .any(|weight| (*weight - spline.weights[0]).abs() > 1e-12);
            }
        }
        "fit_pt_x" => {
            if let Some(fp) = spline.fit_points.get_mut(fit_index) {
                fp.x = v;
            }
        }
        "fit_pt_y" => {
            if let Some(fp) = spline.fit_points.get_mut(fit_index) {
                fp.y = v;
            }
        }
        "fit_pt_z" => {
            if let Some(fp) = spline.fit_points.get_mut(fit_index) {
                fp.z = v;
            }
        }
        "fit_tolerance" if v >= 0.0 => spline.fit_tolerance = v,
        "start_tan_x" => spline.begin_tangent.x = v,
        "start_tan_y" => spline.begin_tangent.y = v,
        "start_tan_z" => spline.begin_tangent.z = v,
        "end_tan_x" => spline.end_tangent.x = v,
        "end_tan_y" => spline.end_tangent.y = v,
        "end_tan_z" => spline.end_tangent.z = v,
        _ => {}
    }
    spline.flags.planar = is_planar(spline);
}

fn apply_grip(spline: &mut Spline, grip_id: usize, apply: GripApply) {
    if grip_id == SPLINE_MODE_GRIP_ID {
        return;
    }
    // A fit spline can display its derived control polygon without changing
    // storage. The first actual CV edit converts that exact derived curve to
    // control-point storage so the dragged vertex remains authoritative when
    // the drawing is saved.
    if spline.cv_frame_visible && spline.control_points.is_empty() {
        if !convert_to_control_method(spline) {
            return;
        }
    }
    let target = if spline.cv_frame_visible || spline.fit_points.is_empty() {
        spline.control_points.get_mut(grip_id)
    } else {
        spline.fit_points.get_mut(grip_id)
    };
    if let Some(cp) = target {
        match apply {
            GripApply::Absolute(p) => {
                cp.x = p.x as f64;
                cp.y = p.y as f64;
                cp.z = p.z as f64;
            }
            GripApply::Translate(d) => {
                cp.x += d.x as f64;
                cp.y += d.y as f64;
                cp.z += d.z as f64;
            }
        }
    }
}

fn apply_transform(spline: &mut Spline, t: &EntityTransform) {
    crate::scene::view::transform::apply_standard_entity_transform(spline, t, |entity, p1, p2| {
        for cp in &mut entity.control_points {
            crate::scene::view::transform::reflect_xy_point(&mut cp.x, &mut cp.y, p1, p2);
        }
        for fp in &mut entity.fit_points {
            crate::scene::view::transform::reflect_xy_point(&mut fp.x, &mut fp.y, p1, p2);
        }
    });
}

impl RenderConvertible for Spline {
    fn to_render(&self, _document: &acadrust::CadDocument) -> Option<RenderEntity> {
        Some(to_render(self))
    }
}

impl crate::entities::traits::Grippable for Spline {
    fn grips(&self) -> Vec<GripDef> {
        grips(self)
    }
    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        apply_grip(self, grip_id, apply);
    }
    fn grip_menu(&self, grip_id: usize) -> Vec<crate::scene::model::object::GripMenuItem> {
        use crate::scene::model::object::{GripMenuAction, GripMenuItem};
        if grip_id == SPLINE_MODE_GRIP_ID {
            return if self.cv_frame_visible || self.fit_points.is_empty() {
                vec![
                    GripMenuItem {
                        label: "Fit",
                        action: GripMenuAction::ShowFit,
                    },
                    GripMenuItem {
                        label: "✓ Control Vertices",
                        action: GripMenuAction::ShowControlVertices,
                    },
                ]
            } else {
                vec![
                    GripMenuItem {
                        label: "✓ Fit",
                        action: GripMenuAction::ShowFit,
                    },
                    GripMenuItem {
                        label: "Control Vertices",
                        action: GripMenuAction::ShowControlVertices,
                    },
                ]
            };
        }
        vec![
            GripMenuItem {
                label: "Stretch",
                action: GripMenuAction::Stretch,
            },
            GripMenuItem {
                label: "Add Vertex",
                action: GripMenuAction::AddVertex,
            },
            GripMenuItem {
                label: "Remove Vertex",
                action: GripMenuAction::RemoveVertex,
            },
            GripMenuItem {
                label: "Refine Vertices",
                action: GripMenuAction::RefineVertices,
            },
        ]
    }
    fn apply_grip_menu(
        &mut self,
        grip_id: usize,
        action: crate::scene::model::object::GripMenuAction,
    ) {
        use crate::scene::model::object::GripMenuAction as A;
        match action {
            A::ShowFit => {
                if !self.fit_points.is_empty() || convert_to_fit_method(self) {
                    self.cv_frame_visible = false;
                    return;
                }
            }
            A::ShowControlVertices if control_vertices(self).len() >= 2 => {
                self.cv_frame_visible = true;
                return;
            }
            _ => {}
        }
        let n = self.control_points.len();
        let min_cv = (self.degree as usize).saturating_add(1).max(2);
        match action {
            A::AddVertex if grip_id < n => {
                let i1 = (grip_id + 1).min(n - 1);
                if i1 == grip_id {
                    return;
                }
                let p0 = &self.control_points[grip_id];
                let p1 = &self.control_points[i1];
                let mid = acadrust::types::Vector3::new(
                    (p0.x + p1.x) * 0.5,
                    (p0.y + p1.y) * 0.5,
                    (p0.z + p1.z) * 0.5,
                );
                self.control_points.insert(i1, mid);
                if !self.weights.is_empty() && self.weights.len() == n {
                    let w = (self.weights[grip_id] + self.weights[i1.min(self.weights.len() - 1)])
                        * 0.5;
                    self.weights.insert(i1, w);
                }
                // Clear knots so to_render rebuilds a uniform knot vector
                // for the new CV count.
                self.knots.clear();
            }
            A::RemoveVertex if grip_id < n && n > min_cv => {
                self.control_points.remove(grip_id);
                if grip_id < self.weights.len() {
                    self.weights.remove(grip_id);
                }
                self.knots.clear();
            }
            A::RefineVertices => {
                // Insert a CV between every adjacent pair (chord midpoints)
                // and rebuild a uniform knot vector.
                if n >= 2 {
                    let mut refined = Vec::with_capacity(n * 2 - 1);
                    let mut refined_w = Vec::with_capacity(n * 2 - 1);
                    let has_w = !self.weights.is_empty() && self.weights.len() == n;
                    for i in 0..n {
                        refined.push(self.control_points[i].clone());
                        if has_w {
                            refined_w.push(self.weights[i]);
                        }
                        if i + 1 < n {
                            let a = &self.control_points[i];
                            let b = &self.control_points[i + 1];
                            refined.push(acadrust::types::Vector3::new(
                                (a.x + b.x) * 0.5,
                                (a.y + b.y) * 0.5,
                                (a.z + b.z) * 0.5,
                            ));
                            if has_w {
                                refined_w.push((self.weights[i] + self.weights[i + 1]) * 0.5);
                            }
                        }
                    }
                    self.control_points = refined;
                    if has_w {
                        self.weights = refined_w;
                    }
                    self.knots.clear();
                }
            }
            _ => {}
        }
    }
}

impl crate::entities::traits::PropertyEditable for Spline {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        properties(self)
    }
    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        apply_geom_prop(self, field, value);
    }
}

impl crate::entities::traits::Transformable for Spline {
    fn apply_transform(&mut self, t: &EntityTransform) {
        apply_transform(self, t);
    }
}
