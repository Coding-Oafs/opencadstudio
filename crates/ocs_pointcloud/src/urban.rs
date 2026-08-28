//! Native urban point-cloud classification using the UPCP ordered-fuser model.
//!
//! The design mirrors `scripts/lidar/boston_upcp_classifier.py`, which remains
//! the reference implementation and regression oracle:
//!
//! 1. stream every physical source record (never a viewer sample or LOD tile);
//! 2. seed trusted labels from existing ASPRS classes (2->9, 17->14, 18->99);
//! 3. run ordered spatial fusers against versioned reference layers;
//! 4. preserve the source `classification` byte untouched;
//! 5. write a separate uint8 `label` extra dimension plus provenance VLR; and
//! 6. publish the result only after a complete, validated write.
//!
//! Point records are copied at the byte level, so every existing dimension
//! (GPS time, RGB, flags, user extra bytes) survives the rewrite unchanged.

use crate::{Error, SamplePoint};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// UPCP label table shared with the Python oracle.
pub const UPCP_LABELS: &[(u8, &str)] = &[
    (0, "Unknown"),
    (1, "Road"),
    (9, "Ground"),
    (10, "Building"),
    (14, "Bridge"),
    (30, "Vegetation"),
    (99, "Noise"),
];

/// ASPRS input class -> UPCP seed label. Classes 9 (water) and 10 (rail) are
/// deliberately absent because UPCP has no equivalent class.
pub const ASPRS_SEEDS: &[(u8, u8)] = &[(2, 9), (17, 14), (18, 99)];

/// Total fallback paved widths in survey feet when both `SURFACE_WD` and
/// `NUM_LANES` are absent, keyed by MassDOT road class.
pub const ROAD_CLASS_WIDTHS_FT: &[(u32, f64)] = &[
    (1, 72.0),
    (2, 60.0),
    (3, 48.0),
    (4, 36.0),
    (5, 24.0),
    (6, 20.0),
];

const MANIFEST_SCHEMA: &str = "OpenCADStudio.UPCP.Boston.batch.v2";
const PROVENANCE_SCHEMA: &str = "OpenCADStudio.UPCP.Boston.v2";
const UPSTREAM_REPOSITORY: &str = "https://github.com/Coding-Oafs/Urban_PointCloud_Processing";
const PROGRESS_CHUNK_POINTS: usize = 50_000;

// ---------------------------------------------------------------------------
// Settings, progress, and result types
// ---------------------------------------------------------------------------

/// Which files an urban classification run covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrbanScope {
    /// Only the currently attached source tile.
    CurrentTile,
    /// Every LAS/LAZ file in the source folder.
    Folder,
}

/// Where reference layers come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum UrbanProfile {
    /// Boston ArcGIS for an EPSG:6492 cloud; local directory otherwise.
    AutoDetect,
    /// Boston Planning buildings, MassDOT roads, and street trees.
    BostonArcGis,
    /// GeoJSON files named `<tile>.buildings|roads|trees.geojson` in a folder.
    LocalDirectory { path: PathBuf },
}

/// User-configurable urban classification inputs. Defaults mirror the plan
/// table: fusers on, +1 survey foot road edge, output beside the source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UrbanClassificationSettings {
    pub scope: UrbanScope,
    pub profile: UrbanProfile,
    /// `None` means `<source folder>/classified`.
    pub output_folder: Option<PathBuf>,
    pub seed_source_classes: bool,
    pub building_fuser: bool,
    pub road_fuser: bool,
    pub vegetation_fuser: bool,
    /// Extra allowance added to half of `SURFACE_WD` (survey feet).
    pub road_edge_allowance_ft: f64,
    /// Street-tree buffer radius (survey feet).
    pub tree_radius_ft: f64,
    /// Store and reuse the exact GeoJSON responses each tile queried.
    pub reference_cache: bool,
    /// Replace existing outputs; completed tiles are skipped when false.
    pub overwrite_outputs: bool,
}

impl Default for UrbanClassificationSettings {
    fn default() -> Self {
        Self {
            scope: UrbanScope::CurrentTile,
            profile: UrbanProfile::AutoDetect,
            output_folder: None,
            seed_source_classes: true,
            building_fuser: true,
            road_fuser: true,
            vegetation_fuser: true,
            road_edge_allowance_ft: 1.0,
            tree_radius_ft: 12.0,
            reference_cache: true,
            overwrite_outputs: false,
        }
    }
}

/// Stage of an in-flight urban classification job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrbanStage {
    LoadingReferences,
    Classifying,
    Validating,
    Completed,
}

/// One progress tick for the job UI: current tile, points, feature counts.
#[derive(Clone, Debug, Serialize)]
pub struct UrbanJobProgress {
    pub tile_index: usize,
    pub tile_total: usize,
    pub tile_name: String,
    pub stage: UrbanStage,
    pub points_processed: u64,
    pub points_total: u64,
    pub building_features: usize,
    pub road_features: usize,
    pub tree_features: usize,
    pub output_path: PathBuf,
    pub elapsed_ms: u128,
}

/// Per-tile outcome recorded in the batch manifest. Field names follow the
/// Python oracle manifest so `audit_boston_classified.py` and the app both
/// keep working against native outputs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UrbanTileStats {
    pub status: String,
    pub source: PathBuf,
    pub output: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_format: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub las_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_classification_counts: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upcp_label_counts: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub building_feature_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub road_feature_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_feature_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_utc: Option<String>,
}

impl UrbanTileStats {
    fn failed(source: &Path, output: PathBuf, status: &str, error: String) -> Self {
        Self {
            status: status.to_string(),
            source: source.to_path_buf(),
            output,
            error: Some(error),
            point_count: None,
            point_format: None,
            las_version: None,
            bounds: None,
            original_classification_counts: None,
            upcp_label_counts: None,
            building_feature_count: None,
            road_feature_count: None,
            tree_feature_count: None,
            elapsed_seconds: None,
            completed_utc: None,
        }
    }

    fn output_for(source: &Path, output_dir: &Path) -> PathBuf {
        let stem = source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        output_dir.join(format!("{stem}_classified.laz"))
    }
}

/// Dataset-level batch manifest, written atomically after every tile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UrbanBatchManifest {
    pub schema: String,
    pub status: String,
    pub started_utc: String,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub methodology: Value,
    pub tiles: Vec<UrbanTileStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_count: Option<usize>,
}

/// Final summary of a folder run.
#[derive(Clone, Debug)]
pub struct UrbanBatchSummary {
    pub manifest_path: PathBuf,
    pub tile_total: usize,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cancelled: bool,
    /// Outputs of tiles that completed in this run or a resumed earlier run.
    pub outputs: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// Raw LAS/LAZ container access
// ---------------------------------------------------------------------------

const LASZIP_RECORD_ID: u16 = 22_204;

/// One raw VLR, header fields plus the record payload verbatim.
#[derive(Clone, Debug)]
struct RawVlr {
    user_id: String,
    record_id: u16,
    description: String,
    data: Vec<u8>,
}

impl RawVlr {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(54 + self.data.len());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        let mut user_id = [0u8; 16];
        let id_bytes = self.user_id.as_bytes();
        user_id[..id_bytes.len().min(16)].copy_from_slice(&id_bytes[..id_bytes.len().min(16)]);
        bytes.extend_from_slice(&user_id);
        bytes.extend_from_slice(&self.record_id.to_le_bytes());
        bytes.extend_from_slice(&(self.data.len() as u16).to_le_bytes());
        let mut description = [0u8; 32];
        let description_bytes = self.description.as_bytes();
        description[..description_bytes.len().min(32)]
            .copy_from_slice(&description_bytes[..description_bytes.len().min(32)]);
        bytes.extend_from_slice(&description);
        bytes.extend_from_slice(&self.data);
        bytes
    }
}

/// Parsed public header block of a LAS/LAZ file.
#[derive(Clone, Debug)]
struct RawHeader {
    raw: Vec<u8>,
    version: (u8, u8),
    point_format: u8,
    record_length: u16,
    point_count: u64,
    scales: [f64; 3],
    offsets: [f64; 3],
    bounds: [f64; 6],
    vlrs: Vec<RawVlr>,
}

impl RawHeader {
    fn classification_offset(&self) -> usize {
        if self.point_format >= 6 {
            16
        } else {
            15
        }
    }

    /// Fixed (non-extra-byte) record length for the point format.
    fn fixed_record_length(&self) -> Option<usize> {
        Some(match self.point_format {
            0 => 20,
            1 => 28,
            2 => 26,
            3 => 34,
            4 => 57,
            5 => 63,
            6 => 30,
            7 => 36,
            8 => 38,
            9 => 59,
            10 => 67,
            _ => return None,
        })
    }

    fn extra_byte_count(&self) -> Result<usize, Error> {
        let fixed = self.fixed_record_length().ok_or_else(|| {
            Error::Urban(format!("unsupported point format {}", self.point_format))
        })?;
        if (self.record_length as usize) < fixed {
            return Err(Error::Urban(format!(
                "record length {} is shorter than point format {} requires",
                self.record_length, self.point_format
            )));
        }
        Ok(self.record_length as usize - fixed)
    }

    fn laszip_vlr(&self) -> Option<&RawVlr> {
        self.vlrs
            .iter()
            .find(|vlr| {
                vlr.user_id == "laszip encoded" && vlr.record_id == LASZIP_RECORD_ID
            })
    }

    fn wkt(&self) -> Option<&[u8]> {
        self.vlrs
            .iter()
            .find(|vlr| vlr.user_id == "LASF_Projection" && vlr.record_id == 2112)
            .map(|vlr| vlr.data.as_slice())
    }

    fn las_version(&self) -> String {
        format!("{}.{}", self.version.0, self.version.1)
    }
}

