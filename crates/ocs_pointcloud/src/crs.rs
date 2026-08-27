//! CRS inspection, guarded horizontal reprojection, and survey-readiness checks.

use crate::{EditStore, Error, ExportProgress, PointPatch, Result};
use las::{Reader, Writer};
use proj4rs::{proj::Proj, transform::transform};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// Reproject a single XY coordinate from `source_epsg` to `target_epsg`.
/// Returns `None` when either projection is unavailable or the transform fails.
/// `z` is passed through unchanged.
pub fn reproject_xy(source_epsg: u16, target_epsg: u16, x: f64, y: f64) -> Option<(f64, f64)> {
    if source_epsg == target_epsg {
        return Some((x, y));
    }
    let source = Proj::from_epsg_code(source_epsg).ok()?;
    let target = Proj::from_epsg_code(target_epsg).ok()?;
    let mut coordinate = (x, y, 0.0);
    if source.is_latlong() {
        coordinate.0 = coordinate.0.to_radians();
        coordinate.1 = coordinate.1.to_radians();
    }
    transform(&source, &target, &mut coordinate).ok()?;
    if target.is_latlong() {
        coordinate.0 = coordinate.0.to_degrees();
        coordinate.1 = coordinate.1.to_degrees();
    }
    Some((coordinate.0, coordinate.1))
}

/// Horizontal unit reported by an EPSG definition (for example `m`, `ft`,
/// `us-ft`, or `degrees`). Drawing spatial settings use this to keep the
/// working unit compatible with the coordinate system.
pub fn epsg_horizontal_unit(epsg: u16) -> Option<&'static str> {
    Proj::from_epsg_code(epsg)
        .ok()
        .map(|projection| projection.units())
}

/// Geographic area of use for an EPSG definition as
/// `[west_longitude, south_latitude, east_longitude, north_latitude]`.
///
/// The bundled WKT2 definitions carry EPSG `BBOX[south,west,north,east]`
/// metadata for projected CRSs. Global Web Mercator and WGS 84 are handled
/// explicitly because their compact WKT definitions do not include a BBOX.
pub fn epsg_area_of_use(epsg: u16) -> Option<[f64; 4]> {
    const MERCATOR_LATITUDE_LIMIT: f64 = 85.051_128_779_806_6;
    if matches!(epsg, 3857 | 4326) {
        return Some([
            -180.0,
            -MERCATOR_LATITUDE_LIMIT,
            180.0,
            MERCATOR_LATITUDE_LIMIT,
        ]);
    }

    let definition = crs_definitions::from_code(epsg)?;
    let marker = definition.wkt.rfind("BBOX[")? + "BBOX[".len();
    let end = definition.wkt[marker..].find(']')? + marker;
    let values = definition.wkt[marker..end]
        .split(',')
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    let [south, west, north, east] = values.as_slice() else {
        return None;
    };
    if !values.iter().all(|value| value.is_finite())
        || west >= east
        || south >= north
        || *west < -180.0
        || *east > 180.0
        || *south < -90.0
        || *north > 90.0
    {
        return None;
    }
    Some([*west, *south, *east, *north])
}

/// Reproject a single XY coordinate from a PROJ.4 source string to `target_epsg`.
/// Used when a projected CRS has no resolvable EPSG code but a parseable WKT.
pub fn reproject_from_proj4(
    source_proj4: &str,
    target_epsg: u16,
    x: f64,
    y: f64,
) -> Option<(f64, f64)> {
    let source = Proj::from_proj_string(source_proj4).ok()?;
    let target = Proj::from_epsg_code(target_epsg).ok()?;
    transform_coordinate(&source, &target, (x, y, 0.0))
        .ok()
        .map(|c| (c.0, c.1))
}

/// Reproject a single XY coordinate from a `CrsInfo` source to `target_epsg`,
/// preferring the PROJ.4 string (accurate for projected CRS whose EPSG code is
/// only the geographic base) over `horizontal_epsg`.
pub fn reproject_from_crs(crs: &CrsInfo, target_epsg: u16, x: f64, y: f64) -> Option<(f64, f64)> {
    if let Some(proj4) = crs.proj4.as_deref() {
        if let Some(out) = reproject_from_proj4(proj4, target_epsg, x, y) {
            return Some(out);
        }
    }
    crs.horizontal_epsg
        .and_then(|epsg| reproject_xy(epsg, target_epsg, x, y))
}

