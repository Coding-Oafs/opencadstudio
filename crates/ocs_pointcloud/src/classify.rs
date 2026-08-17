//! Automated classification: noise detection, attribute rules, and
//! progressive ground densification.
//!
//! Every classifier works over a caller-provided working set (the app's
//! display sample or streamed LOD points) and returns sparse
//! `(source_index, classification)` patches so results flow through the
//! normal edit transactions — audited, undoable, and applied on export.

use crate::{EditStore, PointPatch, SamplePoint};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A sparse reclassification produced by a classifier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassifyResult {
    pub patches: Vec<(u64, u8)>,
}

impl ClassifyResult {
    pub fn len(&self) -> usize {
        self.patches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    /// Commits the result as one undoable transaction per target class, so
    /// undo steps behave exactly like interactive edits.
    pub fn apply_grouped(self, store: &mut EditStore, label: &str) -> usize {
        let mut changed = 0;
        let mut by_class: HashMap<u8, Vec<u64>> = HashMap::new();
        for (index, class) in self.patches {
            by_class.entry(class).or_default().push(index);
        }
        let mut classes: Vec<_> = by_class.into_iter().collect();
        classes.sort_by_key(|(class, _)| *class);
        for (class, indices) in classes {
            changed += store.apply(
                format!("{label} → class {class}"),
                indices,
                PointPatch::classification(class),
            );
        }
        changed
    }
}

// ── Noise / isolated point detection ───────────────────────────────────────

/// Flags points with fewer than `min_neighbors` neighbors inside `radius`
/// metres as noise. A uniform voxel hash keeps the scan linear in practice.
pub fn detect_noise(
    points: &[SamplePoint],
    radius: f64,
    min_neighbors: usize,
    noise_class: u8,
) -> ClassifyResult {
    let mut result = ClassifyResult::default();
    if radius <= 0.0 {
        return result;
    }
    let key = |position: [f64; 3], inverse: f64| -> (i64, i64, i64) {
        (
            (position[0] * inverse).floor() as i64,
            (position[1] * inverse).floor() as i64,
            (position[2] * inverse).floor() as i64,
        )
    };
    let inverse = 1.0 / radius;
    let mut voxels: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (index, point) in points.iter().enumerate() {
        voxels
            .entry(key(point.position, inverse))
            .or_default()
            .push(index);
    }
    let radius_sq = radius * radius;
    for (index, point) in points.iter().enumerate() {
        let cell = key(point.position, inverse);
        let mut neighbors = 0_usize;
        'cell: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(bucket) = voxels.get(&(cell.0 + dx, cell.1 + dy, cell.2 + dz)) else {
                        continue;
                    };
                    for other in bucket {
                        if *other == index {
                            continue;
                        }
                        let delta = points[*other].position;
                        let d = [
                            delta[0] - point.position[0],
                            delta[1] - point.position[1],
                            delta[2] - point.position[2],
                        ];
                        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= radius_sq {
                            neighbors += 1;
                            if neighbors >= min_neighbors {
                                break 'cell;
                            }
                        }
                    }
                }
            }
        }
        if neighbors < min_neighbors {
            result.patches.push((point.source_index, noise_class));
        }
    }
    result
}

// ── Attribute rules ────────────────────────────────────────────────────────

/// Point attribute a rule tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleField {
    Elevation,
    Intensity,
    ReturnNumber,
    Classification,
    PointSource,
    ScanAngle,
}

/// Comparison a rule applies to the field value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RuleOp {
    Less,
    Greater,
    Between,
    Equals,
}

/// One classification rule: points in `from_classes` (empty = any class)
/// whose field satisfies the operation are reclassified to `target_class`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassifyRule {
    pub field: RuleField,
    pub op: RuleOp,
    /// `[low, high]` for `Between`, the threshold otherwise.
    pub values: [f64; 2],
    pub target_class: u8,
    /// Restrict the rule to points currently in these classes.
    pub from_classes: Vec<u8>,
}

impl ClassifyRule {
    fn field_value(&self, point: &SamplePoint) -> f64 {
        match self.field {
            RuleField::Elevation => point.position[2],
            RuleField::Intensity => point.intensity as f64,
            RuleField::ReturnNumber => point.return_number as f64,
            RuleField::Classification => point.classification as f64,
            RuleField::PointSource => point.point_source_id as f64,
            RuleField::ScanAngle => point.scan_angle as f64,
        }
    }

    fn matches(&self, point: &SamplePoint) -> bool {
        if !self.from_classes.is_empty() && !self.from_classes.contains(&point.classification) {
            return false;
        }
        let value = self.field_value(point);
        match self.op {
            RuleOp::Less => value < self.values[0],
            RuleOp::Greater => value > self.values[0],
            RuleOp::Between => value >= self.values[0] && value <= self.values[1],
            RuleOp::Equals => (value - self.values[0]).abs() < f64::EPSILON,
        }
    }
}

