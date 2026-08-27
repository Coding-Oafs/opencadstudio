//! Ground-surface TIN and contour generation.
//!
//! [`Tin`] triangulates a class-filtered point set with Delaunay (via
//! `delaunator`) and interpolates heights by barycentric plane lookup.
//! [`generate_contours`] runs marching triangles over the TIN and chains the
//! crossings into polylines ready for CAD layers.

use crate::SamplePoint;
use delaunator::{triangulate, Point as DPoint};
use std::collections::HashMap;

/// A Delaunay surface over projected point positions.
pub struct Tin {
    points: Vec<[f64; 3]>,
    /// Triangle vertex indices into `points`.
    triangles: Vec<[usize; 3]>,
    /// Triangle ids bucketed by grid cell for point location.
    buckets: HashMap<(i64, i64), Vec<usize>>,
    cell_size: f64,
}

impl Tin {
    /// Builds the TIN from every point whose class is `class` (when given).
    /// Returns `None` when fewer than three points form a surface.
    pub fn from_points(points: &[SamplePoint], class: Option<u8>) -> Option<Self> {
        let selected: Vec<[f64; 3]> = points
            .iter()
            .filter(|point| class.is_none_or(|class| point.classification == class))
            .map(|point| point.position)
            .collect();
        if selected.len() < 3 {
            return None;
        }
        let projected: Vec<DPoint> = selected
            .iter()
            .map(|position| DPoint {
                x: position[0],
                y: position[1],
            })
            .collect();
        let triangulation = triangulate(&projected);
        let triangles: Vec<[usize; 3]> = triangulation
            .triangles
            .chunks_exact(3)
            .map(|chunk| [chunk[0], chunk[1], chunk[2]])
            .collect();
        if triangles.is_empty() {
            return None;
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
            buckets: HashMap::new(),
            cell_size,
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
        Some(tin)
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

    fn containing_triangle(&self, x: f64, y: f64) -> Option<usize> {
        let home = self.cell_of([x, y]);
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
}
