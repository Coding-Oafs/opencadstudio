//! Compact source-index selection sets and reusable attribute filters.

use crate::SamplePoint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRange {
    pub first: u64,
    pub last: u64,
}

/// Sorted inclusive ranges provide compact 64-bit selections without tying the
/// sidecar format to an in-memory bitmap implementation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionSet {
    pub name: String,
    ranges: Vec<IndexRange>,
    len: u64,
}

impl SelectionSet {
    pub fn from_indices(name: impl Into<String>, indices: impl IntoIterator<Item = u64>) -> Self {
        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_unstable();
        indices.dedup();
        let mut ranges: Vec<IndexRange> = Vec::new();
        for index in indices {
            if let Some(last) = ranges.last_mut() {
                if index == last.last.saturating_add(1) {
                    last.last = index;
                    continue;
                }
            }
            ranges.push(IndexRange {
                first: index,
                last: index,
            });
        }
        let len = ranges
            .iter()
            .map(|range| range.last - range.first + 1)
            .sum();
        Self {
            name: name.into(),
            ranges,
            len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn ranges(&self) -> &[IndexRange] {
        &self.ranges
    }

    pub fn contains(&self, index: u64) -> bool {
        self.ranges
            .binary_search_by(|range| {
                if index < range.first {
                    std::cmp::Ordering::Greater
                } else if index > range.last {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.ranges
            .iter()
            .flat_map(|range| range.first..=range.last)
    }

    pub fn union(&self, name: impl Into<String>, other: &Self) -> Self {
        Self::from_indices(name, self.iter().chain(other.iter()))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PointFilter {
    pub classes: Vec<u8>,
    pub returns: Vec<u8>,
    pub sources: Vec<u16>,
    pub elevation: Option<[f64; 2]>,
    pub intensity: Option<[u16; 2]>,
    pub synthetic: Option<bool>,
    pub key_point: Option<bool>,
    pub withheld: Option<bool>,
    pub overlap: Option<bool>,
}

impl PointFilter {
    pub fn matches(&self, point: &SamplePoint) -> bool {
        (self.classes.is_empty() || self.classes.contains(&point.classification))
            && (self.returns.is_empty() || self.returns.contains(&point.return_number))
            && (self.sources.is_empty() || self.sources.contains(&point.point_source_id))
            && self
                .elevation
                .is_none_or(|[low, high]| point.position[2] >= low && point.position[2] <= high)
            && self
                .intensity
                .is_none_or(|[low, high]| point.intensity >= low && point.intensity <= high)
            && self
                .synthetic
                .is_none_or(|value| point.is_synthetic == value)
            && self
                .key_point
                .is_none_or(|value| point.is_key_point == value)
            && self.withheld.is_none_or(|value| point.is_withheld == value)
            && self.overlap.is_none_or(|value| point.is_overlap == value)
    }
}

/// Selects points inside an XY polygon and optional Z slice. The returned set
/// always contains stable source indices suitable for an edit transaction.
pub fn select_polygon(
    points: &[SamplePoint],
    polygon: &[[f64; 2]],
    z_range: Option<[f64; 2]>,
    filter: &PointFilter,
) -> SelectionSet {
    if polygon.len() < 3 {
        return SelectionSet::default();
    }
    SelectionSet::from_indices(
        "polygon",
        points.iter().filter_map(|point| {
            let z_ok = z_range
                .is_none_or(|[low, high]| point.position[2] >= low && point.position[2] <= high);
            (z_ok && filter.matches(point) && point_in_polygon(point.position, polygon))
                .then_some(point.source_index)
        }),
    )
}

pub fn select_brush(
    points: &[SamplePoint],
    center: [f64; 3],
    radius: f64,
    filter: &PointFilter,
) -> SelectionSet {
    let radius_sq = radius.max(0.0).powi(2);
    SelectionSet::from_indices(
        "brush",
        points.iter().filter_map(|point| {
            let dx = point.position[0] - center[0];
            let dy = point.position[1] - center[1];
            let dz = point.position[2] - center[2];
            (filter.matches(point) && dx * dx + dy * dy + dz * dz <= radius_sq)
                .then_some(point.source_index)
        }),
    )
}

pub fn select_nearest(
    points: &[SamplePoint],
    position: [f64; 3],
    max_distance: f64,
    filter: &PointFilter,
) -> SelectionSet {
    let limit_sq = max_distance.max(0.0).powi(2);
    let nearest = points
        .iter()
        .filter(|point| filter.matches(point))
        .filter_map(|point| {
            let distance_sq = point
                .position
                .iter()
                .zip(position)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>();
            (distance_sq <= limit_sq).then_some((distance_sq, point.source_index))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, index)| index);
    SelectionSet::from_indices("single point", nearest)
}

fn point_in_polygon(position: [f64; 3], polygon: &[[f64; 2]]) -> bool {
    let [x, y, _] = position;
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        let crosses = (current[1] > y) != (previous[1] > y)
            && x < (previous[0] - current[0]) * (y - current[1]) / (previous[1] - current[1])
                + current[0];
        if crosses {
            inside = !inside;
        }
        previous = current;
    }
    inside
}