fn read_exact_or_end(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn trim_nuls(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// Parse the LAS public header block and VLRs from the bytes up to the point
/// data offset.
fn parse_raw_header(bytes: Vec<u8>) -> Result<RawHeader, Error> {
    if bytes.len() < 107 || &bytes[0..4] != b"LASF" {
        return Err(Error::Urban("not a LAS file: bad signature".to_string()));
    }
    let u16_at = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let u32_at = |offset: usize| {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let u64_at = |offset: usize| {
        u64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ])
    };
    let f64_at = |offset: usize| f64::from_bits(u64_at(offset));
    // LAS public-header offsets: version major/minor at 24/25 and public
    // header size at 94. Reading header size at 24 accidentally treated the
    // version bytes as a u16 and skipped into the middle of valid VLR data.
    let version = (bytes[24], bytes[25]);
    let header_size = u16_at(94) as usize;
    if header_size < 107 || header_size > bytes.len() {
        return Err(Error::Urban(format!("invalid header size {header_size}")));
    }
    let offset_to_point_data = u32_at(96) as usize;
    if offset_to_point_data < header_size || offset_to_point_data > bytes.len() {
        return Err(Error::Urban(format!(
            "offset to point data {offset_to_point_data} is outside the header block"
        )));
    }
    let v1_4 = version.1 >= 4;
    // Field positions are identical across LAS versions: VLR count u32 at
    // 100, point format u8 at 104, record length u16 at 105, and legacy point
    // count u32 at 107.
    let vlr_count = u32_at(100);
    // LASzip marks compressed point data in the high bit of the format byte;
    // the low six bits remain the logical ASPRS point-format id.
    let point_format = bytes[104] & 0x3f;
    let record_length = u16_at(105);
    let legacy_count = u32_at(107) as u64;
    let point_count = if v1_4 {
        let extended = u64_at(247);
        if extended != 0 {
            extended
        } else {
            legacy_count
        }
    } else {
        legacy_count
    };
    let mut vlrs = Vec::new();
    let mut cursor = header_size;
    for _ in 0..vlr_count {
        if cursor + 54 > offset_to_point_data {
            return Err(Error::Urban("truncated VLR block".to_string()));
        }
        let header = &bytes[cursor..cursor + 54];
        let record_id = u16::from_le_bytes([header[18], header[19]]);
        let record_length = u16::from_le_bytes([header[20], header[21]]) as usize;
        let data_start = cursor + 54;
        let data_end = data_start + record_length;
        if data_end > offset_to_point_data {
            return Err(Error::Urban(
                "VLR extends past the point data offset".to_string(),
            ));
        }
        vlrs.push(RawVlr {
            user_id: trim_nuls(&header[2..18]),
            record_id,
            description: trim_nuls(&header[22..54]),
            data: bytes[data_start..data_end].to_vec(),
        });
        cursor = data_end;
    }
    Ok(RawHeader {
        raw: bytes[..header_size].to_vec(),
        version,
        point_format,
        record_length,
        point_count,
        scales: [f64_at(131), f64_at(139), f64_at(147)],
        offsets: [f64_at(155), f64_at(163), f64_at(171)],
        bounds: [
            f64_at(179),
            f64_at(187),
            f64_at(195),
            f64_at(203),
            f64_at(211),
            f64_at(219),
        ],
        vlrs,
    })
}

fn read_header_only(path: &Path) -> Result<RawHeader, Error> {
    let mut file = File::open(path).map_err(Error::Io)?;
    let mut offset_bytes = [0u8; 100];
    if read_exact_or_end(&mut file, &mut offset_bytes)? != 100 {
        return Err(Error::Urban(
            "file is too short for a LAS header".to_string(),
        ));
    }
    let offset_to_point_data = u32::from_le_bytes([
        offset_bytes[96],
        offset_bytes[97],
        offset_bytes[98],
        offset_bytes[99],
    ]) as u64;
    if &offset_bytes[0..4] != b"LASF" || offset_to_point_data < 107 {
        return Err(Error::Urban("not a LAS file".to_string()));
    }
    let mut full = Vec::new();
    file.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
    let mut limited = file.take(offset_to_point_data);
    limited.read_to_end(&mut full).map_err(Error::Io)?;
    parse_raw_header(full)
}

/// Streaming reader over raw point records of a LAS or LAZ file.
struct RawPointReader {
    header: RawHeader,
    remaining: u64,
    stream: PointDataStream,
}

enum PointDataStream {
    /// Plain LAS: bounded raw reads from the point data offset.
    Plain(BufReader<File>, u64),
    /// LAZ: sequential decompression driven by the LasZip VLR.
    Laz(laz::LasZipDecompressor<'static, BufReader<File>>),
}

impl RawPointReader {
    fn open(path: &Path) -> Result<Self, Error> {
        let mut file = File::open(path).map_err(Error::Io)?;
        let mut offset_bytes = [0u8; 100];
        if read_exact_or_end(&mut file, &mut offset_bytes)? != 100 || &offset_bytes[0..4] != b"LASF"
        {
            return Err(Error::Urban("not a LAS file: bad signature".to_string()));
        }
        let offset_to_point_data = u32::from_le_bytes([
            offset_bytes[96],
            offset_bytes[97],
            offset_bytes[98],
            offset_bytes[99],
        ]) as u64;
        if offset_to_point_data < 107 {
            return Err(Error::Urban(
                "offset to point data is impossibly small".to_string(),
            ));
        }
        file.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
        let mut full = Vec::new();
        let mut limited = file.take(offset_to_point_data);
        limited.read_to_end(&mut full).map_err(Error::Io)?;
        if full.len() != offset_to_point_data as usize {
            return Err(Error::Urban(
                "file ends before the point data offset".to_string(),
            ));
        }
        let header = parse_raw_header(full)?;
        let point_count = header.point_count;
        let reader = BufReader::with_capacity(1 << 20, File::open(path).map_err(Error::Io)?);
        let mut positioned = reader;
        positioned
            .seek(SeekFrom::Start(offset_to_point_data))
            .map_err(Error::Io)?;
        let stream = if let Some(vlr) = header.laszip_vlr() {
            let laz_vlr = laz::LazVlr::from_buffer(vlr.data.clone())
                .map_err(|error| Error::Urban(format!("invalid LasZip VLR: {error}")))?;
            PointDataStream::Laz(laz::LasZipDecompressor::new(positioned, laz_vlr).map_err(
                |error| Error::Urban(format!("cannot start LAZ decompression: {error}")),
            )?)
        } else {
            PointDataStream::Plain(positioned, header.point_count * header.record_length as u64)
        };
        Ok(Self {
            header,
            remaining: point_count,
            stream,
        })
    }

    /// Fill `buf` with up to `max_records` raw records; returns the count.
    fn read_chunk(&mut self, buf: &mut Vec<u8>, max_records: usize) -> Result<usize, Error> {
        let record_length = self.header.record_length as usize;
        let wanted = max_records.min(self.remaining as usize);
        if wanted == 0 {
            return Ok(0);
        }
        buf.clear();
        buf.resize(wanted * record_length, 0);
        match &mut self.stream {
            PointDataStream::Plain(reader, remaining_bytes) => {
                let readable = (buf.len() as u64).min(*remaining_bytes) as usize;
                let filled = read_exact_or_end(reader, &mut buf[..readable]).map_err(Error::Io)?;
                *remaining_bytes -= filled as u64;
                let count = filled / record_length;
                buf.truncate(count * record_length);
                self.remaining -= count as u64;
                Ok(count)
            }
            PointDataStream::Laz(decompressor) => {
                decompressor
                    .decompress_many(&mut buf[..])
                    .map_err(|error| Error::Urban(format!("LAZ decompression failed: {error}")))?;
                self.remaining -= wanted as u64;
                Ok(wanted)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry: masks built from reference layers
// ---------------------------------------------------------------------------

type Point2 = [f64; 2];

#[derive(Clone, Debug)]
struct MaskPolygon {
    exterior: Vec<Point2>,
    holes: Vec<Vec<Point2>>,
    bbox: [f64; 4],
}

#[derive(Clone, Debug, Default)]
struct PolygonMask {
    polygons: Vec<MaskPolygon>,
    grid: Option<UniformGrid>,
}

#[derive(Clone, Debug)]
struct UniformGrid {
    xmin: f64,
    ymin: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

enum RingTest {
    Inside,
    Outside,
    Boundary,
}

fn point_on_segment(a: Point2, b: Point2, x: f64, y: f64) -> bool {
    let cross = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
    let length_sq = (b[0] - a[0]) * (b[0] - a[0]) + (b[1] - a[1]) * (b[1] - a[1]);
    let tolerance = 1e-12 * length_sq.max(1.0);
    if cross.abs() > tolerance {
        return false;
    }
    let min_x = a[0].min(b[0]) - tolerance;
    let max_x = a[0].max(b[0]) + tolerance;
    let min_y = a[1].min(b[1]) - tolerance;
    let max_y = a[1].max(b[1]) + tolerance;
    x >= min_x && x <= max_x && y >= min_y && y <= max_y
}

fn point_in_ring(ring: &[Point2], x: f64, y: f64) -> RingTest {
    let mut inside = false;
    let count = ring.len();
    for index in 0..count {
        let a = ring[index];
        let b = ring[(index + 1) % count];
        if point_on_segment(a, b, x, y) {
            return RingTest::Boundary;
        }
        if (a[1] > y) != (b[1] > y) {
            let intersect_x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if x < intersect_x {
                inside = !inside;
            }
        }
    }
    if inside {
        RingTest::Inside
    } else {
        RingTest::Outside
    }
}

impl MaskPolygon {
    fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.bbox[0] || y < self.bbox[1] || x > self.bbox[2] || y > self.bbox[3] {
            return false;
        }
        match point_in_ring(&self.exterior, x, y) {
            RingTest::Outside => return false,
            RingTest::Boundary => return true,
            RingTest::Inside => {}
        }
        for hole in &self.holes {
            match point_in_ring(hole, x, y) {
                // A point strictly inside a hole is outside the polygon; a
                // point on any boundary is on the polygon boundary.
                RingTest::Inside => return false,
                RingTest::Boundary => return true,
                RingTest::Outside => {}
            }
        }
        true
    }
}

impl PolygonMask {
    fn build(mut polygons: Vec<MaskPolygon>) -> Self {
        polygons.retain(|polygon| polygon.exterior.len() >= 3);
        let grid = if polygons.is_empty() {
            None
        } else {
            Some(Self::build_grid(&polygons))
        };
        Self { polygons, grid }
    }

    fn build_grid(polygons: &[MaskPolygon]) -> UniformGrid {
        let mut xmin = f64::MAX;
        let mut ymin = f64::MAX;
        let mut xmax = f64::MIN;
        let mut ymax = f64::MIN;
        for polygon in polygons {
            xmin = xmin.min(polygon.bbox[0]);
            ymin = ymin.min(polygon.bbox[1]);
            xmax = xmax.max(polygon.bbox[2]);
            ymax = ymax.max(polygon.bbox[3]);
        }
        let mut cell = (xmax - xmin).max(ymax - ymin) / 48.0;
        if !cell.is_finite() || cell <= 0.0 {
            cell = 1.0;
        }
        let cols = (((xmax - xmin) / cell).ceil() as usize).max(1);
        let rows = (((ymax - ymin) / cell).ceil() as usize).max(1);
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); cols * rows];
        for (index, polygon) in polygons.iter().enumerate() {
            let col_min = (((polygon.bbox[0] - xmin) / cell).floor() as isize).max(0) as usize;
            let col_max = (((polygon.bbox[2] - xmin) / cell).floor() as usize).min(cols - 1);
            let row_min = (((polygon.bbox[1] - ymin) / cell).floor() as isize).max(0) as usize;
            let row_max = (((polygon.bbox[3] - ymin) / cell).floor() as usize).min(rows - 1);
            for row in row_min..=row_max {
                for col in col_min..=col_max {
                    cells[row * cols + col].push(index as u32);
                }
            }
        }
        UniformGrid {
            xmin,
            ymin,
            cell,
            cols,
            rows,
            cells,
        }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        let Some(grid) = &self.grid else {
            return false;
        };
        if x < grid.xmin || y < grid.ymin {
            return false;
        }
        let col = (((x - grid.xmin) / grid.cell) as isize).max(0) as usize;
        let row = (((y - grid.ymin) / grid.cell) as isize).max(0) as usize;
        if col >= grid.cols || row >= grid.rows {
            return false;
        }
        grid.cells[row * grid.cols + col]
            .iter()
            .any(|index| self.polygons[*index as usize].contains(x, y))
    }
}

fn ring_bbox(ring: &[Point2]) -> [f64; 4] {
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for point in ring {
        bbox[0] = bbox[0].min(point[0]);
        bbox[1] = bbox[1].min(point[1]);
        bbox[2] = bbox[2].max(point[0]);
        bbox[3] = bbox[3].max(point[1]);
    }
    bbox
}

/// Close a ring if needed so the containment test treats it as a loop.
fn normalize_ring(mut ring: Vec<Point2>) -> Vec<Point2> {
    if ring.len() >= 2 {
        let first = ring[0];
        let last = *ring.last().expect("len checked");
        if (first[0] - last[0]).abs() > f64::EPSILON || (first[1] - last[1]).abs() > f64::EPSILON {
            ring.push(first);
        }
    }
    ring
}

/// Twice the signed area; used only to drop degenerate slivers.
fn ring_area_twice(ring: &[Point2]) -> f64 {
    let mut sum = 0.0;
    let count = ring.len();
    for index in 0..count.saturating_sub(1) {
        let a = ring[index];
        let b = ring[index + 1];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum
}

// ---------------------------------------------------------------------------
// Reference features
// ---------------------------------------------------------------------------

/// Geometry of one reference feature, restricted to what urban fusion needs.
#[derive(Clone, Debug)]
pub enum ReferenceGeometry {
    Point(Point2),
    LineString(Vec<Point2>),
    MultiLineString(Vec<Vec<Point2>>),
    Polygon(Vec<Vec<Point2>>),
    MultiPolygon(Vec<Vec<Vec<Point2>>>),
}

/// One GeoJSON feature: geometry plus raw properties for width rules.
#[derive(Clone, Debug)]
pub struct ReferenceFeature {
    pub geometry: ReferenceGeometry,
    pub properties: Value,
}

/// A loaded reference layer, matching the cached FeatureCollection files.
#[derive(Clone, Debug, Default)]
pub struct ReferenceCollection {
    pub features: Vec<ReferenceFeature>,
    pub from_cache: bool,
    pub query_url: String,
}

/// Which reference layer a provider should load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrbanLayer {
    Buildings,
    Roads,
    Trees,
}

impl UrbanLayer {
    pub(crate) fn file_suffix(self) -> &'static str {
        match self {
            Self::Buildings => "buildings",
            Self::Roads => "roads",
            Self::Trees => "trees",
        }
    }
}

/// Cache file naming shared by every provider: `<tile>.<layer>.geojson`.
pub(crate) fn layer_cache_path(
    references_dir: &Path,
    tile_stem: &str,
    layer: UrbanLayer,
) -> PathBuf {
    references_dir.join(format!("{tile_stem}.{}.geojson", layer.file_suffix()))
}

/// Source of versioned reference layers. Implementations must cache the exact
/// GeoJSON responses beside the batch manifest so runs are reproducible and
/// fully offline reruns are possible.
pub trait UrbanReferenceProvider: Send {
    fn load(
        &mut self,
        layer: UrbanLayer,
        tile_stem: &str,
        bounds: [f64; 4],
        references_dir: &Path,
        use_cache: bool,
    ) -> Result<ReferenceCollection, Error>;
}

/// Reads `<tile>.<layer>.geojson` files from a directory. This covers both
/// user-supplied local profiles and offline reruns from the reference cache.
pub struct LocalVectorProvider {
    directory: PathBuf,
}

impl LocalVectorProvider {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
}

impl UrbanReferenceProvider for LocalVectorProvider {
    fn load(
        &mut self,
        layer: UrbanLayer,
        tile_stem: &str,
        _bounds: [f64; 4],
        _references_dir: &Path,
        _use_cache: bool,
    ) -> Result<ReferenceCollection, Error> {
        let path = layer_cache_path(&self.directory, tile_stem, layer);
        if !path.is_file() {
            return Ok(ReferenceCollection::default());
        }
        let text = fs::read_to_string(&path).map_err(Error::Io)?;
        let mut collection = parse_geojson_collection(&text).map_err(Error::Urban)?;
        collection.query_url = path.display().to_string();
        Ok(collection)
    }
}

/// Parse a GeoJSON FeatureCollection into features.
pub fn parse_geojson_collection(text: &str) -> Result<ReferenceCollection, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
    let mut collection = ReferenceCollection::default();
    let features = match &value {
        Value::Object(object) => match object.get("type").and_then(Value::as_str) {
            Some("FeatureCollection") => object
                .get("features")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            Some("Feature") => vec![value.clone()],
            _ => return Err("expected a GeoJSON FeatureCollection".to_string()),
        },
        _ => return Err("expected a GeoJSON object".to_string()),
    };
    if let Some(url) = value.pointer("/metadata/query_url").and_then(Value::as_str) {
        collection.query_url = url.to_string();
    }
    for feature in features {
        let Some(geometry_value) = feature.get("geometry") else {
            continue;
        };
        let Some(geometry) = parse_geojson_geometry(geometry_value)? else {
            continue;
        };
        collection.features.push(ReferenceFeature {
            geometry,
            properties: feature.get("properties").cloned().unwrap_or(Value::Null),
        });
    }
    Ok(collection)
}

/// Parse one GeoJSON geometry; `None` means empty or unsupported geometry.
pub fn parse_geojson_geometry(value: &Value) -> Result<Option<ReferenceGeometry>, String> {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Ok(None);
    };
    let coordinates = |value: &Value| -> Result<Vec<f64>, String> {
        value
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .map(|item| item.as_f64())
                    .collect::<Option<Vec<f64>>>()
            })
            .ok_or_else(|| "coordinates must be numeric arrays".to_string())
    };
    let ring = |value: &Value| -> Result<Vec<Point2>, String> {
        Ok(value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        let pair = coordinates(item)?;
                        if pair.len() < 2 {
                            return Err("coordinate needs at least x and y".to_string());
                        }
                        Ok([pair[0], pair[1]])
                    })
                    .collect::<Result<Vec<Point2>, String>>()
            })
            .transpose()?
            .unwrap_or_default())
    };
    let geometry = match kind {
        "Point" => {
            let Some(pair) = value.get("coordinates").map(coordinates).transpose()? else {
                return Ok(None);
            };
            if pair.len() < 2 {
                return Ok(None);
            }
            ReferenceGeometry::Point([pair[0], pair[1]])
        }
        "MultiPoint" => return Ok(None),
        "LineString" => {
            let Some(points) = value.get("coordinates").map(ring).transpose()? else {
                return Ok(None);
            };
            ReferenceGeometry::LineString(points)
        }
        "MultiLineString" => {
            let Some(parts) = value.get("coordinates").and_then(Value::as_array) else {
                return Ok(None);
            };
            let mut lines = Vec::new();
            for part in parts {
                let points = ring(part)?;
                if points.len() >= 2 {
                    lines.push(points);
                }
            }
            ReferenceGeometry::MultiLineString(lines)
        }
        "Polygon" => {
            let Some(parts) = value.get("coordinates").and_then(Value::as_array) else {
                return Ok(None);
            };
            let rings = parse_rings(parts)?;
            if rings.is_empty() {
                return Ok(None);
            }
            ReferenceGeometry::Polygon(rings)
        }
        "MultiPolygon" => {
            let Some(polygons) = value.get("coordinates").and_then(Value::as_array) else {
                return Ok(None);
            };
            let mut multipolygon = Vec::new();
            for polygon in polygons {
                let Some(parts) = polygon.as_array() else {
                    continue;
                };
                let rings = parse_rings(parts)?;
                if !rings.is_empty() {
                    multipolygon.push(rings);
                }
            }
            if multipolygon.is_empty() {
                return Ok(None);
            }
            ReferenceGeometry::MultiPolygon(multipolygon)
        }
        _ => return Ok(None),
    };
    Ok(Some(geometry))
}

