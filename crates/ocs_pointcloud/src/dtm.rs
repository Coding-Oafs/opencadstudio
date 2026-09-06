//! Ground-surface TIN and contour generation.
//!
//! [`Tin`] triangulates a class-filtered point set with Delaunay (via
//! `delaunator`) and interpolates heights by barycentric plane lookup.
//! [`generate_contours`] runs marching triangles over the TIN and chains the
//! crossings into polylines ready for CAD layers.

use crate::{surface::Breakline, SamplePoint};
use delaunator::EMPTY;
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};
use std::cell::Cell;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
struct TinVertex {
    position: Point2<f64>,
    z: f64,
}

impl HasPosition for TinVertex {
    type Scalar = f64;

    fn position(&self) -> Point2<Self::Scalar> {
        self.position
    }
}

/// A Delaunay surface over projected point positions.
pub struct Tin {
    points: Vec<[f64; 3]>,
    /// Triangle vertex indices into `points`.
    triangles: Vec<[usize; 3]>,
    /// Delaunator half-edge opposites, parallel to the flat vertex order the
    /// triangulation returned: `halfedges[3 * t + e]` indexes the half-edge
    /// across triangle `t`'s edge `e` (`EMPTY` on the hull). Drives the
    /// point-location walk.
    halfedges: Vec<usize>,
    /// Triangle ids bucketed by grid cell for point location.
    buckets: HashMap<(i64, i64), Vec<usize>>,
    cell_size: f64,
    /// Last triangle a query landed in. Consecutive drape samples walk from
    /// it because they usually land in or beside it. `Cell` keeps queries on
    /// `&self`; a `Tin` is used single-threaded per command.
    hint: Cell<Option<usize>>,
}

impl Tin {
    /// Builds the TIN from every point whose class is `class` (when given).
    /// Returns `None` when fewer than three points form a surface.
    pub fn from_points(points: &[SamplePoint], class: Option<u8>) -> Option<Self> {
        Self::from_points_with_breaklines(points, class, &[]).ok()
    }

