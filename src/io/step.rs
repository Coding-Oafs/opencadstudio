// STEP AP203 export — converts tessellated MeshModels to ISO 10303-21 format.
//
// The output is a minimal but valid STEP AP203 file containing:
//   - One SHAPE_REPRESENTATION per solid mesh
//   - ADVANCED_FACE → PLANE → AXIS2_PLACEMENT_3D for each triangle
//   - VERTEX_POINT / EDGE_CURVE / ORIENTED_EDGE topology
//
// Because building full B-Rep topology from a triangle soup is complex, we use
// a simplified encoding: each triangle becomes a CLOSED_SHELL with three
// ADVANCED_FACEs, each face bounded by three oriented edges.
//
// This produces larger-than-optimal files but is universally importable by
// CAD systems that accept AP203.

use crate::scene::model::mesh_model::MeshModel;
use acadrust::entities::mesh::Mesh;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

const MAX_IMPORT_ENTITIES: usize = 20_000_000;

/// Import faceted STEP geometry. Supports AP203/AP214 boundary faces and the
/// AP242 `TRIANGULATED_FACE_SET` representation. Analytic-only STEP files are
/// rejected explicitly because this importer never substitutes bounding-box
/// or sampled placeholder geometry.
pub fn parse_step(text: &str) -> Result<Mesh, String> {
    let entities = step_entities(text)?;
    let mut points: HashMap<u64, [f64; 3]> = HashMap::new();
    let mut point_lists: HashMap<u64, Vec<[f64; 3]>> = HashMap::new();
    for (&id, expression) in &entities {
        match entity_name(expression) {
            "CARTESIAN_POINT" => {
                if let Some(point) = last_number_triple(expression) {
                    points.insert(id, point);
                }
            }
            "CARTESIAN_POINT_LIST_3D" => {
                let values = double_paren_content(expression)
                    .map(numeric_values)
                    .unwrap_or_default();
                let coords = values.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect();
                point_lists.insert(id, coords);
            }
            _ => {}
        }
    }

    let mut triangles = Vec::new();
    for expression in entities.values() {
        match entity_name(expression) {
            "TRIANGULATED_FACE_SET" => {
                let Some(list_id) = references(expression).first().copied() else {
                    continue;
                };
                let Some(coords) = point_lists.get(&list_id) else {
                    continue;
                };
                let Some(index_list) = double_paren_content(expression) else {
                    continue;
                };
                let indices = integer_values(index_list);
                for face in indices.chunks_exact(3) {
                    if face
                        .iter()
                        .all(|index| *index > 0 && (*index as usize) <= coords.len())
                    {
                        triangles.push([
                            coords[face[0] as usize - 1],
                            coords[face[1] as usize - 1],
                            coords[face[2] as usize - 1],
                        ]);
                    }
                }
            }
            "ADVANCED_FACE" | "FACE_SURFACE" => {
                if let Some(polygon) = step_face_points(expression, &entities, &points) {
                    for index in 1..polygon.len().saturating_sub(1) {
                        triangles.push([polygon[0], polygon[index], polygon[index + 1]]);
                    }
                }
            }
            _ => {}
        }
    }
    if triangles.is_empty() {
        return Err(
            "STEP contains no supported faceted faces (analytic STEP import is not available)"
                .to_string(),
        );
    }
    crate::io::stl::mesh_from_triangles(triangles)
}

fn step_entities(text: &str) -> Result<HashMap<u64, String>, String> {
    let mut entities = HashMap::new();
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] != b'#' {
            cursor += 1;
            continue;
        }
        let id_start = cursor + 1;
        cursor = id_start;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        let id = text[id_start..cursor]
            .parse::<u64>()
            .map_err(|_| "STEP contains an invalid entity id".to_string())?;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        let expression_start = cursor;
        let mut quoted = false;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\'' => {
                    if quoted && bytes.get(cursor + 1) == Some(&b'\'') {
                        cursor += 1;
                    } else {
                        quoted = !quoted;
                    }
                }
                b';' if !quoted => break,
                _ => {}
            }
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(format!("STEP entity #{id} is unterminated"));
        }
        entities.insert(
            id,
            text[expression_start..cursor].trim().to_ascii_uppercase(),
        );
        if entities.len() > MAX_IMPORT_ENTITIES {
            return Err("STEP exceeds the entity safety limit".to_string());
        }
        cursor += 1;
    }
    Ok(entities)
}