fn parse_rings(parts: &[Value]) -> Result<Vec<Vec<Point2>>, String> {
    let coordinates = |value: &Value| -> Result<Vec<f64>, String> {
        value
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .map(|item| item.as_f64())
                    .collect::<Option<Vec<f64>>>()
            })
            .ok_or_else(|| "coordinates must be numeric arrays".to_string())
    };
    let mut rings = Vec::new();
    for part in parts {
        let mut points = Vec::new();
        let Some(pairs) = part.as_array() else {
            continue;
        };
        for pair in pairs {
            let values = coordinates(pair)?;
            if values.len() < 2 {
                continue;
            }
            points.push([values[0], values[1]]);
        }
        points = normalize_ring(points);
        if points.len() >= 3 && ring_area_twice(&points).abs() > 0.0 {
            rings.push(points);
        }
    }
    Ok(rings)
}

// ---------------------------------------------------------------------------
// Mask construction (building / road / tree)
// ---------------------------------------------------------------------------

/// Mask covering every input polygon, boundary inclusive.
fn build_building_mask(collection: &ReferenceCollection) -> PolygonMask {
    let mut polygons = Vec::new();
    for feature in &collection.features {
        let ring_sets: Vec<&Vec<Vec<Point2>>> = match &feature.geometry {
            ReferenceGeometry::Polygon(rings) => vec![rings],
            ReferenceGeometry::MultiPolygon(parts) => parts.iter().collect(),
            _ => continue,
        };
        for rings in ring_sets {
            let Some(exterior) = rings.first().cloned() else {
                continue;
            };
            let holes = rings.iter().skip(1).cloned().collect();
            let bbox = ring_bbox(&exterior);
            polygons.push(MaskPolygon {
                exterior,
                holes,
                bbox,
            });
        }
    }
    PolygonMask::build(polygons)
}

