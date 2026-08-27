//! Export a kernel [`Body`] to an exact ACIS `SatDocument`.
//!
//! Analytic surfaces remain analytic instead of becoming facets.

use acadrust::entities::acis::SatDocument;
use cadkernel::acis::append;
use cadkernel::brep::Body;

/// Returns `None` when the body contains an unsupported record form.
pub fn planar_solid_to_sat(body: &Body) -> Option<SatDocument> {
    let mut document = SatDocument::new();
    append(body, &mut document).ok()?;
    Some(document)
}
