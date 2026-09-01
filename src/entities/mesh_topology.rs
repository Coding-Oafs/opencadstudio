//! Mesh topology analysis and repair operations (3D development plan, Week 2).
//!
//! Everything here is a pure, deterministic function of the entity's vertex
//! and face lists: no display caches, no UI state. Commands layer undo,
//! selection, and cache invalidation on top via
//! [`crate::scene::Scene::update_entity`], which records before-images and
//! rebuilds only the touched handle's LOD set.

use acadrust::entities::mesh::{Mesh, MeshFace};
use rustc_hash::FxHashMap;

/// Per-undirected-edge bookkeeping kept while walking the face list.
#[derive(Debug, Clone, Default)]
struct EdgeUse {
    /// Incident face indices, in first-seen order.
    faces: Vec<u32>,
    /// `directions[i]` is true when `faces[i]` traverses the edge lo -> hi.
    directions: Vec<bool>,
}

/// Face-graph snapshot used by diagnostics and the repair operations.
#[derive(Debug, Clone)]
pub(crate) struct MeshTopology {
    pub vertices: usize,
    pub faces: usize,
    /// Unique undirected edges referenced by faces.
    pub edges: usize,
    /// Edges used by exactly one face (holes / open borders).
    pub boundary_edges: usize,
    /// Edges used by more than two faces.
    pub non_manifold_edges: usize,
    /// Faces with a repeated vertex index.
    pub degenerate_faces: usize,
    /// Faces containing a vertex index outside the mesh vertex array.
    pub invalid_index_faces: usize,
    /// Faces whose vertex set repeats an earlier face's vertex set.
    pub duplicate_faces: usize,
    /// Vertices referenced by no face.
    pub isolated_vertices: usize,
    /// Face-connected components (adjacency through edges shared by two faces).
    pub components: usize,
    /// Closed manifold: non-empty, no boundary, no non-manifold edges.
    pub watertight: bool,
    /// Interior edges traversed in the same direction by both incident faces.
    pub orientation_conflicts: usize,
    edge_uses: FxHashMap<(u32, u32), EdgeUse>,
    /// Boundary half-edges exactly as traversed by their single face.
    boundary_directed: FxHashMap<u32, u32>,
    /// True when a boundary vertex has more than one outgoing boundary edge,
    /// so boundary loops cannot be chained unambiguously.
    boundary_non_manifold: bool,
}