    /// Build a constrained Delaunay surface from cloud points and validated
    /// breaklines. Breakline vertices win exact XY collisions with cloud
    /// points so their surveyed Z controls the discontinuity.
    pub fn from_points_with_breaklines(
        points: &[SamplePoint],
        class: Option<u8>,
        breaklines: &[Breakline],
    ) -> Result<Self, String> {
        let mut selected: Vec<[f64; 3]> = Vec::new();
        let mut at_xy: HashMap<[u64; 2], usize> = HashMap::new();
        let mut constraints = Vec::new();
        let mut insert = |point: [f64; 3], prefer_new_z: bool| {
            let key = [point[0].to_bits(), point[1].to_bits()];
            if let Some(&index) = at_xy.get(&key) {
                if prefer_new_z {
                    selected[index][2] = point[2];
                }
                index
            } else {
                let index = selected.len();
                selected.push(point);
                at_xy.insert(key, index);
                index
            }
        };

        for line in breaklines {
            let indices: Vec<usize> = line
                .vertices
                .iter()
                .copied()
                .filter(|point| point.iter().all(|value| value.is_finite()))
                .map(|point| insert(point, true))
                .collect();
            for pair in indices.windows(2) {
                if pair[0] != pair[1] {
                    constraints.push([pair[0], pair[1]]);
                }
            }
            if line.closed && indices.len() > 2 && indices[0] != indices[indices.len() - 1] {
                constraints.push([indices[indices.len() - 1], indices[0]]);
            }
        }
        for point in points
            .iter()
            .filter(|point| class.is_none_or(|class| point.classification == class))
            .map(|point| point.position)
            .filter(|point| point.iter().all(|value| value.is_finite()))
        {
            insert(point, false);
        }
        drop(insert);
        if selected.len() < 3 {
            return Err("fewer than three unique finite XY points".to_string());
        }

        let vertices: Vec<TinVertex> = selected
            .iter()
            .map(|point| TinVertex {
                position: Point2::new(point[0], point[1]),
                z: point[2],
            })
            .collect();
        let triangulation = ConstrainedDelaunayTriangulation::<TinVertex>::try_bulk_load_cdt(
            vertices,
            constraints,
            |_| {},
        )
        .map_err(|error| format!("constrained triangulation failed: {error:?}"))?;
        let selected: Vec<[f64; 3]> = triangulation
            .vertices()
            .map(|vertex| {
                let data = vertex.data();
                [data.position.x, data.position.y, data.z]
            })
            .collect();
        let triangles: Vec<[usize; 3]> = triangulation
            .inner_faces()
            .map(|face| face.vertices().map(|vertex| vertex.fix().index()))
            .collect();
        if triangles.is_empty() {
            return Err("triangulation produced no triangles".to_string());
        }
        let mut halfedges = vec![EMPTY; triangles.len() * 3];
        let mut open_edges: HashMap<(usize, usize), usize> = HashMap::new();
        for (triangle_index, triangle) in triangles.iter().enumerate() {
            for edge in 0..3 {
                let from = triangle[edge];
                let to = triangle[(edge + 1) % 3];
                let flat = triangle_index * 3 + edge;
                if let Some(opposite) = open_edges.remove(&(to, from)) {
                    halfedges[flat] = opposite;
                    halfedges[opposite] = flat;
                } else {
                    open_edges.insert((from, to), flat);
                }
            }
        }

        let mut bounds = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for position in &selected {
            for axis in 0..2 {
                bounds.0[axis] = bounds.0[axis].min(position[axis]);
                bounds.1[axis] = bounds.1[axis].max(position[axis]);
            }
        }
        let cell_size =
            ((bounds.1[0] - bounds.0[0]).max(bounds.1[1] - bounds.0[1]) / 64.0).max(1.0);
        let mut tin = Self {
            points: selected,
            triangles,
            halfedges,
            buckets: HashMap::new(),
            cell_size,
            hint: Cell::new(None),
        };
        let mut buckets = std::mem::take(&mut tin.buckets);
        for (triangle_index, triangle) in tin.triangles.iter().enumerate() {
            let centroid = [
                (tin.points[triangle[0]][0]
                    + tin.points[triangle[1]][0]
                    + tin.points[triangle[2]][0])
                    / 3.0,
                (tin.points[triangle[0]][1]
                    + tin.points[triangle[1]][1]
                    + tin.points[triangle[2]][1])
                    / 3.0,
            ];
            let cell = tin.cell_of(centroid);
            buckets.entry(cell).or_default().push(triangle_index);
        }
        tin.buckets = buckets;
        Ok(tin)
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    fn cell_of(&self, position: [f64; 2]) -> (i64, i64) {
        (
            (position[0] / self.cell_size).floor() as i64,
            (position[1] / self.cell_size).floor() as i64,
        )
    }

    /// Lawson-style straight-line walk from `start` toward `(x, y)`: cross the
    /// edge whose supporting line separates the target from the current
    /// triangle until the target is inside. Returns `None` when the walk
    /// reaches the hull (target outside the triangulation) or stalls.
    fn walk_to(&self, mut start: usize, x: f64, y: f64) -> Option<usize> {
        // A visibility walk on a Delaunay mesh cannot legitimately revisit a
        // triangle; the cap only guards degenerate input from spinning.
        for _ in 0..self.triangles.len() {
            let triangle = self.triangles[start];
            let a = self.points[triangle[0]];
            let b = self.points[triangle[1]];
            let c = self.points[triangle[2]];
            let side =
                |p: [f64; 3], q: [f64; 3]| (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
            let d1 = side(a, b);
            let d2 = side(b, c);
            let d3 = side(c, a);
            // Delaunay meshes are consistently wound, but the winding itself
            // is arbitrary; adapt the inside test to this triangle's order.
            let ccw = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) >= 0.0;
            let inside = if ccw {
                d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
            } else {
                d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0
            };
            if inside {
                return Some(start);
            }
            let outside = |d: f64| if ccw { d < 0.0 } else { d > 0.0 };
            let edge = if outside(d1) {
                0
            } else if outside(d2) {
                1
            } else if outside(d3) {
                2
            } else {
                // Not inside yet no edge strictly separates — boundary noise;
                // accept the triangle rather than risk a stall.
                return Some(start);
            };
            let opposite = self.halfedges[3 * start + edge];
            if opposite == EMPTY {
                return None;
            }
            start = opposite / 3;
        }
        None
    }

    /// First triangle bucketed nearest to the query cell — a walk start for
    /// the common case where the containing triangle's own centroid is
    /// bucketed far away (long, thin corridor triangles). Rings expand until
    /// they hit any bucket; 72 covers the whole 64×64 centroid grid diagonal,
    /// and dense geometry exits within a ring or two.
    fn nearest_bucketed_triangle(&self, home: (i64, i64)) -> Option<usize> {
        for ring in 0..=72_i64 {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    if ring > 0 && (dx.abs() + dy.abs()) != ring {
                        continue;
                    }
                    if let Some(triangle_ids) = self.buckets.get(&(home.0 + dx, home.1 + dy)) {
                        if let Some(&first) = triangle_ids.first() {
                            return Some(first);
                        }
                    }
                }
            }
        }
        None
    }

    fn containing_triangle(&self, x: f64, y: f64) -> Option<usize> {
        let home = self.cell_of([x, y]);
        // Walk first: from the previous hit (drape samples are sequential),
        // else from any triangle bucketed near the query.
        let walked = self
            .hint
            .get()
            .and_then(|start| self.walk_to(start, x, y))
            .or_else(|| {
                self.nearest_bucketed_triangle(home)
                    .and_then(|start| self.walk_to(start, x, y))
            });
        if let Some(hit) = walked {
            self.hint.set(Some(hit));
            return Some(hit);
        }
        // Safety net for degenerate/failed walks: the original centroid-ring
        // scan, unchanged.
        let found = self.scan_containing_triangle(home, x, y);
        if let Some(hit) = found {
            self.hint.set(Some(hit));
        }
        found
    }

    fn scan_containing_triangle(&self, home: (i64, i64), x: f64, y: f64) -> Option<usize> {
        for ring in 0..=4_i64 {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    if ring > 0 && (dx.abs() + dy.abs()) != ring {
                        continue;
                    }
                    let Some(triangle_ids) = self.buckets.get(&(home.0 + dx, home.1 + dy)) else {
                        continue;
                    };
                    for triangle_index in triangle_ids {
                        let triangle = self.triangles[*triangle_index];
                        let a = self.points[triangle[0]];
                        let b = self.points[triangle[1]];
                        let c = self.points[triangle[2]];
                        let sign = |ax: f64, ay: f64, bx: f64, by: f64| {
                            (bx - ax) * (y - ay) - (by - ay) * (x - ax)
                        };
                        let d1 = sign(a[0], a[1], b[0], b[1]);
                        let d2 = sign(b[0], b[1], c[0], c[1]);
                        let d3 = sign(c[0], c[1], a[0], a[1]);
                        let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
                        let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
                        if !(negative && positive) {
                            return Some(*triangle_index);
                        }
                    }
                }
            }
        }
        None
    }

    /// Interpolated surface height at `(x, y)`, or `None` outside the TIN.
    pub fn z_at(&self, x: f64, y: f64) -> Option<f64> {
        let triangle_index = self.containing_triangle(x, y)?;
        let triangle = self.triangles[triangle_index];
        let a = self.points[triangle[0]];
        let b = self.points[triangle[1]];
        let c = self.points[triangle[2]];
        let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if det.abs() < 1e-12 {
            return Some((a[2] + b[2] + c[2]) / 3.0);
        }
        let w0 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
        let w1 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
        let w2 = 1.0 - w0 - w1;
        Some(w0 * a[2] + w1 * b[2] + w2 * c[2])
    }

    /// Interpolate normally inside the TIN and otherwise return the elevation
    /// of the nearest point on its hull. The caller keeps its original XY;
    /// only the sampled height is clamped.
    pub fn z_at_clamped(&self, x: f64, y: f64) -> Option<f64> {
        if let Some(z) = self.z_at(x, y) {
            return Some(z);
        }
        let mut best: Option<(f64, f64)> = None;
        for (triangle_index, triangle) in self.triangles.iter().enumerate() {
            for edge in 0..3 {
                if self.halfedges[3 * triangle_index + edge] != EMPTY {
                    continue;
                }
                let a = self.points[triangle[edge]];
                let b = self.points[triangle[(edge + 1) % 3]];
                let dx = b[0] - a[0];
                let dy = b[1] - a[1];
                let length2 = dx * dx + dy * dy;
                if length2 <= f64::EPSILON {
                    continue;
                }
                let t = (((x - a[0]) * dx + (y - a[1]) * dy) / length2).clamp(0.0, 1.0);
                let qx = a[0] + t * dx;
                let qy = a[1] + t * dy;
                let distance2 = (x - qx).powi(2) + (y - qy).powi(2);
                let z = a[2] + t * (b[2] - a[2]);
                if best.is_none_or(|(distance, _)| distance2 < distance) {
                    best = Some((distance2, z));
                }
            }
        }
        best.map(|(_, z)| z)
    }
}

