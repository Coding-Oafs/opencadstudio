// STL binary export — converts all tessellated MeshModels in the scene to a
// single binary STL file.
//
// Binary STL format:
//   80-byte header
//   4-byte triangle count (u32 LE)
//   Per triangle (50 bytes):
//     3 × f32 normal
//     3 × 3 × f32 vertices
//     2-byte attribute (0)

use std::io::Write;

use crate::scene::model::mesh_model::MeshModel;
use acadrust::entities::mesh::{Mesh, MeshFace};
use acadrust::types::Vector3;

const MAX_IMPORT_TRIANGLES: usize = 50_000_000;

/// Parse binary or ASCII STL into one persistent CAD MESH entity.
pub fn parse_stl(bytes: &[u8]) -> Result<Mesh, String> {
    if bytes.len() >= 84 {
        let triangle_count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        if triangle_count <= MAX_IMPORT_TRIANGLES
            && 84usize
                .checked_add(triangle_count.saturating_mul(50))
                .is_some_and(|expected| expected == bytes.len())
        {
            return parse_binary_stl(bytes, triangle_count);
        }
    }
    parse_ascii_stl(bytes)
}

pub(crate) fn mesh_from_triangles(
    triangles: impl IntoIterator<Item = [[f64; 3]; 3]>,
) -> Result<Mesh, String> {
    use std::collections::HashMap;
    let mut mesh = Mesh::new();
    mesh.blend_crease = false;
    let mut vertices: HashMap<[u64; 3], usize> = HashMap::new();
    for triangle in triangles {
        let mut face = [0usize; 3];
        for (corner, point) in triangle.into_iter().enumerate() {
            if !point.iter().all(|value| value.is_finite()) {
                return Err("STL contains a non-finite vertex".to_string());
            }
            let key = point.map(f64::to_bits);
            face[corner] = *vertices.entry(key).or_insert_with(|| {
                let index = mesh.vertices.len();
                mesh.vertices
                    .push(Vector3::new(point[0], point[1], point[2]));
                index
            });
        }
        if face[0] != face[1] && face[1] != face[2] && face[2] != face[0] {
            mesh.faces
                .push(MeshFace::triangle(face[0], face[1], face[2]));
        }
    }
    if mesh.faces.is_empty() {
        return Err("STL contains no usable triangles".to_string());
    }
    Ok(mesh)
}

fn parse_binary_stl(bytes: &[u8], triangle_count: usize) -> Result<Mesh, String> {
    let triangles = (0..triangle_count).map(|triangle| {
        let start = 84 + triangle * 50 + 12;
        std::array::from_fn(|corner| {
            std::array::from_fn(|axis| {
                let at = start + (corner * 3 + axis) * 4;
                f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as f64
            })
        })
    });
    mesh_from_triangles(triangles)
}

fn parse_ascii_stl(bytes: &[u8]) -> Result<Mesh, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "STL is neither valid binary nor UTF-8 ASCII".to_string())?;
    let mut points = Vec::new();
    for line in text.lines() {
        let mut words = line.split_ascii_whitespace();
        if !words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("vertex"))
        {
            continue;
        }
        let values: Result<Vec<f64>, _> = words.take(3).map(str::parse).collect();
        let values = values.map_err(|_| "ASCII STL contains an invalid vertex".to_string())?;
        if values.len() != 3 {
            return Err("ASCII STL vertex does not contain three coordinates".to_string());
        }
        points.push([values[0], values[1], values[2]]);
        if points.len() / 3 > MAX_IMPORT_TRIANGLES {
            return Err("STL exceeds the import triangle safety limit".to_string());
        }
    }
    if points.len() % 3 != 0 {
        return Err("ASCII STL contains an incomplete facet".to_string());
    }
    mesh_from_triangles(points.chunks_exact(3).map(|p| [p[0], p[1], p[2]]))
}

/// Build a binary STL byte buffer from a slice of mesh models.
/// Returns `None` if there are no triangles to export.
pub fn build_stl(meshes: &[&MeshModel]) -> Option<Vec<u8>> {
    // Collect all triangles.
    struct Tri {
        normal: [f32; 3],
        v: [[f32; 3]; 3],
    }

    let mut tris: Vec<Tri> = Vec::new();

    for mesh in meshes {
        let verts = &mesh.verts;
        // Reattach the double-single low residuals before writing: `verts`
        // alone is the truncated high half, which displaces survey-scale
        // (UTM/state-plane) geometry by up to a few metres. Reconstructing
        // in f64 and re-rounding to the f32 STL format writes the nearest
        // representable vertex instead of the truncated one.
        let low = &mesh.verts_low;
        let point = |index: usize| -> Option<[f32; 3]> {
            let high = *verts.get(index)?;
            let exact = if low.len() == verts.len() {
                let residual = low[index];
                [
                    high[0] as f64 + residual[0] as f64,
                    high[1] as f64 + residual[1] as f64,
                    high[2] as f64 + residual[2] as f64,
                ]
            } else {
                [high[0] as f64, high[1] as f64, high[2] as f64]
            };
            Some([exact[0] as f32, exact[1] as f32, exact[2] as f32])
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

            // STL normals are per-facet: the geometric cross product, never
            // the smoothed vertex normal (which shades a faceted export with
            // a neighbouring surface's direction).
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let nx = ab[1] * ac[2] - ab[2] * ac[1];
            let ny = ab[2] * ac[0] - ab[0] * ac[2];
            let nz = ab[0] * ac[1] - ab[1] * ac[0];
            let len = (nx * nx + ny * ny + nz * nz).sqrt().max(f32::EPSILON);
            let normal = [nx / len, ny / len, nz / len];

            tris.push(Tri {
                normal,
                v: [a, b, c],
            });
        }
    }

    if tris.is_empty() {
        return None;
    }

    let mut buf: Vec<u8> = Vec::with_capacity(84 + tris.len() * 50);

    // 80-byte header.
    let mut header = [0u8; 80];
    let title = b"Open CAD Studio STL export";
    header[..title.len()].copy_from_slice(title);
    buf.extend_from_slice(&header);

    // Triangle count.
    buf.extend_from_slice(&(tris.len() as u32).to_le_bytes());

    for tri in &tris {
        // Normal.
        for &f in &tri.normal {
            buf.write_all(&f.to_le_bytes()).ok()?;
        }
        // Vertices.
        for v in &tri.v {
            for &f in v {
                buf.write_all(&f.to_le_bytes()).ok()?;
            }
        }
        // Attribute byte count = 0.
        buf.extend_from_slice(&0u16.to_le_bytes());
    }

    Some(buf)
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn imports_ascii_stl() {
        let mesh = parse_stl(b"solid x\n facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 2 0 0\n vertex 0 2 0\n endloop\n endfacet\nendsolid\n").unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.faces.len(), 1);
    }

    #[test]
    fn imports_binary_stl() {
        let mut bytes = vec![0u8; 84 + 50];
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        for (slot, value) in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .enumerate()
        {
            let at = 84 + 12 + slot * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mesh = parse_stl(&bytes).unwrap();
        assert_eq!(mesh.faces.len(), 1);
    }
}