fn positive_number(value: Option<&Value>) -> Option<f64> {
    let number = value.and_then(Value::as_f64)?;
    if number.is_finite() && number > 0.0 {
        Some(number)
    } else {
        None
    }
}

/// Half-width of one road footprint in survey feet, mirroring the oracle's
/// `SURFACE_WD` -> `NUM_LANES * 12` -> class-width fallback, clamped to
/// [6, 80] after the edge allowance.
fn road_half_width_ft(properties: &Value, extra_ft: f64) -> f64 {
    let surface_width = positive_number(properties.get("SURFACE_WD"));
    let lanes = positive_number(properties.get("NUM_LANES"));
    let road_class = positive_number(properties.get("CLASS"))
        .map(|value| value as u32)
        .filter(|value| *value > 0)
        .unwrap_or(5);
    let total_width = if let Some(surface_width) = surface_width {
        surface_width
    } else if let Some(lanes) = lanes {
        lanes * 12.0
    } else {
        ROAD_CLASS_WIDTHS_FT
            .iter()
            .find(|(class, _)| *class == road_class)
            .map(|(_, width)| *width)
            .unwrap_or(24.0)
    };
    (total_width / 2.0 + extra_ft).clamp(6.0, 80.0)
}

/// Build a flat-capped, mitre-joined buffer mask over road centerlines.
///
/// A point is inside the buffered road when it lies in any per-segment
/// rectangle, or in a mitre corner triangle at a convex vertex — exactly the
/// region shapely's `buffer(cap_style="flat", join_style="mitre")` covers.
fn build_road_mask(collection: &ReferenceCollection, extra_ft: f64) -> PolygonMask {
    let mut polygons = Vec::new();
    for feature in &collection.features {
        let half_width = road_half_width_ft(&feature.properties, extra_ft);
        let parts: Vec<&Vec<Point2>> = match &feature.geometry {
            ReferenceGeometry::LineString(points) => vec![points],
            ReferenceGeometry::MultiLineString(lines) => lines.iter().collect(),
            _ => continue,
        };
        for points in parts {
            if points.len() < 2 {
                continue;
            }
            let mut segments = Vec::new();
            for window in points.windows(2) {
                let a = window[0];
                let b = window[1];
                let dx = b[0] - a[0];
                let dy = b[1] - a[1];
                let length = (dx * dx + dy * dy).sqrt();
                if length <= 0.0 {
                    continue;
                }
                segments.push((a, b, [dx / length, dy / length]));
            }
            // Per-segment rectangles with flat caps.
            for (a, b, dir) in &segments {
                let normal = [-dir[1], dir[0]];
                let corners = [
                    [a[0] + normal[0] * half_width, a[1] + normal[1] * half_width],
                    [b[0] + normal[0] * half_width, b[1] + normal[1] * half_width],
                    [b[0] - normal[0] * half_width, b[1] - normal[1] * half_width],
                    [a[0] - normal[0] * half_width, a[1] - normal[1] * half_width],
                ];
                let mut exterior = corners.to_vec();
                exterior.push(corners[0]);
                polygons.push(MaskPolygon {
                    exterior,
                    holes: Vec::new(),
                    bbox: ring_bbox(&corners),
                });
            }
            // Mitre fills at convex interior vertices.
            for index in 1..segments.len() {
                let (_, vertex, dir_in) = segments[index - 1];
                let (vertex_b, _, dir_out) = segments[index];
                debug_assert_eq!(vertex, vertex_b);
                let cross = dir_in[0] * dir_out[1] - dir_in[1] * dir_out[0];
                if cross.abs() <= 1e-12 {
                    continue; // straight or reversed; rectangles already cover
                }
                // The convex (outer) side flips with the turn direction: for a
                // left turn (cross > 0) it is the right side of travel.
                let sign = if cross > 0.0 { -1.0 } else { 1.0 };
                let normal_in = [sign * -dir_in[1], sign * dir_in[0]];
                let normal_out = [sign * -dir_out[1], sign * dir_out[0]];
                let a = [
                    vertex[0] + normal_in[0] * half_width,
                    vertex[1] + normal_in[1] * half_width,
                ];
                let b = [
                    vertex[0] + normal_out[0] * half_width,
                    vertex[1] + normal_out[1] * half_width,
                ];
                // Intersection of the two offset lines through a and b.
                let mitre = line_intersection(a, dir_in, b, dir_out);
                let Some(mitre) = mitre else {
                    continue;
                };
                let mitre_length =
                    ((mitre[0] - vertex[0]).powi(2) + (mitre[1] - vertex[1]).powi(2)).sqrt();
                let mut fills = vec![
                    vec![vertex, a, mitre, vertex],
                    vec![vertex, mitre, b, vertex],
                ];
                if mitre_length > 5.0 * half_width {
                    // Mitre limit exceeded: bevel back to the chord, as GEOS
                    // does with the default limit of 5.
                    fills = vec![vec![vertex, a, b, vertex]];
                }
                for exterior in fills {
                    polygons.push(MaskPolygon {
                        bbox: ring_bbox(&exterior),
                        exterior,
                        holes: Vec::new(),
                    });
                }
            }
        }
    }
    PolygonMask::build(polygons)
}

/// Intersection of `p + t*d` and `q + s*e`.
fn line_intersection(p: Point2, d: Point2, q: Point2, e: Point2) -> Option<Point2> {
    let denominator = d[0] * e[1] - d[1] * e[0];
    if denominator.abs() <= 1e-15 {
        return None;
    }
    let diff = [q[0] - p[0], q[1] - p[1]];
    let t = (diff[0] * e[1] - diff[1] * e[0]) / denominator;
    Some([p[0] + t * d[0], p[1] + t * d[1]])
}

/// Mask covering round-ish buffers (24-gons, `quad_segs=6`) around tree points.
fn build_tree_mask(collection: &ReferenceCollection, radius_ft: f64) -> PolygonMask {
    let mut polygons = Vec::new();
    for feature in &collection.features {
        let ReferenceGeometry::Point(center) = &feature.geometry else {
            continue;
        };
        let mut exterior = Vec::with_capacity(25);
        for step in 0..24 {
            let angle = step as f64 * std::f64::consts::PI / 12.0;
            exterior.push([
                center[0] + radius_ft * angle.cos(),
                center[1] + radius_ft * angle.sin(),
            ]);
        }
        exterior.push(exterior[0]);
        let bbox = [
            center[0] - radius_ft,
            center[1] - radius_ft,
            center[0] + radius_ft,
            center[1] + radius_ft,
        ];
        polygons.push(MaskPolygon {
            exterior,
            holes: Vec::new(),
            bbox,
        });
    }
    PolygonMask::build(polygons)
}

// ---------------------------------------------------------------------------
// Extra Bytes and provenance VLRs
// ---------------------------------------------------------------------------

/// One parsed Extra Bytes descriptor entry (192 bytes in the VLR payload).
#[derive(Clone, Debug)]
struct ExtraByteDescriptor {
    name: String,
    data_type: u8,
    byte_size: usize,
}

fn parse_extra_byte_vlrs(header: &RawHeader) -> Vec<ExtraByteDescriptor> {
    let mut descriptors = Vec::new();
    for vlr in &header.vlrs {
        if vlr.user_id != "LASF_Spec" || vlr.record_id != 4 {
            continue;
        }
        for chunk in vlr.data.chunks(192) {
            if chunk.len() < 192 {
                continue;
            }
            descriptors.push(ExtraByteDescriptor {
                name: trim_nuls(&chunk[4..36]),
                data_type: chunk[2],
                byte_size: extra_byte_size(chunk[2]),
            });
        }
    }
    descriptors
}

fn extra_byte_size(data_type: u8) -> usize {
    match data_type {
        1 => 1,  // uint8
        2 => 1,  // int8
        3 => 2,  // uint16
        4 => 2,  // int16
        5 => 4,  // uint32
        6 => 4,  // int32
        7 => 8,  // uint64
        8 => 8,  // int64
        9 => 4,  // f32
        10 => 8, // f64
        _ => 0,  // 0 = undefined; locate via the record length instead
    }
}

/// Where the uint8 `label` dimension lives in each point record.
#[derive(Clone, Copy, Debug)]
pub struct LabelDimension {
    pub offset: usize,
}

fn find_label_dimension(header: &RawHeader) -> Option<LabelDimension> {
    let fixed = header.fixed_record_length()?;
    let mut offset = fixed;
    for descriptor in parse_extra_byte_vlrs(header) {
        if descriptor.name == "label" && descriptor.data_type == 1 {
            return Some(LabelDimension { offset });
        }
        offset += descriptor.byte_size;
    }
    // Undefined-type descriptors cannot be located reliably; require typed
    // extra bytes so the offset arithmetic is always exact.
    None
}