fn entity_name(expression: &str) -> &str {
    expression
        .trim_start_matches('(')
        .split_once('(')
        .map(|(name, _)| name.trim())
        .unwrap_or("")
}

fn references(expression: &str) -> Vec<u64> {
    let bytes = expression.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'#' {
            at += 1;
            let start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            if let Ok(value) = expression[start..at].parse() {
                out.push(value);
            }
        } else {
            at += 1;
        }
    }
    out
}

fn numeric_values(expression: &str) -> Vec<f64> {
    expression
        .split(|ch: char| !(ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'E' | 'e')))
        .filter(|word| !word.is_empty() && *word != "+" && *word != "-")
        .filter_map(|word| word.parse().ok())
        .collect()
}

fn integer_values(expression: &str) -> Vec<u64> {
    expression
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|word| !word.is_empty())
        .filter_map(|word| word.parse().ok())
        .collect()
}

fn double_paren_content(expression: &str) -> Option<&str> {
    let start = expression.find("((")?;
    let bytes = expression.as_bytes();
    let mut depth = 0i32;
    for at in start..bytes.len() {
        match bytes[at] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&expression[start..=at]);
                }
            }
            _ => {}
        }
    }
    None
}

fn last_number_triple(expression: &str) -> Option<[f64; 3]> {
    let values = numeric_values(expression);
    (values.len() >= 3).then(|| {
        let at = values.len() - 3;
        [values[at], values[at + 1], values[at + 2]]
    })
}

fn step_face_points(
    face: &str,
    entities: &HashMap<u64, String>,
    points: &HashMap<u64, [f64; 3]>,
) -> Option<Vec<[f64; 3]>> {
    let loop_id = references(face).into_iter().find_map(|id| {
        let value = entities.get(&id)?;
        match entity_name(value) {
            "EDGE_LOOP" | "POLY_LOOP" => Some(id),
            "FACE_BOUND" | "FACE_OUTER_BOUND" => references(value).into_iter().find(|loop_id| {
                entities.get(loop_id).is_some_and(|loop_value| {
                    matches!(entity_name(loop_value), "EDGE_LOOP" | "POLY_LOOP")
                })
            }),
            _ => None,
        }
    })?;
    let loop_entity = entities.get(&loop_id)?;
    if entity_name(loop_entity) == "POLY_LOOP" {
        let polygon: Vec<_> = references(loop_entity)
            .into_iter()
            .filter_map(|id| points.get(&id).copied())
            .collect();
        return (polygon.len() >= 3).then_some(polygon);
    }
    let mut polygon = Vec::new();
    for oriented_edge in references(loop_entity) {
        let oriented = entities.get(&oriented_edge)?;
        let edge_curve = references(oriented).into_iter().find(|id| {
            entities
                .get(id)
                .is_some_and(|e| entity_name(e) == "EDGE_CURVE")
        })?;
        let edge = entities.get(&edge_curve)?;
        let edge_vertices: Vec<_> = references(edge)
            .into_iter()
            .filter(|id| {
                entities
                    .get(id)
                    .is_some_and(|e| entity_name(e) == "VERTEX_POINT")
            })
            .collect();
        let vertex_point = if oriented.trim_end().ends_with(".F.)") {
            *edge_vertices.get(1)?
        } else {
            *edge_vertices.first()?
        };
        let point_id = references(entities.get(&vertex_point)?).first().copied()?;
        polygon.push(*points.get(&point_id)?);
    }
    (polygon.len() >= 3).then_some(polygon)
}

