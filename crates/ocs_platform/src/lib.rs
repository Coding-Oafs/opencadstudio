//! OpenCADStudio v2 integrated engineering platform contracts.
//!
//! The types in this crate are deliberately data-engine agnostic. CAD, GIS,
//! LiDAR, raster, and model adapters can all participate in one transaction,
//! workflow, standards package, and provenance record without exposing their
//! internal scene structures.

mod state;
mod tiles3d;

pub use state::*;
pub use tiles3d::{
    export_point_octree_tileset, export_point_tileset, OctreeOptions, OctreeTilesetExport,
    PointOctreeWriter, PointTile, TilesetExport, TilesetStream,
};
