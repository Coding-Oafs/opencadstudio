//! Full-density processing extents that never depend on viewer residency.

use crate::{PointFilter, Result, SamplePoint, SelectionSet};
use las::Reader;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessingExtent {
    All,
    Bounds {
        min: [f64; 3],
        max: [f64; 3],
    },
    Slab {
        origin: [f64; 3],
        normal: [f64; 3],
        total_width: f64,
        vertical_limits: Option<[f64; 2]>,
    },
    Polygon {
        vertices: Vec<[f64; 2]>,
        vertical_limits: Option<[f64; 2]>,
    },
}

impl Default for ProcessingExtent {
    fn default() -> Self {
        Self::All
    }
}

impl ProcessingExtent {
    pub fn validate(&self) -> std::result::Result<(), String> {
        match self {
            Self::All => Ok(()),
            Self::Bounds { min, max } => {
                if min.iter().chain(max).copied().all(f64::is_finite)
                    && min.iter().zip(max).all(|(low, high)| low <= high)
                {
                    Ok(())
                } else {
                    Err("processing bounds must be finite and ordered".to_string())
                }
            }
            Self::Slab {
                origin,
                normal,
                total_width,
                vertical_limits,
            } => {
                validate_vertical_limits(*vertical_limits)?;
                let normal_length_sq = normal.iter().map(|value| value * value).sum::<f64>();
                if origin.iter().copied().all(f64::is_finite)
                    && normal.iter().copied().all(f64::is_finite)
                    && normal_length_sq > f64::EPSILON
                    && total_width.is_finite()
                    && *total_width > 0.0
                {
                    Ok(())
                } else {
                    Err("processing slab must have a valid plane and positive width".to_string())
                }
            }
            Self::Polygon {
                vertices,
                vertical_limits,
            } => {
                validate_vertical_limits(*vertical_limits)?;
                if vertices.len() >= 3 && vertices.iter().flatten().copied().all(f64::is_finite) {
                    Ok(())
                } else {
                    Err("processing polygon requires at least three finite vertices".to_string())
                }
            }
        }
    }

    pub fn contains(&self, position: [f64; 3]) -> bool {
        match self {
            Self::All => true,
            Self::Bounds { min, max } => {
                (0..3).all(|axis| position[axis] >= min[axis] && position[axis] <= max[axis])
            }
            Self::Slab {
                origin,
                normal,
                total_width,
                vertical_limits,
            } => {
                if !within_vertical(position[2], *vertical_limits) {
                    return false;
                }
                let length = normal.iter().map(|value| value * value).sum::<f64>().sqrt();
                if length <= f64::EPSILON {
                    return false;
                }
                let signed_distance = (0..3)
                    .map(|axis| (position[axis] - origin[axis]) * normal[axis])
                    .sum::<f64>()
                    / length;
                signed_distance.abs() <= total_width * 0.5
            }
            Self::Polygon {
                vertices,
                vertical_limits,
            } => {
                within_vertical(position[2], *vertical_limits)
                    && point_in_polygon([position[0], position[1]], vertices)
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullDensityProgress {
    pub scanned: u64,
    pub selected: u64,
    pub source_total: u64,
}

/// Scans every physical source record and calls `visitor` only for records in
/// the processing extent and attribute filter. Display samples and resident
/// `.ocstiles` are intentionally never consulted.
pub fn visit_full_density(
    path: impl AsRef<Path>,
    extent: &ProcessingExtent,
    filter: &PointFilter,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(FullDensityProgress),
    mut visitor: impl FnMut(&SamplePoint) -> Result<()>,
) -> Result<FullDensityProgress> {
    extent.validate().map_err(crate::Error::Crs)?;
    let mut reader = Reader::from_path(path)?;
    let total = reader.header().number_of_points();
    let mut state = FullDensityProgress {
        source_total: total,
        ..Default::default()
    };
    while state.scanned < total {
        let point_data = reader.read_points((total - state.scanned).min(65_536))?;
        if point_data.is_empty() {
            break;
        }
        for point in point_data.points() {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(crate::Error::Cancelled("full-density processing"));
            }
            let point = SamplePoint::from_point(state.scanned, point?);
            state.scanned += 1;
            if extent.contains(point.position) && filter.matches(&point) {
                visitor(&point)?;
                state.selected += 1;
            }
        }
        progress(state.clone());
    }
    Ok(state)
}

pub fn select_full_density(
    path: impl AsRef<Path>,
    name: impl Into<String>,
    extent: &ProcessingExtent,
    filter: &PointFilter,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(FullDensityProgress),
) -> Result<(SelectionSet, FullDensityProgress)> {
    let mut indices = Vec::new();
    let result = visit_full_density(path, extent, filter, cancel, progress, |point| {
        indices.push(point.source_index);
        Ok(())
    })?;
    Ok((SelectionSet::from_indices(name, indices), result))
}

fn validate_vertical_limits(limits: Option<[f64; 2]>) -> std::result::Result<(), String> {
    if let Some([low, high]) = limits {
        if !low.is_finite() || !high.is_finite() || low > high {
            return Err("vertical limits must be finite and ordered".to_string());
        }
    }
    Ok(())
}

fn within_vertical(z: f64, limits: Option<[f64; 2]>) -> bool {
    limits.is_none_or(|[low, high]| z >= low && z <= high)
}

fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        let crosses = (a[1] > point[1]) != (b[1] > point[1]);
        if crosses {
            let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] <= x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_slab_membership_does_not_depend_on_camera_state() {
        let slab = ProcessingExtent::Slab {
            origin: [100.0, 200.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            total_width: 10.0,
            vertical_limits: Some([0.0, 50.0]),
        };
        assert!(slab.contains([250.0, 204.9, 25.0]));
        assert!(!slab.contains([100.0, 205.1, 25.0]));
        assert!(!slab.contains([100.0, 200.0, 60.0]));
    }

    #[test]
    fn polygon_extent_handles_boundary_and_vertical_limits() {
        let polygon = ProcessingExtent::Polygon {
            vertices: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            vertical_limits: Some([2.0, 8.0]),
        };
        assert!(polygon.contains([5.0, 5.0, 5.0]));
        assert!(!polygon.contains([15.0, 5.0, 5.0]));
        assert!(!polygon.contains([5.0, 5.0, 9.0]));
    }
}