/// Reproject a single XY coordinate from `source_epsg` into a `CrsInfo` target
/// (the reverse of [`reproject_from_crs`]).
pub fn reproject_to_crs(source_epsg: u16, crs: &CrsInfo, x: f64, y: f64) -> Option<(f64, f64)> {
    if let Some(proj4) = crs.proj4.as_deref() {
        let source = Proj::from_epsg_code(source_epsg).ok()?;
        let target = Proj::from_proj_string(proj4).ok()?;
        return transform_coordinate(&source, &target, (x, y, 0.0))
            .ok()
            .map(|c| (c.0, c.1));
    }
    crs.horizontal_epsg
        .and_then(|epsg| reproject_xy(source_epsg, epsg, x, y))
}

/// Build the best-available horizontal [`Proj`] for a source CRS: the
/// WKT-derived PROJ.4 string first (accurate for a projected CRS whose WKT
/// omits an EPSG authority on the `PROJCS`, leaving only the geographic base in
/// `horizontal_epsg`), then the resolved EPSG code.
pub(crate) fn projection_from_crs(crs: &CrsInfo) -> Option<Proj> {
    if let Some(proj4) = crs.proj4.as_deref() {
        if let Ok(projection) = Proj::from_proj_string(proj4) {
            return Some(projection);
        }
    }
    if crs.wkt.as_deref().is_some_and(is_projected_wkt)
        && crs.horizontal_epsg.is_some_and(is_geographic_epsg)
    {
        return None;
    }
    crs.horizontal_epsg
        .and_then(|epsg| Proj::from_epsg_code(epsg).ok())
}

/// True when two horizontal CRS descriptors resolve to the same coordinate
/// space. The embedded PROJ.4 definition takes precedence because projected
/// LAS WKT sometimes exposes only its geographic base EPSG authority.
pub fn crs_equivalent(left: &CrsInfo, right: &CrsInfo) -> bool {
    match (left.proj4.as_deref(), right.proj4.as_deref()) {
        (Some(left), Some(right)) => normalize_proj4(left) == normalize_proj4(right),
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => {
            left.horizontal_epsg.is_some() && left.horizontal_epsg == right.horizontal_epsg
        }
    }
}

fn normalize_proj4(value: &str) -> Vec<&str> {
    let mut parts: Vec<_> = value
        .split_whitespace()
        .filter(|part| !matches!(*part, "+type=crs" | "+no_defs"))
        .collect();
    parts.sort_unstable();
    parts
}

/// Horizontal unit reported by the best available CRS definition.
pub fn crs_horizontal_unit(crs: &CrsInfo) -> Option<&'static str> {
    projection_from_crs(crs).map(|projection| projection.units())
}

/// Reproject a coordinate between two complete CRS descriptors. Z is retained
/// verbatim: this function performs a horizontal transformation only.
pub fn reproject_between_crs(
    source: &CrsInfo,
    target: &CrsInfo,
    x: f64,
    y: f64,
) -> Option<(f64, f64)> {
    if crs_equivalent(source, target) {
        return Some((x, y));
    }
    let source = projection_from_crs(source)?;
    let target = projection_from_crs(target)?;
    transform_coordinate(&source, &target, (x, y, 0.0))
        .ok()
        .map(|coordinate| (coordinate.0, coordinate.1))
}

/// Reproject a point slice in place while constructing each projection only
/// once. This is the display/LOD path for mixed-projection datasets.
pub fn reproject_points_between_crs(
    source: &CrsInfo,
    target: &CrsInfo,
    points: &mut [crate::SamplePoint],
) -> Result<()> {
    if crs_equivalent(source, target) || points.is_empty() {
        return Ok(());
    }
    let source_projection = projection_from_crs(source).ok_or_else(|| {
        Error::Crs(format!(
            "source horizontal CRS is unresolved: {}",
            source.label()
        ))
    })?;
    let target_projection = projection_from_crs(target).ok_or_else(|| {
        Error::Crs(format!(
            "target horizontal CRS is unresolved: {}",
            target.label()
        ))
    })?;
    for point in points {
        let coordinate = transform_coordinate(
            &source_projection,
            &target_projection,
            (point.position[0], point.position[1], point.position[2]),
        )
        .map_err(|error| {
            Error::Crs(format!(
                "point {} cannot transform from {} to {}: {error}",
                point.source_index,
                source.horizontal_label(),
                target.horizontal_label()
            ))
        })?;
        if !coordinate.0.is_finite() || !coordinate.1.is_finite() {
            return Err(Error::Crs(format!(
                "point {} transformed to a non-finite coordinate",
                point.source_index
            )));
        }
        point.position[0] = coordinate.0;
        point.position[1] = coordinate.1;
    }
    Ok(())
}

