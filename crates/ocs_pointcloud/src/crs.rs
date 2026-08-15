//! CRS inspection, guarded horizontal reprojection, and survey-readiness checks.

use crate::{EditStore, Error, ExportProgress, PointPatch, Result};
use las::{Reader, Writer};
use proj4rs::{proj::Proj, transform::transform};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// CRS information recovered from LAS WKT or GeoTIFF (E)VLRs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrsInfo {
    pub horizontal_epsg: Option<u16>,
    pub vertical_epsg: Option<u16>,
    pub name: Option<String>,
    pub wkt: Option<String>,
    pub source: Option<String>,
    pub parse_warning: Option<String>,
}

impl CrsInfo {
    pub(crate) fn from_header(header: &las::Header) -> Self {
        let wkt = header.get_wkt_crs_bytes().map(|bytes| {
            String::from_utf8_lossy(bytes)
                .trim_matches(char::from(0))
                .trim()
                .to_string()
        });
        let source = if wkt.is_some() {
            Some("WKT".to_string())
        } else if header.get_geotiff_crs().ok().flatten().is_some() {
            Some("GeoTIFF".to_string())
        } else {
            None
        };
        let name = wkt.as_deref().and_then(wkt_name);
        let (horizontal_epsg, vertical_epsg, parse_warning) = if let Some(wkt) = wkt.as_deref() {
            let (horizontal, vertical) = epsg_from_wkt(wkt);
            let warning = horizontal
                .is_none()
                .then(|| "WKT CRS has no resolvable EPSG authority identifier".to_string());
            (horizontal, vertical, warning)
        } else {
            match header.get_geotiff_crs() {
                Ok(Some(geotiff)) => {
                    let horizontal = geotiff
                        .get_projected_crs_geo_key_value()
                        .or_else(|| geotiff.get_geodetic_crs_geo_key_value())
                        .filter(|code| *code != 0 && *code != 32_767);
                    let vertical = geotiff
                        .get_vertical_crs_geo_key_value()
                        .filter(|code| *code != 0 && *code != 32_767);
                    let warning = horizontal
                        .is_none()
                        .then(|| "GeoTIFF CRS is user-defined or has no EPSG key".to_string());
                    (horizontal, vertical, warning)
                }
                Ok(None) => (None, None, None),
                Err(error) => (None, None, Some(error.to_string())),
            }
        };
        Self {
            horizontal_epsg,
            vertical_epsg,
            name,
            wkt,
            source,
            parse_warning,
        }
    }

    pub fn label(&self) -> String {
        let horizontal = self
            .horizontal_epsg
            .map(|code| format!("EPSG:{code}"))
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| "unresolved CRS".to_string());
        match self.vertical_epsg {
            Some(vertical) => format!("{horizontal} + vertical EPSG:{vertical}"),
            None => horizontal,
        }
    }
}

fn wkt_name(wkt: &str) -> Option<String> {
    let quote = wkt.find('"')?;
    let remainder = &wkt[quote + 1..];
    let end = remainder.find('"')?;
    let name = remainder[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn epsg_from_wkt(wkt: &str) -> (Option<u16>, Option<u16>) {
    let vertical_start = ["VERT_CS[", "VERTCRS[", "VERTICALCRS["]
        .iter()
        .filter_map(|marker| wkt.find(marker))
        .min();
    let (horizontal_wkt, vertical_wkt) = vertical_start
        .map(|start| (&wkt[..start], Some(&wkt[start..])))
        .unwrap_or((wkt, None));
    (
        epsg_authorities(horizontal_wkt).last(),
        vertical_wkt.and_then(|value| epsg_authorities(value).last()),
    )
}

fn epsg_authorities(wkt: &str) -> impl Iterator<Item = u16> + '_ {
    let normalized = wkt
        .replace("AUTHORITY[\"EPSG\",\"", "EPSG:")
        .replace("ID[\"EPSG\",", "EPSG:")
        .replace("ID[\"EPSG\", ", "EPSG:");
    let values: Vec<_> = normalized
        .match_indices("EPSG:")
        .filter_map(|(offset, _)| {
            let digits: String = normalized[offset + 5..]
                .chars()
                .skip_while(|value| value.is_whitespace() || *value == '"')
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse::<u16>().ok().filter(|code| *code != 0)
        })
        .collect();
    values.into_iter()
}

pub fn inspect_crs(path: impl AsRef<Path>) -> Result<CrsInfo> {
    let reader = Reader::from_path(path)?;
    Ok(CrsInfo::from_header(reader.header()))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurveyReadiness {
    Ready,
    Caution(Vec<String>),
    Blocked(Vec<String>),
}

impl SurveyReadiness {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Ready => "ready for survey-derived products".to_string(),
            Self::Caution(messages) => format!("caution: {}", messages.join("; ")),
            Self::Blocked(messages) => format!("blocked: {}", messages.join("; ")),
        }
    }
}

