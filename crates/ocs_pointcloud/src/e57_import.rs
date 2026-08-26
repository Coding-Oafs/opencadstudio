//! Streaming ASTM E57 import into a standard LAS/LAZ source.
//!
//! Scanner poses are applied by the E57 reader, spherical-only records are
//! converted to Cartesian coordinates, and invalid/direction-only records are
//! counted but omitted. The input is opened twice: a bounded-memory bounds pass
//! chooses safe LAS transforms, then a write pass produces an adjacent partial
//! output which is published atomically.

use crate::{Error, ProtectedOutput, Result};
use e57::{CartesianCoordinate, E57Reader};
use las::{point::Format, Builder, Color, Point, Transform, Version, Writer};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum E57ImportStage {
    Inspecting,
    Writing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct E57ImportProgress {
    pub stage: E57ImportStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct E57ImportStats {
    pub output: PathBuf,
    pub scans: usize,
    pub records_read: u64,
    pub points_written: u64,
    pub invalid_points_skipped: u64,
    pub coordinate_metadata_preserved: bool,
}

pub fn import_e57(
    input: &Path,
    output: &Path,
    overwrite: bool,
    mut continue_import: impl FnMut(E57ImportProgress) -> bool,
) -> Result<E57ImportStats> {
    if input == output {
        return Err(Error::SameInputAndOutput(output.to_path_buf()));
    }
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !matches!(extension.to_ascii_lowercase().as_str(), "las" | "laz") {
        return Err(Error::UnsupportedExtension(output.to_path_buf()));
    }

    let (bounds, total, scans, has_color, coordinate_metadata) =
        inspect_points(input, &mut continue_import)?;
    let bounds =
        bounds.ok_or_else(|| Error::E57("file contains no valid 3D points".to_string()))?;

    let mut builder = Builder::default();
    builder.version = Version::new(1, 4);
    builder.point_format = Format::new(if has_color { 2 } else { 0 })?;
    builder.generating_software = "OpenCADStudio E57 Importer 1.1".to_string();
    builder.transforms.x = transform_for(bounds[0], bounds[3]);
    builder.transforms.y = transform_for(bounds[1], bounds[4]);
    builder.transforms.z = transform_for(bounds[2], bounds[5]);
    let mut header = builder.into_header()?;
    let preserve_crs = coordinate_metadata
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|wkt| {
            header
                .set_wkt_crs(wkt.as_bytes().to_vec())
                .map_err(|error| {
                    Error::E57(format!("cannot preserve coordinate metadata: {error}"))
                })
        })
        .transpose()?
        .is_some();

    let reservation = ProtectedOutput::reserve(output, overwrite)?;
    let mut writer = Writer::from_path(reservation.partial_path(), header)?;
    let mut reader = open_reader(input)?;
    let pointclouds = reader.pointclouds();
    let mut stats = E57ImportStats {
        output: output.to_path_buf(),
        scans,
        coordinate_metadata_preserved: preserve_crs,
        ..Default::default()
    };

    for (scan_index, cloud) in pointclouds.iter().enumerate() {
        let points = reader
            .pointcloud_simple(cloud)
            .map_err(|error| Error::E57(error.to_string()))?;
        for point in points {
            let point = point.map_err(|error| Error::E57(error.to_string()))?;
            stats.records_read = stats.records_read.saturating_add(1);
            if let CartesianCoordinate::Valid { x, y, z } = point.cartesian {
                let color = point.color.map(|color| {
                    Color::new(
                        normalized_u16(color.red),
                        normalized_u16(color.green),
                        normalized_u16(color.blue),
                    )
                });
                writer.write_point(Point {
                    x,
                    y,
                    z,
                    intensity: point.intensity.map(normalized_u16).unwrap_or_default(),
                    color,
                    point_source_id: u16::try_from(scan_index + 1).unwrap_or(u16::MAX),
                    return_number: 1,
                    number_of_returns: 1,
                    ..Default::default()
                })?;
                stats.points_written = stats.points_written.saturating_add(1);
            } else {
                stats.invalid_points_skipped = stats.invalid_points_skipped.saturating_add(1);
            }
            if stats.records_read % 65_536 == 0
                && !continue_import(E57ImportProgress {
                    stage: E57ImportStage::Writing,
                    completed: stats.records_read,
                    total,
                })
            {
                return Err(Error::Cancelled("E57 import"));
            }
        }
    }
    if !continue_import(E57ImportProgress {
        stage: E57ImportStage::Writing,
        completed: stats.records_read,
        total,
    }) {
        return Err(Error::Cancelled("E57 import"));
    }
    writer.close()?;
    drop(writer);
    reservation.commit()?;
    Ok(stats)
}

type E57Bounds = Option<[f64; 6]>;

fn inspect_points(
    input: &Path,
    continue_import: &mut impl FnMut(E57ImportProgress) -> bool,
) -> Result<(E57Bounds, u64, usize, bool, Option<String>)> {
    let mut reader = open_reader(input)?;
    let coordinate_metadata = reader.coordinate_metadata().map(ToOwned::to_owned);
    let pointclouds = reader.pointclouds();
    let total = pointclouds.iter().map(|cloud| cloud.records).sum();
    let has_color = pointclouds.iter().any(e57::PointCloud::has_color);
    let mut bounds: E57Bounds = None;
    let mut completed = 0_u64;
    for cloud in &pointclouds {
        let points = reader
            .pointcloud_simple(cloud)
            .map_err(|error| Error::E57(error.to_string()))?;
        for point in points {
            let point = point.map_err(|error| Error::E57(error.to_string()))?;
            completed = completed.saturating_add(1);
            if let CartesianCoordinate::Valid { x, y, z } = point.cartesian {
                if x.is_finite() && y.is_finite() && z.is_finite() {
                    include_point(&mut bounds, [x, y, z]);
                }
            }
            if completed % 65_536 == 0
                && !continue_import(E57ImportProgress {
                    stage: E57ImportStage::Inspecting,
                    completed,
                    total,
                })
            {
                return Err(Error::Cancelled("E57 import"));
            }
        }
    }
    Ok((
        bounds,
        total,
        pointclouds.len(),
        has_color,
        coordinate_metadata,
    ))
}

fn open_reader(input: &Path) -> Result<E57Reader<BufReader<File>>> {
    let file = File::open(input)?;
    E57Reader::new(BufReader::new(file)).map_err(|error| Error::E57(error.to_string()))
}

fn include_point(bounds: &mut E57Bounds, point: [f64; 3]) {
    match bounds {
        Some(bounds) => {
            for axis in 0..3 {
                bounds[axis] = bounds[axis].min(point[axis]);
                bounds[axis + 3] = bounds[axis + 3].max(point[axis]);
            }
        }
        None => *bounds = Some([point[0], point[1], point[2], point[0], point[1], point[2]]),
    }
}

fn transform_for(min: f64, max: f64) -> Transform {
    let offset = min + (max - min) * 0.5;
    let scale = 0.001_f64.max((max - min).abs() / (i32::MAX as f64 * 1.9));
    Transform { scale, offset }
}

fn normalized_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_transform_contains_large_survey_extents() {
        let transform = transform_for(8_000_000.0, 8_100_000.0);
        assert!(transform.inverse(8_000_000.0).is_ok());
        assert!(transform.inverse(8_100_000.0).is_ok());
    }

    #[test]
    fn normalized_attributes_are_clamped() {
        assert_eq!(0, normalized_u16(-1.0));
        assert_eq!(u16::MAX, normalized_u16(2.0));
        assert_eq!(32_768, normalized_u16(0.5));
    }
}