/// Geometric per-facet normal from the (already unit-scaled) triangle.
fn facet_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let nx = ab[1] * ac[2] - ab[2] * ac[1];
    let ny = ab[2] * ac[0] - ab[0] * ac[2];
    let nz = ab[0] * ac[1] - ab[1] * ac[0];
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(f64::EPSILON);
    [(nx / len) as f32, (ny / len) as f32, (nz / len) as f32]
}

/// Millimetres per drawing unit for an INSUNITS code. `None` (unitless /
/// unknown) is treated as 1:1 — the drawing is assumed to already be in
/// millimetres, which is what a save from this app produces.
fn unit_scale_to_mm(insunits: i16) -> f64 {
    match insunits {
        1 => 25.4,                   // Inches
        2 => 304.8,                  // Feet
        3 => 1_609_344.0,            // Miles
        5 => 10.0,                   // Centimeters
        6 => 1_000.0,                // Meters
        7 => 1_000_000.0,            // Kilometers
        8 => 0.000_025_4,            // Microinches
        9 => 0.025_4,                // Mils
        10 => 914.4,                 // Yards
        13 => 0.001,                 // Microns
        14 => 100.0,                 // Decimeters
        21 => 1_200_000.0 / 3_937.0, // US Survey Feet
        _ => 1.0,
    }
}