/// Applies a rule pipeline in order; later rules see earlier rules' results.
pub fn classify_by_rules(points: &[SamplePoint], rules: &[ClassifyRule]) -> ClassifyResult {
    let mut result = ClassifyResult::default();
    if rules.is_empty() {
        return result;
    }
    let mut applied: HashMap<u64, u8> = HashMap::new();
    for point in points {
        let mut classification = point.classification;
        for rule in rules {
            let mut candidate = point.clone();
            candidate.classification = classification;
            if rule.matches(&candidate) {
                classification = rule.target_class;
                applied.insert(point.source_index, rule.target_class);
            }
        }
    }
    let mut patches: Vec<_> = applied.into_iter().collect();
    patches.sort_unstable();
    result.patches = patches;
    result
}

// ── Ground classification (simplified progressive TIN densification) ───────

/// Tuning for [`classify_ground`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundOptions {
    /// Grid cell size (project units) used to seed and refine the surface.
    pub cell_size: f64,
    /// Maximum distance (project units) from the local surface plane.
    pub max_distance: f64,
    /// Maximum angle (degrees) between the surface and a candidate point.
    pub max_angle_degrees: f64,
    /// Densification iterations; each pass refines the surface grid.
    pub iterations: usize,
    /// Class assigned to accepted ground points.
    pub ground_class: u8,
    /// Points above the surface by more than this stay untouched even in
    /// later passes (keeps roofs out of the final surface).
    pub reject_above: f64,
}

impl Default for GroundOptions {
    fn default() -> Self {
        Self {
            cell_size: 10.0,
            max_distance: 0.75,
            max_angle_degrees: 30.0,
            iterations: 5,
            ground_class: 2,
            reject_above: 5.0,
        }
    }
}

/// Classifies bare earth by progressive surface densification: every cell of
/// the grid starts with its lowest point, the surface is interpolated from
/// triangle pairs of neighbouring cells, and each iteration accepts the
/// lowest remaining point per cell that sits within the distance and angle
/// thresholds of the surface — refining the grid as it converges. This is a
/// simplified progressive TIN densification: the surface really is a TIN of
/// cell representatives, but the classic mirror-image edge test is
/// approximated by the plane distance + angle test.
pub fn classify_ground(points: &[SamplePoint], options: &GroundOptions) -> ClassifyResult {
    let mut result = ClassifyResult::default();
    if points.is_empty() || options.cell_size <= 0.0 {
        return result;
    }
    let inverse = 1.0 / options.cell_size;
    let cell_of = |position: [f64; 3]| -> (i64, i64) {
        (
            (position[0] * inverse).floor() as i64,
            (position[1] * inverse).floor() as i64,
        )
    };

    // Cell → points; representative = lowest point currently accepted as
    // ground (initially the lowest point overall, as long as it is not far
    // above the initial seed surface).
    let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (index, point) in points.iter().enumerate() {
        cells.entry(cell_of(point.position)).or_default().push(index);
    }

    // Seed representatives: the lowest point of each cell.
    let mut representatives: HashMap<(i64, i64), usize> = cells
        .iter()
        .map(|(cell, bucket)| {
            let lowest = bucket
                .iter()
                .copied()
                .min_by_key(|index| points[*index].position[2].to_bits())
                .expect("non-empty bucket");
            (*cell, lowest)
        })
        .collect();
    let mut accepted: std::collections::HashSet<u64> = representatives
        .values()
        .map(|index| points[*index].source_index)
        .collect();

    for _ in 0..options.iterations.max(1) {
        let surface = SurfaceGrid::build(&representatives, points, options.cell_size);
        let mut refinements: Vec<((i64, i64), usize)> = Vec::new();
        for (cell, bucket) in &cells {
            // The lowest point not yet accepted that passes the surface
            // tests refines this cell.
            let mut candidates: Vec<usize> = bucket
                .iter()
                .copied()
                .filter(|index| !accepted.contains(&points[*index].source_index))
                .collect();
            if candidates.is_empty() {
                continue;
            }
            candidates.sort_by_key(|index| points[*index].position[2].to_bits());
            let Some(candidate) = candidates.first().copied() else {
                continue;
            };
            let position = points[candidate].position;
            let Some(plane) = surface.locate(position) else {
                continue;
            };
            let residual = position[2] - plane.z_at(position[0], position[1]);
            // Far above the surface stays off the ground forever (roofs);
            // far below is a pit; both hands-off. Near-misses pass on angle.
            if residual > options.reject_above || residual.abs() > options.max_distance {
                continue;
            }
            let span = plane.nearest_vertex_distance(position).max(options.cell_size * 0.25);
            let angle = (residual.abs() / span).atan().to_degrees();
            if angle <= options.max_angle_degrees {
                refinements.push((*cell, candidate));
            }
        }
        if refinements.is_empty() {
            break;
        }
        for (cell, candidate) in refinements {
            representatives.insert(cell, candidate);
            accepted.insert(points[candidate].source_index);
        }
    }

    let mut patches: Vec<_> = accepted
        .into_iter()
        .map(|index| (index, options.ground_class))
        .collect();
    patches.sort_unstable();
    result.patches = patches;
    result
}

