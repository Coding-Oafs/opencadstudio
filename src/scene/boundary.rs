use super::*;

use std::collections::{HashMap, HashSet};

const NODE_EPS: f64 = 1.0e-6;
const ORIENTATION_EPS: f64 = 1.0e-12;
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

#[derive(Clone, Copy, Debug)]
struct SegmentAabb {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

impl SegmentAabb {
    fn new(segment: Segment) -> Self {
        Self {
            min_x: segment.a.x.min(segment.b.x),
            max_x: segment.a.x.max(segment.b.x),
            min_y: segment.a.y.min(segment.b.y),
            max_y: segment.a.y.max(segment.b.y),
        }
    }

    fn min(self, axis: SweepAxis) -> f64 {
        match axis {
            SweepAxis::X => self.min_x,
            SweepAxis::Y => self.min_y,
        }
    }

    fn max(self, axis: SweepAxis) -> f64 {
        match axis {
            SweepAxis::X => self.max_x,
            SweepAxis::Y => self.max_y,
        }
    }
}

#[derive(Clone, Copy)]
enum SweepAxis {
    X,
    Y,
}

enum SegmentIntersection {
    None,
    Point { a: f64, b: f64 },
    Overlap { a: [f64; 2], b: [f64; 2] },
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
    pub fn hatch_boundary_outlines(&self) -> Vec<Vec<[f64; 2]>> {
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
            .map(|ring| ring.into_iter().map(|p| [p.x, p.y]).collect())
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

    // Sweep the less-congested coordinate axis, then run the exact AABB and
    // segment tests only for active candidates. Dense intersections remain
    // output-sensitive, while spatially separated geometry avoids all-pairs
    // work.
    let aabbs: Vec<SegmentAabb> = segments.iter().copied().map(SegmentAabb::new).collect();
    let axis = choose_sweep_axis(&aabbs);
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| aabbs[a].min(axis).total_cmp(&aabbs[b].min(axis)));
    let mut active = Vec::<usize>::new();

    for i in order {
        let current_min = aabbs[i].min(axis);
        active.retain(|&j| aabbs[j].max(axis) + NODE_EPS >= current_min);

        for &j in &active {
            if !segment_aabbs_overlap(aabbs[i], aabbs[j]) {
                continue;
            }

            match segment_intersection(segments[i], segments[j]) {
                SegmentIntersection::None => {}
                SegmentIntersection::Point { a, b } => {
                    cuts[i].push(a.clamp(0.0, 1.0));
                    cuts[j].push(b.clamp(0.0, 1.0));
                }
                SegmentIntersection::Overlap { a, b } => {
                    cuts[i].push(a[0].clamp(0.0, 1.0));
                    cuts[i].push(a[1].clamp(0.0, 1.0));
                    cuts[j].push(b[0].clamp(0.0, 1.0));
                    cuts[j].push(b[1].clamp(0.0, 1.0));
                }
            }
        }

        active.push(i);
    }

    // Split every segment at all of its intersection parameters.
    let mut pieces = Vec::<Segment>::new();

