// ACIS B-rep → truck B-rep → mesh.
//
// Rebuilds every ACIS face as a truck topological `Face` and lets truck's
// meshing kernel triangulate it, routing loaded 3DSOLID / BODY / REGION /
// SURFACE geometry through the same NURBS/B-rep kernel the Model tab builds on
// instead of the bespoke per-surface sampler in `solid3d_tess`.
//
//   plane-surface   → planar face from the sampled boundary loop
//   cone-surface    → partial revolutions via rsweep; closed revolutions use
//                     the parametric sampler
//   sphere-surface  → bespoke sampler, clipped to the boundary window (truck
//                     builds a full ball and cannot trim it to a partial face)
//   torus-surface   → bespoke sampler, clipped to the revolution arc between
//                     the tube's end caps (truck builds a full closed ring)
//   spline-surface  → truck BSplineSurface, grid-sampled (see spline_tess)
//
// Each face is meshed independently and its triangles are oriented outward
// using an analytic per-surface normal — truck's own face orientation is not
// consistent across independently built faces, so normals/winding are derived
// from geometry instead. Face coverage is recorded on `MeshLodSet`; partial
// shells remain displayable but cannot masquerade as complete solid topology.

use truck_meshalgo::tessellation::{MeshableShape, MeshedShape};
use truck_modeling::{builder, Face, InnerSpace, Point3, Rad, Shell, Vector3, Wire};
use truck_polymesh::PolygonMesh;

use acadrust::entities::acis::types::Sense;
use acadrust::entities::acis::{
    SatConeSurface, SatDocument, SatFace, SatPlaneSurface, SatSphereSurface, SatTorusSurface,
};

use crate::scene::convert::solid3d_tess::{
    body_transform, collect_face_loops, cone_axis_span, finalize_mesh, tess_cone_face,
    tess_plane_face, tess_sphere_face, tess_torus_face, LodConfig, BOUNDARY_CHORD_FRAC,
    TRUCK_CHORD_FRAC,
};
use crate::scene::model::mesh_model::MeshLodSet;
use rustc_hash::FxHashMap;

/// Slightly over 2π so revolution builders close the loop.
const FULL: f64 = std::f64::consts::TAU + 0.2;
/// Triangle mesh chord tolerance (world units) for planar faces, where the
/// surface itself adds no curvature.
const MESH_TOL: f64 = 0.01;

/// Analytic outward-normal rule for a face, used to orient triangles and
/// supply smooth per-vertex normals (truck's face orientation is unreliable
/// for independently built faces).
enum Outward {
    /// Constant normal (planar face).
    Const([f64; 3]),
    /// Cone/cylinder: radial away from the axis, tilted by the half-angle.
    Cone {
        center: [f64; 3],
        axis: [f64; 3],
        sin: f64,
        cos: f64,
    },
}

impl Outward {
    fn at(&self, p: [f64; 3]) -> [f64; 3] {
        match self {
            Outward::Const(n) => *n,
            Outward::Cone {
                center,
                axis,
                sin,
                cos,
            } => {
                let d = vsub(p, *center);
                let h = vdot(d, *axis);
                let radial = vnorm(vsub(d, vscale(*axis, h)));
                vnorm(vsub(vscale(radial, *cos), vscale(*axis, *sin)))
            }
        }
    }
}

