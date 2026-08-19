//! Persistent point-cloud display modes and editable class definitions.

use crate::SamplePoint;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    #[default]
    Classification,
    Rgb,
    Intensity,
    Elevation,
    ReturnNumber,
    PointSource,
}

/// How much of a source cloud to load for display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    /// An approximate uniform sample capped by the display point budget.
    #[default]
    Auto,
    /// Keep every Nth source point (an explicit 1-in-N decimation).
    EveryNth(u64),
    /// Keep every point (no decimation) — may exceed memory for large clouds.
    Full,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    pub color_mode: ColorMode,
    /// Camera-facing point diameter in physical pixels.
    pub point_size_px: f32,
    pub point_budget: usize,
    pub gpu_budget_bytes: usize,
    pub cpu_budget_bytes: usize,
    pub hidden_classes: BTreeSet<u8>,
    pub intensity_range: Option<[u16; 2]>,
    pub elevation_range: Option<[f64; 2]>,
    /// Load density for the display sample (see [`Density`]).
    #[serde(default)]
    pub density: Density,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            color_mode: ColorMode::Classification,
            point_size_px: 3.0,
            point_budget: 4_000_000,
            gpu_budget_bytes: 512 * 1024 * 1024,
            cpu_budget_bytes: 1024 * 1024 * 1024,
            hidden_classes: BTreeSet::new(),
            intensity_range: None,
            elevation_range: None,
            density: Density::Auto,
        }
    }
}

impl DisplaySettings {
    pub fn normalized(mut self) -> Self {
        self.point_size_px = self.point_size_px.clamp(1.0, 32.0);
        self.point_budget = self.point_budget.clamp(1_000, 100_000_000);
        self.gpu_budget_bytes = self
            .gpu_budget_bytes
            .clamp(32 * 1024 * 1024, 16 * 1024 * 1024 * 1024);
        self.cpu_budget_bytes = self
            .cpu_budget_bytes
            .clamp(64 * 1024 * 1024, 64 * 1024 * 1024 * 1024);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassDefinition {
    pub code: u8,
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassTable {
    pub classes: BTreeMap<u8, ClassDefinition>,
}

impl Default for ClassTable {
    fn default() -> Self {
        let mut table = Self {
            classes: BTreeMap::new(),
        };
        for (code, name, color) in [
            (0, "Created, never classified", [140, 140, 140]),
            (1, "Unclassified", [210, 210, 210]),
            (2, "Ground", [163, 107, 56]),
            (3, "Low vegetation", [115, 199, 87]),
            (4, "Medium vegetation", [51, 166, 56]),
            (5, "High vegetation", [13, 107, 26]),
            (6, "Building", [230, 56, 46]),
            (7, "Low point (noise)", [219, 51, 199]),
            (9, "Water", [41, 122, 242]),
            (10, "Rail", [120, 120, 120]),
            (11, "Road surface", [75, 75, 75]),
            (12, "Overlap", [255, 199, 31]),
            (13, "Wire guard", [235, 170, 120]),
            (14, "Wire conductor", [235, 120, 120]),
            (15, "Transmission tower", [190, 90, 210]),
            (16, "Wire connector", [250, 170, 50]),
            (17, "Bridge deck", [38, 217, 242]),
            (18, "High noise", [219, 51, 199]),
        ] {
            table.upsert(ClassDefinition {
                code,
                name: name.into(),
                color,
                visible: true,
                locked: false,
            });
        }
        table
    }
}

impl ClassTable {
    pub fn upsert(&mut self, definition: ClassDefinition) {
        self.classes.insert(definition.code, definition);
    }

    pub fn remove(&mut self, code: u8) -> Option<ClassDefinition> {
        self.classes.remove(&code)
    }

    pub fn color(&self, code: u8) -> [u8; 3] {
        self.classes
            .get(&code)
            .map_or([235, 235, 235], |definition| definition.color)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassStatistics {
    pub total: u64,
    pub synthetic: u64,
    pub key_points: u64,
    pub withheld: u64,
    pub overlap: u64,
}

pub fn classification_statistics(
    points: impl IntoIterator<Item = SamplePoint>,
) -> BTreeMap<u8, ClassStatistics> {
    let mut result: BTreeMap<u8, ClassStatistics> = BTreeMap::new();
    for point in points {
        let stats = result.entry(point.classification).or_default();
        stats.total += 1;
        stats.synthetic += u64::from(point.is_synthetic);
        stats.key_points += u64::from(point.is_key_point);
        stats.withheld += u64::from(point.is_withheld);
        stats.overlap += u64::from(point.is_overlap);
    }
    result
}