fn label_extra_bytes_vlr() -> RawVlr {
    let mut data = vec![0u8; 192];
    data[2] = 1; // data_type: uint8
    data[4..9].copy_from_slice(b"label");
    let description = b"UPCP urban class label";
    data[36..36 + description.len()].copy_from_slice(description);
    RawVlr {
        user_id: "LASF_Spec".to_string(),
        record_id: 4,
        description: "Extra bytes record".to_string(),
        data,
    }
}

/// Public information about an urban-classified file, for the viewer layer.
#[derive(Clone, Debug)]
pub struct UrbanLabelInfo {
    pub label: LabelDimension,
    pub record_length: usize,
    pub point_format: u8,
    pub provenance: Option<String>,
}

/// Inspect a LAS/LAZ file for a readable UPCP `label` dimension. Returns
/// `None` when the file has no typed uint8 `label` extra byte.
pub fn inspect_urban_label(path: &Path) -> Result<Option<UrbanLabelInfo>, Error> {
    let header = read_header_only(path)?;
    let Some(label) = find_label_dimension(&header) else {
        return Ok(None);
    };
    let provenance = header
        .vlrs
        .iter()
        .find(|vlr| vlr.user_id == "OpenCADStudio" && vlr.record_id == 1001)
        .map(|vlr| String::from_utf8_lossy(&vlr.data).to_string());
    Ok(Some(UrbanLabelInfo {
        label,
        record_length: header.record_length as usize,
        point_format: header.point_format,
        provenance,
    }))
}

/// Fill `points[i].label` from the file's `label` extra byte in one sequential
/// full-density pass. Returns `false` when the file carries no label
/// dimension, leaving the points untouched.
pub fn attach_sample_labels(path: &Path, points: &mut [SamplePoint]) -> Result<bool, Error> {
    let Some(info) = inspect_urban_label(path)? else {
        return Ok(false);
    };
    let wanted: std::collections::BTreeMap<u64, ()> = points
        .iter()
        .map(|point| (point.source_index, ()))
        .collect();
    let mut labels: std::collections::BTreeMap<u64, u8> = std::collections::BTreeMap::new();
    let mut reader = RawPointReader::open(path)?;
    let record_length = reader.header.record_length as usize;
    let label_offset = info.label.offset.min(record_length.saturating_sub(1));
    let mut buf = Vec::new();
    let mut index: u64 = 0;
    loop {
        let count = reader.read_chunk(&mut buf, PROGRESS_CHUNK_POINTS)?;
        if count == 0 {
            break;
        }
        for offset in 0..count {
            let source_index = index + offset as u64;
            if wanted.contains_key(&source_index) {
                let start = offset * record_length;
                labels.insert(source_index, buf[start + label_offset]);
            }
        }
        index += count as u64;
    }
    for point in points.iter_mut() {
        point.label = labels.get(&point.source_index).copied();
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Time helper
// ---------------------------------------------------------------------------

pub(crate) fn unix_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// Format a Unix timestamp (milliseconds) as `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn iso_utc(unix_ms: u128) -> String {
    let seconds = (unix_ms / 1000) as i64;
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

// ---------------------------------------------------------------------------
// Classification core
// ---------------------------------------------------------------------------

struct UrbanMasks {
    buildings: PolygonMask,
    roads: PolygonMask,
    trees: PolygonMask,
    building_features: usize,
    road_features: usize,
    tree_features: usize,
}

fn load_masks(
    provider: &mut dyn UrbanReferenceProvider,
    tile_stem: &str,
    bounds: [f64; 4],
    references_dir: &Path,
    settings: &UrbanClassificationSettings,
) -> Result<UrbanMasks, Error> {
    let load = |provider: &mut dyn UrbanReferenceProvider,
                layer: UrbanLayer,
                enabled: bool|
     -> Result<ReferenceCollection, Error> {
        if !enabled {
            return Ok(ReferenceCollection::default());
        }
        provider.load(
            layer,
            tile_stem,
            bounds,
            references_dir,
            settings.reference_cache,
        )
    };
    let buildings = load(provider, UrbanLayer::Buildings, settings.building_fuser)?;
    let roads = load(provider, UrbanLayer::Roads, settings.road_fuser)?;
    let trees = load(provider, UrbanLayer::Trees, settings.vegetation_fuser)?;
    Ok(UrbanMasks {
        building_features: buildings.features.len(),
        road_features: roads.features.len(),
        tree_features: trees.features.len(),
        buildings: build_building_mask(&buildings),
        roads: build_road_mask(&roads, settings.road_edge_allowance_ft),
        trees: build_tree_mask(&trees, settings.tree_radius_ft),
    })
}

fn seed_label(classification: u8) -> u8 {
    ASPRS_SEEDS
        .iter()
        .find(|(class, _)| *class == classification)
        .map(|(_, label)| *label)
        .unwrap_or(0)
}

fn histogram_json(counts: &[u64; 256]) -> Map<String, Value> {
    let mut map = Map::new();
    for (value, count) in counts.iter().enumerate() {
        if *count > 0 {
            map.insert(value.to_string(), Value::from(*count));
        }
    }
    map
}

fn upcp_labels_json() -> Value {
    serde_json::Map::from_iter(
        UPCP_LABELS
            .iter()
            .map(|(code, name)| (code.to_string(), Value::from(*name))),
    )
    .into()
}

fn asprs_seeds_json() -> Value {
    serde_json::Map::from_iter(
        ASPRS_SEEDS
            .iter()
            .map(|(class, label)| (class.to_string(), Value::from(*label))),
    )
    .into()
}

fn provenance_json(settings: &UrbanClassificationSettings) -> Value {
    json!({
        "schema": PROVENANCE_SCHEMA,
        "engine": "ocs_pointcloud-native",
        "upstream": UPSTREAM_REPOSITORY,
        "source_classification_preserved": true,
        "source_classification_dimension": Value::Null,
        "asprs_display_mapping": Value::Null,
        "label_dimension": "label",
        "labels": upcp_labels_json(),
        "asprs_seeds": asprs_seeds_json(),
        "target_wkid": Value::Null,
        "road_extra_ft": settings.road_edge_allowance_ft,
        "tree_radius_ft": settings.tree_radius_ft,
        "created_utc": iso_utc(unix_ms_now()),
    })
}

fn tile_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Classify one tile: stream every source record, fuse labels, write a
/// validated `.partial` LAZ, then atomically publish it.
pub fn classify_urban_tile(
    source: &Path,
    output_dir: &Path,
    settings: &UrbanClassificationSettings,
    provider: &mut dyn UrbanReferenceProvider,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(UrbanJobProgress),
) -> Result<UrbanTileStats, Error> {
    let started = Instant::now();
    let tile_stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| Error::Urban("source filename is not valid Unicode".to_string()))?
        .to_string();
    let output = UrbanTileStats::output_for(source, output_dir);
    if output.exists() && !settings.overwrite_outputs {
        return Err(Error::OutputExists(output));
    }
    let mut reader = RawPointReader::open(source)?;
    let header = reader.header.clone();
    if find_label_dimension(&header).is_some() {
        return Err(Error::Urban(format!(
            "source already contains a label dimension: {}",
            source.display()
        )));
    }
    // A typed label byte can only be appended safely when every existing
    // extra byte is declared, so its offset is unambiguous.
    let declared_bytes: usize = parse_extra_byte_vlrs(&header)
        .iter()
        .map(|descriptor| descriptor.byte_size)
        .sum();
    if header.extra_byte_count()? != declared_bytes {
        return Err(Error::Urban(format!(
            "source has undeclared extra bytes ({} bytes present, {} declared); \
             declare them or strip them before urban classification",
            header.extra_byte_count()?,
            declared_bytes
        )));
    }
    header
        .fixed_record_length()
        .ok_or_else(|| Error::Urban(format!("unsupported point format {}", header.point_format)))?;
    let class_offset = header.classification_offset();
    let record_length = header.record_length as usize;
    let out_record_length = record_length + 1;
    let bounds = [
        header.bounds[0].min(header.bounds[3]),
        header.bounds[1].min(header.bounds[4]),
        header.bounds[3].max(header.bounds[0]),
        header.bounds[4].max(header.bounds[1]),
    ];
    let references_dir = output_dir.join("references");

    let emit = |stage: UrbanStage,
                processed: u64,
                masks: Option<&UrbanMasks>,
                progress: &mut dyn FnMut(UrbanJobProgress)| {
        progress(UrbanJobProgress {
            tile_index: 1,
            tile_total: 1,
            tile_name: tile_name_of(source),
            stage,
            points_processed: processed,
            points_total: header.point_count,
            building_features: masks.map(|masks| masks.building_features).unwrap_or(0),
            road_features: masks.map(|masks| masks.road_features).unwrap_or(0),
            tree_features: masks.map(|masks| masks.tree_features).unwrap_or(0),
            output_path: output.clone(),
            elapsed_ms: started.elapsed().as_millis(),
        });
    };

    emit(UrbanStage::LoadingReferences, 0, None, progress);
    let masks = load_masks(provider, &tile_stem, bounds, &references_dir, settings)?;
    emit(UrbanStage::LoadingReferences, 0, Some(&masks), progress);

    let partial = output_dir.join(format!("{tile_stem}_classified.laz.partial"));
    if partial.exists() {
        fs::remove_file(&partial).map_err(Error::Io)?;
    }
    let mut out_vlrs: Vec<RawVlr> = header
        .vlrs
        .iter()
        .filter(|vlr| {
            !(vlr.user_id == "laszip encoded" && vlr.record_id == LASZIP_RECORD_ID)
        })
        .cloned()
        .collect();
    out_vlrs.push(label_extra_bytes_vlr());
    out_vlrs.push(RawVlr {
        user_id: "OpenCADStudio".to_string(),
        record_id: 1001,
        description: "UPCP Boston classifier".to_string(),
        data: serde_json::to_vec(&provenance_json(settings))
            .map_err(|error| Error::Urban(format!("cannot serialize provenance: {error}")))?,
    });

    let result = write_classified_laz(
        &partial,
        &header,
        &out_vlrs,
        out_record_length as u16,
        &mut reader,
        &masks,
        settings,
        class_offset,
        cancel,
        &mut |processed| emit(UrbanStage::Classifying, processed, Some(&masks), progress),
    );
    let (original_counts, label_counts, processed) = match result {
        Ok(counts) => counts,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    };

    emit(UrbanStage::Validating, processed, Some(&masks), progress);
    if let Err(error) =
        validate_classified_output(&partial, source, &original_counts, &label_counts)
    {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if settings.overwrite_outputs && output.exists() {
        fs::remove_file(&output).map_err(Error::Io)?;
    }
    fs::rename(&partial, &output).map_err(Error::Io)?;
    emit(UrbanStage::Completed, processed, Some(&masks), progress);

    Ok(UrbanTileStats {
        status: "completed".to_string(),
        source: source.to_path_buf(),
        output,
        error: None,
        point_count: Some(processed),
        point_format: Some(header.point_format),
        las_version: Some(header.las_version()),
        bounds: Some(bounds),
        original_classification_counts: Some(histogram_json(&original_counts)),
        upcp_label_counts: Some(histogram_json(&label_counts)),
        building_feature_count: Some(masks.building_features),
        road_feature_count: Some(masks.road_features),
        tree_feature_count: Some(masks.tree_features),
        elapsed_seconds: Some(started.elapsed().as_secs_f64()),
        completed_utc: Some(iso_utc(unix_ms_now())),
    })
}

/// Stream source records through the fusers into a fresh LAZ container.
#[allow(clippy::too_many_arguments)]
fn write_classified_laz(
    dest: &Path,
    header: &RawHeader,
    out_vlrs: &[RawVlr],
    out_record_length: u16,
    reader: &mut RawPointReader,
    masks: &UrbanMasks,
    settings: &UrbanClassificationSettings,
    class_offset: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<([u64; 256], [u64; 256], u64), Error> {
    let file = File::create(dest).map_err(Error::Io)?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);

    // Output LazVlr covers the source items plus the appended label byte.
    let extra_bytes = header.extra_byte_count()? + 1;
    let laz_vlr = laz::LazVlrBuilder::default()
        .with_point_format(header.point_format, extra_bytes as u16)
        .map_err(|error| Error::Urban(format!("cannot build LasZip VLR: {error}")))?
        .build();
    let mut laz_payload = Vec::new();
    laz_vlr.write_to(&mut laz_payload).map_err(Error::Io)?;
    let laszip_vlr = RawVlr {
        user_id: "laszip encoded".to_string(),
        record_id: LASZIP_RECORD_ID,
        description: "laszip encoded".to_string(),
        data: laz_payload,
    };
    let v1_4 = header.version.1 >= 4;
    let mut all_vlrs: Vec<&RawVlr> = out_vlrs.iter().collect();
    all_vlrs.push(&laszip_vlr);
    let vlr_bytes: usize = all_vlrs.iter().map(|vlr| 54 + vlr.data.len()).sum();
    let offset_to_point_data = header.raw.len() as u64 + vlr_bytes as u64;

    // Patch the copied header: record length, VLR count, point-data offset,
    // and (for 1.4) a zeroed EVLR section since we write none.
    let mut header_bytes = header.raw.clone();
    let patch_u16 = |bytes: &mut [u8], offset: usize, value: u16| {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
    };
    let patch_u32 = |bytes: &mut [u8], offset: usize, value: u32| {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
    };
    let patch_u64 = |bytes: &mut [u8], offset: usize, value: u64| {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes())
    };
    header_bytes[104] = header.point_format | 0x80;
    patch_u16(&mut header_bytes, 105, out_record_length);
    patch_u32(&mut header_bytes, 100, all_vlrs.len() as u32);
    patch_u32(&mut header_bytes, 96, offset_to_point_data as u32);
    if v1_4 {
        patch_u64(&mut header_bytes, 235, 0);
        patch_u32(&mut header_bytes, 243, 0);
    }
    writer.write_all(&header_bytes).map_err(Error::Io)?;
    for vlr in &all_vlrs {
        writer.write_all(&vlr.to_bytes()).map_err(Error::Io)?;
    }
    debug_assert_eq!(
        writer.stream_position().map_err(Error::Io)?,
        offset_to_point_data
    );

    let mut compressor = laz::LasZipCompressor::new(&mut writer, laz_vlr)
        .map_err(|error| Error::Urban(format!("cannot start LAZ compression: {error}")))?;

    let mut original_counts = [0u64; 256];
    let mut label_counts = [0u64; 256];
    let mut processed: u64 = 0;
    let record_length = header.record_length as usize;
    let mut in_buf = Vec::new();
    let mut out_buf = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            // Finish the container so the file is a well-formed (if partial)
            // stream, then surface cancellation; the caller deletes it.
            let _ = compressor.done();
            return Err(Error::Cancelled("urban classification"));
        }
        let count = reader.read_chunk(&mut in_buf, PROGRESS_CHUNK_POINTS)?;
        if count == 0 {
            break;
        }
        out_buf.clear();
        out_buf.resize(count * out_record_length as usize, 0);
        for index in 0..count {
            let source_start = index * record_length;
            let record = &in_buf[source_start..source_start + record_length];
            let classification = record[class_offset];
            let x = i32::from_le_bytes([record[0], record[1], record[2], record[3]]) as f64
                * header.scales[0]
                + header.offsets[0];
            let y = i32::from_le_bytes([record[4], record[5], record[6], record[7]]) as f64
                * header.scales[1]
                + header.offsets[1];
            let mut label = if settings.seed_source_classes {
                seed_label(classification)
            } else {
                0
            };
            if settings.building_fuser && classification == 1 && masks.buildings.contains(x, y) {
                label = 10;
            }
            if settings.road_fuser && classification == 2 && masks.roads.contains(x, y) {
                label = 1;
            }
            if settings.vegetation_fuser
                && classification == 1
                && label == 0
                && masks.trees.contains(x, y)
            {
                label = 30;
            }
            let out_start = index * out_record_length as usize;
            out_buf[out_start..out_start + record_length].copy_from_slice(record);
            out_buf[out_start + record_length] = label;
            original_counts[classification as usize] += 1;
            label_counts[label as usize] += 1;
        }
        compressor
            .compress_many(&mut out_buf)
            .map_err(|error| Error::Urban(format!("LAZ compression failed: {error}")))?;
        processed += count as u64;
        progress(processed);
    }
    compressor
        .done()
        .map_err(|error| Error::Urban(format!("cannot finish LAZ: {error}")))?;
    Ok((original_counts, label_counts, processed))
}

