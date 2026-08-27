//! Deterministic topology checks for editable feature layers.

use crate::{FeatureLayer, Geometry};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TopologyRules {
    pub require_closed_rings: bool,
    pub reject_self_intersections: bool,
    pub reject_overlaps: bool,
    pub reject_duplicate_vertices: bool,
}

impl TopologyRules {
    pub fn editing_defaults() -> Self {
        Self {
            require_closed_rings: true,
            reject_self_intersections: true,
            reject_overlaps: true,
            reject_duplicate_vertices: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyIssueKind {
    OpenRing,
    DuplicateVertex,
    SelfIntersection,
    Overlap,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopologyIssue {
    pub kind: TopologyIssueKind,
    pub feature_ids: Vec<u64>,
    pub location: Option<[f64; 2]>,
    pub detail: String,
}

pub fn validate_topology(layer: &FeatureLayer, rules: TopologyRules) -> Vec<TopologyIssue> {
    let mut issues = Vec::new();
    for feature in &layer.features {
        for ring in rings(&feature.geometry) {
            if rules.require_closed_rings && ring.first() != ring.last() {
                issues.push(TopologyIssue {
                    kind: TopologyIssueKind::OpenRing,
                    feature_ids: vec![feature.id],
                    location: ring.first().copied(),
                    detail: "polygon ring is not closed".into(),
                });
            }
            if rules.reject_duplicate_vertices {
                for pair in ring.windows(2) {
                    if pair[0] == pair[1] {
                        issues.push(TopologyIssue {
                            kind: TopologyIssueKind::DuplicateVertex,
                            feature_ids: vec![feature.id],
                            location: Some(pair[0]),
                            detail: "consecutive polygon vertices are identical".into(),
                        });
                    }
                }
            }
            if rules.reject_self_intersections {
                for (a_index, a) in ring.windows(2).enumerate() {
                    for (b_index, b) in ring.windows(2).enumerate().skip(a_index + 2) {
                        if b_index == a_index + 1 || (a_index == 0 && b_index + 1 == ring.len() - 1)
                        {
                            continue;
                        }
                        if let Some(location) = segment_intersection(a[0], a[1], b[0], b[1]) {
                            issues.push(TopologyIssue {
                                kind: TopologyIssueKind::SelfIntersection,
                                feature_ids: vec![feature.id],
                                location: Some(location),
                                detail: "polygon ring crosses itself".into(),
                            });
                        }
                    }
                }
            }
        }
    }
    if rules.reject_overlaps {
        for (left_index, left) in layer.features.iter().enumerate() {
            let Some(left_bounds) = left.geometry.envelope() else {
                continue;
            };
            for right in layer.features.iter().skip(left_index + 1) {
                let Some(right_bounds) = right.geometry.envelope() else {
                    continue;
                };
                if !bounds_overlap_area(left_bounds, right_bounds) {
                    continue;
                }
                let sample = geometry_vertices(&left.geometry)
                    .into_iter()
                    .find(|point| right.geometry.contains(point[0], point[1]))
                    .or_else(|| {
                        geometry_vertices(&right.geometry)
                            .into_iter()
                            .find(|point| left.geometry.contains(point[0], point[1]))
                    });
                if let Some(location) = sample {
                    issues.push(TopologyIssue {
                        kind: TopologyIssueKind::Overlap,
                        feature_ids: vec![left.id, right.id],
                        location: Some(location),
                        detail: "polygon interiors overlap".into(),
                    });
                }
            }
        }
    }
    issues
}

fn rings(geometry: &Geometry) -> Vec<&Vec<[f64; 2]>> {
    match geometry {
        Geometry::Polygon(rings) => rings.iter().collect(),
        Geometry::MultiPolygon(polygons) => {
            polygons.iter().flat_map(|rings| rings.iter()).collect()
        }
        _ => Vec::new(),
    }
}

fn geometry_vertices(geometry: &Geometry) -> Vec<[f64; 2]> {
    match geometry {
        Geometry::Polygon(rings) => rings.iter().flatten().copied().collect(),
        Geometry::MultiPolygon(polygons) => polygons.iter().flatten().flatten().copied().collect(),
        _ => Vec::new(),
    }
}

fn bounds_overlap_area(left: [f64; 4], right: [f64; 4]) -> bool {
    left[0].max(right[0]) < left[2].min(right[2]) && left[1].max(right[1]) < left[3].min(right[3])
}

fn segment_intersection(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let denominator = (a[0] - b[0]) * (c[1] - d[1]) - (a[1] - b[1]) * (c[0] - d[0]);
    if denominator.abs() <= 1e-12 {
        return None;
    }
    let ab = a[0] * b[1] - a[1] * b[0];
    let cd = c[0] * d[1] - c[1] * d[0];
    let x = (ab * (c[0] - d[0]) - (a[0] - b[0]) * cd) / denominator;
    let y = (ab * (c[1] - d[1]) - (a[1] - b[1]) * cd) / denominator;
    let within = |value: f64, first: f64, second: f64| {
        value >= first.min(second) - 1e-9 && value <= first.max(second) + 1e-9
    };
    (within(x, a[0], b[0])
        && within(y, a[1], b[1])
        && within(x, c[0], d[0])
        && within(y, c[1], d[1]))
    .then_some([x, y])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FeatureLayer;
    use std::collections::BTreeMap;

    #[test]
    fn finds_open_self_crossing_and_overlapping_polygons() {
        let mut layer = FeatureLayer::new("lots", 4326);
        layer.push(
            Geometry::Polygon(vec![vec![
                [0.0, 0.0],
                [4.0, 4.0],
                [0.0, 4.0],
                [4.0, 0.0],
                [0.0, 0.0],
            ]]),
            BTreeMap::new(),
        );
        layer.push(
            Geometry::Polygon(vec![vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]]]),
            BTreeMap::new(),
        );
        let issues = validate_topology(&layer, TopologyRules::editing_defaults());
        assert!(issues
            .iter()
            .any(|issue| issue.kind == TopologyIssueKind::SelfIntersection));
        assert!(issues
            .iter()
            .any(|issue| issue.kind == TopologyIssueKind::OpenRing));
        assert!(issues
            .iter()
            .any(|issue| issue.kind == TopologyIssueKind::Overlap));
    }
}
