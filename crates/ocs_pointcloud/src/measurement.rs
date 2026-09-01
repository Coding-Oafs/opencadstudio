//! Survey-oriented point/cloud/surface measurements.

use crate::SamplePoint;
use serde::{Deserialize, Serialize};
use std::fmt;

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

/// Hard ceiling for one interactive drape operation. This prevents an
/// accidental microscopic spacing value from allocating an unbounded path.
pub const MAX_DRAPED_PATH_POINTS: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum DrapeError {
    NotEnoughPoints,
    InvalidSpacing,
    InvalidVerticalOffset,
    NonFinitePoint {
        point_index: usize,
    },
    TooManyPoints {
        requested: usize,
        maximum: usize,
    },
    OutsideSurface {
        sample_index: usize,
        position: [f64; 2],
    },
}

impl fmt::Display for DrapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughPoints => formatter.write_str("a path needs at least two points"),
            Self::InvalidSpacing => formatter.write_str("spacing must be finite and positive"),
            Self::InvalidVerticalOffset => formatter.write_str("vertical offset must be finite"),
            Self::NonFinitePoint { point_index } => {
                write!(formatter, "source point {point_index} is not finite")
            }
            Self::TooManyPoints { requested, maximum } => write!(
                formatter,
                "drape would create {requested} points, over the {maximum} point limit"
            ),
            Self::OutsideSurface {
                sample_index,
                position,
            } => write!(
                formatter,
                "sample {sample_index} at ({:.6}, {:.6}) is outside the surface",
                position[0], position[1]
            ),
        }
    }
}

impl std::error::Error for DrapeError {}

#[derive(Clone, Debug, PartialEq)]
pub struct DrapedPath {
    pub points: Vec<[f64; 3]>,
    pub source_points: usize,
    pub generated_points: usize,
    pub closed: bool,
}

/// Densifies straight path segments in XY and samples every output point from
/// a terrain surface. The operation is atomic: if any sample is outside the
/// surface, no partial path is returned.
pub fn drape_path(
    points: &[[f64; 3]],
    closed: bool,
    maximum_segment_length: f64,
    vertical_offset: f64,
    surface: &impl SurfaceSampler,
) -> Result<DrapedPath, DrapeError> {
    if points.len() < 2 {
        return Err(DrapeError::NotEnoughPoints);
    }
    if !maximum_segment_length.is_finite() || maximum_segment_length <= 0.0 {
        return Err(DrapeError::InvalidSpacing);
    }
    if !vertical_offset.is_finite() {
        return Err(DrapeError::InvalidVerticalOffset);
    }
    for (point_index, point) in points.iter().enumerate() {
        if !point.iter().all(|coordinate| coordinate.is_finite()) {
            return Err(DrapeError::NonFinitePoint { point_index });
        }
    }

    let segment_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut subdivisions = Vec::with_capacity(segment_count);
    let mut requested = 1_usize;
    for segment_index in 0..segment_count {
        let start = points[segment_index];
        let end = points[(segment_index + 1) % points.len()];
        let horizontal_length = (end[0] - start[0]).hypot(end[1] - start[1]);
        let steps_f64 = (horizontal_length / maximum_segment_length).ceil().max(1.0);
        if steps_f64 > MAX_DRAPED_PATH_POINTS as f64 {
            return Err(DrapeError::TooManyPoints {
                requested: MAX_DRAPED_PATH_POINTS.saturating_add(1),
                maximum: MAX_DRAPED_PATH_POINTS,
            });
        }
        let steps = steps_f64 as usize;
        requested = requested.saturating_add(steps);
        subdivisions.push(steps);
    }
    if closed {
        // The last segment ends at the already-emitted first point.
        requested = requested.saturating_sub(1);
    }
    if requested > MAX_DRAPED_PATH_POINTS {
        return Err(DrapeError::TooManyPoints {
            requested,
            maximum: MAX_DRAPED_PATH_POINTS,
        });
    }

    let mut draped = Vec::with_capacity(requested);
    let sample = |x: f64, y: f64, sample_index: usize| {
        surface
            .elevation_at(x, y)
            .map(|z| [x, y, z + vertical_offset])
            .ok_or(DrapeError::OutsideSurface {
                sample_index,
                position: [x, y],
            })
    };
    draped.push(sample(points[0][0], points[0][1], 0)?);
    for segment_index in 0..segment_count {
        let start = points[segment_index];
        let end = points[(segment_index + 1) % points.len()];
        let steps = subdivisions[segment_index];
        for step in 1..=steps {
            if closed && segment_index + 1 == segment_count && step == steps {
                break;
            }
            let fraction = step as f64 / steps as f64;
            let x = start[0] + (end[0] - start[0]) * fraction;
            let y = start[1] + (end[1] - start[1]) * fraction;
            let sample_index = draped.len();
            draped.push(sample(x, y, sample_index)?);
        }
    }

    Ok(DrapedPath {
        generated_points: draped.len().saturating_sub(points.len()),
        points: draped,
        source_points: points.len(),
        closed,
    })
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

    struct Plane;

    impl SurfaceSampler for Plane {
        fn elevation_at(&self, x: f64, y: f64) -> Option<f64> {
            (x >= 0.0 && y >= 0.0 && x <= 10.0 && y <= 10.0).then_some(2.0 * x - y)
        }
    }

    #[test]
    fn drape_path_densifies_and_samples_a_surface() {
        let result = drape_path(
            &[[0.0, 0.0, 99.0], [4.0, 0.0, 99.0]],
            false,
            1.5,
            0.25,
            &Plane,
        )
        .expect("drape");
        assert_eq!(4, result.points.len());
        assert_eq!(2, result.generated_points);
        assert_eq!([0.0, 0.0, 0.25], result.points[0]);
        assert_eq!([4.0, 0.0, 8.25], result.points[3]);
        for pair in result.points.windows(2) {
            let distance = (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
            assert!(distance <= 1.5 + f64::EPSILON);
        }
    }

    #[test]
    fn closed_drape_does_not_duplicate_the_first_point() {
        let result = drape_path(
            &[[1.0, 1.0, 0.0], [3.0, 1.0, 0.0], [3.0, 3.0, 0.0]],
            true,
            10.0,
            0.0,
            &Plane,
        )
        .expect("drape");
        assert!(result.closed);
        assert_eq!(3, result.points.len());
        assert_ne!(result.points.first(), result.points.last());
    }

    #[test]
    fn drape_fails_atomically_outside_the_surface() {
        let error = drape_path(
            &[[1.0, 1.0, 0.0], [12.0, 1.0, 0.0]],
            false,
            2.0,
            0.0,
            &Plane,
        )
        .expect_err("outside surface");
        assert!(matches!(error, DrapeError::OutsideSurface { .. }));
    }

    #[test]
    fn drape_rejects_unbounded_subdivision() {
        let error = drape_path(
            &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            false,
            1.0e-12,
            0.0,
            &Plane,
        )
        .expect_err("point limit");
        assert!(matches!(error, DrapeError::TooManyPoints { .. }));
    }

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
