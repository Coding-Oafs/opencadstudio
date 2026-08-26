//! Survey-oriented point/cloud/surface measurements.

use crate::SamplePoint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorMeasurement {
    pub delta: [f64; 3],
    pub horizontal: f64,
    pub slope_distance: f64,
    pub vertical: f64,
    pub azimuth_degrees: f64,
    pub grade_percent: Option<f64>,
}

pub fn point_to_point(from: [f64; 3], to: [f64; 3]) -> VectorMeasurement {
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let horizontal = delta[0].hypot(delta[1]);
    let slope_distance = horizontal.hypot(delta[2]);
    let mut azimuth = delta[0].atan2(delta[1]).to_degrees();
    if azimuth < 0.0 {
        azimuth += 360.0;
    }
    VectorMeasurement {
        delta,
        horizontal,
        slope_distance,
        vertical: delta[2],
        azimuth_degrees: azimuth,
        grade_percent: (horizontal > f64::EPSILON).then_some(delta[2] / horizontal * 100.0),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaneMeasurement {
    pub signed_distance: f64,
    pub distance: f64,
    pub projected_point: [f64; 3],
}

pub fn point_to_plane(
    point: [f64; 3],
    plane_origin: [f64; 3],
    plane_normal: [f64; 3],
) -> Option<PlaneMeasurement> {
    let length = plane_normal
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if !length.is_finite() || length <= f64::EPSILON {
        return None;
    }
    let normal = plane_normal.map(|value| value / length);
    let signed_distance = (0..3)
        .map(|axis| (point[axis] - plane_origin[axis]) * normal[axis])
        .sum::<f64>();
    let projected_point = std::array::from_fn(|axis| point[axis] - signed_distance * normal[axis]);
    Some(PlaneMeasurement {
        signed_distance,
        distance: signed_distance.abs(),
        projected_point,
    })
}

pub trait SurfaceSampler {
    fn elevation_at(&self, x: f64, y: f64) -> Option<f64>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMeasurement {
    pub point: [f64; 3],
    pub surface_elevation: f64,
    pub signed_vertical_distance: f64,
}

pub fn point_to_surface(
    point: [f64; 3],
    surface: &impl SurfaceSampler,
) -> Option<SurfaceMeasurement> {
    let surface_elevation = surface.elevation_at(point[0], point[1])?;
    Some(SurfaceMeasurement {
        point,
        surface_elevation,
        signed_vertical_distance: point[2] - surface_elevation,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CloudDistanceStatistics {
    pub compared_points: u64,
    pub minimum: f64,
    pub maximum: f64,
    pub mean: f64,
    pub root_mean_square: f64,
    pub percentile_95: f64,
}

/// Exact nearest-neighbour cloud distance for bounded review selections. Large
/// production jobs should spatially partition inputs and combine the returned
/// statistics rather than materializing a viewer sample.
pub fn cloud_to_cloud(
    source: &[SamplePoint],
    reference: &[SamplePoint],
) -> Option<CloudDistanceStatistics> {
    if source.is_empty() || reference.is_empty() {
        return None;
    }
    let mut distances = Vec::with_capacity(source.len());
    let mut sum = 0.0;
    let mut sum_squares = 0.0;
    for point in source {
        let distance_sq = reference
            .iter()
            .map(|other| squared_distance(point.position, other.position))
            .fold(f64::INFINITY, f64::min);
        let distance = distance_sq.sqrt();
        distances.push(distance);
        sum += distance;
        sum_squares += distance_sq;
    }
    distances.sort_by(f64::total_cmp);
    let count = distances.len() as f64;
    let percentile_index = ((distances.len() - 1) as f64 * 0.95).round() as usize;
    Some(CloudDistanceStatistics {
        compared_points: distances.len() as u64,
        minimum: distances[0],
        maximum: *distances.last().unwrap_or(&distances[0]),
        mean: sum / count,
        root_mean_square: (sum_squares / count).sqrt(),
        percentile_95: distances[percentile_index],
    })
}

fn squared_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    (0..3)
        .map(|axis| {
            let delta = left[axis] - right[axis];
            delta * delta
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_vector_reports_survey_components() {
        let value = point_to_point([0.0, 0.0, 10.0], [3.0, 4.0, 15.0]);
        assert_eq!(5.0, value.horizontal);
        assert!((value.slope_distance - 50.0_f64.sqrt()).abs() < 1e-12);
        assert_eq!(Some(100.0), value.grade_percent);
    }

    #[test]
    fn plane_distance_is_signed_and_projects_point() {
        let value = point_to_plane([2.0, 3.0, 8.0], [0.0, 0.0, 5.0], [0.0, 0.0, 2.0]).unwrap();
        assert_eq!(3.0, value.signed_distance);
        assert_eq!([2.0, 3.0, 5.0], value.projected_point);
    }
}