/// Re-open the completed output and verify every invariant before publishing:
/// header agreement, label/provenance presence, and a full lockstep byte
/// comparison of every point record against the source.
fn validate_classified_output(
    path: &Path,
    source: &Path,
    original_counts: &[u64; 256],
    label_counts: &[u64; 256],
) -> Result<(), Error> {
    let mut output_reader = RawPointReader::open(path)?;
    let output_header = output_reader.header.clone();
    let mut source_reader = RawPointReader::open(source)?;
    let source_header = source_reader.header.clone();
    if output_header.point_count != source_header.point_count {
        return Err(Error::Urban(format!(
            "point-count mismatch: {} != {}",
            output_header.point_count, source_header.point_count
        )));
    }
    if output_header.point_format != source_header.point_format {
        return Err(Error::Urban("output point format changed".to_string()));
    }
    if output_header.scales != source_header.scales
        || output_header.offsets != source_header.offsets
    {
        return Err(Error::Urban(
            "output coordinate scales or offsets changed".to_string(),
        ));
    }
    if output_header.wkt() != source_header.wkt() {
        return Err(Error::Urban("output CRS changed".to_string()));
    }
    let label = find_label_dimension(&output_header)
        .ok_or_else(|| Error::Urban("output label dimension is missing".to_string()))?;
    if output_header
        .vlrs
        .iter()
        .find(|vlr| vlr.user_id == "OpenCADStudio" && vlr.record_id == 1001)
        .is_none()
    {
        return Err(Error::Urban("output provenance VLR is missing".to_string()));
    }
    let mut seen_original = [0u64; 256];
    let mut seen_labels = [0u64; 256];
    let class_offset = output_header.classification_offset();
    let source_record_length = source_header.record_length as usize;
    let mut source_buf = Vec::new();
    let mut out_buf = Vec::new();
    loop {
        let source_count = source_reader.read_chunk(&mut source_buf, PROGRESS_CHUNK_POINTS)?;
        let out_count = output_reader.read_chunk(&mut out_buf, PROGRESS_CHUNK_POINTS)?;
        if source_count != out_count {
            return Err(Error::Urban("output stream length diverged".to_string()));
        }
        if source_count == 0 {
            break;
        }
        for index in 0..source_count {
            let source_start = index * source_record_length;
            let out_start = index * (source_record_length + 1);
            if source_buf[source_start..source_start + source_record_length]
                != out_buf[out_start..out_start + source_record_length]
            {
                return Err(Error::Urban(
                    "output point record bytes diverged from the source".to_string(),
                ));
            }
            let classification = out_buf[out_start + class_offset];
            let label_value = out_buf[out_start + label.offset];
            seen_original[classification as usize] += 1;
            seen_labels[label_value as usize] += 1;
        }
    }
    if seen_original != *original_counts {
        return Err(Error::Urban(
            "source classification histogram changed".to_string(),
        ));
    }
    if seen_labels != *label_counts {
        return Err(Error::Urban("output label histogram diverged".to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Batch orchestration
// ---------------------------------------------------------------------------

/// Methodology block shared by the manifest, matching the oracle's schema.
fn methodology_json(settings: &UrbanClassificationSettings) -> Value {
    json!({
        "upstream_repository": UPSTREAM_REPOSITORY,
        "engine": "ocs_pointcloud-native",
        "source_classification_preserved": true,
        "source_classification_dimension": Value::Null,
        "asprs_display_mapping": Value::Null,
        "label_dimension": "label",
        "labels": upcp_labels_json(),
        "asprs_seeds": asprs_seeds_json(),
        "building_rule": "ASPRS class 1 inside reference building footprints -> UPCP 10",
        "road_rule": "ASPRS class 2 inside width-buffered road centerlines -> UPCP 1",
        "vegetation_rule": "remaining ASPRS class 1 inside street-tree buffers -> UPCP 30",
        "water_and_rail": "retained in ASPRS classification; UPCP label remains 0",
        "road_extra_ft": settings.road_edge_allowance_ft,
        "tree_radius_ft": settings.tree_radius_ft,
        "building_fuser": settings.building_fuser,
        "road_fuser": settings.road_fuser,
        "vegetation_fuser": settings.vegetation_fuser,
        "reference_data_warning":
            "Reference layers are current and may differ from the LiDAR epoch.",
    })
}

fn write_manifest_atomic(path: &Path, manifest: &UrbanBatchManifest) -> Result<(), Error> {
    let temporary = path.with_extension("json.partial");
    let text = serde_json::to_string_pretty(manifest)
        .map_err(|error| Error::Urban(format!("cannot serialize manifest: {error}")))?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)
        .map_err(Error::Io)?;
    writeln!(file, "{text}").map_err(Error::Io)?;
    file.flush().map_err(Error::Io)?;
    drop(file);
    fs::rename(&temporary, path).map_err(Error::Io)?;
    Ok(())
}

fn list_las_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut sources: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(Error::Io)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_ascii_lowercase())
                    .as_deref(),
                Some("las") | Some("laz")
            )
        })
        .collect();
    sources.sort();
    Ok(sources)
}