/// Reproject an XYZ envelope between complete CRS descriptors. Horizontal
/// edges are densified to capture curvature; Z is preserved.
pub fn reproject_bounds_between_crs(
    min: [f64; 3],
    max: [f64; 3],
    source: &CrsInfo,
    target: &CrsInfo,
) -> Option<([f64; 3], [f64; 3])> {
    if min.iter().chain(max.iter()).any(|value| !value.is_finite())
        || min[0] > max[0]
        || min[1] > max[1]
        || min[2] > max[2]
    {
        return None;
    }
    if crs_equivalent(source, target) {
        return Some((min, max));
    }
    let source_projection = projection_from_crs(source)?;
    let target_projection = projection_from_crs(target)?;
    let mut out_min = [f64::INFINITY, f64::INFINITY, min[2]];
    let mut out_max = [f64::NEG_INFINITY, f64::NEG_INFINITY, max[2]];
    const STEPS: usize = 8;
    for step in 0..=STEPS {
        let t = step as f64 / STEPS as f64;
        let x = min[0] + (max[0] - min[0]) * t;
        let y = min[1] + (max[1] - min[1]) * t;
        for (sample_x, sample_y) in [(x, min[1]), (x, max[1]), (min[0], y), (max[0], y)] {
            let coordinate = transform_coordinate(
                &source_projection,
                &target_projection,
                (sample_x, sample_y, 0.0),
            )
            .ok()?;
            out_min[0] = out_min[0].min(coordinate.0);
            out_min[1] = out_min[1].min(coordinate.1);
            out_max[0] = out_max[0].max(coordinate.0);
            out_max[1] = out_max[1].max(coordinate.1);
        }
    }
    out_min
        .iter()
        .chain(out_max.iter())
        .all(|value| value.is_finite())
        .then_some((out_min, out_max))
}

/// True when `epsg` resolves to a geographic (angular) CRS.
fn is_geographic_epsg(epsg: u16) -> bool {
    Proj::from_epsg_code(epsg).is_ok_and(|projection| projection.is_latlong())
}

fn is_projected_epsg(epsg: u16) -> bool {
    Proj::from_epsg_code(epsg).is_ok_and(|projection| !projection.is_latlong())
}

/// CRS information recovered from LAS WKT or GeoTIFF (E)VLRs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrsInfo {
    pub horizontal_epsg: Option<u16>,
    pub vertical_epsg: Option<u16>,
    pub name: Option<String>,
    pub wkt: Option<String>,
    /// PROJ.4 string derived from the WKT when the projected CRS carries no
    /// EPSG authority (so `horizontal_epsg` alone would fall back to the
    /// geographic base CRS and reproject in the wrong units).
    pub proj4: Option<String>,
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
        let embedded_proj4 = wkt.as_deref().and_then(proj4_from_wkt);
        let (horizontal_epsg, vertical_epsg, parse_warning) = if let Some(wkt) = wkt.as_deref() {
            let (horizontal, vertical) = epsg_from_wkt(wkt);
            let projected = is_projected_wkt(wkt);
            let projected_epsg = horizontal.filter(|code| !is_geographic_epsg(*code));
            let warning = if projected && projected_epsg.is_none() && embedded_proj4.is_some() {
                Some(
                    "projected WKT has no projected EPSG authority; using its embedded projection"
                        .to_string(),
                )
            } else if horizontal.is_none() && embedded_proj4.is_none() {
                Some("WKT CRS has no resolvable horizontal definition".to_string())
            } else {
                None
            };
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
        // Prefer an authoritative projected EPSG definition. The hand-built
        // projection is the fallback only when WKT lacks a projected authority.
        let proj4 = embedded_proj4.filter(|_| {
            !horizontal_epsg.is_some_and(is_projected_epsg)
                && wkt.as_deref().is_some_and(is_projected_wkt)
        });
        Self {
            horizontal_epsg,
            vertical_epsg,
            name,
            wkt,
            proj4,
            source,
            parse_warning,
        }
    }

    /// Human-readable horizontal CRS, avoiding the misleading geographic-EPSG
    /// fallback described in [`Self::label`].
    pub fn horizontal_label(&self) -> String {
        match self.horizontal_epsg {
            Some(code) if is_geographic_epsg(code) && self.proj4.is_some() => {
                self.name.clone().unwrap_or_else(|| format!("EPSG:{code}"))
            }
            Some(code) => format!("EPSG:{code}"),
            None => self
                .name
                .clone()
                .unwrap_or_else(|| "unresolved CRS".to_string()),
        }
    }

    pub fn label(&self) -> String {
        // A projected CRS whose WKT omits an EPSG authority on the PROJCS
        // falls back to its geographic base in `horizontal_epsg` (e.g. 6318).
        // Show the WKT name rather than misleadingly reporting a degree CRS.
        let horizontal = self.horizontal_label();
        match self.vertical_epsg {
            Some(vertical) => format!("{horizontal} + vertical EPSG:{vertical}"),
            None => horizontal,
        }
    }

    pub fn is_resolvable(&self) -> bool {
        if self.proj4.is_none()
            && self.wkt.as_deref().is_some_and(is_projected_wkt)
            && self.horizontal_epsg.is_some_and(is_geographic_epsg)
        {
            return false;
        }
        projection_from_crs(self).is_some()
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
    let horizontal_codes: Vec<_> = epsg_authorities(horizontal_wkt).collect();
    let horizontal = if is_projected_wkt(horizontal_wkt) {
        horizontal_codes
            .iter()
            .rev()
            .copied()
            .find(|code| is_projected_epsg(*code))
            .or_else(|| {
                horizontal_codes
                    .iter()
                    .rev()
                    .copied()
                    .find(|code| Proj::from_epsg_code(*code).is_ok())
            })
    } else {
        horizontal_codes
            .iter()
            .rev()
            .copied()
            .find(|code| Proj::from_epsg_code(*code).is_ok())
    };
    (
        horizontal,
        vertical_wkt.and_then(|value| epsg_authorities(value).last()),
    )
}

