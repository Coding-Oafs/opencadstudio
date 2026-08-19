// Shared B-spline utilities for modify commands (TRIM, BREAK, OFFSET, LENGTHEN).
//
// The evaluation lives in cadkernel; this file converts a drawing's SPLINE
// entity to and from it and keeps the knot-domain call shapes the commands
// were written against.
//
// Weights are carried through now. The previous path evaluated splines with a
// non-rational B-spline and cleared the weights on every split, which turns a
// piece of a NURBS circle into a piece of a parabola — a change that looks
// fine until it is measured.

use acadrust::entities::Spline;
use cadkernel::geom2d::{NurbsCurve, Parameterization};
use cadkernel::space::Plane;
use acadrust::types::Vector3;
use acadrust::Handle;

/// The drawing plane a spline sits on.
///
/// Read from whichever point list the spline actually stores: a fit-point
/// spline carries no control points, and defaulting its elevation to zero
/// would drop every such spline to the ground plane on the first edit.
fn elevation(spl: &Spline) -> f64 {
    spl.control_points
        .first()
        .or_else(|| spl.fit_points.first())
        .map(|v| v.z)
        .unwrap_or(0.0)
}

// ── Conversion ─────────────────────────────────────────────────────────────

/// Convert an acadrust `Spline` to a kernel `NurbsCurve`, weights included.
///
/// A SPLINE is stored one of two ways and both turn up: a control polygon the
/// curve approaches, or fit points it must pass through with optional end
/// tangents. Control points win when they describe a curve; otherwise the fit
/// points are interpolated into one, so a fit-point spline is as editable as
/// any other instead of failing every command that touches it.
///
/// `None` only when neither form has enough points to describe a curve.
///
/// Reads the stored points as XY directly, which is right for the splines the
/// editing commands work on — they run in the world frame. A spline on an
/// extruded plane needs [`spline_to_nurbs_on`] instead.
pub fn spline_to_nurbs(spl: &Spline) -> Option<NurbsCurve> {
    spline_to_nurbs_with(spl, |p| [p.x, p.y], |v| [v.x, v.y])
}

/// Return the compact control-polygon form of a spline for interactive grips.
///
/// The kernel's fit-point interpolator intentionally stores one Bezier span
/// per fit-point interval. That is exact and convenient for evaluation, but
/// its repeated internal knots expose three controls per interval to an
/// editor. An open cubic interpolation has an equivalent simple-knot form
/// with only `fit point count + 2` controls. Solve that form here so the grip
/// polygon stays compact without changing the curve or its saved fit points.
pub fn spline_control_curve(spl: &Spline) -> Option<NurbsCurve> {
    let exact = spline_to_nurbs(spl)?;
    if !spl.control_points.is_empty()
        || spl.flags.closed
        || spl.flags.periodic
        || spl.fit_points.len() < 2
    {
        return Some(exact);
    }

    let fit: Vec<[f64; 2]> = spl.fit_points.iter().map(|p| [p.x, p.y]).collect();
    let parameters = fit_parameters(&fit, spl.knot_parameterization);
    let control_count = fit.len() + 2;
    let mut knots = vec![parameters[0]; 4];
    knots.extend(parameters.iter().take(fit.len() - 1).skip(1).copied());
    knots.extend([parameters[fit.len() - 1]; 4]);

    let mut matrix = Vec::with_capacity(control_count);
    let mut right = Vec::with_capacity(control_count);
    for (&parameter, &point) in parameters.iter().zip(&fit) {
        matrix.push(bspline_basis(&knots, 3, parameter));
        right.push(point);
    }

    // Scaling the derivative equations by the domain width keeps the solve
    // well conditioned for drawings whose coordinates are very large/small.
    let (start, end) = exact.domain();
    let width = (end - start).abs().max(1e-9);
    matrix.push(
        bspline_basis_derivative(&knots, 3, parameters[0])
            .into_iter()
            .map(|value| value * width)
            .collect(),
    );
    let start_slope = exact.derivative_at_knot(start);
    right.push([start_slope[0] * width, start_slope[1] * width]);
    matrix.push(
        bspline_basis_derivative(&knots, 3, parameters[fit.len() - 1])
            .into_iter()
            .map(|value| value * width)
            .collect(),
    );
    let end_slope = exact.derivative_at_knot(end);
    right.push([end_slope[0] * width, end_slope[1] * width]);

    let controls = solve_control_points(matrix, right)?;
    NurbsCurve::new(3, controls, knots, None)
}

fn fit_parameters(points: &[[f64; 2]], parameterization: i32) -> Vec<f64> {
    let mut parameters = vec![0.0; points.len()];
    for i in 1..points.len() {
        let dx = points[i][0] - points[i - 1][0];
        let dy = points[i][1] - points[i - 1][1];
        let chord = (dx * dx + dy * dy).sqrt().max(1e-9);
        let step = match parameterization {
            2 => 1.0,
            1 => chord.sqrt(),
            _ => chord,
        };
        parameters[i] = parameters[i - 1] + step;
    }
    parameters
}