// ── Bucketed TIN surface ───────────────────────────────────────────────────

/// A triangle of neighbouring cell representatives.
struct Plane {
    vertices: [[f64; 3]; 3],
}

impl Plane {
    fn z_at(&self, x: f64, y: f64) -> f64 {
        let [a, b, c] = self.vertices;
        let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if det.abs() < 1e-12 {
            return (a[2] + b[2] + c[2]) / 3.0;
        }
        let w0 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
        let w1 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
        let w2 = 1.0 - w0 - w1;
        w0 * a[2] + w1 * b[2] + w2 * c[2]
    }

    fn nearest_vertex_distance(&self, position: [f64; 3]) -> f64 {
        self.vertices
            .iter()
            .map(|vertex| {
                let dx = vertex[0] - position[0];
                let dy = vertex[1] - position[1];
                let dz = vertex[2] - position[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .fold(f64::INFINITY, f64::min)
    }
}

/// Triangles between cell representatives, bucketed by their lower-left cell
/// for O(1) point location with a two-ring fallback for sparse edges.
struct SurfaceGrid {
    triangles: HashMap<(i64, i64), Vec<Plane>>,
    cell_size: f64,
}

impl SurfaceGrid {
    fn build(
        representatives: &HashMap<(i64, i64), usize>,
        points: &[SamplePoint],
        cell_size: f64,
    ) -> Self {
        let position = |cell: &(i64, i64)| -> [f64; 3] { points[representatives[cell]].position };
        let mut triangles: HashMap<(i64, i64), Vec<Plane>> = HashMap::new();
        for cell in representatives.keys() {
            let east = (cell.0 + 1, cell.1);
            let north = (cell.0, cell.1 + 1);
            let northeast = (cell.0 + 1, cell.1 + 1);
            if representatives.contains_key(&east)
                && representatives.contains_key(&north)
                && representatives.contains_key(&northeast)
            {
                let a = position(cell);
                let b = position(&east);
                let c = position(&north);
                let d = position(&northeast);
                triangles.entry(*cell).or_default().push(Plane { vertices: [a, b, d] });
                triangles.entry(*cell).or_default().push(Plane { vertices: [a, d, c] });
            }
        }
        Self { triangles, cell_size }
    }

    fn cell_of(&self, x: f64, y: f64) -> (i64, i64) {
        ((x / self.cell_size).floor() as i64, (y / self.cell_size).floor() as i64)
    }

    fn contains_2d(&self, plane: &Plane, x: f64, y: f64) -> bool {
        let [a, b, c] = plane.vertices;
        let sign = |ax: f64, ay: f64, bx: f64, by: f64| (bx - ax) * (y - ay) - (by - ay) * (x - ax);
        let d1 = sign(a[0], a[1], b[0], b[1]);
        let d2 = sign(b[0], b[1], c[0], c[1]);
        let d3 = sign(c[0], c[1], a[0], a[1]);
        let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(has_negative && has_positive)
    }

    /// The plane covering `(x, y)` in its own cell, else the nearest-ring
    /// cell that covers it, else the nearest plane within two rings.
    fn locate(&self, position: [f64; 3]) -> Option<&Plane> {
        let home = self.cell_of(position[0], position[1]);
        for ring in 0_i64..=2 {
            let mut nearest: Option<(f64, &Plane)> = None;
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    let distance_from_center = dx.abs() + dy.abs();
                    if ring > 0 && distance_from_center != ring {
                        continue; // ring perimeter only
                    }
                    let Some(planes) = self.triangles.get(&(home.0 + dx, home.1 + dy)) else {
                        continue;
                    };
                    for plane in planes {
                        if self.contains_2d(plane, position[0], position[1]) {
                            return Some(plane);
                        }
                        let centroid = [
                            (plane.vertices[0][0]
                                + plane.vertices[1][0]
                                + plane.vertices[2][0])
                                / 3.0,
                            (plane.vertices[0][1]
                                + plane.vertices[1][1]
                                + plane.vertices[2][1])
                                / 3.0,
                        ];
                        let ddx = centroid[0] - position[0];
                        let ddy = centroid[1] - position[1];
                        let distance = ddx * ddx + ddy * ddy;
                        if nearest.as_ref().is_none_or(|(best, _)| distance < *best) {
                            nearest = Some((distance, plane));
                        }
                    }
                }
            }
            // Within the home ring, prefer a covering triangle; the fallback
            // only fires once the perimeter scan found nothing covering.
            if ring == 2 {
                return nearest.map(|(_, plane)| plane);
            }
            if ring > 0 {
                if let Some((_, plane)) = nearest {
                    return Some(plane);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(source_index: u64, x: f64, y: f64, z: f64, class: u8) -> SamplePoint {
        SamplePoint {
            source_index,
            position: [x, y, z],
            intensity: 100,
            classification: class,
            return_number: 1,
            number_of_returns: 1,
            scan_angle: 0.0,
            user_data: 0,
            point_source_id: 7,
            gps_time: None,
            color: None,
            nir: None,
            is_synthetic: false,
            is_key_point: false,
            is_withheld: false,
            is_overlap: false,
        }
    }

    /// Gently sloped ground grid with a low-point band, plus one spike per
    /// region that must never be classified as ground.
    fn scene_with_ground() -> Vec<SamplePoint> {
        let mut points = Vec::new();
        let mut index = 0_u64;
        for gx in 0..40 {
            for gy in 0..40 {
                let x = gx as f64 * 2.5;
                let y = gy as f64 * 2.5;
                let z = 10.0 + (x + y) * 0.01 + ((gx + gy) % 3) as f64 * 0.05;
                points.push(point(index, x, y, z, 1));
                index += 1;
                // Buildings: 8 m above ground.
                if (5..9).contains(&gx) && (5..9).contains(&gy) {
                    points.push(point(index, x, y, z + 8.0, 6));
                    index += 1;
                }
            }
        }
        points
    }

    #[test]
    fn noise_detection_flags_isolated_spikes() {
        let mut points = Vec::new();
        let mut index = 0_u64;
        for gx in 0..20 {
            for gy in 0..20 {
                points.push(point(index, gx as f64, gy as f64, 0.0, 1));
                index += 1;
            }
        }
        // Two isolated points far from the surface.
        points.push(point(index, 50.5, 50.5, 3.0, 1));
        points.push(point(index + 1, 60.5, 60.5, -3.0, 1));
        let noise = detect_noise(&points, 1.5, 3, 7);
        assert_eq!(2, noise.len(), "only the two isolated points are noise");
        let flagged: Vec<u64> = noise.patches.iter().map(|(i, _)| *i).collect();
        assert!(flagged.contains(&index));
        assert!(flagged.contains(&(index + 1)));
    }

    #[test]
    fn attribute_rules_reclassify_by_band_and_guard_source_classes() {
        let points = vec![
            point(0, 0.0, 0.0, 5.0, 1),
            point(1, 1.0, 0.0, 15.0, 1),
            point(2, 2.0, 0.0, 25.0, 2),
        ];
        let rules = vec![ClassifyRule {
            field: RuleField::Elevation,
            op: RuleOp::Between,
            values: [10.0, 20.0],
            target_class: 9,
            from_classes: vec![1],
        }];
        let result = classify_by_rules(&points, &rules);
        assert_eq!(vec![(1, 9)], result.patches);
    }

    #[test]
    fn ground_classifier_seeds_surface_and_rejects_buildings() {
        let points = scene_with_ground();
        let options = GroundOptions {
            cell_size: 10.0,
            max_distance: 0.6,
            max_angle_degrees: 30.0,
            iterations: 4,
            ground_class: 2,
            reject_above: 5.0,
        };
        let result = classify_ground(&points, &options);
        assert!(!result.is_empty());
        let by_index: std::collections::HashMap<u64, u8> =
            result.patches.into_iter().collect();
        let mut ground_count = 0_usize;
        let mut roof_leak = 0_usize;
        for point in &points {
            match by_index.get(&point.source_index) {
                Some(_) => {
                    // Accepted as ground: must not be an 8 m roof point.
                    if point.classification == 6 {
                        roof_leak += 1;
                    }
                    ground_count += 1;
                }
                None => {}
            }
        }
        assert_eq!(0, roof_leak, "no roof point may be classified as ground");
        // Seeding alone accepts one point per 10 m cell (4x4 grid = 16);
        // densification must accept substantially more true ground.
        assert!(
            ground_count > 100,
            "densification should accept far more than the seeds alone: {ground_count}"
        );
    }

    #[test]
    fn ground_result_applies_as_undoable_transactions() {
        let points = scene_with_ground();
        let result = classify_ground(&points, &GroundOptions::default());
        let mut store = EditStore::default();
        let changed = result.apply_grouped(&mut store, "auto ground");
        assert!(changed > 0);
        assert_eq!(1, store.transaction_count());
        assert!(store.undo().is_some());
        assert_eq!(0, store.len());
    }
}
