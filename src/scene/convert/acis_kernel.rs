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

use crate::scene::model::mesh_model::{MeshLodSet, MeshModel};

/// How far a triangle may sit from the surface it lies on.
const SAG: f64 = 0.05;

/// What counts as the same point when the kernel reads a body over.
const TOL: f64 = 1e-9;

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

    // The renderer holds each position as a coarse float plus a fine
    // correction, so a solid at survey coordinates keeps its last millimetres
    // instead of losing them to f32.
    let mut verts = Vec::with_capacity(positions.len());
    let mut verts_low = Vec::with_capacity(positions.len());
    for point in &positions {
        let high = [point[0] as f32, point[1] as f32, point[2] as f32];
        verts.push(high);
        verts_low.push([
            (point[0] - high[0] as f64) as f32,
            (point[1] - high[1] as f64) as f32,
            (point[2] - high[2] as f64) as f32,
        ]);
    }

    let mut set = MeshLodSet::from_single(MeshModel {
        name,
        verts,
        verts_low,
        normals,
        indices,
        triangle_material_handles: Vec::new(),
        triangle_colors: Vec::new(),
        color,
        selected: false,
    });
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
    bodies
        .iter()
        .flat_map(|body| brep::edge_polylines(body, sag))
        .collect()
}