fn bspline_basis(knots: &[f64], degree: usize, parameter: f64) -> Vec<f64> {
    let count = knots.len() - degree - 1;
    if (parameter - knots[knots.len() - 1]).abs() < 1e-12 {
        let mut endpoint = vec![0.0; count];
        let last_span = knots
            .windows(2)
            .rposition(|span| span[0] < span[1])
            .unwrap_or(count - 1);
        endpoint[last_span.min(count - 1)] = 1.0;
        return endpoint;
    }

    let mut values: Vec<f64> = knots
        .windows(2)
        .map(|span| {
            if span[0] <= parameter && parameter < span[1] {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    for order in 1..=degree {
        let next_count = knots.len() - order - 1;
        let mut next = vec![0.0; next_count];
        for i in 0..next_count {
            let left_span = knots[i + order] - knots[i];
            let right_span = knots[i + order + 1] - knots[i + 1];
            let left = if left_span.abs() < 1e-15 {
                0.0
            } else {
                (parameter - knots[i]) / left_span * values[i]
            };
            let right = if right_span.abs() < 1e-15 {
                0.0
            } else {
                (knots[i + order + 1] - parameter) / right_span * values[i + 1]
            };
            next[i] = left + right;
        }
        values = next;
    }
    values
}

fn bspline_basis_derivative(knots: &[f64], degree: usize, parameter: f64) -> Vec<f64> {
    let lower = bspline_basis(knots, degree - 1, parameter);
    let count = knots.len() - degree - 1;
    (0..count)
        .map(|i| {
            let left_span = knots[i + degree] - knots[i];
            let right_span = knots[i + degree + 1] - knots[i + 1];
            let left = if left_span.abs() < 1e-15 {
                0.0
            } else {
                degree as f64 / left_span * lower[i]
            };
            let right = if right_span.abs() < 1e-15 {
                0.0
            } else {
                degree as f64 / right_span * lower[i + 1]
            };
            left - right
        })
        .collect()
}

fn solve_control_points(
    mut matrix: Vec<Vec<f64>>,
    mut right: Vec<[f64; 2]>,
) -> Option<Vec<[f64; 2]>> {
    let count = right.len();
    for column in 0..count {
        let pivot = (column..count).max_by(|&a, &b| {
            matrix[a][column]
                .abs()
                .total_cmp(&matrix[b][column].abs())
        })?;
        if matrix[pivot][column].abs() < 1e-14 {
            return None;
        }
        matrix.swap(column, pivot);
        right.swap(column, pivot);

        for row in column + 1..count {
            let factor = matrix[row][column] / matrix[column][column];
            for col in column..count {
                matrix[row][col] -= factor * matrix[column][col];
            }
            right[row][0] -= factor * right[column][0];
            right[row][1] -= factor * right[column][1];
        }
    }

    let mut result = vec![[0.0; 2]; count];
    for row in (0..count).rev() {
        for axis in 0..2 {
            let known: f64 = (row + 1..count)
                .map(|column| matrix[row][column] * result[column][axis])
                .sum();
            result[row][axis] = (right[row][axis] - known) / matrix[row][row];
        }
    }
    Some(result)
}

/// [`spline_to_nurbs`] with the points expressed in `plane`'s coordinates.
///
/// The two differ only for a spline whose extrusion normal is not +Z. Where
/// it is, the projection reduces to reading X and Y, so this is the same
/// conversion with the frame made explicit.
///
/// `None` additionally when the plane is degenerate, since there is then no
/// coordinate to express the points in.
pub fn spline_to_nurbs_on(spl: &Spline, plane: &Plane) -> Option<NurbsCurve> {
    let point = |p: &Vector3| plane.project([p.x, p.y, p.z]).unwrap_or([p.x, p.y]);
    let vector = |v: &Vector3| plane.project_vector([v.x, v.y, v.z]).unwrap_or([v.x, v.y]);
    plane.normal()?;
    spline_to_nurbs_with(spl, point, vector)
}

fn spline_to_nurbs_with(
    spl: &Spline,
    point: impl Fn(&Vector3) -> [f64; 2],
    vector: impl Fn(&Vector3) -> [f64; 2],
) -> Option<NurbsCurve> {
    let degree = (spl.degree.max(1)) as usize;
    let control_points: Vec<[f64; 2]> = spl.control_points.iter().map(&point).collect();
    let weights = (!spl.weights.is_empty()).then(|| spl.weights.clone());

    if let Some(curve) =
        NurbsCurve::new(degree, control_points, spl.knots.clone(), weights)
    {
        return Some(curve);
    }

    // No usable control polygon, so this is a fit-point spline.
    let mut fit: Vec<[f64; 2]> = spl.fit_points.iter().map(&point).collect();
    if spl.flags.closed || spl.flags.periodic {
        // The interpolation is a clamped solve and does not model a wrap, so
        // a closed spline came back as an open curve that never returned to
        // its start — and a TRIM against it then cut nothing along the seam.
        // Repeating the first point closes it. The seam is C¹ rather than the
        // C² the rest of the curve has, which is the honest limit of a
        // clamped solve; the alternative was a curve with a gap in it.
        if let (Some(&first), Some(&last)) = (fit.first(), fit.last()) {
            if first != last {
                fit.push(first);
            }
        }
    }
    let tangent = |v: &Vector3| {
        let flat = vector(v);
        (flat[0] * flat[0] + flat[1] * flat[1] > 1e-18).then_some(flat)
    };
    NurbsCurve::interpolate(
        &fit,
        tangent(&spl.begin_tangent),
        tangent(&spl.end_tangent),
        match spl.knot_parameterization {
            2 => Parameterization::Uniform,
            1 => Parameterization::Centripetal,
            _ => Parameterization::Chord,
        },
    )
}

/// Rebuild an acadrust `Spline` from a kernel curve.
///
/// The `template` supplies entity common data and the Z elevation; splines
/// here are planar, so the kernel's 2D points are lifted back onto it.
pub fn nurbs_to_spline(curve: &NurbsCurve, template: &Spline) -> Spline {
    let z = elevation(template);
    let mut spl = template.clone();
    spl.common.handle = Handle::NULL;
    spl.degree = curve.degree() as i32;
    spl.knots = curve.knots().to_vec();
    spl.control_points = curve
        .control_points()
        .iter()
        .map(|p| Vector3::new(p[0], p[1], z))
        .collect();
    spl.weights = if curve.is_rational() {
        curve.weights().to_vec()
    } else {
        Vec::new()
    };
    // Fit points and their tangents describe the original interpolation, which
    // a piece of it no longer satisfies. What comes back is a control-point
    // spline.
    spl.fit_points.clear();
    spl.begin_tangent = Vector3::new(0.0, 0.0, 0.0);
    spl.end_tangent = Vector3::new(0.0, 0.0, 0.0);
    spl
}

// ── Domain and splitting ───────────────────────────────────────────────────

/// The knot values the spline is defined over, `(start, end)`.
pub fn spline_range(spl: &Spline) -> Option<(f64, f64)> {
    Some(spline_to_nurbs(spl)?.domain())
}

/// Split a spline at knot value `t`, returning `(left, right)`.
///
/// Both halves are exact — the kernel raises the cut to full knot
/// multiplicity rather than refitting — and a rational spline stays rational.
/// `None` when the cut falls at or outside an end.
pub fn spline_cut(spl: &Spline, t: f64) -> Option<(Spline, Spline)> {
    let curve = spline_to_nurbs(spl)?;
    let (start, end) = curve.domain();
    if (end - start).abs() < 1e-12 {
        return None;
    }
    let normalised = (t - start) / (end - start);
    let (left, right) = curve.split_at(normalised)?;
    Some((
        nurbs_to_spline(&left, spl),
        nurbs_to_spline(&right, spl),
    ))
}

// ── Sampling ───────────────────────────────────────────────────────────────

/// Sample the spline's XY projection at `n+1` evenly spaced knot values.
/// Returns `(knot_params, xy_points)`, both of length `n+1`.
pub fn spline_sample_xy(spl: &Spline, n: usize) -> (Vec<f64>, Vec<[f64; 2]>) {
    let Some(curve) = spline_to_nurbs(spl) else {
        return (vec![], vec![]);
    };
    let (t0, t1) = curve.domain();
    let ts: Vec<f64> = (0..=n)
        .map(|i| t0 + (t1 - t0) * (i as f64 / n as f64))
        .collect();
    let pts: Vec<[f64; 2]> = ts.iter().map(|t| curve.point_at_knot(*t)).collect();
    (ts, pts)
}

/// The knot parameter closest to the XY point `(x, y)`.
pub fn spline_nearest_t(spl: &Spline, x: f64, y: f64) -> Option<f64> {
    let curve = spline_to_nurbs(spl)?;
    let (t0, t1) = curve.domain();
    let normalised = curve.parameter_at([x, y]);
    Some(t0 + normalised * (t1 - t0))
}

/// Normalise an actual knot parameter into `0..=1` over `[t0, t1]`.
pub fn t_to_rel(t_actual: f64, t0: f64, t1: f64) -> f64 {
    if (t1 - t0).abs() < 1e-12 {
        return 0.0;
    }
    ((t_actual - t0) / (t1 - t0)).clamp(0.0, 1.0)
}

// ── Wire preview ───────────────────────────────────────────────────────────

/// World-space wire points for a Spline.
///
/// Sampled per knot span rather than uniformly, so a curve whose knots bunch
/// up is cut finely where its shape actually changes.
pub fn spline_pts_wire(spl: &Spline) -> Vec<[f32; 3]> {
    let Some(curve) = spline_to_nurbs(spl) else {
        return vec![];
    };
    let elev = elevation(spl) as f32;
    curve
        .tessellate(16)
        .iter()
        .map(|p| [p[0] as f32, p[1] as f32, elev])
        .collect()
}