impl crate::measurement::SurfaceSampler for Tin {
    fn elevation_at(&self, x: f64, y: f64) -> Option<f64> {
        self.z_at(x, y)
    }

    fn clamped_elevation_at(&self, x: f64, y: f64) -> Option<f64> {
        self.z_at_clamped(x, y)
    }
}

/// One contour line: a chained polyline at a constant elevation.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    pub elevation: f64,
    pub points: Vec<[f64; 3]>,
}

/// Generates contours at `interval` above `base` over the TIN, chaining
/// per-triangle crossings into polylines.
pub fn generate_contours(tin: &Tin, interval: f64, base: f64) -> Vec<Contour> {
    let mut contours = Vec::new();
    if interval <= 0.0 {
        return contours;
    }
    let mut z_bounds = (f64::INFINITY, f64::NEG_INFINITY);
    let mut heights: Vec<f64> = Vec::with_capacity(tin.points.len());
    for point in &tin.points {
        z_bounds.0 = z_bounds.0.min(point[2]);
        z_bounds.1 = z_bounds.1.max(point[2]);
        heights.push(point[2]);
    }
    heights.sort_by(f64::total_cmp);
    // Levels that exactly hit a vertex create coincident crossings at that
    // vertex and fragment the chains; nudge them off.
    let nudge_level = |level: f64, heights: &[f64]| -> f64 {
        let mut level = level;
        let mut probe =
            heights.partition_point(|z| z.total_cmp(&level) == std::cmp::Ordering::Less);
        while probe < heights.len() && (heights[probe] - level).abs() < 1e-9 {
            level += interval * 1e-6;
            probe += 1;
        }
        level
    };
    // Contours live strictly inside (z_min, z_max): a level at the exact
    // minimum would hug the data boundary, not a real interval line.
    let k = (((z_bounds.0 - base) / interval) + 1e-9).ceil().max(1.0);
    let first_level = base + k * interval;
    let mut level = nudge_level(first_level, &heights);
    while level < z_bounds.1 {
        contours.extend(contours_at(tin, level));
        level = nudge_level(level + interval, &heights);
    }
    contours
}