/// Tessellate an ACIS SAT document by rebuilding it as truck faces.
///
/// Returns `None` when no face could be rebuilt (caller should fall back to
/// the bespoke sampler).
pub fn tessellate_sat_truck(
    sat: &SatDocument,
    name: String,
    color: [f32; 4],
    _facet_res: f64,
) -> Option<MeshLodSet> {
    // Vertices accumulate in f64 world/local space; `finalize_mesh` splits them
    // into the double-single (high, low) pair once, so a solid at UTM scale
    // keeps full precision instead of quantizing to the f32 grid.
    let mut verts: Vec<[f64; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let face_materials: FxHashMap<i32, acadrust::Handle> = sat
        .records
        .iter()
        .filter(|record| record.entity_type == "material-adesk-attrib")
        .filter_map(|record| {
            let owner = record.token_pointer(2)?.0;
            let handle = record.token(3)?.as_integer()?;
            (owner >= 0 && handle > 0)
                .then(|| (owner, acadrust::Handle::new(handle as u64)))
        })
        .collect();
    let face_colors: FxHashMap<i32, [f32; 4]> = sat
        .records
        .iter()
        .filter(|record| record.entity_type == "color-adesk-attrib")
        .filter_map(|record| {
            let owner = record.token_pointer(2)?.0;
            let value = record.token(3)?.as_integer()?;
            let face_color = if (1..=255).contains(&value) {
                acadrust::Color::from_index(value as i16)
            } else if value > 257 {
                acadrust::Color::from_true_color_value(value as i32)
            } else {
                return None;
            };
            Some((
                owner,
                {
                    let mut rgba =
                        crate::scene::convert::tess_util::aci_to_rgba(&face_color);
                    rgba[3] = color[3];
                    rgba
                },
            ))
        })
        .collect();
    let mut triangle_material_handles: Vec<Option<acadrust::Handle>> = Vec::new();
    let mut triangle_colors: Vec<Option<[f32; 4]>> = Vec::new();
    let face_record_indices: Vec<i32> = sat
        .records
        .iter()
        .filter(|record| record.is_a("face"))
        .map(|record| record.index)
        .collect();
    let mut complete = true;

    for (face, face_record_index) in sat.faces().into_iter().zip(face_record_indices) {
        let Some(surf_rec) = sat.resolve(face.surface()) else {
            complete = false;
            continue;
        };
        let before = indices.len();
        let mut appended = false;
        if let Some((faces, outward, tol)) = build_face_group(sat, &face, surf_rec) {
            if !faces.is_empty() {
                let shell: Shell = faces.into();
                let poly = shell.triangulation(tol).to_polygon();
                if !poly.tri_faces().is_empty() {
                    append_group(&mut verts, &mut normals, &mut indices, &poly, &outward);
                    appended = true;
                }
            }
        }
        // Truck's rsweep/triangulation degenerates to zero triangles on some
        // short-arc cones (a curved wall face whose profile passes through the
        // local origin), leaving a see-through gap. Fall back to the bespoke
        // parametric sampler for just that face — it writes into the same
        // buffers, so the shared finalize below still applies uniformly.
        if !appended {
            bespoke_face(sat, &face, surf_rec, &mut verts, &mut normals, &mut indices);
            appended = indices.len() > before;
        }
        if !appended {
            complete = false;
        } else {
            let material = face_materials.get(&face_record_index).copied();
            triangle_material_handles.extend(
                std::iter::repeat(material).take((indices.len() - before) / 3),
            );
            let face_color = face_colors.get(&face_record_index).copied();
            triangle_colors.extend(
                std::iter::repeat(face_color).take((indices.len() - before) / 3),
            );
        }
    }

    if indices.is_empty() {
        return None;
    }

    let mesh = finalize_mesh(
        name,
        verts,
        normals,
        indices,
        triangle_material_handles,
        triangle_colors,
        color,
        body_transform(sat),
    );
    let mut set = MeshLodSet::from_lods(vec![mesh]);
    set.complete = complete;
    Some(set)
}

/// Fill one face with the bespoke parametric sampler (body-local verts into the
/// shared mesh buffers) when truck produced no triangles for it. Mirrors the
/// surface dispatch of the standalone sampler.
fn bespoke_face(
    sat: &SatDocument,
    face: &SatFace,
    surf_rec: &acadrust::entities::acis::SatRecord,
    v: &mut Vec<[f64; 3]>,
    n: &mut Vec<[f32; 3]>,
    i: &mut Vec<u32>,
) {
    match surf_rec.entity_type.as_str() {
        "plane-surface" => {
            if let Some(p) = SatPlaneSurface::from_record(surf_rec) {
                tess_plane_face(sat, face, &p, LodConfig::HIGH.chord_frac, v, n, i);
            }
        }
        "cone-surface" => {
            if let Some(c) = SatConeSurface::from_record(surf_rec) {
                tess_cone_face(sat, face, &c, LodConfig::HIGH, v, n, i);
            }
        }
        "sphere-surface" => {
            if let Some(s) = SatSphereSurface::from_record(surf_rec) {
                tess_sphere_face(sat, face, &s, LodConfig::HIGH, v, n, i);
            }
        }
        "torus-surface" => {
            if let Some(t) = SatTorusSurface::from_record(surf_rec) {
                tess_torus_face(sat, face, &t, LodConfig::HIGH, v, n, i);
            }
        }
        "spline-surface" | "meshsurf-surface" | "bs3-surface" => {
            crate::scene::convert::spline_tess::tess_spline_face(
                sat,
                face,
                LodConfig::HIGH,
                v,
                n,
                i,
            );
        }
        _ => {}
    }
}

/// Build the truck face(s) + outward rule for one analytic ACIS face.
fn build_face_group(
    sat: &SatDocument,
    face: &SatFace,
    surf_rec: &acadrust::entities::acis::SatRecord,
) -> Option<(Vec<Face>, Outward, f64)> {
    // Chord tolerance scaled to a curved surface's radius, so the facet count
    // is radius-independent (matching the circle/arc wire tessellation) rather
    // than exploding on large radii.
    let curve_tol = |radius: f64| (radius.abs() * TRUCK_CHORD_FRAC).max(1e-6);
    match surf_rec.entity_type.as_str() {
        "plane-surface" => {
            let plane = SatPlaneSurface::from_record(surf_rec)?;
            let f = plane_face(sat, face)?;
            let (nx, ny, nz) = plane.normal();
            let n = if matches!(face.sense(), Sense::Reversed) {
                [-nx, -ny, -nz]
            } else {
                [nx, ny, nz]
            };
            Some((vec![f], Outward::Const(vnorm(n)), MESH_TOL))
        }
        "cone-surface" => {
            let cone = SatConeSurface::from_record(surf_rec)?;
            let tol = curve_tol(cone.radius());
            let (faces, out) = cone_faces(sat, face, &cone)?;
            Some((faces, out, tol))
        }
        // Truck builds a sphere as a full revolution and cannot trim it to the
        // face's boundary, so a partial sphere renders as a whole ball. Route
        // sphere faces to the bespoke sampler instead — it meshes only the
        // boundary's parametric window (see `tess_sphere_face`).
        "sphere-surface" => None,
        // Like a sphere, truck builds a torus as a full revolution and cannot
        // trim it to the face's boundary, so an open "C" tube renders as a whole
        // closed ring. Route torus faces to the bespoke sampler, which meshes
        // only the revolution arc between the tube's end caps (see
        // `tess_torus_face` / `torus_phi_range`).
        "torus-surface" => None,
        _ => None,
    }
}

// ── Planar face ────────────────────────────────────────────────────────────

/// Build a planar truck face from a face's sampled boundary loop. Curved
/// boundary edges (circles) are sampled into line segments, which keeps the
/// wire planar so `try_attach_plane` can fit the plane.
fn plane_face(sat: &SatDocument, face: &SatFace) -> Option<Face> {
    // A pierced face (e.g. a wall with a window opening) has one outer boundary
    // loop plus an inner loop per hole. `try_attach_plane` fits the plane from
    // the first wire and cuts the rest as holes, letting truck's mesher trim
    // them. ACIS records no outer-vs-hole flag — the kernel classifies loops at
    // runtime from geometry — so we derive the outer boundary here. (#123)
    let loops = collect_face_loops(sat, face, BOUNDARY_CHORD_FRAC);
    if loops.is_empty() {
        return None;
    }
    // The outer boundary is the loop that encloses the rest — i.e. the one with
    // the largest area. ACIS does not guarantee it comes first in the face's
    // loop list (a pierced wall lists its window holes before the wall edge),
    // so pick it by area rather than trusting index 0. (#123)
    let outer_idx = (0..loops.len())
        .max_by(|&a, &b| {
            vlen(loop_normal(&loops[a]))
                .partial_cmp(&vlen(loop_normal(&loops[b])))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap();
    if loops[outer_idx].len() < 3 {
        return None;
    }
    // `try_attach_plane` fits its plane normal from the first (outer) wire, so
    // feeding the outer in its sampled order makes truck read it as CCW. truck
    // then cuts an inner wire as a hole only when it winds the *opposite* way,
    // so reverse any hole loop whose area normal agrees with the outer's. The
    // outer normal has the largest magnitude (largest area), keeping the sign
    // test robust against sampling noise on small holes. (#123)
    let outer_n = loop_normal(&loops[outer_idx]);
    let mut wires: Vec<Wire> = Vec::new();
    wires.push(polygon_wire(&loops[outer_idx], false)?);
    for (i, lp) in loops.iter().enumerate() {
        if i == outer_idx {
            continue;
        }
        let same = vdot(loop_normal(lp), outer_n) > 0.0;
        if let Some(w) = polygon_wire(lp, same) {
            wires.push(w);
        }
    }
    builder::try_attach_plane(&wires).ok()
}

fn polygon_wire(pts: &[[f64; 3]], reverse: bool) -> Option<Wire> {
    if pts.len() < 3 {
        return None;
    }
    let verts: Vec<_> = if reverse {
        pts.iter()
            .rev()
            .map(|p| builder::vertex(Point3::new(p[0], p[1], p[2])))
            .collect()
    } else {
        pts.iter()
            .map(|p| builder::vertex(Point3::new(p[0], p[1], p[2])))
            .collect()
    };
    let n = verts.len();
    let edges: Vec<_> = (0..n)
        .map(|i| builder::line(&verts[i], &verts[(i + 1) % n]))
        .collect();
    Some(edges.into())
}

/// Newell area-weighted normal of a closed 3-D polygon (orientation only).
fn loop_normal(pts: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    let m = pts.len();
    for i in 0..m {
        let a = pts[i];
        let b = pts[(i + 1) % m];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

// ── Cone / cylinder face ─────────────────────────────────────────────────────

/// Build a pierced cone/cylinder lateral surface as revolution faces. A
/// single-loop face uses the parametric sampler, avoiding partial revolution
/// segments that can disappear when the profile crosses a coordinate plane.
fn cone_faces(
    sat: &SatDocument,
    face: &SatFace,
    cone: &SatConeSurface,
) -> Option<(Vec<Face>, Outward)> {
    let (cx, cy, cz) = cone.center();
    let (ax, ay, az) = cone.axis();
    let (ux, uy, uz) = cone.major_axis();
    let radius = cone.radius();
    let sin = cone.sin_half_angle();
    let cos = cone.cos_half_angle();

    let axis = norm(Vector3::new(ax, ay, az));
    let udir = norm(Vector3::new(ux, uy, uz));
    // v = axis × u completes the right-handed frame; angles increase from u
    // toward v (CCW about the axis), matching `builder::rsweep`.
    let vdir = Vector3::new(
        axis.y * udir.z - axis.z * udir.y,
        axis.z * udir.x - axis.x * udir.z,
        axis.x * udir.y - axis.y * udir.x,
    );
    let center = Point3::new(cx, cy, cz);

    let (hmin, hmax) =
        cone_axis_span(sat, face, cone, [axis.x, axis.y, axis.z], [cx, cy, cz])?;
    let r_at = |h: f64| {
        if cos.abs() > 1e-9 {
            radius + h * sin / cos
        } else {
            radius
        }
    };

    let loops = collect_face_loops(sat, face, BOUNDARY_CHORD_FRAC);
    if loops.len() == 1 {
        return None;
    }
    let arcs: Vec<(f64, f64)> = loops
        .iter()
        .map(|lp| cone_boundary_arc(lp, [cx, cy, cz], axis, udir, vdir))
        .collect();
    let outer_idx = arcs
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)?;
    let (theta0, sweep) = arcs[outer_idx];
    let closed = sweep >= std::f64::consts::TAU;
    let hole_indices: Vec<usize> = arcs
        .iter()
        .enumerate()
        .filter_map(|(i, arc)| (i != outer_idx && arc.1 < std::f64::consts::TAU).then_some(i))
        .collect();
    if closed && hole_indices.is_empty() {
        return None;
    }
    // Radial direction at the arc's start angle (u rotated by theta0 about axis).
    let rad0 = udir * theta0.cos() + vdir * theta0.sin();

    let p0 = center + rad0 * r_at(hmin) + axis * hmin;
    let p1 = center + rad0 * r_at(hmax) + axis * hmax;
    let profile = builder::line(&builder::vertex(p0), &builder::vertex(p1));
    let shell: Shell = builder::rsweep(&profile, center, axis, Rad(sweep));
    let mut faces: Vec<Face> = shell.face_iter().cloned().collect();
    if closed {
        let lateral = faces.first_mut()?;
        for index in hole_indices {
            let wire = polygon_wire(&loops[index], false)?;
            lateral.try_add_boundary(wire).ok()?;
        }
    }

    let out = Outward::Cone {
        center: [cx, cy, cz],
        axis: [axis.x, axis.y, axis.z],
        sin,
        cos,
    };
    Some((faces, out))
}

/// Smallest angular arc `(theta_start, sweep)` about the cone axis enclosing the
/// face's boundary points, robust to the ±π seam. Returns `(0.0, FULL)` when the
/// points wrap the whole revolution (a closed rim) or the boundary is unusable,
/// so a real cylinder/cone still sweeps a complete (overlapped) circle.
fn cone_boundary_arc(
    poly: &[[f64; 3]],
    center: [f64; 3],
    _axis: Vector3,
    udir: Vector3,
    vdir: Vector3,
) -> (f64, f64) {
    use std::f64::consts::TAU;
    let mut angles: Vec<f64> = Vec::new();
    for p in poly {
        let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
        let ru = d[0] * udir.x + d[1] * udir.y + d[2] * udir.z;
        let rv = d[0] * vdir.x + d[1] * vdir.y + d[2] * vdir.z;
        if ru.hypot(rv) > 1e-9 {
            angles.push(rv.atan2(ru));
        }
    }
    if angles.len() < 2 {
        return (0.0, FULL);
    }
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Largest angular gap between consecutive samples (wrapping past the seam);
    // the enclosing arc is its complement.
    let mut max_gap = 0.0;
    let mut gap_at = angles.len() - 1;
    for i in 0..angles.len() {
        let a = angles[i];
        let b = if i + 1 < angles.len() {
            angles[i + 1]
        } else {
            angles[0] + TAU
        };
        let g = b - a;
        if g > max_gap {
            max_gap = g;
            gap_at = i;
        }
    }
    let span = TAU - max_gap;
    // Nearly a full revolution → treat as a closed rim.
    if span >= TAU * 0.98 - 1e-9 || span < 1e-6 {
        return (0.0, FULL);
    }
    // Start at the sample just after the largest gap, sweep CCW by `span`.
    let start = angles[(gap_at + 1) % angles.len()];
    (start, span)
}

// ── Mesh append with analytic outward normals ────────────────────────────────

/// Append one face's triangulation to `mesh`, computing smooth per-vertex
/// normals from the outward rule and flipping any triangle whose winding
/// disagrees with that outward direction.
fn append_group(
    verts: &mut Vec<[f64; 3]>,
    normals: &mut Vec<[f32; 3]>,
    out_indices: &mut Vec<u32>,
    poly: &PolygonMesh,
    outward: &Outward,
) {
    let positions = poly.positions();
    let base = verts.len() as u32;
    for p in positions {
        let pos = [p.x, p.y, p.z];
        verts.push(pos);
        let n = outward.at(pos);
        normals.push([n[0] as f32, n[1] as f32, n[2] as f32]);
    }
    for tri in poly.tri_faces() {
        let (i0, i1, i2) = (tri[0].pos, tri[1].pos, tri[2].pos);
        let a = pt(positions[i0]);
        let b = pt(positions[i1]);
        let c = pt(positions[i2]);
        let gn = vcross(vsub(b, a), vsub(c, a));
        let cen = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let out = outward.at(cen);
        let (j0, j1, j2) = (base + i0 as u32, base + i1 as u32, base + i2 as u32);
        if vdot(gn, out) < 0.0 {
            out_indices.extend_from_slice(&[j0, j2, j1]);
        } else {
            out_indices.extend_from_slice(&[j0, j1, j2]);
        }
    }
}

#[inline]
fn pt(p: Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

// ── Vector helpers (cgmath Vector3) ──────────────────────────────────────────

#[inline]
fn norm(v: Vector3) -> Vector3 {
    let len = v.magnitude();
    if len < 1e-12 {
        Vector3::unit_z()
    } else {
        v / len
    }
}

// ── Vector helpers ([f64; 3]) ────────────────────────────────────────────────

#[inline]
fn vsub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn vscale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
fn vdot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn vcross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn vlen(a: [f64; 3]) -> f64 {
    vdot(a, a).sqrt()
}
#[inline]
fn vnorm(a: [f64; 3]) -> [f64; 3] {
    let l = vlen(a);
    if l < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}
