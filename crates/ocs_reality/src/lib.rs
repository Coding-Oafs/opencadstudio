//! Deterministic reality-to-model foundations.
//!
//! These algorithms operate on explicit full-density inputs supplied by the
//! job engine; they never read viewer LOD state. The module covers the shared
//! mathematical core for primitive fitting, linear extraction, corridor
//! stationing, surface comparison, change detection, and LOD1 buildings.

use ocs_gis::Geometry;
use ocs_pointcloud::{DensityRequirement, ToolDescriptor, ToolRequirements, UndoBehavior};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaneFit {
    /// Unit normal in the equation `normal·p + offset = 0`.
    pub normal: [f64; 3],
    pub offset: f64,
    pub rms_error: f64,
    pub maximum_error: f64,
    pub point_count: usize,
}

/// Least-squares fit of `z = ax + by + c`. This is the appropriate stable
/// model for terrain and roof patches that are not vertical.
pub fn fit_plane(points: &[[f64; 3]]) -> Result<PlaneFit, String> {
    if points.len() < 3 || points.iter().flatten().any(|value| !value.is_finite()) {
        return Err("plane fitting requires at least three finite points".into());
    }
    let mut matrix = [[0.0; 3]; 3];
    let mut rhs = [0.0; 3];
    for [x, y, z] in points {
        let row = [*x, *y, 1.0];
        for i in 0..3 {
            rhs[i] += row[i] * z;
            for j in 0..3 {
                matrix[i][j] += row[i] * row[j];
            }
        }
    }
    let [a, b, c] = solve::<3>(matrix, rhs).ok_or("plane points are degenerate")?;
    let length = (a * a + b * b + 1.0).sqrt();
    let normal = [a / length, b / length, -1.0 / length];
    let offset = c / length;
    let mut squared = 0.0;
    let mut maximum = 0.0_f64;
    for point in points {
        let residual = dot(normal, *point) + offset;
        squared += residual * residual;
        maximum = maximum.max(residual.abs());
    }
    Ok(PlaneFit {
        normal,
        offset,
        rms_error: (squared / points.len() as f64).sqrt(),
        maximum_error: maximum,
        point_count: points.len(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphereFit {
    pub center: [f64; 3],
    pub radius: f64,
    pub rms_error: f64,
    pub point_count: usize,
}

pub fn fit_sphere(points: &[[f64; 3]]) -> Result<SphereFit, String> {
    if points.len() < 4 || points.iter().flatten().any(|value| !value.is_finite()) {
        return Err("sphere fitting requires at least four finite points".into());
    }
    // Linearized sphere: 2cx*x + 2cy*y + 2cz*z + k = x²+y²+z².
    let mut matrix = [[0.0; 4]; 4];
    let mut rhs = [0.0; 4];
    for [x, y, z] in points {
        let row = [2.0 * x, 2.0 * y, 2.0 * z, 1.0];
        let target = x * x + y * y + z * z;
        for i in 0..4 {
            rhs[i] += row[i] * target;
            for j in 0..4 {
                matrix[i][j] += row[i] * row[j];
            }
        }
    }
    let [cx, cy, cz, k] = solve::<4>(matrix, rhs).ok_or("sphere points are degenerate")?;
    let radius_squared = k + cx * cx + cy * cy + cz * cz;
    if radius_squared <= 0.0 {
        return Err("sphere fit produced a non-positive radius".into());
    }
    let radius = radius_squared.sqrt();
    let center = [cx, cy, cz];
    let squared = points
        .iter()
        .map(|point| (distance(*point, center) - radius).powi(2))
        .sum::<f64>();
    Ok(SphereFit {
        center,
        radius,
        rms_error: (squared / points.len() as f64).sqrt(),
        point_count: points.len(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearFeature {
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub direction: [f64; 3],
    pub rms_cross_track: f64,
    pub point_count: usize,
}

/// Principal-axis fit for curb, rail, paint-line, and wire candidates.
pub fn extract_linear_feature(points: &[[f64; 3]]) -> Result<LinearFeature, String> {
    if points.len() < 2 || points.iter().flatten().any(|value| !value.is_finite()) {
        return Err("linear extraction requires at least two finite points".into());
    }
    let centroid = mean_point(points);
    let mut direction = normalize(sub(points[points.len() - 1], points[0]))?;
    // Power iteration over the 3x3 covariance matrix, seeded by endpoints.
    let mut covariance = [[0.0; 3]; 3];
    for point in points {
        let delta = sub(*point, centroid);
        for i in 0..3 {
            for j in 0..3 {
                covariance[i][j] += delta[i] * delta[j];
            }
        }
    }
    for _ in 0..32 {
        let next = [
            dot(covariance[0], direction),
            dot(covariance[1], direction),
            dot(covariance[2], direction),
        ];
        direction = normalize(next)?;
    }
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut squared = 0.0;
    for point in points {
        let delta = sub(*point, centroid);
        let along = dot(delta, direction);
        low = low.min(along);
        high = high.max(along);
        let closest = add(centroid, scale(direction, along));
        squared += distance(*point, closest).powi(2);
    }
    Ok(LinearFeature {
        start: add(centroid, scale(direction, low)),
        end: add(centroid, scale(direction, high)),
        direction,
        rms_cross_track: (squared / points.len() as f64).sqrt(),
        point_count: points.len(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationOffset {
    pub station: f64,
    pub offset: f64,
    pub elevation: f64,
    pub segment: usize,
    pub closest: [f64; 3],
}

/// Project a survey point onto a 3D alignment polyline and report cumulative
/// station plus signed XY offset.
pub fn station_on_alignment(
    alignment: &[[f64; 3]],
    point: [f64; 3],
) -> Result<StationOffset, String> {
    if alignment.len() < 2 || alignment.iter().flatten().any(|value| !value.is_finite()) {
        return Err("alignment requires at least two finite vertices".into());
    }
    let mut best: Option<(f64, StationOffset)> = None;
    let mut accumulated = 0.0;
    for (segment, vertices) in alignment.windows(2).enumerate() {
        let vector = sub(vertices[1], vertices[0]);
        let length_squared = dot(vector, vector);
        if length_squared <= f64::EPSILON {
            continue;
        }
        let t = (dot(sub(point, vertices[0]), vector) / length_squared).clamp(0.0, 1.0);
        let closest = add(vertices[0], scale(vector, t));
        let distance_squared = dot(sub(point, closest), sub(point, closest));
        let cross = vector[0] * (point[1] - closest[1]) - vector[1] * (point[0] - closest[0]);
        let offset =
            distance([point[0], point[1], 0.0], [closest[0], closest[1], 0.0]) * cross.signum();
        let candidate = StationOffset {
            station: accumulated + length_squared.sqrt() * t,
            offset,
            elevation: point[2],
            segment,
            closest,
        };
        if best
            .as_ref()
            .is_none_or(|(best_distance, _)| distance_squared < *best_distance)
        {
            best = Some((distance_squared, candidate));
        }
        accumulated += length_squared.sqrt();
    }
    best.map(|(_, station)| station)
        .ok_or_else(|| "alignment contains only duplicate vertices".into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceComparison {
    pub cell_area: f64,
    pub compared_cells: usize,
    pub cut_volume: f64,
    pub fill_volume: f64,
    pub minimum_delta: f64,
    pub maximum_delta: f64,
    pub mean_delta: f64,
    pub deltas: Vec<Option<f64>>,
}

pub fn compare_surfaces(
    before: &[Option<f64>],
    after: &[Option<f64>],
    cell_width: f64,
    cell_height: f64,
) -> Result<SurfaceComparison, String> {
    if before.len() != after.len()
        || before.is_empty()
        || !cell_width.is_finite()
        || !cell_height.is_finite()
        || cell_width <= 0.0
        || cell_height <= 0.0
    {
        return Err("surface grids must match and use positive finite cell dimensions".into());
    }
    let area = cell_width * cell_height;
    let mut deltas = Vec::with_capacity(before.len());
    let mut compared = 0;
    let mut cut = 0.0;
    let mut fill = 0.0;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    let mut total = 0.0;
    for (before, after) in before.iter().zip(after) {
        let delta = before.zip(*after).map(|(before, after)| after - before);
        if let Some(delta) = delta {
            compared += 1;
            minimum = minimum.min(delta);
            maximum = maximum.max(delta);
            total += delta;
            if delta < 0.0 {
                cut += -delta * area;
            } else {
                fill += delta * area;
            }
        }
        deltas.push(delta);
    }
    if compared == 0 {
        return Err("surface grids have no mutually valid cells".into());
    }
    Ok(SurfaceComparison {
        cell_area: area,
        compared_cells: compared,
        cut_volume: cut,
        fill_volume: fill,
        minimum_delta: minimum,
        maximum_delta: maximum,
        mean_delta: total / compared as f64,
        deltas,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub candidate_index: usize,
    pub nearest_distance: f64,
    pub changed: bool,
}

pub fn detect_changes(
    reference: &[[f64; 3]],
    candidate: &[[f64; 3]],
    threshold: f64,
) -> Result<Vec<ChangeRecord>, String> {
    if reference.is_empty() || !threshold.is_finite() || threshold < 0.0 {
        return Err(
            "change detection requires reference points and a non-negative threshold".into(),
        );
    }
    Ok(candidate
        .iter()
        .enumerate()
        .map(|(candidate_index, point)| {
            let nearest_distance = reference
                .iter()
                .map(|reference| distance(*point, *reference))
                .fold(f64::INFINITY, f64::min);
            ChangeRecord {
                candidate_index,
                nearest_distance,
                changed: nearest_distance > threshold,
            }
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lod1Building {
    pub footprint: Geometry,
    pub base_elevation: f64,
    pub roof_elevation: f64,
    pub point_count: usize,
}

pub fn reconstruct_lod1_building(
    points: &[[f64; 3]],
    ground_elevation: f64,
) -> Result<Lod1Building, String> {
    if points.len() < 3 || !ground_elevation.is_finite() {
        return Err("building reconstruction requires three points and a ground elevation".into());
    }
    let mut xy: Vec<[f64; 2]> = points.iter().map(|point| [point[0], point[1]]).collect();
    xy.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then(left[1].total_cmp(&right[1]))
    });
    xy.dedup();
    if xy.len() < 3 {
        return Err("building points do not span a footprint".into());
    }
    let mut lower = Vec::new();
    for point in &xy {
        while lower.len() >= 2
            && cross2(lower[lower.len() - 2], lower[lower.len() - 1], *point) <= 0.0
        {
            lower.pop();
        }
        lower.push(*point);
    }
    let mut upper = Vec::new();
    for point in xy.iter().rev() {
        while upper.len() >= 2
            && cross2(upper[upper.len() - 2], upper[upper.len() - 1], *point) <= 0.0
        {
            upper.pop();
        }
        upper.push(*point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower.push(lower[0]);
    let mut elevations: Vec<f64> = points.iter().map(|point| point[2]).collect();
    elevations.sort_by(f64::total_cmp);
    let roof_elevation = elevations[elevations.len() / 2];
    if roof_elevation <= ground_elevation {
        return Err("roof elevation must be above ground".into());
    }
    Ok(Lod1Building {
        footprint: Geometry::Polygon(vec![lower]),
        base_elevation: ground_elevation,
        roof_elevation,
        point_count: points.len(),
    })
}

pub fn reality_tools() -> Vec<ToolDescriptor> {
    [
        (
            "reality.fit.plane",
            "Fit plane",
            UndoBehavior::DerivedOutput,
        ),
        (
            "reality.fit.sphere",
            "Fit sphere",
            UndoBehavior::DerivedOutput,
        ),
        (
            "reality.extract.linear",
            "Extract linear feature",
            UndoBehavior::DerivedOutput,
        ),
        (
            "reality.surface.compare",
            "Compare surfaces",
            UndoBehavior::DerivedOutput,
        ),
        (
            "reality.change.detect",
            "Detect change",
            UndoBehavior::DerivedOutput,
        ),
        (
            "reality.building.lod1",
            "Build LOD1 building",
            UndoBehavior::DerivedOutput,
        ),
    ]
    .into_iter()
    .map(|(id, name, undo)| ToolDescriptor {
        id: id.into(),
        name: name.into(),
        category: "Reality to Model".into(),
        description: format!("OpenCADStudio {name}"),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        requirements: ToolRequirements {
            density: DensityRequirement::FullSource,
            requires_crs: true,
            background: true,
            cancellable: true,
            checkpointable: true,
            undo,
            ..Default::default()
        },
        api_version: 1,
    })
    .collect()
}

fn solve<const N: usize>(mut matrix: [[f64; N]; N], mut rhs: [f64; N]) -> Option<[f64; N]> {
    for pivot in 0..N {
        let row = (pivot..N).max_by(|left, right| {
            matrix[*left][pivot]
                .abs()
                .total_cmp(&matrix[*right][pivot].abs())
        })?;
        if matrix[row][pivot].abs() <= 1e-12 {
            return None;
        }
        matrix.swap(pivot, row);
        rhs.swap(pivot, row);
        let divisor = matrix[pivot][pivot];
        for column in pivot..N {
            matrix[pivot][column] /= divisor;
        }
        rhs[pivot] /= divisor;
        for row in 0..N {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot];
            for column in pivot..N {
                matrix[row][column] -= factor * matrix[pivot][column];
            }
            rhs[row] -= factor * rhs[pivot];
        }
    }
    Some(rhs)
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}
fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}
fn sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}
fn scale(value: [f64; 3], factor: f64) -> [f64; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}
fn normalize(value: [f64; 3]) -> Result<[f64; 3], String> {
    let length = dot(value, value).sqrt();
    (length > f64::EPSILON)
        .then(|| scale(value, 1.0 / length))
        .ok_or_else(|| "direction is degenerate".into())
}
fn distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    dot(sub(left, right), sub(left, right)).sqrt()
}
fn mean_point(points: &[[f64; 3]]) -> [f64; 3] {
    scale(
        points.iter().copied().fold([0.0; 3], add),
        1.0 / points.len() as f64,
    )
}
fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_plane_sphere_and_linear_feature() {
        let plane_points: Vec<[f64; 3]> = (-2..=2)
            .flat_map(|x| {
                (-2..=2).map(move |y| [x as f64, y as f64, 2.0 * x as f64 - y as f64 + 5.0])
            })
            .collect();
        let plane = fit_plane(&plane_points).unwrap();
        assert!(plane.rms_error < 1e-10);
        let sphere_points = vec![
            [3.0, 2.0, 3.0],
            [-1.0, 2.0, 3.0],
            [1.0, 4.0, 3.0],
            [1.0, 0.0, 3.0],
            [1.0, 2.0, 5.0],
            [1.0, 2.0, 1.0],
        ];
        let sphere = fit_sphere(&sphere_points).unwrap();
        assert!(distance(sphere.center, [1.0, 2.0, 3.0]) < 1e-10);
        assert!((sphere.radius - 2.0).abs() < 1e-10);
        let line =
            extract_linear_feature(&[[0.0, 0.0, 0.0], [5.0, 5.0, 0.0], [10.0, 10.0, 0.0]]).unwrap();
        assert!(line.rms_cross_track < 1e-10);
    }

    #[test]
    fn reports_station_cut_fill_change_and_building() {
        let station = station_on_alignment(
            &[[0.0, 0.0, 0.0], [100.0, 0.0, 10.0], [100.0, 100.0, 20.0]],
            [100.0, 25.0, 12.0],
        )
        .unwrap();
        assert!((station.station - 125.573693).abs() < 1e-5);
        let comparison = compare_surfaces(
            &[Some(1.0), Some(2.0), Some(5.0), None],
            &[Some(2.0), Some(1.0), Some(5.5), Some(9.0)],
            2.0,
            2.0,
        )
        .unwrap();
        assert_eq!(comparison.fill_volume, 6.0);
        assert_eq!(comparison.cut_volume, 4.0);
        let changes =
            detect_changes(&[[0.0, 0.0, 0.0]], &[[0.1, 0.0, 0.0], [5.0, 0.0, 0.0]], 0.5).unwrap();
        assert_eq!(changes.iter().filter(|change| change.changed).count(), 1);
        let building = reconstruct_lod1_building(
            &[
                [0.0, 0.0, 10.0],
                [10.0, 0.0, 10.2],
                [10.0, 5.0, 9.9],
                [0.0, 5.0, 10.1],
            ],
            1.0,
        )
        .unwrap();
        assert!(matches!(building.footprint, Geometry::Polygon(_)));
        assert!(building.roof_elevation > 9.0);
        assert_eq!(reality_tools().len(), 6);
    }
}