impl MeshTopology {
    pub(crate) fn analyze(mesh: &Mesh) -> Self {
        let mut topology = Self {
            vertices: mesh.vertices.len(),
            faces: mesh.faces.len(),
            edges: 0,
            boundary_edges: 0,
            non_manifold_edges: 0,
            degenerate_faces: 0,
            invalid_index_faces: 0,
            duplicate_faces: 0,
            isolated_vertices: 0,
            components: 0,
            watertight: false,
            orientation_conflicts: 0,
            edge_uses: FxHashMap::default(),
            boundary_directed: FxHashMap::default(),
            boundary_non_manifold: false,
        };

        let mut referenced = vec![false; mesh.vertices.len()];
        let mut vertex_sets: FxHashMap<Vec<usize>, ()> = FxHashMap::default();

        for (face_index, face) in mesh.faces.iter().enumerate() {
            let indices = &face.vertices;
            if indices.iter().any(|&index| index >= mesh.vertices.len()) {
                topology.invalid_index_faces += 1;
                continue;
            }
            let mut seen_in_face = FxHashMap::<usize, ()>::default();
            let mut degenerate = false;
            for &index in indices {
                if seen_in_face.insert(index, ()).is_some() {
                    degenerate = true;
                }
                if let Some(slot) = referenced.get_mut(index) {
                    *slot = true;
                }
            }
            if degenerate {
                topology.degenerate_faces += 1;
            } else {
                // Sorted vertex sets catch duplicates regardless of winding;
                // rotated copies of one cycle share the same set.
                let mut key = indices.clone();
                key.sort_unstable();
                if vertex_sets.insert(key, ()).is_some() {
                    topology.duplicate_faces += 1;
                }
            }

            for corner in 0..indices.len() {
                let a = indices[corner];
                let b = indices[(corner + 1) % indices.len()];
                if a == b {
                    continue;
                }
                let (lo, hi, forward) = if a < b {
                    (a as u32, b as u32, true)
                } else {
                    (b as u32, a as u32, false)
                };
                let use_ = topology.edge_uses.entry((lo, hi)).or_default();
                use_.faces.push(face_index as u32);
                use_.directions.push(forward);
            }
        }

        topology.isolated_vertices = referenced.iter().filter(|used| !**used).count();
        topology.edges = topology.edge_uses.len();

        for (&(lo, hi), use_) in &topology.edge_uses {
            match use_.faces.len() {
                1 => {
                    topology.boundary_edges += 1;
                    // Record the half-edge exactly as its face traverses it.
                    let (from, to) = if use_.directions[0] {
                        (lo, hi)
                    } else {
                        (hi, lo)
                    };
                    if topology.boundary_directed.insert(from, to).is_some() {
                        topology.boundary_non_manifold = true;
                    }
                }
                2 => {
                    if use_.directions[0] == use_.directions[1] {
                        topology.orientation_conflicts += 1;
                    }
                }
                _ => topology.non_manifold_edges += 1,
            }
        }
        let mut incoming = FxHashMap::<u32, usize>::default();
        for &to in topology.boundary_directed.values() {
            let count = incoming.entry(to).or_default();
            *count += 1;
            if *count > 1 {
                topology.boundary_non_manifold = true;
            }
        }
        if topology
            .boundary_directed
            .keys()
            .any(|vertex| incoming.get(vertex).copied().unwrap_or(0) != 1)
        {
            topology.boundary_non_manifold = true;
        }

        // Face components through manifold interior edges only.
        let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); mesh.faces.len()];
        for use_ in topology.edge_uses.values() {
            if use_.faces.len() == 2 {
                adjacency[use_.faces[0] as usize].push(use_.faces[1]);
                adjacency[use_.faces[1] as usize].push(use_.faces[0]);
            }
        }
        let mut assigned = vec![u32::MAX; mesh.faces.len()];
        for seed in 0..mesh.faces.len() {
            if assigned[seed] != u32::MAX {
                continue;
            }
            let component = topology.components as u32;
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(seed as u32);
            assigned[seed] = component;
            while let Some(face) = queue.pop_front() {
                for &neighbor in &adjacency[face as usize] {
                    if assigned[neighbor as usize] == u32::MAX {
                        assigned[neighbor as usize] = component;
                        queue.push_back(neighbor);
                    }
                }
            }
            topology.components += 1;
        }

        topology.watertight = !mesh.faces.is_empty()
            && topology.invalid_index_faces == 0
            && topology.degenerate_faces == 0
            && topology.duplicate_faces == 0
            && topology.boundary_edges == 0
            && topology.non_manifold_edges == 0
            && topology.orientation_conflicts == 0;

        topology
    }

    /// Boundary loops as ordered vertex cycles following the direction the
    /// existing faces traverse. Hole fills must use the reversed order.
    /// Returns `None` when boundary chaining is ambiguous.
    pub(crate) fn boundary_loops(&self) -> Option<Vec<Vec<usize>>> {
        if self.boundary_non_manifold {
            return None;
        }
        let mut loops = Vec::new();
        let mut consumed = std::collections::HashSet::new();
        let mut starts: Vec<u32> = self.boundary_directed.keys().copied().collect();
        starts.sort_unstable();
        for start in starts {
            if consumed.contains(&start) {
                continue;
            }
            let mut cycle = Vec::new();
            let mut current = start;
            loop {
                if consumed.contains(&current) {
                    if current == start {
                        break;
                    }
                    return None;
                }
                consumed.insert(current);
                cycle.push(current as usize);
                let Some(&next) = self.boundary_directed.get(&current) else {
                    return None;
                };
                current = next;
                if current == start {
                    break;
                }
            }
            loops.push(cycle);
        }
        Some(loops)
    }

    /// One line per fact for the MESHDIAGNOSE command report.
    pub(crate) fn report_lines(&self) -> Vec<String> {
        vec![
            format!(
                "MESHDIAGNOSE: {} vertices, {} faces, {} edges",
                self.vertices, self.faces, self.edges
            ),
            format!(
                "MESHDIAGNOSE: {} component(s); watertight: {}; boundary edges: {}; non-manifold edges: {}",
                self.components,
                if self.watertight { "yes" } else { "no" },
                self.boundary_edges,
                self.non_manifold_edges
            ),
            format!(
                "MESHDIAGNOSE: invalid-index faces: {}; degenerate faces: {}; duplicate faces: {}; isolated vertices: {}; orientation conflicts: {}",
                self.invalid_index_faces,
                self.degenerate_faces,
                self.duplicate_faces,
                self.isolated_vertices,
                self.orientation_conflicts
            ),
        ]
    }

    /// True when the edge (lo, hi) is used by exactly two faces and `face` is
    /// one of them; returns the other face index.
    fn manifold_neighbor(&self, lo: u32, hi: u32, face: usize) -> Option<usize> {
        let use_ = self.edge_uses.get(&(lo, hi))?;
        if use_.faces.len() != 2 {
            return None;
        }
        if use_.faces[0] as usize == face {
            Some(use_.faces[1] as usize)
        } else if use_.faces[1] as usize == face {
            Some(use_.faces[0] as usize)
        } else {
            None
        }
    }
}

