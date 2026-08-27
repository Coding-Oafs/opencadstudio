//! Extensible production classifier pipeline for full-density working sets.

use crate::{
    classify_by_rules, classify_ground, detect_noise, ClassifyResult, ClassifyRule, GroundOptions,
    RasterSurface, SamplePoint, SurfaceSampler,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildingClassifier {
    pub footprints: Vec<Vec<[f64; 2]>>,
    pub from_classes: Vec<u8>,
    pub target_class: u8,
    pub min_height_above_ground: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoadCenterline {
    pub vertices: Vec<[f64; 2]>,
    pub total_width: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoadClassifier {
    pub centerlines: Vec<RoadCenterline>,
    pub edge_allowance: f64,
    pub from_classes: Vec<u8>,
    pub target_class: u8,
    pub max_height_above_ground: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VegetationClassifier {
    pub from_classes: Vec<u8>,
    pub low_class: u8,
    pub medium_class: u8,
    pub high_class: u8,
    pub low_max_height: f64,
    pub medium_max_height: f64,
}

impl Default for VegetationClassifier {
    fn default() -> Self {
        Self {
            from_classes: vec![1, 3, 4, 5],
            low_class: 3,
            medium_class: 4,
            high_class: 5,
            low_max_height: 0.5,
            medium_max_height: 2.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClassifierStage {
    Ground(GroundStage),
    Noise(NoiseStage),
    Rules { rules: Vec<ClassifyRule> },
    Building(BuildingClassifier),
    Road(RoadClassifier),
    Vegetation(VegetationClassifier),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundStage {
    pub cell_size: f64,
    pub max_distance: f64,
    pub max_angle_degrees: f64,
    pub iterations: usize,
    pub ground_class: u8,
    pub reject_above: f64,
}

impl From<&GroundStage> for GroundOptions {
    fn from(value: &GroundStage) -> Self {
        Self {
            cell_size: value.cell_size,
            max_distance: value.max_distance,
            max_angle_degrees: value.max_angle_degrees,
            iterations: value.iterations,
            ground_class: value.ground_class,
            reject_above: value.reject_above,
        }
    }
}

impl Default for GroundStage {
    fn default() -> Self {
        let value = GroundOptions::default();
        Self {
            cell_size: value.cell_size,
            max_distance: value.max_distance,
            max_angle_degrees: value.max_angle_degrees,
            iterations: value.iterations,
            ground_class: value.ground_class,
            reject_above: value.reject_above,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoiseStage {
    pub radius: f64,
    pub min_neighbors: usize,
    pub target_class: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassificationPipeline {
    pub stages: Vec<ClassifierStage>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageStatistics {
    pub stage: String,
    pub changed: u64,
    pub histogram: BTreeMap<u8, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PipelineResult {
    pub patches: Vec<(u64, u8)>,
    pub stages: Vec<StageStatistics>,
    pub final_histogram: BTreeMap<u8, u64>,
}

impl ClassificationPipeline {
    /// Runs ordered stages. Each later stage sees the classes produced by all
    /// earlier stages, matching a professional classification macro.
    pub fn classify(
        &self,
        source: &[SamplePoint],
        ground_surface: Option<&RasterSurface>,
    ) -> PipelineResult {
        let mut working = source.to_vec();
        let mut accumulated = BTreeMap::<u64, u8>::new();
        let mut stage_stats = Vec::new();
        for stage in &self.stages {
            let (name, result) = match stage {
                ClassifierStage::Ground(options) => (
                    "ground".to_string(),
                    classify_ground(&working, &GroundOptions::from(options)),
                ),
                ClassifierStage::Noise(options) => (
                    "noise".to_string(),
                    detect_noise(
                        &working,
                        options.radius,
                        options.min_neighbors,
                        options.target_class,
                    ),
                ),
                ClassifierStage::Rules { rules } => {
                    ("rules".to_string(), classify_by_rules(&working, rules))
                }
                ClassifierStage::Building(options) => (
                    "building".to_string(),
                    classify_buildings(&working, options, ground_surface),
                ),
                ClassifierStage::Road(options) => (
                    "road".to_string(),
                    classify_roads(&working, options, ground_surface),
                ),
                ClassifierStage::Vegetation(options) => (
                    "vegetation".to_string(),
                    classify_vegetation(&working, options, ground_surface),
                ),
            };
            let mut histogram = BTreeMap::new();
            for (index, classification) in result.patches {
                if let Some(point) = working.iter_mut().find(|point| point.source_index == index) {
                    point.classification = classification;
                    accumulated.insert(index, classification);
                    *histogram.entry(classification).or_default() += 1;
                }
            }
            stage_stats.push(StageStatistics {
                stage: name,
                changed: histogram.values().sum(),
                histogram,
            });
        }
        let mut final_histogram = BTreeMap::new();
        for point in &working {
            *final_histogram.entry(point.classification).or_default() += 1;
        }
        PipelineResult {
            patches: accumulated.into_iter().collect(),
            stages: stage_stats,
            final_histogram,
        }
    }
}

pub fn classify_buildings(
    points: &[SamplePoint],
    options: &BuildingClassifier,
    ground: Option<&impl SurfaceSampler>,
) -> ClassifyResult {
    let patches = points
        .iter()
        .filter(|point| class_allowed(point.classification, &options.from_classes))
        .filter(|point| {
            options
                .footprints
                .iter()
                .any(|polygon| point_in_polygon([point.position[0], point.position[1]], polygon))
        })
        .filter(|point| {
            options.min_height_above_ground.is_none_or(|minimum| {
                ground
                    .and_then(|surface| surface.elevation_at(point.position[0], point.position[1]))
                    .is_some_and(|z| point.position[2] - z >= minimum)
            })
        })
        .map(|point| (point.source_index, options.target_class))
        .collect();
    ClassifyResult { patches }
}

pub fn classify_roads(
    points: &[SamplePoint],
    options: &RoadClassifier,
    ground: Option<&impl SurfaceSampler>,
) -> ClassifyResult {
    let patches = points
        .iter()
        .filter(|point| class_allowed(point.classification, &options.from_classes))
        .filter(|point| {
            options.centerlines.iter().any(|line| {
                let threshold = line.total_width.max(0.0) * 0.5 + options.edge_allowance.max(0.0);
                line.vertices.windows(2).any(|segment| {
                    point_segment_distance(
                        [point.position[0], point.position[1]],
                        segment[0],
                        segment[1],
                    ) <= threshold
                })
            })
        })
        .filter(|point| {
            options.max_height_above_ground.is_none_or(|maximum| {
                ground
                    .and_then(|surface| surface.elevation_at(point.position[0], point.position[1]))
                    .is_some_and(|z| (point.position[2] - z).abs() <= maximum)
            })
        })
        .map(|point| (point.source_index, options.target_class))
        .collect();
    ClassifyResult { patches }
}

pub fn classify_vegetation(
    points: &[SamplePoint],
    options: &VegetationClassifier,
    ground: Option<&impl SurfaceSampler>,
) -> ClassifyResult {
    let Some(ground) = ground else {
        return ClassifyResult::default();
    };
    let patches = points
        .iter()
        .filter(|point| class_allowed(point.classification, &options.from_classes))
        .filter_map(|point| {
            let elevation = ground.elevation_at(point.position[0], point.position[1])?;
            let height = point.position[2] - elevation;
            if height <= 0.0 {
                return None;
            }
            let classification = if height <= options.low_max_height {
                options.low_class
            } else if height <= options.medium_max_height {
                options.medium_class
            } else {
                options.high_class
            };
            Some((point.source_index, classification))
        })
        .collect();
    ClassifyResult { patches }
}

fn class_allowed(classification: u8, allowed: &[u8]) -> bool {
    allowed.is_empty() || allowed.contains(&classification)
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
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let crossing_x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] <= crossing_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn point_segment_distance(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let segment = [b[0] - a[0], b[1] - a[1]];
    let length_sq = segment[0] * segment[0] + segment[1] * segment[1];
    if length_sq <= f64::EPSILON {
        return (point[0] - a[0]).hypot(point[1] - a[1]);
    }
    let t = (((point[0] - a[0]) * segment[0] + (point[1] - a[1]) * segment[1]) / length_sq)
        .clamp(0.0, 1.0);
    let closest = [a[0] + t * segment[0], a[1] + t * segment[1]];
    (point[0] - closest[0]).hypot(point[1] - closest[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(index: u64, position: [f64; 3], classification: u8) -> SamplePoint {
        SamplePoint {
            source_index: index,
            position,
            intensity: 0,
            classification,
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

    #[test]
    fn ordered_building_then_road_pipeline_has_deterministic_priority() {
        let points = vec![point(7, [5.0, 5.0, 1.0], 1)];
        let pipeline = ClassificationPipeline {
            stages: vec![
                ClassifierStage::Building(BuildingClassifier {
                    footprints: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]],
                    from_classes: vec![1],
                    target_class: 6,
                    min_height_above_ground: None,
                }),
                ClassifierStage::Road(RoadClassifier {
                    centerlines: vec![RoadCenterline {
                        vertices: vec![[0.0, 5.0], [10.0, 5.0]],
                        total_width: 4.0,
                    }],
                    edge_allowance: 0.0,
                    from_classes: vec![6],
                    target_class: 11,
                    max_height_above_ground: None,
                }),
            ],
        };
        let result = pipeline.classify(&points, None);
        assert_eq!(vec![(7, 11)], result.patches);
        assert_eq!(Some(&1), result.final_histogram.get(&11));
    }
}