fn is_projected_wkt(wkt: &str) -> bool {
    ["PROJCS[", "PROJCRS[", "PROJECTEDCRS["]
        .iter()
        .any(|marker| wkt.contains(marker))
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

/// Build a PROJ.4 string from a projected-CRS WKT. Returns `None` when the WKT
/// is geographic (no `PROJECTION` element) or uses an unsupported projection.
///
/// This covers the common real-world case where a LAS 1.4 WKT names a projected
/// CRS (e.g. `NAD83(2011) / Massachusetts Mainland (ft)`) but carries no EPSG
/// authority on the `PROJCS` itself — only on the base `GEOGCS`. Without this,
/// `epsg_from_wkt` falls back to the geographic code and reprojects feet as
/// degrees.
fn proj4_from_wkt(wkt: &str) -> Option<String> {
    let proj_name =
        wkt_quoted_after("PROJECTION[", wkt).or_else(|| wkt_quoted_after("METHOD[", wkt))?;
    let proj_name = normalize_wkt_name(&proj_name);
    let proj = match proj_name.as_str() {
        "lambert_conformal_conic_2sp"
        | "lambert_conformal_conic_1sp"
        | "lambert_conformal_conic"
        | "lambert_conic_conformal_2sp"
        | "lambert_conic_conformal_1sp" => "lcc",
        "transverse_mercator" => "tmerc",
        "mercator_1sp" | "mercator_2sp" | "mercator_auxiliary_sphere" => "merc",
        "albers_conic_equal_area" => "aea",
        "polar_stereographic" | "stereographic" => "stere",
        "lambert_azimuthal_equal_area" => "laea",
        "equidistant_cylindrical" => "eqc",
        "cylindrical_equal_area" => "cea",
        "hotine_oblique_mercator" => "omerc",
        _ => return None,
    };
    let mut parts = vec![format!("+proj={proj}")];

    // WKT1 expresses false easting/northing in the projected CRS's declared
    // linear unit. PROJ.4, however, requires x_0/y_0 in metres even when
    // `+units=ft` says the input/output coordinates are feet. Keeping the raw
    // foot values shifts state-plane data by hundreds of kilometres (the
    // Boston USGS fixture landed in the Atlantic near 27 N, 75 W).
    let horizontal_unit = horizontal_wkt_unit(wkt);
    let linear_to_metre = horizontal_unit.as_ref().map_or(1.0, |(_, factor)| *factor);

    for (name, value) in wkt_parameters(wkt) {
        let name = normalize_wkt_name(&name);
        let key = match name.as_str() {
            "latitude_of_origin"
            | "latitude_of_center"
            | "latitude_of_natural_origin"
            | "latitude_of_false_origin" => "lat_0",
            "central_meridian"
            | "longitude_of_center"
            | "longitude_of_natural_origin"
            | "longitude_of_false_origin" => "lon_0",
            "standard_parallel_1" | "latitude_of_1st_standard_parallel" => "lat_1",
            "standard_parallel_2" | "latitude_of_2nd_standard_parallel" => "lat_2",
            "false_easting" | "easting_at_false_origin" => "x_0",
            "false_northing" | "northing_at_false_origin" => "y_0",
            "scale_factor" | "scale_factor_at_natural_origin" => "k_0",
            _ => continue,
        };
        let value = if matches!(key, "x_0" | "y_0") {
            value * linear_to_metre
        } else {
            value
        };
        parts.push(format!("+{key}={value}"));
    }

    // Horizontal ellipsoid (from the datum's SPHEROID).
    if let Some((name, a, rf)) = wkt_spheroid(wkt) {
        match ellipsoid_key(&name) {
            Some(key) => parts.push(format!("+ellps={key}")),
            None => {
                parts.push(format!("+a={a}"));
                parts.push(format!("+rf={rf}"));
            }
        }
    }

    // Horizontal linear unit: the first non-angular UNIT (skips GEOGCS "degree").
    if let Some((name, factor)) = horizontal_unit {
        match unit_key(&name) {
            Some(key) => parts.push(format!("+units={key}")),
            None => parts.push(format!("+to_meter={factor}")),
        }
    }

    parts.push("+no_defs".to_string());
    Some(parts.join(" "))
}

/// Projected horizontal unit, excluding any vertical CRS appended by a
/// compound WKT. The first non-angular unit in the horizontal component is the
/// coordinate unit used by WKT1 projection parameters.
fn horizontal_wkt_unit(wkt: &str) -> Option<(String, f64)> {
    let vertical_start = ["VERT_CS[", "VERTCRS[", "VERTICALCRS["]
        .iter()
        .filter_map(|marker| wkt.find(marker))
        .min()
        .unwrap_or(wkt.len());
    wkt_units(&wkt[..vertical_start])
        .into_iter()
        .rev()
        .find(|(name, _)| !is_angular_unit(name))
}

fn is_angular_unit(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("degree") || name.contains("radian") || name.contains("grad")
}

fn normalize_wkt_name(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() {
                value
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

/// The quoted name of the first `KEY["..."]` occurrence after `marker`.
fn wkt_quoted_after(marker: &str, wkt: &str) -> Option<String> {
    let pos = wkt.find(marker)? + marker.len();
    let rest = &wkt[pos..];
    let start = rest.find('"')? + 1;
    let name = &rest[start..];
    let end = name.find('"')?;
    Some(name[..end].to_string())
}

/// All `PARAMETER["name", value]` pairs in `wkt`.
fn wkt_parameters(wkt: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let mut rest = wkt;
    while let Some(pos) = rest.find("PARAMETER[") {
        let after = &rest[pos + "PARAMETER[".len()..];
        let Some(q) = after.find('"') else { break };
        let name_seg = &after[q + 1..];
        let Some(q2) = name_seg.find('"') else { break };
        let name = name_seg[..q2].to_string();
        if let Some(value) = wkt_number_after(&name_seg[q2 + 1..]) {
            out.push((name, value));
        }
        rest = after;
    }
    out
}

/// The first `SPHEROID["name", a, rf, ...]` as `(name, semi-major, inv-flattening)`.
fn wkt_spheroid(wkt: &str) -> Option<(String, f64, f64)> {
    let (pos, marker_len) = wkt
        .find("SPHEROID[")
        .map(|pos| (pos, "SPHEROID[".len()))
        .or_else(|| wkt.find("ELLIPSOID[").map(|pos| (pos, "ELLIPSOID[".len())))?;
    let pos = pos + marker_len;
    let after = &wkt[pos..];
    let q = after.find('"')?;
    let name_seg = &after[q + 1..];
    let q2 = name_seg.find('"')?;
    let name = name_seg[..q2].to_string();
    let nums: Vec<f64> = name_seg[q2 + 1..]
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .take(2)
        .collect();
    match nums.as_slice() {
        [a, rf] => Some((name, *a, *rf)),
        _ => None,
    }
}

/// All `UNIT["name", factor]` pairs in `wkt`.
fn wkt_units(wkt: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let mut rest = wkt;
    while let Some((pos, marker_len)) = ["LENGTHUNIT[", "UNIT["]
        .iter()
        .filter_map(|marker| rest.find(marker).map(|pos| (pos, marker.len())))
        .min_by_key(|(pos, _)| *pos)
    {
        let after = &rest[pos + marker_len..];
        let Some(q) = after.find('"') else { break };
        let name_seg = &after[q + 1..];
        let Some(q2) = name_seg.find('"') else { break };
        let name = name_seg[..q2].to_string();
        if let Some(factor) = wkt_number_after(&name_seg[q2 + 1..]) {
            out.push((name, factor));
        }
        rest = after;
    }
    out
}

/// Parse the leading number of a `", value"` tail.
fn wkt_number_after(tail: &str) -> Option<f64> {
    let num: String = tail
        .trim_start_matches(|c: char| c == ',' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    num.parse::<f64>().ok()
}

fn ellipsoid_key(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    if n.contains("grs") {
        return Some("GRS80");
    }
    if n.contains("wgs") {
        return Some("WGS84");
    }
    if n.contains("clarke 1866") {
        return Some("clrk66");
    }
    if n.contains("clarke 1880") {
        return Some("clrk80");
    }
    if n.contains("international") || n.contains("hayford") {
        return Some("intl");
    }
    if n.contains("airy") {
        return Some("airy");
    }
    if n.contains("bessel") {
        return Some("bessel");
    }
    None
}

fn unit_key(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    if n == "metre" || n == "meter" || n == "m" {
        return Some("m");
    }
    if n.contains("us survey") || n == "foot_us" || n == "us-ft" {
        return Some("us-ft");
    }
    if n.contains("foot") || n == "ft" {
        return Some("ft");
    }
    None
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
    } else if !crs.is_resolvable() {
        blocked.push("horizontal CRS could not be resolved to a usable projection".to_string());
    }
    let geographic = projection_from_crs(crs).is_some_and(|projection| projection.is_latlong());
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
    // A projected CRS whose WKT omits an EPSG authority on the PROJCS resolves
    // `horizontal_epsg` to only its geographic base (e.g. 6318). Reject the
    // no-op only when the source is unambiguously that same EPSG; otherwise
    // prefer the WKT-derived PROJ.4 string so state-plane feet are not
    // reprojected as degrees.
    if source_crs.proj4.is_none() && source_crs.horizontal_epsg == Some(target_epsg) {
        return Err(Error::Crs(format!(
            "source and target horizontal CRS are both EPSG:{target_epsg}"
        )));
    }
    let source_projection = projection_from_crs(&source_crs).ok_or_else(|| {
        Error::Crs(
            "source horizontal CRS is not resolvable (no projected EPSG or PROJ.4)".to_string(),
        )
    })?;
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

pub(crate) fn output_transform(low: f64, high: f64, base_scale: f64) -> las::Transform {
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

pub(crate) fn transform_coordinate(
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
    fn builds_proj4_from_projected_wkt_without_epsg_authority() {
        // Boston USGS LiDAR: NAD83(2011) / Massachusetts Mainland (ft), an LCC
        // state-plane CRS whose PROJCS carries no EPSG authority.
        let wkt = "COMPD_CS[\"NAD83(2011) / Massachusetts Mainland (ft) + NAVD88 height (ftUS)\",\
PROJCS[\"NAD83(2011) / Massachusetts Mainland (ft)\",GEOGCS[\"NAD83(2011)\",DATUM[\"NAD83_National_Spatial_Reference_System_2011\",SPHEROID[\"GRS 1980\",6378137,298.257222101,AUTHORITY[\"EPSG\",\"7019\"]],AUTHORITY[\"EPSG\",\"1116\"]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433],AUTHORITY[\"EPSG\",\"6318\"]],PROJECTION[\"Lambert_Conformal_Conic_2SP\"],PARAMETER[\"latitude_of_origin\",41],PARAMETER[\"central_meridian\",-71.5],PARAMETER[\"standard_parallel_1\",42.6833333333333],PARAMETER[\"standard_parallel_2\",41.7166666666667],PARAMETER[\"false_easting\",656167.979002625],PARAMETER[\"false_northing\",2460629.92125984],UNIT[\"International foot\",0.3048]],VERT_CS[\"NAVD88 height (ftUS)\",UNIT[\"US survey foot\",0.304800609601219],AUTHORITY[\"EPSG\",\"6360\"]]]";
        let proj4 = proj4_from_wkt(wkt).expect("projected WKT should parse");
        assert!(proj4.starts_with("+proj=lcc"), "got: {proj4}");
        assert!(proj4.contains("+lat_0=41"), "got: {proj4}");
        assert!(proj4.contains("+lon_0=-71.5"), "got: {proj4}");
        assert!(proj4.contains("+x_0=200000"), "got: {proj4}");
        assert!(proj4.contains("+y_0=749999"), "got: {proj4}");
        assert!(proj4.contains("+ellps=GRS80"), "got: {proj4}");
        assert!(proj4.contains("+units=ft"), "got: {proj4}");

        // Centre of a real Boston USGS tile in the WKT's international-foot
        // coordinates. This previously transformed to the Atlantic near
        // (-75.56, 26.98) because the false offsets were treated as metres.
        let (longitude, latitude) = reproject_from_proj4(&proj4, 4326, 787_148.208, 2_940_613.759)
            .expect("state-plane coordinate should transform");
        assert!(
            (-71.1..=-70.8).contains(&longitude),
            "longitude={longitude}"
        );
        assert!((42.2..=42.5).contains(&latitude), "latitude={latitude}");
    }

    #[test]
    fn proj4_from_geographic_wkt_is_none() {
        let wkt = "GEOGCS[\"NAD83\",DATUM[\"North American Datum 1983\",SPHEROID[\"GRS 1980\",6378137,298.257222101]],UNIT[\"degree\",0.0174532925199433],AUTHORITY[\"EPSG\",\"4269\"]]";
        assert!(proj4_from_wkt(wkt).is_none());
    }

    #[test]
    fn epsg_area_of_use_reads_projected_bbox_and_global_fallbacks() {
        let new_york = epsg_area_of_use(2263).expect("New York State Plane area");
        assert!(new_york[0] < -74.0 && new_york[2] > -72.0, "{new_york:?}");
        assert!(new_york[1] < 40.8 && new_york[3] > 41.0, "{new_york:?}");
        assert_eq!(
            epsg_area_of_use(3857),
            Some([-180.0, -85.051_128_779_806_6, 180.0, 85.051_128_779_806_6,])
        );
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

    #[test]
    fn projection_from_crs_prefers_proj4_over_geographic_fallback() {
        // California State Plane Zone 3 (international feet): the PROJCS carries
        // no EPSG authority, so `epsg_from_wkt` falls back to the geographic
        // base (6318) while `proj4_from_wkt` derives the real LCC projection.
        let wkt = "COMPD_CS[\"NAD83(2011) / California zone 3 (ft) + NAVD88 height (ftUS)\",PROJCS[\"NAD83(2011) / California zone 3 (ft)\",GEOGCS[\"NAD83(2011)\",DATUM[\"NAD83_National_Spatial_Reference_System_2011\",SPHEROID[\"GRS 1980\",6378137,298.257222101,AUTHORITY[\"EPSG\",\"7019\"]],AUTHORITY[\"EPSG\",\"1116\"]],PRIMEM[\"Greenwich\",0,AUTHORITY[\"EPSG\",\"8901\"]],UNIT[\"degree\",0.0174532925199433,AUTHORITY[\"EPSG\",\"9122\"]],AUTHORITY[\"EPSG\",\"6318\"]],PROJECTION[\"Lambert_Conformal_Conic_2SP\"],PARAMETER[\"latitude_of_origin\",36.5],PARAMETER[\"central_meridian\",-120.5],PARAMETER[\"standard_parallel_1\",38.4333333333333],PARAMETER[\"standard_parallel_2\",37.0666666666667],PARAMETER[\"false_easting\",6561679.79002625],PARAMETER[\"false_northing\",1640419.94750656],UNIT[\"International foot\",0.3048],AXIS[\"Easting\",EAST],AXIS[\"Northing\",NORTH]],VERT_CS[\"NAVD88 height (ftUS)\",VERT_DATUM[\"North American Vertical Datum 1988\",2005,AUTHORITY[\"EPSG\",\"5103\"]],UNIT[\"US survey foot\",0.304800609601219,AUTHORITY[\"EPSG\",\"9003\"]],AXIS[\"Gravity-related height\",UP],AUTHORITY[\"EPSG\",\"6360\"]]]";
        let proj4 = proj4_from_wkt(wkt).expect("projected WKT parses");
        assert!(proj4.starts_with("+proj=lcc"), "got: {proj4}");
        assert_eq!(epsg_from_wkt(wkt), (Some(6318), Some(6360)));

        let crs = CrsInfo {
            horizontal_epsg: Some(6318),
            vertical_epsg: Some(6360),
            name: Some("NAD83(2011) / California zone 3 (ft)".to_string()),
            wkt: Some(wkt.to_string()),
            proj4: Some(proj4),
            source: Some("WKT".to_string()),
            parse_warning: None,
        };

        let projection = projection_from_crs(&crs).expect("resolvable projection");
        assert!(
            !projection.is_latlong(),
            "must resolve to the projected CRS"
        );
        assert!(
            crs.label().contains("California zone 3"),
            "label must not show the geographic fallback: {}",
            crs.label()
        );

        // Centre of the R0_C0 tile in state-plane international feet, near Palo Alto.
        let (longitude, latitude) = reproject_from_crs(&crs, 4326, 6_064_521.0, 1_985_653.0)
            .expect("state-plane coordinate should transform");
        assert!(
            (-122.4..=-121.8).contains(&longitude),
            "longitude={longitude}"
        );
        assert!((37.2..=37.8).contains(&latitude), "latitude={latitude}");

        let metadata = crate::CloudMetadata {
            point_count: 1,
            version_major: 1,
            version_minor: 4,
            point_format: 6,
            compressed: true,
            bounds_min: [6_064_500.0, 1_985_600.0, 0.0],
            bounds_max: [6_064_600.0, 1_985_700.0, 10.0],
            scales: [0.01; 3],
            offsets: [0.0; 3],
            system_identifier: String::new(),
            generating_software: String::new(),
            creation_date: None,
            file_source_id: 0,
            has_crs: true,
            crs,
            vlr_count: 1,
            evlr_count: 0,
        };
        assert!(matches!(
            assess_survey_readiness(&metadata),
            SurveyReadiness::Ready
        ));
    }

    #[test]
    fn custom_wkt2_projection_is_resolvable_without_root_epsg() {
        let wkt = r#"PROJCRS["NAD83(2011) / California zone 3 (ftUS)",BASEGEOGCRS["NAD83(2011)",DATUM["NAD83 (National Spatial Reference System 2011)",ELLIPSOID["GRS 1980",6378137,298.257222101,LENGTHUNIT["metre",1]]],ID["EPSG",6318]],CONVERSION["SPCS83 California zone 3",METHOD["Lambert Conic Conformal (2SP)"],PARAMETER["Latitude of false origin",36.5],PARAMETER["Longitude of false origin",-120.5],PARAMETER["Latitude of 1st standard parallel",38.4333333333333],PARAMETER["Latitude of 2nd standard parallel",37.0666666666667],PARAMETER["Easting at false origin",6561666.667],PARAMETER["Northing at false origin",1640416.667]],CS[Cartesian,2],AXIS["easting",east,LENGTHUNIT["US survey foot",0.304800609601219]],AXIS["northing",north,LENGTHUNIT["US survey foot",0.304800609601219]]]"#;
        assert_eq!(epsg_from_wkt(wkt).0, Some(6318));
        let proj4 = proj4_from_wkt(wkt).expect("custom WKT2 projection");
        assert!(proj4.contains("+proj=lcc"), "{proj4}");
        assert!(proj4.contains("+units=us-ft"), "{proj4}");
        assert!(proj4.contains("+x_0=2000000"), "{proj4}");

        let custom = CrsInfo {
            horizontal_epsg: Some(6318),
            name: Some("NAD83(2011) / California zone 3 (ftUS)".into()),
            wkt: Some(wkt.into()),
            proj4: Some(proj4),
            ..Default::default()
        };
        assert!(custom.is_resolvable());
        let (longitude, latitude) = reproject_between_crs(
            &custom,
            &CrsInfo {
                horizontal_epsg: Some(4326),
                ..Default::default()
            },
            6_064_521.0,
            1_985_653.0,
        )
        .expect("custom WKT2 to WGS84");
        assert!(
            (-122.4..=-121.8).contains(&longitude),
            "longitude={longitude}"
        );
        assert!((37.2..=37.8).contains(&latitude), "latitude={latitude}");
    }

    #[test]
    fn bounds_and_points_share_the_same_mixed_crs_transform() {
        let source = CrsInfo {
            horizontal_epsg: Some(4326),
            ..Default::default()
        };
        let target = CrsInfo {
            horizontal_epsg: Some(3857),
            ..Default::default()
        };
        let mut points = vec![crate::SamplePoint {
            source_index: 7,
            position: [-71.0589, 42.3601, 15.0],
            intensity: 0,
            classification: 2,
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
        }];
        reproject_points_between_crs(&source, &target, &mut points).unwrap();
        let (min, max) = reproject_bounds_between_crs(
            [-71.1, 42.3, 10.0],
            [-71.0, 42.4, 20.0],
            &source,
            &target,
        )
        .unwrap();
        assert!((min[0]..=max[0]).contains(&points[0].position[0]));
        assert!((min[1]..=max[1]).contains(&points[0].position[1]));
        assert_eq!(points[0].position[2], 15.0);
    }
}