/// Classify every LAS/LAZ file in `input_dir` into `output_dir`, resuming
/// past completed tiles and recording per-tile results in the manifest.
pub fn classify_urban_folder(
    input_dir: &Path,
    output_dir: &Path,
    settings: &UrbanClassificationSettings,
    provider: &mut dyn UrbanReferenceProvider,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(UrbanJobProgress),
) -> Result<UrbanBatchSummary, Error> {
    if input_dir == output_dir {
        return Err(Error::Urban(
            "output directory must be separate from the source directory".to_string(),
        ));
    }
    fs::create_dir_all(output_dir).map_err(Error::Io)?;
    let sources = list_las_files(input_dir)?;
    if sources.is_empty() {
        return Err(Error::Urban(format!(
            "no matching LAS/LAZ files in {}",
            input_dir.display()
        )));
    }

    let manifest_path = output_dir.join("classification_manifest.json");
    let mut manifest = if manifest_path.is_file() {
        let text = fs::read_to_string(&manifest_path).map_err(Error::Io)?;
        serde_json::from_str(&text)
            .map_err(|error| Error::Urban(format!("invalid classification manifest: {error}")))?
    } else {
        UrbanBatchManifest {
            schema: MANIFEST_SCHEMA.to_string(),
            status: "running".to_string(),
            started_utc: iso_utc(unix_ms_now()),
            input_dir: input_dir.to_path_buf(),
            output_dir: output_dir.to_path_buf(),
            methodology: methodology_json(settings),
            tiles: Vec::new(),
            completed_utc: None,
            failure_count: None,
        }
    };
    let mut summary = UrbanBatchSummary {
        manifest_path: manifest_path.clone(),
        tile_total: sources.len(),
        completed: 0,
        failed: 0,
        skipped: 0,
        cancelled: false,
        outputs: manifest
            .tiles
            .iter()
            .filter(|tile| tile.status == "completed" && tile.output.is_file())
            .map(|tile| tile.output.clone())
            .collect(),
    };

    let mut cancelled = false;
    for (index, source) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let position = manifest
            .tiles
            .iter()
            .position(|tile| tile.source == *source);
        let already_done = position
            .and_then(|position| manifest.tiles.get(position))
            .map(|tile| tile.status == "completed" && tile.output.is_file())
            .unwrap_or(false);
        if already_done && !settings.overwrite_outputs {
            summary.skipped += 1;
            continue;
        }
        let output = UrbanTileStats::output_for(source, output_dir);
        let mut indexed_progress = |tick: UrbanJobProgress| {
            progress(UrbanJobProgress {
                tile_index: index + 1,
                tile_total: sources.len(),
                ..tick
            });
        };
        let tile_result = classify_urban_tile(
            source,
            output_dir,
            settings,
            provider,
            cancel,
            &mut indexed_progress,
        );
        let tile = match tile_result {
            Ok(stats) => {
                summary.completed += 1;
                summary.outputs.push(stats.output.clone());
                stats
            }
            Err(Error::Cancelled(operation)) => {
                cancelled = true;
                let tile =
                    UrbanTileStats::failed(source, output, "cancelled", operation.to_string());
                upsert_tile(&mut manifest, position, tile);
                break;
            }
            Err(error) => {
                summary.failed += 1;
                UrbanTileStats::failed(source, output, "failed", error.to_string())
            }
        };
        upsert_tile(&mut manifest, position, tile);
        write_manifest_atomic(&manifest_path, &manifest)?;
    }
    summary.cancelled = cancelled;
    manifest.status = if cancelled {
        "cancelled".to_string()
    } else if summary.failed > 0 {
        "completed_with_failures".to_string()
    } else {
        "completed".to_string()
    };
    manifest.completed_utc = Some(iso_utc(unix_ms_now()));
    manifest.failure_count = Some(summary.failed);
    write_manifest_atomic(&manifest_path, &manifest)?;
    Ok(summary)
}

