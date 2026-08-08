use super::*;

use std::collections::{HashMap, HashSet};

const NODE_EPS: f64 = 1.0e-6;
const PARAM_EPS: f64 = 1.0e-9;
const AREA_EPS: f64 = 1.0e-10;

#[derive(Clone, Copy, Debug)]
struct P2 {
    x: f64,
    y: f64,
}

impl P2 {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
        }
    }

    fn distance2(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    a: P2,
    b: P2,
}

impl Segment {
    fn new(a: P2, b: P2) -> Self {
        Self { a, b }
    }
}

impl Scene {
    /// Build closed planar regions from the visible wire geometry.
    ///
    /// Unlike `closed_outlines()`, the source entities do not need to be closed
    /// individually. Intersections are inserted as temporary graph vertices and
    /// the bounded faces of that planar graph are returned as hatch candidates.
    ///
    /// Curved entities participate through their already-tessellated WireModel
    /// geometry, so arcs, circles, ellipses and splines can take part in the
    /// boundary search without modifying the source entities.
    pub fn hatch_boundary_outlines(&self) -> Vec<Vec<[f32; 2]>> {
        let wires = self.entity_wires();
        let mut segments = Vec::<Segment>::new();

        for wire in wires.iter() {
            let mut previous: Option<P2> = None;

            for (index, high) in wire.points.iter().copied().enumerate() {
                // NaNs delimit independent segments inside some WireModels,
                // notably polylines stored as A-B | B-C | C-D.
                if !high[0].is_finite() || !high[1].is_finite() {
                    previous = None;
                    continue;
                }

                let low = wire.points_low.get(index).copied().unwrap_or([0.0; 3]);

                let current = P2::new(
                    high[0] as f64 + low[0] as f64,
                    high[1] as f64 + low[1] as f64,
                );

                if let Some(prev) = previous {
                    if prev.distance2(current) > NODE_EPS * NODE_EPS {
                        segments.push(Segment::new(prev, current));
                    }
                }

                previous = Some(current);
            }
        }

        build_planar_outlines(&segments)
            .into_iter()
            .map(|ring| ring.into_iter().map(|p| [p.x as f32, p.y as f32]).collect())
            .collect()
    }
}

/// Build all bounded faces produced by a collection of planar segments.
///
/// Every intersection splits the participating segments virtually. The
/// resulting pieces are assembled into an undirected planar graph and its
/// bounded faces are traced using a half-edge walk.
fn build_planar_outlines(segments: &[Segment]) -> Vec<Vec<P2>> {
    if segments.is_empty() {
        return Vec::new();
    }

    // Each original segment starts with its two endpoints as split positions.
    let mut cuts: Vec<Vec<f64>> = vec![vec![0.0, 1.0]; segments.len()];

    // Find all pairwise intersections. The AABB test avoids doing the more
    // expensive intersection calculation for obviously unrelated segments.
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            if !segment_aabbs_overlap(segments[i], segments[j]) {
                continue;
            }

            if let Some((ti, tj)) = segment_intersection(segments[i], segments[j]) {
                cuts[i].push(ti.clamp(0.0, 1.0));
                cuts[j].push(tj.clamp(0.0, 1.0));
            }
        }
    }

    // Split every segment at all of its intersection parameters.
    let mut pieces = Vec::<Segment>::new();

    for (segment, params) in segments.iter().copied().zip(cuts.iter_mut()) {
        params.sort_by(|a, b| a.total_cmp(b));
        params.dedup_by(|a, b| (*a - *b).abs() <= PARAM_EPS);

        for pair in params.windows(2) {
            let t0 = pair[0];
            let t1 = pair[1];

            if t1 - t0 <= PARAM_EPS {
                continue;
            }

            let a = segment.a.lerp(segment.b, t0);
            let b = segment.a.lerp(segment.b, t1);

            if a.distance2(b) > NODE_EPS * NODE_EPS {
                pieces.push(Segment::new(a, b));
            }
        }
    }

    // Convert split segment endpoints to graph nodes.
    let mut nodes = Vec::<P2>::new();
    let mut node_map = HashMap::<(i64, i64), usize>::new();
    let mut edges = HashSet::<(usize, usize)>::new();

    for piece in pieces {
        let a = node_for_point(piece.a, &mut nodes, &mut node_map);
        let b = node_for_point(piece.b, &mut nodes, &mut node_map);

        if a == b {
            continue;
        }

        let edge = if a < b { (a, b) } else { (b, a) };
        edges.insert(edge);
    }

    if edges.is_empty() {
        return Vec::new();
    }

    // Build adjacency lists.
    let mut adjacency = vec![Vec::<usize>::new(); nodes.len()];

    for &(a, b) in &edges {
        adjacency[a].push(b);
        adjacency[b].push(a);
    }

    // A planar half-edge walk needs the neighbors around every vertex sorted
    // by polar angle.
    for (vertex, neighbors) in adjacency.iter_mut().enumerate() {
        let origin = nodes[vertex];

        neighbors.sort_by(|&a, &b| {
            let aa = (nodes[a].y - origin.y).atan2(nodes[a].x - origin.x);
            let ab = (nodes[b].y - origin.y).atan2(nodes[b].x - origin.x);
            aa.total_cmp(&ab)
        });

        neighbors.dedup();
    }

    let mut visited = HashSet::<(usize, usize)>::new();
    let mut faces = Vec::<Vec<P2>>::new();

    // Each undirected edge represents two directed half-edges. Walking each
    // unused half-edge while always taking the clockwise neighbor at the next
    // vertex traces one face.
    for u in 0..nodes.len() {
        for &v in &adjacency[u] {
            if visited.contains(&(u, v)) {
                continue;
            }

            let start = (u, v);
            let mut current = start;
            let mut ring = Vec::<P2>::new();
            let mut closed = false;

            // A valid planar face cannot require more directed edges than exist
            // in the complete graph. This also protects against malformed input.
            let max_steps = edges.len() * 2 + 1;

            for _ in 0..max_steps {
                if visited.contains(&current) {
                    break;
                }

                visited.insert(current);

                let (from, to) = current;
                ring.push(nodes[from]);

                let neighbors = &adjacency[to];
                if neighbors.is_empty() {
                    break;
                }

                let Some(incoming_index) = neighbors.iter().position(|&neighbor| neighbor == from)
                else {
                    break;
                };

                // Neighbors are sorted counter-clockwise. Taking the previous
                // one keeps the bounded face on the left side of the half-edge.
                let next_index = if incoming_index == 0 {
                    neighbors.len() - 1
                } else {
                    incoming_index - 1
                };

                let next = neighbors[next_index];
                current = (to, next);

                if current == start {
                    closed = true;
                    break;
                }
            }

            if !closed || ring.len() < 3 {
                continue;
            }

            let area = signed_area(&ring);

            // The reverse traversal produces the unbounded exterior face.
            // With the walk rule above, bounded faces are counter-clockwise.
            if area > AREA_EPS {
                faces.push(ring);
            }
        }
    }

    faces
}

