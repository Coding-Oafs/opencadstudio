// How finely a curve is sampled, for the whole of one render frame.
//
// Every curve in a drawing is tessellated to a chord height — how far the
// polyline drawn for it may sit from the curve itself — and the right value
// depends on the zoom. At a metre per pixel a millimetre of error is
// invisible; zoomed to a millimetre per pixel it is the whole picture.
//
// The Scene sets this once per frame from `world_per_pixel`, targeting about
// half a pixel. Every tessellation inside that frame reads the same atomic,
// including the ones running on rayon workers, which is why it is a global
// rather than a parameter: threading a tolerance through every entity
// converter's signature would touch every one of them to say the same thing.
//
// Zero means "no frame is being drawn" — a load, a snap, a hit test — and the
// floor is used instead. That is what `BlockCache::build` expects.

use std::sync::atomic::{AtomicU64, Ordering};

/// The finest a curve is ever sampled, in world units, and the value used
/// when no frame has set one.
const CURVE_TOL: f64 = 0.005;

static CURVE_TOL_BITS: AtomicU64 = AtomicU64::new(0);

/// Sets the per-frame curve tolerance. `None` — or anything not finite and
/// positive — reverts to the floor.
pub fn set_curve_tol_override(tol: Option<f64>) {
    let bits = match tol {
        Some(value) if value > 0.0 && value.is_finite() => value.to_bits(),
        _ => 0,
    };
    CURVE_TOL_BITS.store(bits, Ordering::Relaxed);
}

/// The tolerance to sample at, never below the floor — so zooming a long way
/// in cannot ask for a sampling finer than the baseline quality.
pub(crate) fn current_curve_tol() -> f64 {
    let bits = CURVE_TOL_BITS.load(Ordering::Relaxed);
    if bits == 0 {
        CURVE_TOL
    } else {
        f64::from_bits(bits).max(CURVE_TOL)
    }
}

/// `Some(tol)` only while a frame's override is in force — that is, while
/// something is being drawn rather than loaded, snapped or hit-tested. Hatch
/// boundaries use it to decide whether zoom-adaptive sampling applies at all.
pub(crate) fn active_curve_tol() -> Option<f64> {
    let bits = CURVE_TOL_BITS.load(Ordering::Relaxed);
    (bits != 0).then(|| f64::from_bits(bits).max(CURVE_TOL))
}