fn upsert_tile(manifest: &mut UrbanBatchManifest, position: Option<usize>, tile: UrbanTileStats) {
    match position {
        Some(position) => manifest.tiles[position] = tile,
        None => manifest.tiles.push(tile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_las(path: &Path, point_format: u8, records: &[Vec<u8>], vlrs: &[RawVlr]) {
        let fixed = match point_format {
            0 => 20,
            1 => 28,
            _ => panic!("extend fixture formats as needed"),
        };
        let record_length = records.first().map(|r| r.len()).unwrap_or(fixed);
        let mut header = vec![0u8; 375];
        header[0..4].copy_from_slice(b"LASF");
        header[24..26].copy_from_slice(&(375u16).to_le_bytes());
        header[25] = 1;
        header[26] = 4;
        let vlr_bytes: usize = vlrs.iter().map(|v| 54 + v.data.len()).sum();
        header[96..100].copy_from_slice(&((375 + vlr_bytes) as u32).to_le_bytes());
        header[100..104].copy_from_slice(&(vlrs.len() as u32).to_le_bytes());
        header[104] = point_format;
        header[105..107].copy_from_slice(&(record_length as u16).to_le_bytes());
        header[107..111].copy_from_slice(&(records.len() as u32).to_le_bytes());
        let scales = [0.001f64, 0.001, 0.001];
        for (index, scale) in scales.iter().enumerate() {
            header[131 + index * 8..139 + index * 8]
                .copy_from_slice(&scale.to_bits().to_le_bytes());
        }
        let offsets = [0.0f64, 0.0, 0.0];
        for (index, offset) in offsets.iter().enumerate() {
            header[155 + index * 8..163 + index * 8]
                .copy_from_slice(&offset.to_bits().to_le_bytes());
        }
        // Header bounds from the records so providers see a real envelope.
        let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for record in records {
            let x = i32::from_le_bytes([record[0], record[1], record[2], record[3]]) as f64 * 0.001;
            let y = i32::from_le_bytes([record[4], record[5], record[6], record[7]]) as f64 * 0.001;
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        }
        for (index, value) in bounds.iter().enumerate() {
            header[179 + index * 8..187 + index * 8]
                .copy_from_slice(&value.to_bits().to_le_bytes());
            header[203 + index * 8..211 + index * 8]
                .copy_from_slice(&value.to_bits().to_le_bytes());
        }
        header[247..255].copy_from_slice(&(records.len() as u64).to_le_bytes());
        let mut bytes = header;
        for vlr in vlrs {
            bytes.extend_from_slice(&vlr.to_bytes());
        }
        for record in records {
            bytes.extend_from_slice(record);
        }
        fs::write(path, bytes).unwrap();
    }

    /// Formats 0-5 record: classification sits at offset 15.
    fn legacy_record(x: i32, y: i32, z: i32, classification: u8, extra: &[u8]) -> Vec<u8> {
        let mut record = Vec::with_capacity(20 + extra.len());
        record.extend_from_slice(&x.to_le_bytes());
        record.extend_from_slice(&y.to_le_bytes());
        record.extend_from_slice(&z.to_le_bytes());
        record.extend_from_slice(&[0u8; 2]); // intensity
        record.push(0); // return/scan flags
        record.push(classification);
        record.push(0); // scan angle rank
        record.push(0); // user data
        record.extend_from_slice(&[0u8; 2]); // point source id
        debug_assert_eq!(record.len(), 20);
        record.extend_from_slice(extra);
        record
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocs-urban-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct EmptyProvider;

    impl UrbanReferenceProvider for EmptyProvider {
        fn load(
            &mut self,
            _layer: UrbanLayer,
            _tile_stem: &str,
            _bounds: [f64; 4],
            _references_dir: &Path,
            _use_cache: bool,
        ) -> Result<ReferenceCollection, Error> {
            Ok(ReferenceCollection::default())
        }
    }

    #[test]
    fn extra_bytes_and_dimensions_round_trip_unchanged() {
        let dir = temp_dir("roundtrip");
        let source = dir.join("tile.las");
        // Two format-1 points (28 bytes) plus a *declared* user extra byte,
        // carrying distinctive payload bytes that must survive the rewrite.
        let user_extra = {
            let mut data = vec![0u8; 192];
            data[2] = 2; // data_type: int8
            data[4..8].copy_from_slice(b"user");
            RawVlr {
                user_id: "LASF_Spec".to_string(),
                record_id: 4,
                description: "Extra bytes record".to_string(),
                data,
            }
        };
        let mut first = legacy_record(1000, 2000, 3000, 3, &[]);
        first.extend_from_slice(&1.5f64.to_bits().to_le_bytes());
        first.push(0xAB);
        let mut second = legacy_record(-4000, 5000, -6000, 18, &[]);
        second.extend_from_slice(&(-2.25f64).to_bits().to_le_bytes());
        second.push(0xCD);
        write_test_las(&source, 1, &[first, second], &[user_extra]);

        let output_dir = dir.join("classified");
        fs::create_dir_all(&output_dir).unwrap();
        let settings = UrbanClassificationSettings::default();
        let cancel = AtomicBool::new(false);
        let mut progress = |_tick: UrbanJobProgress| {};
        let stats = classify_urban_tile(
            &source,
            &output_dir,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        )
        .unwrap();
        assert_eq!(stats.status, "completed");
        assert_eq!(stats.point_count, Some(2));

        let output = output_dir.join("tile_classified.laz");
        assert!(output.is_file());
        let info = inspect_urban_label(&output)
            .unwrap()
            .expect("label present after classification");
        assert_eq!(info.label.offset, 29); // after the 29-byte records
                                           // The validation pass compared every record byte in lockstep, so a
                                           // successful return is the fidelity proof; check the histograms too.
        let labels = stats.upcp_label_counts.unwrap();
        assert_eq!(labels.get("99").and_then(Value::as_u64), Some(1)); // class 18 seeded
        assert_eq!(labels.get("0").and_then(Value::as_u64), Some(1)); // class 3 stays unknown
    }

    #[test]
    fn refuses_existing_output_and_labelled_sources() {
        let dir = temp_dir("refusals");
        let source = dir.join("tile.las");
        write_test_las(&source, 0, &[legacy_record(1, 1, 1, 2, &[])], &[]);
        let output_dir = dir.join("classified");
        fs::create_dir_all(&output_dir).unwrap();
        let cancel = AtomicBool::new(false);
        let mut progress = |_tick: UrbanJobProgress| {};
        let settings = UrbanClassificationSettings::default();
        let stats = classify_urban_tile(
            &source,
            &output_dir,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        )
        .unwrap();
        assert_eq!(stats.status, "completed");
        // Second run must refuse without overwrite.
        let again = classify_urban_tile(
            &source,
            &output_dir,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        );
        assert!(matches!(again, Err(Error::OutputExists(_))));
        // Overwrite succeeds.
        let settings = UrbanClassificationSettings {
            overwrite_outputs: true,
            ..Default::default()
        };
        assert!(classify_urban_tile(
            &source,
            &output_dir,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress
        )
        .is_ok());
        // Classified outputs may not be re-classified (label dimension exists).
        let labelled = output_dir.join("tile_classified.laz");
        let settings = UrbanClassificationSettings::default();
        let reclassified = classify_urban_tile(
            &labelled,
            &output_dir,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        );
        assert!(reclassified.is_err());
    }

    #[test]
    fn cancellation_leaves_no_partial_output() {
        let dir = temp_dir("cancel");
        let source = dir.join("tile.las");
        let records: Vec<Vec<u8>> = (0..2000)
            .map(|index| legacy_record(index, index, 1, 2, &[]))
            .collect();
        write_test_las(&source, 0, &records, &[]);
        let output_dir = dir.join("out");
        fs::create_dir_all(&output_dir).unwrap();
        let cancel = AtomicBool::new(true); // cancel before the first chunk
        let mut progress = |_tick: UrbanJobProgress| {};
        let result = classify_urban_tile(
            &source,
            &output_dir,
            &UrbanClassificationSettings::default(),
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        );
        assert!(matches!(result, Err(Error::Cancelled(_))));
        assert!(!output_dir.join("tile_classified.laz").exists());
        assert!(!output_dir.join("tile_classified.laz.partial").exists());
    }

    #[test]
    fn fusers_match_reference_geometry_including_boundaries() {
        let dir = temp_dir("fusers");
        let source = dir.join("tile.las");
        // Building footprint [10,10]-[30,40]; road along y=50 (SURFACE_WD 40
        // -> half 20 + 1); tree at (60,10) with the default 12 ft radius.
        let buildings = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[10,10],[30,10],[30,40],[10,40],[10,10]]]}}]}"#;
        let roads = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"SURFACE_WD":40},"geometry":{"type":"LineString","coordinates":[[0,50],[100,50]]}}]}"#;
        let trees = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[60,10]}}]}"#;
        let references = dir.join("references");
        fs::create_dir_all(&references).unwrap();
        fs::write(references.join("tile.buildings.geojson"), buildings).unwrap();
        fs::write(references.join("tile.roads.geojson"), roads).unwrap();
        fs::write(references.join("tile.trees.geojson"), trees).unwrap();

        let mut provider = LocalVectorProvider::new(&references);
        let records = vec![
            legacy_record(20_000, 20_000, 5, 1, &[]), // inside building -> 10
            legacy_record(10_000, 10_000, 5, 1, &[]), // on building corner -> 10 (boundary)
            legacy_record(50_000, 50_000, 5, 2, &[]), // on road centerline -> 1
            legacy_record(50_000, 71_000, 5, 2, &[]), // road edge y=71 boundary -> 1
            legacy_record(50_000, 72_000, 5, 2, &[]), // beyond road edge -> 9 (ground seed)
            legacy_record(60_000, 10_000, 5, 1, &[]), // tree center -> 30
            legacy_record(20_000, 20_000, 5, 3, &[]), // class 3: no fuser -> 0
            legacy_record(1_000, 1_000, 5, 18, &[]),  // noise seed -> 99
        ];
        write_test_las(&source, 0, &records, &[]);
        let output_dir = dir.join("out");
        fs::create_dir_all(&output_dir).unwrap();
        let settings = UrbanClassificationSettings {
            profile: UrbanProfile::LocalDirectory {
                path: references.clone(),
            },
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let mut progress = |_tick: UrbanJobProgress| {};
        let stats = classify_urban_tile(
            &source,
            &output_dir,
            &settings,
            &mut provider,
            &cancel,
            &mut progress,
        )
        .unwrap();
        let labels = stats.upcp_label_counts.unwrap();
        assert_eq!(
            labels.get("10").and_then(Value::as_u64),
            Some(2),
            "building fuser"
        );
        assert_eq!(
            labels.get("1").and_then(Value::as_u64),
            Some(2),
            "road fuser incl. boundary"
        );
        assert_eq!(
            labels.get("9").and_then(Value::as_u64),
            Some(1),
            "ground beyond road edge"
        );
        assert_eq!(
            labels.get("30").and_then(Value::as_u64),
            Some(1),
            "tree fuser"
        );
        assert_eq!(
            labels.get("0").and_then(Value::as_u64),
            Some(1),
            "unclassified class"
        );
        assert_eq!(
            labels.get("99").and_then(Value::as_u64),
            Some(1),
            "noise seed"
        );
    }

    #[test]
    fn road_mask_corners_use_mitre_fills() {
        // An L-shaped road: the mitre fill extends the mask past the inner
        // corner rectangles at the outside of the turn.
        let roads = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"SURFACE_WD":20},
             "geometry":{"type":"LineString","coordinates":[[0,0],[100,0],[100,100]]}}]}"#;
        let collection = parse_geojson_collection(roads).unwrap();
        let mask = build_road_mask(&collection, 1.0); // half width 11
                                                      // Inside the corner rectangles.
        assert!(mask.contains(50.0, 5.0));
        assert!(mask.contains(95.0, 50.0));
        // Beyond the corner on the outside: the mitre fill covers the wedge
        // near (100, 0) out to the intersection of the offset lines.
        assert!(mask.contains(105.0, 4.0));
        assert!(mask.contains(96.0, -10.0));
        // Outside the buffered road entirely.
        assert!(!mask.contains(50.0, 20.0));
        assert!(!mask.contains(120.0, 50.0));
        // Mitre tip sits near (111, -11)/(111, 11) style extremes for a right
        // angle; just beyond it must be outside.
        assert!(!mask.contains(113.0, -10.0));
    }

    #[test]
    fn road_width_fallbacks_and_priority() {
        // Lanes fallback: 2 lanes -> 24 total -> half 12+1=13.
        assert_eq!(road_half_width_ft(&json!({"NUM_LANES": 2}), 1.0), 13.0);
        // Class fallback: class 3 -> 48 total -> half 24+1=25.
        assert_eq!(road_half_width_ft(&json!({"CLASS": 3}), 1.0), 25.0);
        // Unknown fields -> 24 total -> half 13.
        assert_eq!(road_half_width_ft(&json!({}), 1.0), 13.0);
        // Clamped to [6, 80].
        assert_eq!(road_half_width_ft(&json!({"SURFACE_WD": 2.0}), 1.0), 6.0);
        assert_eq!(road_half_width_ft(&json!({"SURFACE_WD": 400.0}), 1.0), 80.0);
        // SURFACE_WD wins over lanes.
        assert_eq!(
            road_half_width_ft(&json!({"SURFACE_WD": 40.0, "NUM_LANES": 8}), 1.0),
            21.0
        );
    }

    #[test]
    fn folder_run_skips_completed_tiles_on_resume() {
        let dir = temp_dir("folder");
        let input = dir.join("input");
        let output = dir.join("out");
        fs::create_dir_all(&input).unwrap();
        write_test_las(
            &input.join("a.las"),
            0,
            &[
                legacy_record(1, 1, 1, 2, &[]),
                legacy_record(2, 2, 2, 18, &[]),
            ],
            &[],
        );
        write_test_las(
            &input.join("b.las"),
            0,
            &[legacy_record(3, 3, 3, 1, &[])],
            &[],
        );
        let cancel = AtomicBool::new(false);
        let mut progress = |_tick: UrbanJobProgress| {};
        let settings = UrbanClassificationSettings::default();
        let summary = classify_urban_folder(
            &input,
            &output,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        )
        .unwrap();
        assert_eq!(summary.completed, 2);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.outputs.len(), 2);

        // Re-run: both tiles are completed on disk, so they are skipped.
        let summary = classify_urban_folder(
            &input,
            &output,
            &settings,
            &mut EmptyProvider,
            &cancel,
            &mut progress,
        )
        .unwrap();
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.outputs.len(), 2);

        // The manifest keeps the oracle-compatible schema and status.
        let text = fs::read_to_string(output.join("classification_manifest.json")).unwrap();
        let manifest: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest["schema"], "OpenCADStudio.UPCP.Boston.batch.v2");
        assert_eq!(manifest["status"], "completed");
        assert_eq!(manifest["tiles"].as_array().unwrap().len(), 2);
    }
}