fn contours_at(tin: &Tin, level: f64) -> Vec<Contour> {
    // Segments per level, then chained by quantized endpoints.
    let mut segments: Vec<([f64; 3], [f64; 3])> = Vec::new();
    for triangle in &tin.triangles {
        let a = tin.points[triangle[0]];
        let b = tin.points[triangle[1]];
        let c = tin.points[triangle[2]];
        let mut crossings: Vec<[f64; 3]> = Vec::with_capacity(2);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let dp = p[2] - level;
            let dq = q[2] - level;
            if (dp < 0.0 && dq < 0.0) || (dp > 0.0 && dq > 0.0) {
                continue;
            }
            if dp == dq {
                continue; // edge parallel to the level
            }
            let t = dp / (dp - dq);
            crossings.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1]), level]);
        }
        if crossings.len() == 2 {
            let (first, second) = (crossings[0], crossings[1]);
            // Ignore degenerate zero-length segments.
            if first != second {
                segments.push((first, second));
            }
        }
    }

    // Chain segments: each endpoint key maps to (segment, end) pairs.
    let quantize = |point: [f64; 3]| -> (i64, i64) {
        (
            (point[0] * 1e6).round() as i64,
            (point[1] * 1e6).round() as i64,
        )
    };
    let mut endpoints: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (index, (start, end)) in segments.iter().enumerate() {
        endpoints
            .entry(quantize(*start))
            .or_default()
            .push((index, 0));
        endpoints
            .entry(quantize(*end))
            .or_default()
            .push((index, 1));
    }
    let mut used = vec![false; segments.len()];
    let mut contours = Vec::new();
    for start_index in 0..segments.len() {
        if used[start_index] {
            continue;
        }
        used[start_index] = true;
        let mut points = vec![segments[start_index].0, segments[start_index].1];
        // Extend forward from the tail, then backward from the head.
        loop {
            let tail = *points.last().expect("non-empty chain");
            let mut extended = false;
            if let Some(candidates) = endpoints.get(&quantize(tail)) {
                for (index, end) in candidates {
                    if used[*index] {
                        continue;
                    }
                    let (start, end_point) = segments[*index];
                    let next = if *end == 0 { end_point } else { start };
                    used[*index] = true;
                    points.push(next);
                    extended = true;
                    break;
                }
            }
            if !extended {
                break;
            }
        }
        loop {
            let head = points[0];
            let mut extended = false;
            if let Some(candidates) = endpoints.get(&quantize(head)) {
                for (index, end) in candidates {
                    if used[*index] {
                        continue;
                    }
                    let (start, end_point) = segments[*index];
                    let next = if *end == 0 { end_point } else { start };
                    used[*index] = true;
                    points.insert(0, next);
                    extended = true;
                    break;
                }
            }
            if !extended {
                break;
            }
        }
        if points.len() >= 2 {
            contours.push(Contour {
                elevation: level,
                points,
            });
        }
    }
    contours
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(source_index: u64, x: f64, y: f64, z: f64, class: u8) -> SamplePoint {
        SamplePoint {
            source_index,
            position: [x, y, z],
            intensity: 0,
            classification: class,
            return_number: 1,
            number_of_returns: 1,
            scan_angle: 0.0,
            user_data: 0,
            point_source_id: 0,
            gps_time: None,
            color: None,
            nir: None,
            label: None,
            is_synthetic: false,
            is_key_point: false,
            is_withheld: false,
            is_overlap: false,
        }
    }

    /// Tilted plane z = 0.5x, 10x10 grid from x=y=0..18.
    fn tilted_plane() -> Vec<SamplePoint> {
        let mut points = Vec::new();
        let mut index = 0_u64;
        for gx in 0..10i64 {
            for gy in 0..10i64 {
                let x = gx as f64 * 2.0;
                let y = gy as f64 * 2.0;
                points.push(point(index, x, y, x * 0.5, 2));
                index += 1;
            }
        }
        points
    }

    #[test]
    fn tin_interpolates_the_plane_exactly() {
        let points = tilted_plane();
        let tin = Tin::from_points(&points, Some(2)).expect("tin");
        assert!(tin.triangle_count() > 100);
        for (x, y) in [(1.0, 1.0), (5.5, 3.25), (16.9, 8.1)] {
            let z = tin.z_at(x, y).expect("interior point");
            assert!((z - x * 0.5).abs() < 1e-9, "z at ({x},{y}) was {z}");
        }
        assert!(tin.z_at(-5.0, -5.0).is_none(), "outside the hull");
    }

    #[test]
    fn contours_of_a_plane_are_level_lines() {
        let points = tilted_plane();
        let tin = Tin::from_points(&points, Some(2)).expect("tin");
        let contours = generate_contours(&tin, 1.0, 0.0);
        // Plane rises 0..9 → levels 1..8, each crossing the square once.
        assert_eq!(8, contours.len());
        for (index, contour) in contours.iter().enumerate() {
            assert!(contour.points.len() >= 2);
            for point in &contour.points {
                assert!((point[2] - contour.elevation).abs() < 1e-6);
            }
            // Levels may be nudged a hair off exact to dodge vertex hits.
            assert!((contour.elevation - (index as f64 + 1.0)).abs() < 1e-3);
        }
        for pair in contours.windows(2) {
            let spacing = pair[1].elevation - pair[0].elevation;
            assert!((spacing - 1.0).abs() < 1e-3, "level spacing was {spacing}");
        }
    }

    #[test]
    fn flat_planes_produce_no_contours() {
        let points: Vec<_> = (0..25)
            .map(|index| point(index, (index % 5) as f64, (index / 5) as f64, 7.0, 2))
            .collect();
        let tin = Tin::from_points(&points, Some(2)).expect("tin");
        assert!(generate_contours(&tin, 1.0, 0.0).is_empty());
    }

    #[test]
    fn class_filter_selects_ground_only() {
        let mut points = tilted_plane();
        // A roof point cluster far above must not join the TIN.
        points.push(point(999, 1.0, 1.0, 50.0, 6));
        let tin = Tin::from_points(&points, Some(2)).expect("tin");
        assert!((tin.z_at(1.0, 1.0).expect("z") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn corridor_thin_triangle_point_location_walks() {
        // A 6400 × 2 strip triangulated into four 3200-unit slivers. The
        // query sits inside a sliver whose centroid buckets more than four
        // cells away, so the original centroid ring scan returned None and
        // aborted corridor drapes for points that ARE inside the hull.
        let mut points = Vec::new();
        for (index, &(x, y)) in [
            (0.0, 0.0),
            (3200.0, 0.05),
            (6400.0, 0.0),
            (0.0, 2.0),
            (3200.0, 1.95),
            (6400.0, 2.0),
        ]
        .iter()
        .enumerate()
        {
            points.push(point(index as u64, x, y, x / 1000.0, 2));
        }
        let tin = Tin::from_points(&points, Some(2)).expect("tin");
        assert!(tin.triangle_count() >= 4);

        let (x, y) = (1601.0, 0.5);
        let hit = tin.containing_triangle(x, y).expect("inside a sliver");
        // The regression premise: the found triangle's own centroid is more
        // than the ring scan's four cells away from the query cell.
        let triangle = tin.triangles[hit];
        let centroid = [
            (tin.points[triangle[0]][0] + tin.points[triangle[1]][0] + tin.points[triangle[2]][0])
                / 3.0,
            (tin.points[triangle[0]][1] + tin.points[triangle[1]][1] + tin.points[triangle[2]][1])
                / 3.0,
        ];
        let query_cell = tin.cell_of([x, y]);
        let centroid_cell = tin.cell_of(centroid);
        let chebyshev = (query_cell.0 - centroid_cell.0)
            .abs()
            .max((query_cell.1 - centroid_cell.1).abs());
        assert!(
            chebyshev > 4,
            "regression premise failed: centroid only {chebyshev} cells away"
        );

        let z = tin.z_at(x, y).expect("z inside the strip");
        assert!((z - x / 1000.0).abs() < 1e-9);

        // Genuinely outside the hull: above and beside the 2-unit strip.
        assert!(tin.z_at(3200.0, 50.0).is_none());
        assert!(tin.z_at(-10.0, 1.0).is_none());
    }

    #[test]
    fn consecutive_walk_queries_stay_inside_the_hull() {
        // Sequential samples along the strip — the drape access pattern. Every
        // query must resolve (hint-driven walk) and hit the tilted plane.
        let mut points = Vec::new();
        for (index, &(x, y)) in [
            (0.0, 0.0),
            (3200.0, 0.05),
            (6400.0, 0.0),
            (0.0, 2.0),
            (3200.0, 1.95),
            (6400.0, 2.0),
        ]
        .iter()
        .enumerate()
        {
            points.push(point(index as u64, x, y, x / 1000.0, 2));
        }
        let tin = Tin::from_points(&points, Some(2)).expect("tin");
        for step in 0..=64 {
            let x = step as f64 * 100.0;
            let y = if step % 2 == 0 { 0.4 } else { 1.6 };
            let z = tin.z_at(x, y).unwrap_or_else(|| {
                panic!("sample {step} at ({x}, {y}) should be inside the strip")
            });
            assert!((z - x / 1000.0).abs() < 1e-9);
        }
    }

    #[test]
    fn constrained_tin_contains_every_breakline_segment() {
        let points = vec![
            point(0, 0.0, 0.0, 0.0, 2),
            point(1, 10.0, 0.0, 0.0, 2),
            point(2, 10.0, 10.0, 0.0, 2),
            point(3, 0.0, 10.0, 0.0, 2),
        ];
        let breakline = Breakline {
            id: "ridge".to_string(),
            vertices: vec![[0.0, 5.0, 10.0], [5.0, 5.0, 10.0], [10.0, 5.0, 10.0]],
            closed: false,
        };
        let tin = Tin::from_points_with_breaklines(&points, Some(2), &[breakline]).unwrap();
        let index = |x: f64, y: f64| {
            tin.points
                .iter()
                .position(|point| point[0] == x && point[1] == y)
                .unwrap()
        };
        for (a, b) in [
            (index(0.0, 5.0), index(5.0, 5.0)),
            (index(5.0, 5.0), index(10.0, 5.0)),
        ] {
            assert!(
                tin.triangles
                    .iter()
                    .any(|triangle| { triangle.contains(&a) && triangle.contains(&b) }),
                "missing constrained edge {a}-{b}"
            );
        }
        assert!((tin.z_at(5.0, 5.0).unwrap() - 10.0).abs() < 1.0e-9);
    }

    #[test]
    fn clamped_height_uses_the_nearest_hull_edge() {
        let points = vec![
            point(0, 0.0, 0.0, 0.0, 2),
            point(1, 10.0, 0.0, 10.0, 2),
            point(2, 0.0, 10.0, 0.0, 2),
        ];
        let tin = Tin::from_points(&points, Some(2)).unwrap();
        assert!(tin.z_at(5.0, -2.0).is_none());
        assert!((tin.z_at_clamped(5.0, -2.0).unwrap() - 5.0).abs() < 1.0e-9);
    }
}
