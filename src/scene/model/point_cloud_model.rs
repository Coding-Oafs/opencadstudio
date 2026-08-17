//! Session-only point data consumed by the native GPU point pipeline.
//!
//! Points carry source attributes, not resolved colors: the GPU computes
//! colorization from [`PointStyle`] each frame, so changing color mode, class
//! visibility, or the class table costs one small uniform write instead of a
//! full instance-buffer rebuild.

use std::sync::Arc;

/// One displayable LiDAR point with the attributes the shader colors from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointCloudPoint {
    pub position: [f64; 3],
    pub classification: u8,
    pub intensity: u16,
    pub return_number: u8,
    pub point_source_id: u16,
    /// 16-bit source RGB; `None` falls back to the class color.
    pub color: Option<[u16; 3]>,
    pub selected: bool,
}

/// Shader color-mode discriminants. Keep in sync with `point_cloud.wgsl`.
pub const COLOR_MODE_CLASSIFICATION: u32 = 0;
pub const COLOR_MODE_RGB: u32 = 1;
pub const COLOR_MODE_INTENSITY: u32 = 2;
pub const COLOR_MODE_ELEVATION: u32 = 3;
pub const COLOR_MODE_RETURN: u32 = 4;
pub const COLOR_MODE_SOURCE: u32 = 5;

pub const CLASS_COUNT: usize = 256;

/// One uploadable region of the point stream: a source's tile (or whole
/// bounded sample) with a stable identity and a content revision. The GPU
/// arena pages chunks in and out individually instead of rebuilding the
/// entire instance buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointChunk {
    /// Stable identity of the content (source id + tile); equal keys mean
    /// the same region of the same data.
    pub key: u64,
    /// Content revision: changes when rendered attributes inside the chunk
    /// change (edits, selections).
    pub generation: u64,
    /// Range within `PointCloudModel::points`.
    pub offset: u32,
    pub len: u32,
}

/// Per-frame colorization state uploaded as one uniform write.
#[derive(Clone, Debug, PartialEq)]
pub struct PointStyle {
    pub color_mode: u32,
    pub point_size_px: f32,
    /// One visibility bit per ASPRS class (`class_visible[c / 32] >> c % 32`).
    pub class_visible: [u32; 8],
    /// Linear RGBA per class; unknown classes use the table's fallback.
    pub class_colors: [[f32; 4]; CLASS_COUNT],
    pub intensity_range: [f32; 2],
    pub elevation_range: [f32; 2],
}

impl Default for PointStyle {
    fn default() -> Self {
        Self {
            color_mode: COLOR_MODE_CLASSIFICATION,
            point_size_px: 3.0,
            class_visible: [u32::MAX; 8],
            class_colors: [[0.92, 0.92, 0.92, 1.0]; CLASS_COUNT],
            intensity_range: [0.0, u16::MAX as f32],
            elevation_range: [0.0, 0.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointCloudModel {
    pub points: Arc<Vec<PointCloudPoint>>,
    pub point_size_px: f32,
    pub style: PointStyle,
    /// Chunked upload plan; empty models or non-tiled paths leave this
    /// empty, and the renderer falls back to a full instance upload.
    pub chunks: Vec<PointChunk>,
    /// Bumps when the point set or any per-point attribute changes: the
    /// instance buffer must be rebuilt.
    pub geometry_generation: u64,
    /// Bumps on style-only changes: just the style uniform is rewritten.
    pub style_generation: u64,
}

impl Default for PointCloudModel {
    fn default() -> Self {
        Self {
            points: Arc::new(Vec::new()),
            point_size_px: 3.0,
            style: PointStyle::default(),
            chunks: Vec::new(),
            geometry_generation: 0,
            style_generation: 0,
        }
    }
}
