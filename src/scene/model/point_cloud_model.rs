//! Session-only point data consumed by the native GPU point pipeline.

use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointCloudPoint {
    pub position: [f64; 3],
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointCloudModel {
    pub points: Arc<Vec<PointCloudPoint>>,
    pub point_size_px: f32,
    /// Changes whenever the point set, colours, visibility or edits change.
    pub generation: u64,
}

impl Default for PointCloudModel {
    fn default() -> Self {
        Self {
            points: Arc::new(Vec::new()),
            point_size_px: 3.0,
            generation: 0,
        }
    }
}