/// Traversal direction of one undirected edge inside one face: true when the
/// face walks it lo -> hi.
fn edge_direction_in_face(mesh: &Mesh, face: usize, lo: usize, hi: usize) -> bool {
    let indices = &mesh.faces[face].vertices;
    for corner in 0..indices.len() {
        let a = indices[corner];
        let b = indices[(corner + 1) % indices.len()];
        let (a_lo, a_hi) = if a < b { (a, b) } else { (b, a) };
        if a_lo == lo && a_hi == hi {
            return a_lo == a;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WeldReport {
    pub merged_vertices: usize,
    pub dropped_faces: usize,
}

/// Merges vertices closer than `tolerance` (0 = exact coincident merge),
/// drops faces that become degenerate or duplicates, and compacts the vertex
/// list. Deterministic: every merge targets the lowest matching index.
pub(crate) fn weld_vertices(mesh: &mut Mesh, tolerance: f64) -> WeldReport {
    let mut report = WeldReport::default();
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 {
        tolerance
    } else {
        0.0
    };

    let mut remap: Vec<usize> = Vec::with_capacity(mesh.vertices.len());
    if tolerance == 0.0 {
        let mut exact: FxHashMap<[u64; 3], usize> = FxHashMap::default();
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            let key = [vertex.x.to_bits(), vertex.y.to_bits(), vertex.z.to_bits()];
            let representative = *exact.entry(key).or_insert(index);
            remap.push(representative);
            if representative != index {
                report.merged_vertices += 1;
            }
        }
    } else {
        // Quantized spatial hash with a 27-cell neighbourhood probe, so pairs
        // straddling a cell boundary still merge.
        let mut cells: FxHashMap<[i64; 3], Vec<usize>> = FxHashMap::default();
        let cell_of = |x: f64, y: f64, z: f64| {
            [
                (x / tolerance).floor() as i64,
                (y / tolerance).floor() as i64,
                (z / tolerance).floor() as i64,
            ]
        };
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            let center = cell_of(vertex.x, vertex.y, vertex.z);
            let mut best: Option<usize> = None;
            'search: for dx in -1i64..=1 {
                for dy in -1i64..=1 {
                    for dz in -1i64..=1 {
                        let probe = [center[0] + dx, center[1] + dy, center[2] + dz];
                        let Some(candidates) = cells.get(&probe) else {
                            continue;
                        };
                        for &candidate in candidates {
                            let other = mesh.vertices[candidate];
                            let dxv = vertex.x - other.x;
                            let dyv = vertex.y - other.y;
                            let dzv = vertex.z - other.z;
                            let squared = dxv * dxv + dyv * dyv + dzv * dzv;
                            if squared <= tolerance * tolerance
                                && best.is_none_or(|current| candidate < current)
                            {
                                best = Some(candidate);
                                if candidate == 0 {
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
            match best {
                Some(representative) => {
                    remap.push(representative);
                    report.merged_vertices += 1;
                }
                None => {
                    cells.entry(center).or_default().push(index);
                    remap.push(index);
                }
            }
        }
    }

    apply_vertex_remap(mesh, &remap, &mut report.dropped_faces);
    report
}

/// Applies a vertex index remap (old index -> kept old index), drops
/// degenerate and duplicate faces, removes unreferenced vertices, and
/// rebuilds the edge list.
fn apply_vertex_remap(mesh: &mut Mesh, remap: &[usize], dropped_faces: &mut usize) {
    let mut kept_faces: Vec<MeshFace> = Vec::with_capacity(mesh.faces.len());
    let mut seen_vertex_sets: FxHashMap<Vec<usize>, ()> = FxHashMap::default();
    for face in &mesh.faces {
        let Some(mapped) = face
            .vertices
            .iter()
            .map(|&vertex| remap.get(vertex).copied())
            .collect::<Option<Vec<_>>>()
        else {
            *dropped_faces += 1;
            continue;
        };
        let mut unique = mapped.clone();
        unique.sort_unstable();
        unique.dedup();
        // Degenerate after the merge (a repeated index) or too small to keep.
        if mapped.len() < 3 || unique.len() != mapped.len() {
            *dropped_faces += 1;
            continue;
        }
        // A duplicate uses the same vertices regardless of start corner or
        // winding. This also removes an oppositely wound coincident face.
        if seen_vertex_sets.insert(unique, ()).is_some() {
            *dropped_faces += 1;
            continue;
        }
        let mut face = face.clone();
        face.vertices = mapped;
        kept_faces.push(face);
    }
    mesh.faces = kept_faces;
    compact_vertices(mesh);
}

/// Removes vertices referenced by no face and reindexes the face list.
fn compact_vertices(mesh: &mut Mesh) {
    // Imported files are validated, but hand-authored/plugin meshes can still
    // be malformed. Never let a repair command index outside the vertex list.
    mesh.faces.retain(|face| {
        face.vertices.len() >= 3
            && face
                .vertices
                .iter()
                .all(|&index| index < mesh.vertices.len())
    });
    let mut used = vec![false; mesh.vertices.len()];
    for face in &mesh.faces {
        for &index in &face.vertices {
            used[index] = true;
        }
    }
    let mut compact = vec![usize::MAX; mesh.vertices.len()];
    let mut next = 0usize;
    for (index, is_used) in used.iter().enumerate() {
        if *is_used {
            compact[index] = next;
            next += 1;
        }
    }
    mesh.vertices = mesh
        .vertices
        .iter()
        .enumerate()
        .filter(|(index, _)| used[*index])
        .map(|(_, vertex)| *vertex)
        .collect();
    for face in &mut mesh.faces {
        for index in &mut face.vertices {
            *index = compact[*index];
        }
    }
    mesh.edges.clear();
    mesh.compute_edges();
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FillReport {
    pub filled_loops: usize,
    pub added_faces: usize,
}

/// Closes every boundary loop of at most `max_boundary_edges` edges (3 or 4
/// become a single face; longer loops get a centroid fan). Fill faces walk
/// each boundary edge opposite to its existing face, keeping orientation
/// consistent.
pub(crate) fn fill_small_holes(
    mesh: &mut Mesh,
    max_boundary_edges: usize,
) -> Result<FillReport, String> {
    let topology = MeshTopology::analyze(mesh);
    let Some(loops) = topology.boundary_loops() else {
        return Err(
            "mesh boundary is non-manifold; hole filling needs an unambiguous boundary".to_string(),
        );
    };
    let mut report = FillReport::default();
    for cycle in loops {
        if cycle.len() < 3 || cycle.len() > max_boundary_edges {
            continue;
        }
        // The cycle follows the direction used by existing faces, so the fill
        // traverses the reversed cycle.
        let mut fill: Vec<usize> = cycle.clone();
        fill.reverse();
        if fill.len() <= 4 {
            mesh.add_face(MeshFace::new(fill));
            report.added_faces += 1;
        } else {
            let mut centroid = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
            for &index in &fill {
                centroid.x += mesh.vertices[index].x;
                centroid.y += mesh.vertices[index].y;
                centroid.z += mesh.vertices[index].z;
            }
            let n = fill.len() as f64;
            centroid.x /= n;
            centroid.y /= n;
            centroid.z /= n;
            let center = mesh.add_vertex(centroid);
            // Fan over consecutive pairs of the fill cycle, including the
            // wrapping pair (last, first).
            for window in fill.windows(2) {
                mesh.add_triangle(center, window[0], window[1]);
                report.added_faces += 1;
            }
            let (first, last) = (
                *fill.first().expect("non-empty cycle"),
                *fill.last().expect("non-empty cycle"),
            );
            mesh.add_triangle(center, last, first);
            report.added_faces += 1;
        }
        report.filled_loops += 1;
    }
    if report.filled_loops > 0 {
        mesh.edges.clear();
        mesh.compute_edges();
    }
    Ok(report)
}

/// Removes the given faces (0-based) and compacts unreferenced vertices.
/// Returns how many distinct in-range faces were removed.
pub(crate) fn delete_faces(mesh: &mut Mesh, face_indices: &[usize]) -> usize {
    let mut remove = vec![false; mesh.faces.len()];
    let mut removed = 0usize;
    for &index in face_indices {
        if index < mesh.faces.len() && !remove[index] {
            remove[index] = true;
            removed += 1;
        }
    }
    if removed == 0 {
        return 0;
    }
    let kept_faces: Vec<MeshFace> = mesh
        .faces
        .iter()
        .enumerate()
        .filter(|(index, _)| !remove[*index])
        .map(|(_, face)| face.clone())
        .collect();
    mesh.faces = kept_faces;
    compact_vertices(mesh);
    removed
}

/// Propagates a consistent winding across each face-connected component by
/// BFS, flipping faces as needed. Non-manifold edges are not crossed.
/// Returns the number of flipped faces.
pub(crate) fn orient_consistently(mesh: &mut Mesh) -> usize {
    let topology = MeshTopology::analyze(mesh);
    let mut flipped = 0usize;
    let mut visited = vec![false; mesh.faces.len()];
    for seed in 0..mesh.faces.len() {
        if visited[seed] {
            continue;
        }
        visited[seed] = true;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(seed);
        while let Some(face) = queue.pop_front() {
            let indices = mesh.faces[face].vertices.clone();
            for corner in 0..indices.len() {
                let a = indices[corner];
                let b = indices[(corner + 1) % indices.len()];
                if a == b {
                    continue;
                }
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                let Some(other) = topology.manifold_neighbor(lo as u32, hi as u32, face) else {
                    continue;
                };
                if visited[other] {
                    continue;
                }
                visited[other] = true;
                // The neighbor must traverse the shared edge opposite to this
                // face for outward normals to agree.
                if edge_direction_in_face(mesh, face, lo, hi)
                    == edge_direction_in_face(mesh, other, lo, hi)
                {
                    mesh.faces[other].reverse();
                    flipped += 1;
                }
                queue.push_back(other);
            }
        }
    }
    if flipped > 0 {
        mesh.edges.clear();
        mesh.compute_edges();
    }
    flipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::types::Vector3;

    fn v(x: f64, y: f64, z: f64) -> Vector3 {
        Vector3::new(x, y, z)
    }

    /// Consistently oriented closed tetrahedron: faces
    /// (0,2,1), (0,1,3), (1,2,3), (2,0,3).
    fn tetrahedron() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.vertices = vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
        ];
        for face in [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]] {
            mesh.faces
                .push(MeshFace::triangle(face[0], face[1], face[2]));
        }
        mesh
    }

    #[test]
    fn closed_tetrahedron_is_watertight_and_oriented() {
        let topology = MeshTopology::analyze(&tetrahedron());
        assert!(topology.watertight);
        assert_eq!(0, topology.boundary_edges);
        assert_eq!(0, topology.orientation_conflicts);
        assert_eq!(1, topology.components);
        assert_eq!(0, topology.non_manifold_edges);
        assert_eq!(0, topology.duplicate_faces);
        assert_eq!(0, topology.degenerate_faces);
    }

    #[test]
    fn analyze_counts_components_and_non_manifold_edges() {
        let mut mesh = Mesh::new();
        for vertex in [v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)] {
            mesh.add_vertex(vertex);
        }
        mesh.add_triangle(0, 1, 2);
        for vertex in [v(5.0, 0.0, 0.0), v(6.0, 0.0, 0.0), v(5.0, 1.0, 0.0)] {
            mesh.add_vertex(vertex);
        }
        mesh.add_triangle(3, 4, 5);
        // Two extra triangles fanned on edge 0-1: three faces share it.
        mesh.add_vertex(v(0.5, 0.0, 1.0));
        mesh.add_vertex(v(0.5, 0.0, -1.0));
        mesh.add_triangle(0, 1, 6);
        mesh.add_triangle(0, 1, 7);
        let topology = MeshTopology::analyze(&mesh);
        assert_eq!(
            4, topology.components,
            "the triple-used edge is non-manifold and must not join any faces"
        );
        assert_eq!(1, topology.non_manifold_edges);
        assert!(!topology.watertight);
    }

    #[test]
    fn analyze_reports_degenerate_duplicate_and_isolated() {
        let mut mesh = tetrahedron();
        mesh.faces.push(MeshFace::triangle(0, 1, 1)); // degenerate
        mesh.faces.push(MeshFace::triangle(2, 1, 0)); // duplicate of face 0's cycle
        mesh.vertices.push(v(9.0, 9.0, 9.0)); // isolated
        let topology = MeshTopology::analyze(&mesh);
        assert_eq!(1, topology.degenerate_faces);
        assert_eq!(1, topology.duplicate_faces);
        assert_eq!(1, topology.isolated_vertices);
    }

    #[test]
    fn malformed_indices_are_reported_and_repairs_do_not_panic() {
        let mut mesh = tetrahedron();
        mesh.faces.push(MeshFace::triangle(0, 1, 99));
        let topology = MeshTopology::analyze(&mesh);
        assert_eq!(1, topology.invalid_index_faces);
        assert!(!topology.watertight);

        let report = weld_vertices(&mut mesh, 0.0);
        assert_eq!(1, report.dropped_faces);
        assert!(mesh
            .faces
            .iter()
            .flat_map(|face| &face.vertices)
            .all(|index| *index < mesh.vertices.len()));
    }

    #[test]
    fn weld_removes_reversed_duplicate_faces() {
        let mut mesh = Mesh::new();
        mesh.vertices = vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)];
        mesh.faces.push(MeshFace::triangle(0, 1, 2));
        mesh.faces.push(MeshFace::triangle(2, 1, 0));
        let report = weld_vertices(&mut mesh, 0.0);
        assert_eq!(1, report.dropped_faces);
        assert_eq!(1, mesh.faces.len());
    }

    #[test]
    fn weld_merges_coincident_vertices_across_a_seam() {
        let mut mesh = Mesh::new();
        // Two triangles geometrically sharing edge A-B, but with duplicated
        // copies of both endpoints.
        mesh.vertices = vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 0.0), // A'
            v(1.0, 0.0, 0.0), // B'
            v(0.0, -1.0, 0.0),
        ];
        mesh.faces.push(MeshFace::triangle(0, 1, 2));
        mesh.faces.push(MeshFace::triangle(3, 5, 4));
        let report = weld_vertices(&mut mesh, 1.0e-9);
        assert_eq!(2, report.merged_vertices);
        assert_eq!(0, report.dropped_faces, "the seam faces stay distinct");
        assert_eq!(4, mesh.vertices.len());
        assert_eq!(2, mesh.faces.len());
        let topology = MeshTopology::analyze(&mesh);
        assert_eq!(
            1, topology.components,
            "the welded edge glues both triangles"
        );
        assert_eq!(4, topology.boundary_edges);
    }

    #[test]
    fn weld_drops_faces_that_collapse_or_duplicate() {
        let mut mesh = Mesh::new();
        mesh.vertices = vec![v(0.0, 0.0, 0.0), v(0.0, 0.0005, 0.0), v(1.0, 0.0, 0.0)];
        mesh.faces.push(MeshFace::triangle(0, 1, 2));
        let report = weld_vertices(&mut mesh, 0.001);
        assert_eq!(1, report.merged_vertices);
        assert_eq!(1, report.dropped_faces, "collapsed face is degenerate");
        assert!(mesh.faces.is_empty());

        let mut mesh = Mesh::new();
        mesh.vertices = vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)];
        mesh.faces.push(MeshFace::triangle(0, 1, 2));
        mesh.faces.push(MeshFace::triangle(1, 2, 0)); // rotated duplicate
        let report = weld_vertices(&mut mesh, 0.0);
        assert_eq!(0, report.merged_vertices);
        assert_eq!(1, report.dropped_faces);
        assert_eq!(1, mesh.faces.len());
    }

    #[test]
    fn fill_small_holes_closes_a_missing_triangle_watertight() {
        let mut mesh = tetrahedron();
        mesh.faces.remove(3);
        let topology = MeshTopology::analyze(&mesh);
        assert_eq!(3, topology.boundary_edges);
        assert_eq!(1, topology.boundary_loops().expect("loops").len());
        let report = fill_small_holes(&mut mesh, 8).expect("fill");
        assert_eq!(1, report.filled_loops);
        assert_eq!(1, report.added_faces);
        let repaired = MeshTopology::analyze(&mesh);
        assert!(repaired.watertight);
        assert_eq!(0, repaired.orientation_conflicts);
    }

    #[test]
    fn fill_closes_both_sides_of_a_strip_with_centroid_fans() {
        // Triangle strip between an inner and an outer pentagon: two boundary
        // loops, each of five edges.
        let mut mesh = Mesh::new();
        for step in 0..5 {
            let angle = step as f64 * std::f64::consts::TAU / 5.0;
            mesh.add_vertex(v(angle.cos(), angle.sin(), 0.0));
        }
        for step in 0..5 {
            let angle = step as f64 * std::f64::consts::TAU / 5.0;
            mesh.add_vertex(v(2.0 * angle.cos(), 2.0 * angle.sin(), 0.0));
        }
        for step in 0..5 {
            let next = (step + 1) % 5;
            mesh.add_triangle(5 + step, 5 + next, next);
            mesh.add_triangle(5 + step, next, step);
        }
        let topology = MeshTopology::analyze(&mesh);
        assert_eq!(0, topology.orientation_conflicts);
        assert_eq!(10, topology.boundary_edges);
        assert_eq!(2, topology.boundary_loops().expect("loops").len());
        let report = fill_small_holes(&mut mesh, 5).expect("fill");
        assert_eq!(2, report.filled_loops);
        assert_eq!(10, report.added_faces, "one fan triangle per boundary edge");
        let repaired = MeshTopology::analyze(&mesh);
        assert!(repaired.watertight);
        assert_eq!(0, repaired.orientation_conflicts);
        assert_eq!(12, mesh.vertices.len(), "5 inner + 5 outer + 2 centroids");
    }

    #[test]
    fn fill_respects_the_size_limit() {
        let mut mesh = tetrahedron();
        mesh.faces.remove(3);
        let report = fill_small_holes(&mut mesh, 2).expect("fill");
        assert_eq!(0, report.filled_loops);
        assert_eq!(3, mesh.faces.len(), "mesh untouched");
    }

    #[test]
    fn delete_faces_removes_and_compacts_unused_vertices() {
        let mut mesh = tetrahedron();
        mesh.vertices.push(v(9.0, 9.0, 9.0));
        assert_eq!(1, delete_faces(&mut mesh, &[0]));
        assert_eq!(3, mesh.faces.len());
        assert_eq!(4, mesh.vertices.len(), "isolated vertex compacted away");
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| (vertex.x - 9.0).abs() > 0.5));
        assert_eq!(0, delete_faces(&mut mesh, &[99]));
    }

    #[test]
    fn orient_consistently_flips_a_reversed_patch() {
        let mut mesh = tetrahedron();
        mesh.faces[1].reverse();
        let topology = MeshTopology::analyze(&mesh);
        assert!(topology.orientation_conflicts >= 2);
        let flipped = orient_consistently(&mut mesh);
        assert_eq!(1, flipped);
        let repaired = MeshTopology::analyze(&mesh);
        assert_eq!(0, repaired.orientation_conflicts);
        assert!(repaired.watertight);
    }
}