/// Build a STEP AP203 text representation from a slice of mesh models.
///
/// The file declares millimetres (`SI_UNIT(.MILLI.,.METRE.)`), so geometry in
/// drawings whose INSUNITS is not millimetres is scaled to mm first — an
/// inch-drawing exported raw used to label inch-magnitude numbers as
/// millimetres, a 25.4× scale error in every consumer.
///
/// Returns `None` if there are no triangles to export.
pub fn build_step(meshes: &[&MeshModel], insunits: i16) -> Option<String> {
    let scale = unit_scale_to_mm(insunits);
    // Collect all triangles as (v0, v1, v2, normal).
    struct Tri {
        v: [[f64; 3]; 3],
        n: [f32; 3],
    }

    let mut tris: Vec<Tri> = Vec::new();
    for mesh in meshes {
        let verts = &mesh.verts;
        let normals = &mesh.normals;
        let low = &mesh.verts_low;
        // Reattach the double-single low residuals in f64 before scaling:
        // `verts` alone is the truncated high half, which displaces
        // survey-scale geometry by up to a few metres.
        let point = |index: usize| -> Option<[f64; 3]> {
            let high = *verts.get(index)?;
            let exact = |hi: f32, lo: f32| hi as f64 + lo as f64;
            Some(if low.len() == verts.len() {
                [
                    exact(high[0], low[index][0]) * scale,
                    exact(high[1], low[index][1]) * scale,
                    exact(high[2], low[index][2]) * scale,
                ]
            } else {
                [
                    high[0] as f64 * scale,
                    high[1] as f64 * scale,
                    high[2] as f64 * scale,
                ]
            })
        };
        let idx = &mesh.indices;
        let n_tri = idx.len() / 3;
        for t in 0..n_tri {
            let i0 = idx[t * 3] as usize;
            let i1 = idx[t * 3 + 1] as usize;
            let i2 = idx[t * 3 + 2] as usize;
            let (Some(a), Some(b), Some(c)) = (point(i0), point(i1), point(i2)) else {
                continue;
            };
            let n = if !normals.is_empty() && i0 < normals.len() {
                normals[i0]
            } else {
                facet_normal(a, b, c)
            };
            tris.push(Tri { v: [a, b, c], n });
        }
    }

    if tris.is_empty() {
        return None;
    }

    // ── Emit STEP ─────────────────────────────────────────────────────────
    // Entity ID counter (STEP ids start at #1).
    let mut next_id: usize = 1;
    let mut data = String::new();

    // Closure to allocate the next ID.
    let mut alloc = || {
        let id = next_id;
        next_id += 1;
        id
    };

    // Collect face IDs for the shell.
    let mut face_ids: Vec<usize> = Vec::with_capacity(tris.len());

    for tri in &tris {
        // Each triangle: 3 vertices, 3 edges, 1 face.

        // Vertex points.
        let vp: [usize; 3] = [alloc(), alloc(), alloc()];
        // Cartesian points for vertices.
        let cp: [usize; 3] = [alloc(), alloc(), alloc()];
        // Line curves for edges.
        let lc: [usize; 3] = [alloc(), alloc(), alloc()];
        // Direction refs for lines (reusing cp[0] as direction — simplified).
        let dir: [usize; 3] = [alloc(), alloc(), alloc()];
        // Vertex-point refs.
        let vpref: [usize; 3] = [alloc(), alloc(), alloc()];
        // Edge curves.
        let ec: [usize; 3] = [alloc(), alloc(), alloc()];
        // Oriented edges.
        let oe: [usize; 3] = [alloc(), alloc(), alloc()];
        // Edge loop.
        let el = alloc();
        // Plane normal direction and axis placement.
        let norm_dir = alloc();
        let plane_ax = alloc();
        let plane = alloc();
        // Advanced face.
        let face_id = alloc();
        face_ids.push(face_id);

        // Emit cartesian points.
        for k in 0..3 {
            let [x, y, z] = tri.v[k];
            writeln!(
                data,
                "#{} = CARTESIAN_POINT('',({:.6},{:.6},{:.6}));",
                cp[k], x, y, z
            )
            .ok();
            writeln!(data, "#{} = VERTEX_POINT('',#{});", vpref[k], cp[k]).ok();
        }
        // Emit vertex points (binding).
        for k in 0..3 {
            writeln!(data, "#{} = VERTEX_POINT('',#{});", vp[k], cp[k]).ok();
            // (duplicate of vpref; we keep vp[] to reference in edge curves)
            let _ = vp[k]; // suppress unused warning
        }

        // Emit edge directions and line curves.
        for k in 0..3 {
            let k1 = (k + 1) % 3;
            let [dx, dy, dz] = [
                tri.v[k1][0] - tri.v[k][0],
                tri.v[k1][1] - tri.v[k][1],
                tri.v[k1][2] - tri.v[k][2],
            ];
            let len = (dx * dx + dy * dy + dz * dz).sqrt().max(f64::EPSILON);
            writeln!(
                data,
                "#{} = DIRECTION('',({:.6},{:.6},{:.6}));",
                dir[k],
                dx / len,
                dy / len,
                dz / len
            )
            .ok();
            writeln!(
                data,
                "#{} = LINE('',#{},VECTOR('',#{},1.));",
                lc[k], cp[k], dir[k]
            )
            .ok();
            writeln!(
                data,
                "#{} = EDGE_CURVE('',#{},#{},#{},.T.);",
                ec[k], vpref[k], vpref[k1], lc[k]
            )
            .ok();
            writeln!(data, "#{} = ORIENTED_EDGE('',*,*,#{},.T.);", oe[k], ec[k]).ok();
        }

        // Edge loop and face normal.
        writeln!(
            data,
            "#{} = EDGE_LOOP('',({},{},{}));",
            el,
            format!("#{}", oe[0]),
            format!("#{}", oe[1]),
            format!("#{}", oe[2])
        )
        .ok();

        let [nx, ny, nz] = tri.n;
        writeln!(
            data,
            "#{} = DIRECTION('',({:.6},{:.6},{:.6}));",
            norm_dir, nx, ny, nz
        )
        .ok();
        writeln!(
            data,
            "#{} = AXIS2_PLACEMENT_3D('',#{},#{},#{});",
            plane_ax, cp[0], norm_dir, dir[0]
        )
        .ok();
        writeln!(data, "#{} = PLANE('',#{});", plane, plane_ax).ok();
        writeln!(
            data,
            "#{} = ADVANCED_FACE('',(FACE_BOUND('',#{},.T.)),#{},.T.);",
            face_id, el, plane
        )
        .ok();
    }

    // Closed shell wrapping all faces.
    let shell_id = alloc();
    let face_list: String = face_ids
        .iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",");
    writeln!(data, "#{} = CLOSED_SHELL('',({face_list}));", shell_id).ok();

    // Manifold solid B-rep.
    let msb_id = alloc();
    writeln!(
        data,
        "#{} = MANIFOLD_SOLID_BREP('OpenCADStudio_Solid',#{});",
        msb_id, shell_id
    )
    .ok();

    // Shape representation.
    let sr_id = alloc();
    let pu_id = alloc();
    let gc_id = alloc();
    writeln!(
        data,
        "#{} = (LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));",
        pu_id
    )
    .ok();
    writeln!(data, "#{} = GEOMETRIC_REPRESENTATION_CONTEXT(3);", gc_id).ok();
    writeln!(
        data,
        "#{sr_id} = SHAPE_REPRESENTATION('OpenCADStudio_Shape',(#{}),#{gc_id});",
        msb_id
    )
    .ok();

    // ── Assemble file ─────────────────────────────────────────────────────
    let ts = chrono_timestamp();
    let file = format!(
        "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION(('Open CAD Studio STEP export'),'2;1');\n\
         FILE_NAME('{ts}','','',(''),'',' ',' ');\n\
         FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));\n\
         ENDSEC;\n\
         DATA;\n\
         {data}\
         ENDSEC;\n\
         END-ISO-10303-21;\n"
    );

    Some(file)
}