    for (segment, params) in segments.iter().copied().zip(cuts.iter_mut()) {
        let segment_len = segment.a.distance2(segment.b).sqrt();
        let param_merge_eps = (NODE_EPS / segment_len).min(1.0);
        params.sort_by(|a, b| a.total_cmp(b));
        params.dedup_by(|a, b| (*a - *b).abs() <= param_merge_eps);

        for pair in params.windows(2) {
            let t0 = pair[0];
            let t1 = pair[1];

            if (t1 - t0) * segment_len <= NODE_EPS {
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
    let mut node_map = HashMap::<(i64, i64), Vec<usize>>::new();
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

fn node_for_point(
    p: P2,
    nodes: &mut Vec<P2>,
    map: &mut HashMap<(i64, i64), Vec<usize>>,
) -> usize {
    let key = node_key(p);

    for dx in -1_i64..=1 {
        for dy in -1_i64..=1 {
            let neighbor = (key.0.saturating_add(dx), key.1.saturating_add(dy));
            if let Some(indices) = map.get(&neighbor) {
                if let Some(&index) = indices
                    .iter()
                    .find(|&&index| nodes[index].distance2(p) <= NODE_EPS * NODE_EPS)
                {
                    return index;
                }
            }
        }
    }

    let index = nodes.len();
    nodes.push(p);
    map.entry(key).or_default().push(index);
    index
}

fn node_key(p: P2) -> (i64, i64) {
    (
        (p.x / NODE_EPS).floor() as i64,
        (p.y / NODE_EPS).floor() as i64,
    )
}

fn choose_sweep_axis(aabbs: &[SegmentAabb]) -> SweepAxis {
    if normalized_interval_span(aabbs, SweepAxis::X)
        <= normalized_interval_span(aabbs, SweepAxis::Y)
    {
        SweepAxis::X
    } else {
        SweepAxis::Y
    }
}

fn normalized_interval_span(aabbs: &[SegmentAabb], axis: SweepAxis) -> f64 {
    let min = aabbs
        .iter()
        .map(|aabb| aabb.min(axis))
        .fold(f64::INFINITY, f64::min);
    let max = aabbs
        .iter()
        .map(|aabb| aabb.max(axis))
        .fold(f64::NEG_INFINITY, f64::max);
    let extent = max - min;

    if !extent.is_finite() || extent <= NODE_EPS {
        return f64::INFINITY;
    }

    let total: f64 = aabbs
        .iter()
        .map(|aabb| aabb.max(axis) - aabb.min(axis) + NODE_EPS)
        .sum();
    total / extent
}

fn segment_aabbs_overlap(a: SegmentAabb, b: SegmentAabb) -> bool {
    a.max_x + NODE_EPS >= b.min_x
        && b.max_x + NODE_EPS >= a.min_x
        && a.max_y + NODE_EPS >= b.min_y
        && b.max_y + NODE_EPS >= a.min_y
}

/// Intersection parameters of two finite XY segments.
fn segment_intersection(a: Segment, b: Segment) -> SegmentIntersection {
    let rx = a.b.x - a.a.x;
    let ry = a.b.y - a.a.y;

    let sx = b.b.x - b.a.x;
    let sy = b.b.y - b.a.y;

    let r2 = rx * rx + ry * ry;
    let s2 = sx * sx + sy * sy;
    if r2 <= f64::EPSILON || s2 <= f64::EPSILON {
        return SegmentIntersection::None;
    }
    let r_len = r2.sqrt();
    let s_len = s2.sqrt();
    let cross = rx * sy - ry * sx;
    let qpx = b.a.x - a.a.x;
    let qpy = b.a.y - a.a.y;

    if cross.abs() <= ORIENTATION_EPS * r_len * s_len {
        if (qpx * ry - qpy * rx).abs() > NODE_EPS * r_len {
            return SegmentIntersection::None;
        }

        let b0_on_a = (qpx * rx + qpy * ry) / r2;
        let b1_on_a = b0_on_a + (sx * rx + sy * ry) / r2;
        let lo = b0_on_a.min(b1_on_a).max(0.0);
        let hi = b0_on_a.max(b1_on_a).min(1.0);
        let a_param_eps = NODE_EPS / r_len;

        if hi < lo - a_param_eps {
            return SegmentIntersection::None;
        }

        let lo = lo.clamp(0.0, 1.0);
        let hi = hi.clamp(0.0, 1.0);
        let lo_point = a.a.lerp(a.b, lo);
        let hi_point = a.a.lerp(a.b, hi);
        let b_lo = ((lo_point.x - b.a.x) * sx + (lo_point.y - b.a.y) * sy) / s2;
        let b_hi = ((hi_point.x - b.a.x) * sx + (hi_point.y - b.a.y) * sy) / s2;

        if (hi - lo) * r_len <= NODE_EPS {
            return SegmentIntersection::Point {
                a: (lo + hi) * 0.5,
                b: (b_lo + b_hi) * 0.5,
            };
        }

        return SegmentIntersection::Overlap {
            a: [lo, hi],
            b: [b_lo, b_hi],
        };
    }

    let t = (qpx * sy - qpy * sx) / cross;
    let u = (qpx * ry - qpy * rx) / cross;
    let t_eps = NODE_EPS / r_len;
    let u_eps = NODE_EPS / s_len;

    if t >= -t_eps && t <= 1.0 + t_eps && u >= -u_eps && u <= 1.0 + u_eps {
        SegmentIntersection::Point { a: t, b: u }
    } else {
        SegmentIntersection::None
    }
}

fn signed_area(poly: &[P2]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }

    let origin = poly[0];
    let mut area = 0.0;

    for i in 1..poly.len() - 1 {
        let a = poly[i];
        let b = poly[i + 1];
        area += (a.x - origin.x) * (b.y - origin.y)
            - (b.x - origin.x) * (a.y - origin.y);
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

    #[test]
    fn collinear_overlap_closes_rectangle() {
        let segments = vec![
            s(0.0, 0.0, 7.0, 0.0),
            s(3.0, 0.0, 10.0, 0.0),
            s(10.0, 0.0, 10.0, 10.0),
            s(10.0, 10.0, 0.0, 10.0),
            s(0.0, 10.0, 0.0, 0.0),
        ];

        let faces = build_planar_outlines(&segments);

        assert_eq!(faces.len(), 1);
        assert!((signed_area(&faces[0]) - 100.0).abs() < 1.0e-6);
    }

    #[test]
    fn node_merge_crosses_bucket_boundary() {
        let mut nodes = Vec::new();
        let mut map = HashMap::new();
        let a = node_for_point(P2::new(NODE_EPS * 0.99, 0.0), &mut nodes, &mut map);
        let b = node_for_point(P2::new(NODE_EPS * 1.01, 0.0), &mut nodes, &mut map);

        assert_eq!(a, b);
    }

    #[test]
    fn node_merge_rejects_distant_diagonal_points() {
        let mut nodes = Vec::new();
        let mut map = HashMap::new();
        let a = node_for_point(
            P2::new(NODE_EPS * 0.51, NODE_EPS * 0.51),
            &mut nodes,
            &mut map,
        );
        let b = node_for_point(
            P2::new(NODE_EPS * 1.49, NODE_EPS * 1.49),
            &mut nodes,
            &mut map,
        );

        assert_ne!(a, b);
    }

    #[test]
    fn signed_area_is_stable_at_large_coordinates() {
        let base = 1.0e12;
        let poly = vec![
            P2::new(base, base),
            P2::new(base + 3.0, base),
            P2::new(base + 3.0, base + 4.0),
            P2::new(base, base + 4.0),
        ];

        assert!((signed_area(&poly) - 12.0).abs() < 1.0e-9);
    }

    #[test]
    fn long_segments_keep_world_scale_cuts() {
        let segments = vec![
            s(0.0, 0.0, 1.0e12, 0.0),
            s(0.0, 1.0, 1.0e12, 1.0),
            s(1.0, -1.0, 1.0, 2.0),
            s(2.0, -1.0, 2.0, 2.0),
        ];

        let faces = build_planar_outlines(&segments);

        assert_eq!(faces.len(), 1);
        assert!((signed_area(&faces[0]) - 1.0).abs() < 1.0e-9);
    }

    #[test]
    fn long_near_collinear_segments_overlap_within_node_tolerance() {
        let a = s(0.0, 0.0, 1.0e9, 1.0e-3);
        let b = s(5.0e8, 5.0e-4 + 5.0e-7, 1.5e9, 1.5e-3 + 5.0e-7);

        assert!(matches!(
            segment_intersection(a, b),
            SegmentIntersection::Overlap { .. }
        ));
    }
}
