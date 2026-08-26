//! Raster surface products and breakline quality checks.

use crate::{
    visit_full_density, FullDensityProgress, PointFilter, ProcessingExtent, ProtectedOutput,
    Result, SamplePoint, SurfaceSampler,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridStatistic {
    Minimum,
    #[default]
    Maximum,
    Mean,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RasterSurface {
    pub origin: [f64; 2],
    pub cell_size: f64,
    pub columns: usize,
    pub rows: usize,
    pub no_data: f64,
    /// Row-major values from the lower-left origin.
    pub values: Vec<f64>,
    pub counts: Vec<u32>,
}

impl RasterSurface {
    pub fn from_points(
        points: &[SamplePoint],
        cell_size: f64,
        classification: Option<u8>,
        statistic: GridStatistic,
    ) -> Option<Self> {
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return None;
        }
        let selected: Vec<_> = points
            .iter()
            .filter(|point| classification.is_none_or(|class| point.classification == class))
            .filter(|point| point.position.into_iter().all(f64::is_finite))
            .collect();
        if selected.is_empty() {
            return None;
        }
        let min_x = selected
            .iter()
            .map(|point| point.position[0])
            .fold(f64::INFINITY, f64::min);
        let min_y = selected
            .iter()
            .map(|point| point.position[1])
            .fold(f64::INFINITY, f64::min);
        let max_x = selected
            .iter()
            .map(|point| point.position[0])
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = selected
            .iter()
            .map(|point| point.position[1])
            .fold(f64::NEG_INFINITY, f64::max);
        let columns = (((max_x - min_x) / cell_size).floor() as usize).saturating_add(1);
        let rows = (((max_y - min_y) / cell_size).floor() as usize).saturating_add(1);
        let cell_count = columns.checked_mul(rows)?;
        if cell_count > 100_000_000 {
            return None;
        }
        let no_data = -9999.0;
        let mut values = vec![no_data; cell_count];
        let mut counts = vec![0_u32; cell_count];
        for point in selected {
            let column =
                (((point.position[0] - min_x) / cell_size).floor() as usize).min(columns - 1);
            let row = (((point.position[1] - min_y) / cell_size).floor() as usize).min(rows - 1);
            let index = row * columns + column;
            match statistic {
                GridStatistic::Minimum => {
                    if counts[index] == 0 {
                        values[index] = point.position[2];
                    } else {
                        values[index] = values[index].min(point.position[2]);
                    }
                }
                GridStatistic::Maximum => {
                    if counts[index] == 0 {
                        values[index] = point.position[2];
                    } else {
                        values[index] = values[index].max(point.position[2]);
                    }
                }
                GridStatistic::Mean => {
                    if counts[index] == 0 {
                        values[index] = 0.0;
                    }
                    values[index] += point.position[2];
                }
            }
            counts[index] = counts[index].saturating_add(1);
        }
        if statistic == GridStatistic::Mean {
            for (value, count) in values.iter_mut().zip(&counts) {
                if *count == 0 {
                    *value = no_data;
                } else {
                    *value /= *count as f64;
                }
            }
        }
        Some(Self {
            origin: [min_x, min_y],
            cell_size,
            columns,
            rows,
            no_data,
            values,
            counts,
        })
    }

    pub fn dtm(points: &[SamplePoint], cell_size: f64, ground_class: u8) -> Option<Self> {
        Self::from_points(
            points,
            cell_size,
            Some(ground_class),
            GridStatistic::Minimum,
        )
    }

    pub fn dsm(points: &[SamplePoint], cell_size: f64) -> Option<Self> {
        Self::from_points(points, cell_size, None, GridStatistic::Maximum)
    }

    pub fn value(&self, column: usize, row: usize) -> Option<f64> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        let value = self.values[row * self.columns + column];
        (value != self.no_data).then_some(value)
    }

    pub fn hillshade(&self, azimuth_degrees: f64, altitude_degrees: f64) -> Vec<u8> {
        let azimuth = (360.0 - azimuth_degrees + 90.0).to_radians();
        let zenith = (90.0 - altitude_degrees).to_radians();
        let mut shade = vec![0_u8; self.values.len()];
        if self.columns < 3 || self.rows < 3 {
            return shade;
        }
        for row in 1..self.rows - 1 {
            for column in 1..self.columns - 1 {
                let neighbors = [
                    self.value(column - 1, row - 1),
                    self.value(column, row - 1),
                    self.value(column + 1, row - 1),
                    self.value(column - 1, row),
                    self.value(column + 1, row),
                    self.value(column - 1, row + 1),
                    self.value(column, row + 1),
                    self.value(column + 1, row + 1),
                ];
                if neighbors.iter().any(Option::is_none) {
                    continue;
                }
                let z: Vec<f64> = neighbors.into_iter().flatten().collect();
                let dz_dx = ((z[2] + 2.0 * z[4] + z[7]) - (z[0] + 2.0 * z[3] + z[5]))
                    / (8.0 * self.cell_size);
                let dz_dy = ((z[5] + 2.0 * z[6] + z[7]) - (z[0] + 2.0 * z[1] + z[2]))
                    / (8.0 * self.cell_size);
                let slope = dz_dx.hypot(dz_dy).atan();
                let aspect = dz_dy.atan2(-dz_dx);
                let illumination = zenith.cos() * slope.cos()
                    + zenith.sin() * slope.sin() * (azimuth - aspect).cos();
                shade[row * self.columns + column] =
                    (illumination.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        shade
    }

    /// Writes an ESRI ASCII grid through an adjacent partial file and publishes
    /// it only after a complete flush.
    pub fn write_ascii_grid(&self, path: impl AsRef<Path>, overwrite: bool) -> io::Result<PathBuf> {
        let output = ProtectedOutput::reserve(path, overwrite)?;
        let mut writer = BufWriter::new(File::create(output.partial_path())?);
        writeln!(writer, "ncols {}", self.columns)?;
        writeln!(writer, "nrows {}", self.rows)?;
        writeln!(writer, "xllcorner {:.12}", self.origin[0])?;
        writeln!(writer, "yllcorner {:.12}", self.origin[1])?;
        writeln!(writer, "cellsize {:.12}", self.cell_size)?;
        writeln!(writer, "NODATA_value {:.12}", self.no_data)?;
        for row in (0..self.rows).rev() {
            for column in 0..self.columns {
                if column > 0 {
                    write!(writer, " ")?;
                }
                write!(writer, "{:.6}", self.values[row * self.columns + column])?;
            }
            writeln!(writer)?;
        }
        writer.flush()?;
        drop(writer);
        output.commit()
    }

    pub fn write_hillshade_pgm(
        &self,
        path: impl AsRef<Path>,
        azimuth_degrees: f64,
        altitude_degrees: f64,
        overwrite: bool,
    ) -> io::Result<PathBuf> {
        let output = ProtectedOutput::reserve(path, overwrite)?;
        let mut writer = BufWriter::new(File::create(output.partial_path())?);
        writeln!(writer, "P5\n{} {}\n255", self.columns, self.rows)?;
        let shade = self.hillshade(azimuth_degrees, altitude_degrees);
        for row in (0..self.rows).rev() {
            let start = row * self.columns;
            writer.write_all(&shade[start..start + self.columns])?;
        }
        writer.flush()?;
        drop(writer);
        output.commit()
    }
}

/// Builds a raster from physical LAS/LAZ records in two bounded-memory passes.
/// The first establishes the selected world bounds; the second aggregates
/// directly into grid cells. Viewer samples and resident LOD tiles are never
/// consulted.
pub fn rasterize_full_density(
    path: impl AsRef<Path>,
    cell_size: f64,
    classification: Option<u8>,
    statistic: GridStatistic,
    extent: &ProcessingExtent,
    filter: &PointFilter,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(FullDensityProgress),
) -> Result<(RasterSurface, FullDensityProgress)> {
    if !cell_size.is_finite() || cell_size <= 0.0 {
        return Err(crate::Error::InvalidLimit("surface cell size"));
    }
    let mut bounds: Option<([f64; 2], [f64; 2])> = None;
    visit_full_density(
        path.as_ref(),
        extent,
        filter,
        cancel,
        |state| progress(state),
        |point| {
            if classification.is_none_or(|class| point.classification == class) {
                bounds = Some(match bounds {
                    None => (
                        [point.position[0], point.position[1]],
                        [point.position[0], point.position[1]],
                    ),
                    Some((mut min, mut max)) => {
                        min[0] = min[0].min(point.position[0]);
                        min[1] = min[1].min(point.position[1]);
                        max[0] = max[0].max(point.position[0]);
                        max[1] = max[1].max(point.position[1]);
                        (min, max)
                    }
                });
            }
            Ok(())
        },
    )?;
    let (min, max) = bounds.ok_or_else(|| {
        crate::Error::Crs("surface extent contains no matching points".to_string())
    })?;
    let columns = (((max[0] - min[0]) / cell_size).floor() as usize).saturating_add(1);
    let rows = (((max[1] - min[1]) / cell_size).floor() as usize).saturating_add(1);
    let cell_count = columns
        .checked_mul(rows)
        .filter(|count| *count <= 100_000_000)
        .ok_or_else(|| crate::Error::Crs("surface grid exceeds 100 million cells".to_string()))?;
    let no_data = -9999.0;
    let mut surface = RasterSurface {
        origin: min,
        cell_size,
        columns,
        rows,
        no_data,
        values: vec![no_data; cell_count],
        counts: vec![0; cell_count],
    };
    let final_progress = visit_full_density(
        path,
        extent,
        filter,
        cancel,
        |state| progress(state),
        |point| {
            if classification.is_some_and(|class| point.classification != class) {
                return Ok(());
            }
            let column =
                (((point.position[0] - min[0]) / cell_size).floor() as usize).min(columns - 1);
            let row = (((point.position[1] - min[1]) / cell_size).floor() as usize).min(rows - 1);
            let index = row * columns + column;
            match statistic {
                GridStatistic::Minimum => {
                    if surface.counts[index] == 0 {
                        surface.values[index] = point.position[2];
                    } else {
                        surface.values[index] = surface.values[index].min(point.position[2]);
                    }
                }
                GridStatistic::Maximum => {
                    if surface.counts[index] == 0 {
                        surface.values[index] = point.position[2];
                    } else {
                        surface.values[index] = surface.values[index].max(point.position[2]);
                    }
                }
                GridStatistic::Mean => {
                    if surface.counts[index] == 0 {
                        surface.values[index] = 0.0;
                    }
                    surface.values[index] += point.position[2];
                }
            }
            surface.counts[index] = surface.counts[index].saturating_add(1);
            Ok(())
        },
    )?;
    if statistic == GridStatistic::Mean {
        for (value, count) in surface.values.iter_mut().zip(&surface.counts) {
            if *count == 0 {
                *value = no_data;
            } else {
                *value /= *count as f64;
            }
        }
    }
    Ok((surface, final_progress))
}

impl SurfaceSampler for RasterSurface {
    fn elevation_at(&self, x: f64, y: f64) -> Option<f64> {
        let column = ((x - self.origin[0]) / self.cell_size).floor();
        let row = ((y - self.origin[1]) / self.cell_size).floor();
        if column < 0.0 || row < 0.0 {
            return None;
        }
        self.value(column as usize, row as usize)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Breakline {
    pub id: String,
    pub vertices: Vec<[f64; 3]>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreaklineIssueKind {
    TooFewVertices,
    DuplicateVertex,
    ZeroLengthSegment,
    SelfIntersection,
    ElevationSpike,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BreaklineIssue {
    pub breakline_id: String,
    pub segment: Option<usize>,
    pub kind: BreaklineIssueKind,
    pub detail: String,
}

pub fn validate_breaklines(lines: &[Breakline], max_grade: Option<f64>) -> Vec<BreaklineIssue> {
    let mut issues = Vec::new();
    for line in lines {
        if line.vertices.len() < 2 + usize::from(line.closed) {
            issues.push(issue(
                line,
                None,
                BreaklineIssueKind::TooFewVertices,
                "not enough vertices",
            ));
            continue;
        }
        let segment_count = line.vertices.len() - 1 + usize::from(line.closed);
        for segment in 0..segment_count {
            let a = line.vertices[segment];
            let b = line.vertices[(segment + 1) % line.vertices.len()];
            if a == b {
                issues.push(issue(
                    line,
                    Some(segment),
                    BreaklineIssueKind::ZeroLengthSegment,
                    "segment endpoints are identical",
                ));
                continue;
            }
            if let Some(max_grade) = max_grade {
                let horizontal = (b[0] - a[0]).hypot(b[1] - a[1]);
                if horizontal > f64::EPSILON && (b[2] - a[2]).abs() / horizontal > max_grade {
                    issues.push(issue(
                        line,
                        Some(segment),
                        BreaklineIssueKind::ElevationSpike,
                        "segment exceeds the configured grade limit",
                    ));
                }
            }
        }
        let mut seen = BTreeMap::<(u64, u64, u64), usize>::new();
        for (index, vertex) in line.vertices.iter().enumerate() {
            let key = (
                vertex[0].to_bits(),
                vertex[1].to_bits(),
                vertex[2].to_bits(),
            );
            if seen.insert(key, index).is_some() {
                issues.push(issue(
                    line,
                    Some(index),
                    BreaklineIssueKind::DuplicateVertex,
                    "vertex is repeated",
                ));
            }
        }
        for left in 0..segment_count {
            let left_next = (left + 1) % line.vertices.len();
            for right in left + 1..segment_count {
                let right_next = (right + 1) % line.vertices.len();
                if left == right_next || right == left_next {
                    continue;
                }
                if segments_intersect_xy(
                    line.vertices[left],
                    line.vertices[left_next],
                    line.vertices[right],
                    line.vertices[right_next],
                ) {
                    issues.push(issue(
                        line,
                        Some(left),
                        BreaklineIssueKind::SelfIntersection,
                        &format!("segment {left} intersects segment {right}"),
                    ));
                }
            }
        }
    }
    issues
}

fn issue(
    line: &Breakline,
    segment: Option<usize>,
    kind: BreaklineIssueKind,
    detail: &str,
) -> BreaklineIssue {
    BreaklineIssue {
        breakline_id: line.id.clone(),
        segment,
        kind,
        detail: detail.to_string(),
    }
}

fn segments_intersect_xy(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> bool {
    fn orientation(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }
    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);
    ab_c * ab_d < 0.0 && cd_a * cd_b < 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64, z: f64, class: u8) -> SamplePoint {
        SamplePoint {
            source_index: 0,
            position: [x, y, z],
            intensity: 0,
            classification: class,
            return_number: 1,
            number_of_returns: 1,
            scan_angle: 0.0,
            user_data: 0,
            point_source_id: 0,
            gps_time: None,
            color: None,
            nir: None,
            is_synthetic: false,
            is_key_point: false,
            is_withheld: false,
            is_overlap: false,
        }
    }

    #[test]
    fn dsm_and_dtm_choose_opposite_cell_extremes() {
        let points = vec![point(0.0, 0.0, 1.0, 2), point(0.1, 0.1, 9.0, 6)];
        let dsm = RasterSurface::dsm(&points, 1.0).unwrap();
        let dtm = RasterSurface::dtm(&points, 1.0, 2).unwrap();
        assert_eq!(Some(9.0), dsm.value(0, 0));
        assert_eq!(Some(1.0), dtm.value(0, 0));
    }

    #[test]
    fn breakline_validation_finds_crossings_and_grade_spikes() {
        let line = Breakline {
            id: "b1".to_string(),
            vertices: vec![
                [0.0, 0.0, 0.0],
                [10.0, 10.0, 20.0],
                [0.0, 10.0, 20.0],
                [10.0, 0.0, 20.0],
            ],
            closed: false,
        };
        let issues = validate_breaklines(&[line], Some(1.0));
        assert!(issues
            .iter()
            .any(|item| item.kind == BreaklineIssueKind::SelfIntersection));
        assert!(issues
            .iter()
            .any(|item| item.kind == BreaklineIssueKind::ElevationSpike));
    }
}