fn node_for_point(p: P2, nodes: &mut Vec<P2>, map: &mut HashMap<(i64, i64), usize>) -> usize {
    let key = node_key(p);

    if let Some(&index) = map.get(&key) {
        return index;
    }

    let index = nodes.len();
    nodes.push(p);
    map.insert(key, index);
    index
}

fn node_key(p: P2) -> (i64, i64) {
    (
        (p.x / NODE_EPS).round() as i64,
        (p.y / NODE_EPS).round() as i64,
    )
}

fn segment_aabbs_overlap(a: Segment, b: Segment) -> bool {
    let a_min_x = a.a.x.min(a.b.x);
    let a_max_x = a.a.x.max(a.b.x);
    let a_min_y = a.a.y.min(a.b.y);
    let a_max_y = a.a.y.max(a.b.y);

    let b_min_x = b.a.x.min(b.b.x);
    let b_max_x = b.a.x.max(b.b.x);
    let b_min_y = b.a.y.min(b.b.y);
    let b_max_y = b.a.y.max(b.b.y);

    a_max_x + NODE_EPS >= b_min_x
        && b_max_x + NODE_EPS >= a_min_x
        && a_max_y + NODE_EPS >= b_min_y
        && b_max_y + NODE_EPS >= a_min_y
}

/// Intersection parameters of two finite XY segments.
///
/// Collinear overlaps intentionally return None. Shared endpoints and
/// T-junctions are handled by the normal non-parallel case and/or node merging.
fn segment_intersection(a: Segment, b: Segment) -> Option<(f64, f64)> {
    let rx = a.b.x - a.a.x;
    let ry = a.b.y - a.a.y;

    let sx = b.b.x - b.a.x;
    let sy = b.b.y - b.a.y;

    let cross = rx * sy - ry * sx;

    if cross.abs() <= 1.0e-12 {
        return None;
    }

    let qpx = b.a.x - a.a.x;
    let qpy = b.a.y - a.a.y;

    let t = (qpx * sy - qpy * sx) / cross;
    let u = (qpx * ry - qpy * rx) / cross;

    if t >= -PARAM_EPS && t <= 1.0 + PARAM_EPS && u >= -PARAM_EPS && u <= 1.0 + PARAM_EPS {
        Some((t, u))
    } else {
        None
    }
}

fn signed_area(poly: &[P2]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }

    let mut area = 0.0;

    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        area += a.x * b.y - b.x * a.y;
    }

    area * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ax: f64, ay: f64, bx: f64, by: f64) -> Segment {
        Segment::new(P2::new(ax, ay), P2::new(bx, by))
    }

    #[test]
    fn four_unjoined_lines_form_one_face() {
        let segments = vec![
            s(0.0, 0.0, 10.0, 0.0),
            s(10.0, 0.0, 10.0, 10.0),
            s(10.0, 10.0, 0.0, 10.0),
            s(0.0, 10.0, 0.0, 0.0),
        ];

        let faces = build_planar_outlines(&segments);

        assert_eq!(faces.len(), 1);
        assert!((signed_area(&faces[0]) - 100.0).abs() < 1.0e-6);
    }

    #[test]
    fn lines_may_extend_past_the_boundary() {
        let segments = vec![
            s(-5.0, 0.0, 15.0, 0.0),
            s(-5.0, 10.0, 15.0, 10.0),
            s(0.0, -5.0, 0.0, 15.0),
            s(10.0, -5.0, 10.0, 15.0),
        ];

        let faces = build_planar_outlines(&segments);

        assert_eq!(faces.len(), 1);
        assert!((signed_area(&faces[0]) - 100.0).abs() < 1.0e-6);
    }

    #[test]
    fn open_geometry_does_not_create_a_face() {
        let segments = vec![
            s(0.0, 0.0, 10.0, 0.0),
            s(10.0, 0.0, 10.0, 10.0),
            s(10.0, 10.0, 0.0, 10.0),
        ];

        let faces = build_planar_outlines(&segments);

        assert!(faces.is_empty());
    }
}