/// Returns an ISO 8601-like timestamp string for the STEP file header.
fn chrono_timestamp() -> String {
    // Use seconds since Unix epoch for a simple timestamp without chrono dep.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (hh, mm, ss) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    // Civil-from-days (Hinnant): exact year/month/day including leap years —
    // the old 365-day approximation drifted the header date by a day every
    // four years.
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_ap242_triangulated_face_set() {
        let step = "ISO-10303-21;DATA;\n\
#1=CARTESIAN_POINT_LIST_3D('',((0.,0.,0.),(2.,0.,0.),(0.,2.,0.)));\n\
#2=TRIANGULATED_FACE_SET('',#1,$,.T.,((1,2,3)),$);\n\
ENDSEC;END-ISO-10303-21;";
        let mesh = parse_step(step).expect("AP242 triangle");
        assert_eq!(3, mesh.vertices.len());
        assert_eq!(1, mesh.faces.len());
    }

    #[test]
    fn imports_ap203_face_bound_and_honors_reversed_edge() {
        let step = "ISO-10303-21;DATA;\n\
#1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(2.,0.,0.));\n\
#3=CARTESIAN_POINT('',(0.,2.,0.));\n\
#11=VERTEX_POINT('',#1);#12=VERTEX_POINT('',#2);#13=VERTEX_POINT('',#3);\n\
#21=EDGE_CURVE('',#11,#12,#1,.T.);#22=EDGE_CURVE('',#13,#12,#1,.T.);\n\
#23=EDGE_CURVE('',#13,#11,#1,.T.);\n\
#31=ORIENTED_EDGE('',*,*,#21,.T.);#32=ORIENTED_EDGE('',*,*,#22,.F.);\n\
#33=ORIENTED_EDGE('',*,*,#23,.T.);#40=EDGE_LOOP('',(#31,#32,#33));\n\
#41=FACE_OUTER_BOUND('',#40,.T.);#50=ADVANCED_FACE('',(#41),#1,.T.);\n\
ENDSEC;END-ISO-10303-21;";
        let mesh = parse_step(step).expect("AP203 face");
        assert_eq!(1, mesh.faces.len());
        assert_eq!(3, mesh.vertices.len());
    }

    #[test]
    fn rejects_analytic_only_step_instead_of_making_placeholders() {
        let error = parse_step("#1=SPHERICAL_SURFACE('',#2,10.0);")
            .expect_err("analytic-only input must be explicit");
        assert!(error.contains("analytic STEP import is not available"));
    }
}
