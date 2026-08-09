//! Drawing an ACIS solid by lifting it into the geometry kernel.
//!
//! A DWG or DXF carries its solids as ACIS records — analytic surfaces and the
//! topology joining them — and the kernel holds exactly that. So the shortest
//! path from a file to a picture is to lift the document into a `Body` and ask
//! the kernel for triangles, rather than to re-derive each surface's extent by
//! sampling it.
//!
//! # Why it can still fall short
//!
//! A face on a surface the kernel does not model, a curve it has no form for,
//! a pointer graph that does not hold together: [`lift`] reports each as a
//! [`Loss`] rather than quietly dropping it. What comes back then is a body
//! with faces missing, and the mesh it makes has holes — which is why the
//! result is marked incomplete and the caller keeps its own sampler for those.
//!
//! Saying so is the point. A partial mesh that claimed to be whole would show
//! a solid with a wall missing and nothing to suggest anything was wrong.

use acadrust::acis::lift;
use acadrust::entities::acis::SatDocument;
use acadrust::kernel::brep;

use crate::scene::convert::solid3d_tess::{body_transform, finalize_mesh};
use crate::scene::model::mesh_model::MeshLodSet;

/// How far a triangle may sit from the surface it lies on.
const SAG: f64 = 0.05;

/// What counts as the same point when the kernel reads a body over.
///
/// A micrometre, in a drawing measured in metres. Not slackness: an edge is
/// shared by two faces, and in a real file it cannot sit exactly on both,
/// because the two surfaces were fitted separately and written to finite
/// precision. Asked for exactness the kernel decides the edge is not on its
/// own plane, declines to project it, and the face is dropped — twenty-six
/// walls of one building went missing at a nanometre that no drawing means.
///
/// Loosening further buys almost nothing: a hundredth of this recovers one
/// more face in sixty thousand, and past that the tolerance would start
/// accepting geometry that really is wrong.
const TOL: f64 = 1e-6;

/// Tessellate an ACIS document by lifting it into the kernel.
///
/// `None` when nothing in the document lifts at all. The result's `complete`
/// flag says whether every face made it; a caller with a fallback sampler
/// uses it to decide whether to run one.
pub fn tessellate_sat(
    document: &SatDocument,
    name: String,
    color: [f32; 4],
    sag: f64,
) -> Option<MeshLodSet> {
    let (bodies, loss) = lift(document);
    if bodies.is_empty() {
        return None;
    }
    let sag = if sag > 0.0 { sag } else { SAG };

    // Positions stay f64 until `finalize_mesh` splits them into the coarse
    // and fine pair, so a solid at survey coordinates keeps its millimetres.
    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    // A face the kernel holds but cannot express in its surface's own
    // parameters leaves a hole, the same as one that never lifted — so both
    // are counted before calling the mesh whole.
    let mut undrawn = 0usize;
    for body in &bodies {
        for face in body.face_keys() {
            let Some(mesh) = brep::mesh::face(body, face, sag, TOL) else {
                undrawn += 1;
                continue;
            };
            let base = positions.len() as u32;
            positions.extend_from_slice(&mesh.positions);
            normals.extend(
                mesh.normals
                    .iter()
                    .map(|n| [n[0] as f32, n[1] as f32, n[2] as f32]),
            );
            indices.extend(
                mesh.triangles
                    .iter()
                    .flat_map(|t| [base + t[0] as u32, base + t[1] as u32, base + t[2] as u32]),
            );
        }
    }
    if indices.is_empty() {
        return None;
    }

    // ACIS keeps a body's geometry in its own local frame and records where it
    // sits in a separate `transform` record. Skipping that leaves every solid
    // stacked at the origin — which is what a BIM file looks like when each
    // component is placed rather than authored in world coordinates.
    let mut set = MeshLodSet::from_single(finalize_mesh(
        name,
        positions,
        normals,
        indices,
        Vec::new(),
        Vec::new(),
        color,
        body_transform(document),
    ));
    set.complete = loss.is_empty() && undrawn == 0;
    Some(set)
}

/// The edges of every body in an ACIS document, as polylines.
///
/// What draws a solid's wireframe and what a click hit-tests against. Taken
/// from the kernel's own curves rather than from the mesh, so a rim is a
/// circle sampled to tolerance instead of whatever the triangulation left
/// along it.
///
/// Not called yet: the solid tessellator keeps its own feature-edge pass,
/// which also carries isolines. Here because it is the kernel's answer to the
/// same question, and the two should converge on it.
#[allow(dead_code)]
pub fn edge_polylines(document: &SatDocument, sag: f64) -> Vec<Vec<[f64; 3]>> {
    let (bodies, _) = lift(document);
    let sag = if sag > 0.0 { sag } else { SAG };
    let placement = body_transform(document);
    bodies
        .iter()
        .flat_map(|body| brep::edge_polylines(body, sag))
        .map(|polyline| {
            polyline
                .into_iter()
                .map(|point| placed(point, placement))
                .collect()
        })
        .collect()
}

/// A body-local point moved to where the body sits.
///
/// ACIS treats points as row vectors — `p' = scale·(p·M) + T` — so the stored
/// 3×3 is indexed transposed from a column-vector multiply. Getting that the
/// wrong way round mirrors a placed solid rather than moving it.
fn placed(point: [f64; 3], xform: Option<([f64; 9], [f64; 3], f64)>) -> [f64; 3] {
    let Some((m, translation, scale)) = xform else {
        return point;
    };
    let [x, y, z] = point;
    [
        scale * (x * m[0] + y * m[3] + z * m[6]) + translation[0],
        scale * (x * m[1] + y * m[4] + z * m[7]) + translation[1],
        scale * (x * m[2] + y * m[5] + z * m[8]) + translation[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter turn about z, written the way ACIS writes it: row-major, and
    /// applied to points as row vectors.
    fn quarter_turn() -> Option<([f64; 9], [f64; 3], f64)> {
        Some((
            [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            [10.0, 20.0, 30.0],
            2.0,
        ))
    }

    #[test]
    fn a_placement_turns_the_way_acis_means_it_to() {
        // Deliberately asymmetric: a transposed multiply turns the other way,
        // so this catches the one mistake the convention invites.
        let moved = placed([1.0, 0.0, 0.0], quarter_turn());
        assert!((moved[0] - 10.0).abs() < 1e-12, "{moved:?}");
        assert!((moved[1] - 22.0).abs() < 1e-12, "{moved:?}");
        assert!((moved[2] - 30.0).abs() < 1e-12, "{moved:?}");
    }

    #[test]
    fn a_body_with_no_transform_stays_where_it_is() {
        // Many solids store absolute geometry and carry no transform record.
        // Treating that as anything but identity moves them off their own
        // coordinates.
        let point = [3.0, -4.0, 5.0];
        assert_eq!(placed(point, None), point);
    }

    #[test]
    fn the_scale_reaches_the_translation_only_once() {
        // `p' = scale·(p·M) + T`: the translation is not scaled. Folding the
        // scale into it as well puts a placed solid at twice its offset,
        // which reads as a plausible position and is the wrong one.
        let moved = placed([0.0, 0.0, 0.0], quarter_turn());
        assert_eq!(moved, [10.0, 20.0, 30.0]);
    }
}