/// Rejects ambiguous or geographic survey coordinates before derived geometry
/// such as surfaces, contours, breaklines, or classifiers is generated.
pub fn assess_survey_readiness(metadata: &crate::CloudMetadata) -> SurveyReadiness {
    let mut blocked = Vec::new();
    let mut caution = Vec::new();
    let crs = &metadata.crs;
    if !metadata.has_crs {
        blocked.push("no CRS is declared".to_string());
    } else if crs.horizontal_epsg.is_none() {
        blocked.push("horizontal CRS could not be resolved to an EPSG code".to_string());
    }
    let geographic = crs.horizontal_epsg == Some(4326)
        || crs.wkt.as_deref().is_some_and(|wkt| {
            (wkt.contains("GEOGCS[") || wkt.contains("GEODCRS["))
                && !wkt.contains("PROJCS[")
                && !wkt.contains("PROJCRS[")
        });
    if geographic {
        blocked.push(
            "horizontal coordinates are angular; reproject to a suitable projected survey CRS"
                .to_string(),
        );
    }
    if crs.vertical_epsg.is_none() {
        caution.push(
            "vertical datum/units are not resolved; Z will be treated as source survey units"
                .to_string(),
        );
    }
    if let Some(warning) = &crs.parse_warning {
        caution.push(format!("CRS parser warning: {warning}"));
    }
    let span = [
        metadata.bounds_max[0] - metadata.bounds_min[0],
        metadata.bounds_max[1] - metadata.bounds_min[1],
        metadata.bounds_max[2] - metadata.bounds_min[2],
    ];
    if span.iter().any(|value| !value.is_finite() || *value < 0.0) {
        blocked.push("cloud bounds are invalid".to_string());
    }
    if metadata
        .scales
        .iter()
        .any(|scale| !scale.is_finite() || *scale <= 0.0)
    {
        blocked.push("LAS coordinate scale is invalid".to_string());
    }
    if !blocked.is_empty() {
        SurveyReadiness::Blocked(blocked)
    } else if !caution.is_empty() {
        SurveyReadiness::Caution(caution)
    } else {
        SurveyReadiness::Ready
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReprojectionStats {
    pub points_read: u64,
    pub points_written: u64,
    pub target_horizontal_epsg: u16,
    pub vertical_values_preserved: u64,
}

/// Streams a cloud to a new LAS/LAZ, applies sparse edits, and reprojects XY.
/// Z values are deliberately preserved because horizontal EPSG conversion does
/// not define a safe vertical datum transformation.
pub fn reproject_with_patches_progress(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    edits: &EditStore,
    target_epsg: u16,
    mut continue_export: impl FnMut(ExportProgress) -> bool,
) -> Result<ReprojectionStats> {
    const CHUNK_SIZE: u64 = 65_536;
    let input = input.as_ref();
    let output = output.as_ref();
    super::validate_output_path(input, output)?;

    let mut reader = Reader::from_path(input)?;
    let source_crs = CrsInfo::from_header(reader.header());
    let source_epsg = source_crs.horizontal_epsg.ok_or_else(|| {
        Error::Crs("source horizontal CRS is not a resolvable EPSG code".to_string())
    })?;
    if source_epsg == target_epsg {
        return Err(Error::Crs(format!(
            "source and target horizontal CRS are both EPSG:{target_epsg}"
        )));
    }
    let source_projection = Proj::from_epsg_code(source_epsg)
        .map_err(|error| Error::Crs(format!("EPSG:{source_epsg}: {error}")))?;
    let target_projection = Proj::from_epsg_code(target_epsg)
        .map_err(|error| Error::Crs(format!("EPSG:{target_epsg}: {error}")))?;
    let target_definition = crs_definitions::from_code(target_epsg).ok_or_else(|| {
        Error::Crs(format!(
            "EPSG:{target_epsg} is not in the bundled CRS database"
        ))
    })?;

    let source_bounds = reader.header().bounds();
    let target_bounds = transformed_xy_bounds(
        &source_projection,
        &target_projection,
        [source_bounds.min.x, source_bounds.min.y],
        [source_bounds.max.x, source_bounds.max.y],
    )?;
    let mut builder = las::Builder::from(reader.header().clone());
    if builder.version.major < 1 || builder.version.minor < 4 {
        builder.version = las::Version::new(1, 4);
    }
    let target_base_scale = if target_projection.is_latlong() {
        1.0e-8
    } else {
        0.001
    };
    builder.transforms.x = output_transform(target_bounds[0], target_bounds[2], target_base_scale);
    builder.transforms.y = output_transform(target_bounds[1], target_bounds[3], target_base_scale);
    let mut header = builder.into_header()?;
    header
        .set_wkt_crs(target_definition.wkt.as_bytes().to_vec())
        .map_err(|error| Error::Crs(format!("cannot write target WKT: {error}")))?;
    let point_count = header.number_of_points();
    let temporary = super::temporary_output_path(output);
    let mut temporary_guard = super::TemporaryOutput::new(temporary.clone());
    let mut writer = Writer::from_path(&temporary, header)?;
    let mut stats = ReprojectionStats {
        target_horizontal_epsg: target_epsg,
        ..ReprojectionStats::default()
    };

    while stats.points_read < point_count {
        let point_data = reader.read_points((point_count - stats.points_read).min(CHUNK_SIZE))?;
        if point_data.is_empty() {
            break;
        }
        for point in point_data.points() {
            let mut point = point?;
            if let Some(patch) = edits.patch_for(stats.points_read) {
                apply_patch_for_reprojection(&mut point, patch)?;
            }
            let original_z = point.z;
            let coordinate = transform_coordinate(
                &source_projection,
                &target_projection,
                (point.x, point.y, point.z),
            )
            .map_err(|error| Error::Crs(format!("point {}: {error}", stats.points_read)))?;
            if !coordinate.0.is_finite() || !coordinate.1.is_finite() {
                return Err(Error::Crs(format!(
                    "point {} transformed to a non-finite coordinate",
                    stats.points_read
                )));
            }
            point.x = coordinate.0;
            point.y = coordinate.1;
            point.z = original_z;
            writer.write_point(point)?;
            stats.points_read += 1;
            stats.points_written += 1;
            stats.vertical_values_preserved += 1;
        }
        if !continue_export(ExportProgress {
            points_read: stats.points_read,
            total_points: point_count,
        }) {
            return Err(Error::Cancelled("point-cloud reprojection"));
        }
    }

    writer.close()?;
    drop(writer);
    fs::rename(&temporary, output)?;
    temporary_guard.commit();
    Ok(stats)
}

fn output_transform(low: f64, high: f64, base_scale: f64) -> las::Transform {
    let span = (high - low).abs();
    let safe_scale = (span / 4_000_000_000.0).max(base_scale);
    las::Transform {
        scale: safe_scale,
        offset: low + (high - low) * 0.5,
    }
}

fn transformed_xy_bounds(
    source: &Proj,
    target: &Proj,
    min: [f64; 2],
    max: [f64; 2],
) -> Result<[f64; 4]> {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    // Densify the source envelope so curved projection edges are represented,
    // not only the four corners. A small safety margin is added below.
    for y_step in 0..=16 {
        for x_step in 0..=16 {
            if x_step != 0 && x_step != 16 && y_step != 0 && y_step != 16 {
                continue;
            }
            let x = min[0] + (max[0] - min[0]) * x_step as f64 / 16.0;
            let y = min[1] + (max[1] - min[1]) * y_step as f64 / 16.0;
            let coordinate = transform_coordinate(source, target, (x, y, 0.0))
                .map_err(|error| Error::Crs(format!("cannot transform source bounds: {error}")))?;
            bounds[0] = bounds[0].min(coordinate.0);
            bounds[1] = bounds[1].min(coordinate.1);
            bounds[2] = bounds[2].max(coordinate.0);
            bounds[3] = bounds[3].max(coordinate.1);
        }
    }
    if bounds.iter().any(|value| !value.is_finite()) {
        return Err(Error::Crs(
            "transformed source bounds are not finite".to_string(),
        ));
    }
    let margin_x = ((bounds[2] - bounds[0]).abs() * 1.0e-6).max(0.01);
    let margin_y = ((bounds[3] - bounds[1]).abs() * 1.0e-6).max(0.01);
    Ok([
        bounds[0] - margin_x,
        bounds[1] - margin_y,
        bounds[2] + margin_x,
        bounds[3] + margin_y,
    ])
}

fn transform_coordinate(
    source: &Proj,
    target: &Proj,
    mut coordinate: (f64, f64, f64),
) -> std::result::Result<(f64, f64, f64), proj4rs::errors::Error> {
    if source.is_latlong() {
        coordinate.0 = coordinate.0.to_radians();
        coordinate.1 = coordinate.1.to_radians();
    }
    transform(source, target, &mut coordinate)?;
    if target.is_latlong() {
        coordinate.0 = coordinate.0.to_degrees();
        coordinate.1 = coordinate.1.to_degrees();
    }
    Ok(coordinate)
}

fn apply_patch_for_reprojection(point: &mut las::Point, patch: PointPatch) -> Result<()> {
    if let Some(classification) = patch.classification {
        super::apply_classification(point, classification)?;
    }
    if let Some(value) = patch.synthetic {
        point.is_synthetic = value;
    }
    if let Some(value) = patch.key_point {
        point.is_key_point = value;
    }
    if let Some(value) = patch.withheld {
        point.is_withheld = value;
    }
    if let Some(value) = patch.overlap {
        point.is_overlap = value;
    }
    if let Some(value) = patch.elevation {
        point.z = value;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_first_wkt_name() {
        assert_eq!(
            Some("NAD83 / Illinois East (ftUS)".to_string()),
            wkt_name("PROJCS[\"NAD83 / Illinois East (ftUS)\",GEOGCS[...]\"]")
        );
    }

    #[test]
    fn extracts_horizontal_and_vertical_epsg_from_wkt() {
        let wkt = "COMPD_CS[\"survey\",PROJCS[\"horizontal\",AUTHORITY[\"EPSG\",\"3435\"]],VERT_CS[\"NAVD88\",AUTHORITY[\"EPSG\",\"5703\"]]]";
        assert_eq!((Some(3435), Some(5703)), epsg_from_wkt(wkt));
    }

    #[test]
    fn missing_crs_blocks_survey_products() {
        let metadata = crate::CloudMetadata {
            point_count: 1,
            version_major: 1,
            version_minor: 4,
            point_format: 6,
            compressed: false,
            bounds_min: [0.0; 3],
            bounds_max: [1.0; 3],
            scales: [0.01; 3],
            offsets: [0.0; 3],
            system_identifier: String::new(),
            generating_software: String::new(),
            creation_date: None,
            file_source_id: 0,
            has_crs: false,
            crs: CrsInfo::default(),
            vlr_count: 0,
            evlr_count: 0,
        };
        assert!(matches!(
            assess_survey_readiness(&metadata),
            SurveyReadiness::Blocked(_)
        ));
    }
}
