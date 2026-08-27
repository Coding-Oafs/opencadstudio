//! Minimal, standards-shaped 3D Tiles 1.0 point export.
//!
//! A tileset contains one PNTS content tile. Higher-level streamers can shard
//! the same `PointTile` contract into an octree without changing provenance.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointTile {
    pub positions: Vec<[f64; 3]>,
    pub geometric_error: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TilesetExport {
    pub tileset: PathBuf,
    pub content: PathBuf,
    pub point_count: usize,
    pub byte_length: u64,
}

pub fn export_point_tileset(
    directory: impl AsRef<Path>,
    tile: &PointTile,
    overwrite: bool,
) -> io::Result<TilesetExport> {
    if tile.positions.is_empty()
        || tile
            .positions
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        || !tile.geometric_error.is_finite()
        || tile.geometric_error < 0.0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tile requires finite points and error",
        ));
    }
    let directory = directory.as_ref();
    let tileset = directory.join("tileset.json");
    let content = directory.join("root.pnts");
    if !overwrite && (tileset.exists() || content.exists()) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "3D Tiles output already exists",
        ));
    }
    fs::create_dir_all(directory)?;
    let (minimum, maximum) = bounds(&tile.positions);
    let center = [
        (minimum[0] + maximum[0]) * 0.5,
        (minimum[1] + maximum[1]) * 0.5,
        (minimum[2] + maximum[2]) * 0.5,
    ];
    let mut feature_json = serde_json::to_vec(&serde_json::json!({
        "POINTS_LENGTH": tile.positions.len(),
        "POSITION": {"byteOffset": 0},
        "RTC_CENTER": center,
    }))?;
    pad(&mut feature_json, 8, b' ');
    let mut feature_binary = Vec::with_capacity(tile.positions.len() * 12);
    for point in &tile.positions {
        for axis in 0..3 {
            feature_binary.extend_from_slice(&((point[axis] - center[axis]) as f32).to_le_bytes());
        }
    }
    pad(&mut feature_binary, 8, 0);
    let byte_length = 28 + feature_json.len() + feature_binary.len();
    let mut pnts = Vec::with_capacity(byte_length);
    pnts.extend_from_slice(b"pnts");
    for value in [
        1_u32,
        byte_length as u32,
        feature_json.len() as u32,
        feature_binary.len() as u32,
        0,
        0,
    ] {
        pnts.extend_from_slice(&value.to_le_bytes());
    }
    pnts.extend_from_slice(&feature_json);
    pnts.extend_from_slice(&feature_binary);
    let tileset_json = serde_json::to_vec_pretty(&serde_json::json!({
        "asset": {"version": "1.0", "generator": "OpenCADStudio"},
        "geometricError": tile.geometric_error,
        "root": {
            "boundingVolume": {"box": [
                center[0], center[1], center[2],
                (maximum[0] - minimum[0]) * 0.5, 0.0, 0.0,
                0.0, (maximum[1] - minimum[1]) * 0.5, 0.0,
                0.0, 0.0, (maximum[2] - minimum[2]) * 0.5
            ]},
            "geometricError": 0.0,
            "refine": "ADD",
            "content": {"uri": "root.pnts"}
        }
    }))?;
    let partial_content = directory.join("root.pnts.partial");
    let partial_tileset = directory.join("tileset.json.partial");
    fs::write(&partial_content, &pnts)?;
    fs::write(&partial_tileset, tileset_json)?;
    if overwrite {
        let _ = fs::remove_file(&content);
        let _ = fs::remove_file(&tileset);
    }
    fs::rename(&partial_content, &content)?;
    if let Err(error) = fs::rename(&partial_tileset, &tileset) {
        let _ = fs::remove_file(&content);
        return Err(error);
    }
    Ok(TilesetExport {
        tileset,
        content,
        point_count: tile.positions.len(),
        byte_length: byte_length as u64,
    })
}

fn bounds(points: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for point in points {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    (minimum, maximum)
}

fn pad(bytes: &mut Vec<u8>, alignment: usize, value: u8) {
    while bytes.len() % alignment != 0 {
        bytes.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_atomic_pnts_and_tileset() {
        let directory = std::env::temp_dir().join(format!("ocs-3dtiles-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let export = export_point_tileset(
            &directory,
            &PointTile {
                positions: vec![[1000.0, 2000.0, 10.0], [1010.0, 2020.0, 30.0]],
                geometric_error: 1.0,
            },
            false,
        )
        .unwrap();
        let bytes = fs::read(&export.content).unwrap();
        assert_eq!(&bytes[..4], b"pnts");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&export.tileset).unwrap()).unwrap();
        assert_eq!(json["root"]["content"]["uri"], "root.pnts");
        assert!(export_point_tileset(
            &directory,
            &PointTile {
                positions: vec![[0.0; 3]],
                geometric_error: 0.0
            },
            false
        )
        .is_err());
        assert!(!directory.join("root.pnts.partial").exists());
        let _ = fs::remove_dir_all(directory);
    }
}
