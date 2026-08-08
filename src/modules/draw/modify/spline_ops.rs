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
use acadrust::kernel::geom2d::NurbsCurve;
use acadrust::types::Vector3;
use acadrust::Handle;

// ── Conversion ─────────────────────────────────────────────────────────────

/// Convert an acadrust `Spline` to a kernel `NurbsCurve`, weights included.
///
/// Returns `None` when there are too few control points for the degree, which
/// is the one case that has no curve to evaluate. A knot vector of the wrong
/// length is repaired by the kernel rather than rejected.
pub fn spline_to_nurbs(spl: &Spline) -> Option<NurbsCurve> {
    let control_points: Vec<[f64; 2]> = spl
        .control_points
        .iter()
        .map(|p| [p.x, p.y])
        .collect();
    let degree = (spl.degree.max(1)) as usize;
    let weights = (!spl.weights.is_empty()).then(|| spl.weights.clone());
    NurbsCurve::new(degree, control_points, spl.knots.clone(), weights)
}

/// Rebuild an acadrust `Spline` from a kernel curve.
///
/// The `template` supplies entity common data and the Z elevation; splines
/// here are planar, so the kernel's 2D points are lifted back onto it.
pub fn nurbs_to_spline(curve: &NurbsCurve, template: &Spline) -> Spline {
    let z = template.control_points.first().map(|v| v.z).unwrap_or(0.0);
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
    // Fit points describe the original interpolation, which a split no longer
    // satisfies.
    spl.fit_points.clear();
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
    let elev = spl
        .control_points
        .first()
        .map(|v| v.z as f32)
        .unwrap_or(0.0);
    curve
        .tessellate(16)
        .iter()
        .map(|p| [p[0] as f32, p[1] as f32, elev])
        .collect()
}
