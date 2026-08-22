//! Rebuildable, disk-backed tiled level-of-detail cache.
//!
//! Every source point is written once to a leaf tile. Coarser levels retain a
//! deterministic uniform subset, allowing the renderer to choose a bounded
//! point count without loading the LAS/LAZ source or the complete cache.

use crate::{CloudMetadata, SamplePoint, SourceFingerprint};
use las::Reader;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    error, fmt, fs,
    fs::{File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const CACHE_FORMAT_VERSION: u32 = 2;
const RECORD_SIZE: usize = 61;
const MAX_OPEN_TILE_FILES: usize = 64;
/// Per-tile write buffer. A large buffer cuts flush/reopen churn across the
/// hundreds of millions of point-writes a full-density tile cache performs.
const WRITER_BUFFER_BYTES: usize = 1 << 20;

#[derive(Debug)]
pub enum TileCacheError {
    Las(las::Error),
    Io(io::Error),
    Json(serde_json::Error),
    InvalidOptions(&'static str),
    CacheExists(PathBuf),
    Cancelled,
    InvalidCache(String),
}

impl fmt::Display for TileCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Las(error) => write!(f, "LAS/LAZ error: {error}"),
            Self::Io(error) => write!(f, "tile-cache I/O error: {error}"),
            Self::Json(error) => write!(f, "tile-cache manifest error: {error}"),
            Self::InvalidOptions(message) => write!(f, "invalid tile-cache options: {message}"),
            Self::CacheExists(path) => write!(f, "tile cache already exists: {}", path.display()),
            Self::Cancelled => write!(f, "tile-cache build cancelled"),
            Self::InvalidCache(message) => write!(f, "invalid tile cache: {message}"),
        }
    }
}

impl error::Error for TileCacheError {}

impl From<las::Error> for TileCacheError {
    fn from(value: las::Error) -> Self {
        Self::Las(value)
    }
}

impl From<io::Error> for TileCacheError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for TileCacheError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type TileCacheResult<T> = std::result::Result<T, TileCacheError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TileKey {
    pub level: u8,
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileEntry {
    pub key: TileKey,
    pub file_name: String,
    pub point_count: u64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
}

impl TileEntry {
    pub fn intersects(&self, min: [f64; 3], max: [f64; 3]) -> bool {
        (0..3).all(|axis| self.bounds_max[axis] >= min[axis] && self.bounds_min[axis] <= max[axis])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileCacheManifest {
    pub format_version: u32,
    pub source_path: PathBuf,
    pub source_fingerprint: SourceFingerprint,
    pub source_metadata: CloudMetadata,
    pub leaf_level: u8,
    pub target_leaf_points: u64,
    pub record_size: usize,
    pub tiles: Vec<TileEntry>,
}

impl TileCacheManifest {
    pub fn open(cache_dir: impl AsRef<Path>) -> TileCacheResult<Self> {
        let bytes = fs::read(cache_dir.as_ref().join("manifest.json"))?;
        let manifest: Self = serde_json::from_slice(&bytes)?;
        if manifest.format_version != CACHE_FORMAT_VERSION {
            return Err(TileCacheError::InvalidCache(format!(
                "format {} is not supported (expected {})",
                manifest.format_version, CACHE_FORMAT_VERSION
            )));
        }
        if manifest.record_size != RECORD_SIZE {
            return Err(TileCacheError::InvalidCache(format!(
                "record size {} is not supported (expected {})",
                manifest.record_size, RECORD_SIZE
            )));
        }
        Ok(manifest)
    }

    pub fn validate_source(&self, source: impl AsRef<Path>) -> TileCacheResult<()> {
        if self.source_fingerprint.matches_path(source.as_ref()) {
            Ok(())
        } else {
            Err(TileCacheError::InvalidCache(format!(
                "source fingerprint changed for {}",
                source.as_ref().display()
            )))
        }
    }

    /// Chooses the finest available level whose intersecting tiles fit the
    /// requested point budget. A minimum of the root LOD is always returned.
    pub fn select_tiles(
        &self,
        query_min: [f64; 3],
        query_max: [f64; 3],
        point_budget: u64,
    ) -> Vec<TileEntry> {
        let budget = point_budget.max(1);
        for level in (0..=self.leaf_level).rev() {
            let tiles: Vec<_> = self
                .tiles
                .iter()
                .filter(|tile| tile.key.level == level && tile.intersects(query_min, query_max))
                .cloned()
                .collect();
            let count = tiles.iter().map(|tile| tile.point_count).sum::<u64>();
            if count <= budget || level == 0 {
                return tiles;
            }
        }
        Vec::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileCacheOptions {
    pub target_leaf_points: u64,
    pub read_chunk_size: u64,
    pub max_depth: u8,
}

impl Default for TileCacheOptions {
    fn default() -> Self {
        Self {
            target_leaf_points: 65_536,
            read_chunk_size: 65_536,
            max_depth: 12,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexProgress {
    pub points_read: u64,
    pub total_points: u64,
    pub tiles_created: usize,
}

/// Builds a cache in a temporary sibling directory and publishes it only after
/// the manifest and tile streams are complete. Returning `false` from
/// `continue_building` cancels safely and leaves no published cache.
pub fn build_tiled_cache(
    source: impl AsRef<Path>,
    cache_dir: impl AsRef<Path>,
    options: TileCacheOptions,
    mut continue_building: impl FnMut(IndexProgress) -> bool,
) -> TileCacheResult<TileCacheManifest> {
    if options.target_leaf_points == 0 {
        return Err(TileCacheError::InvalidOptions(
            "target_leaf_points must be greater than zero",
        ));
    }
    if options.read_chunk_size == 0 {
        return Err(TileCacheError::InvalidOptions(
            "read_chunk_size must be greater than zero",
        ));
    }
    let source = source.as_ref();
    let cache_dir = cache_dir.as_ref();
    if cache_dir.exists() {
        return Err(TileCacheError::CacheExists(cache_dir.to_owned()));
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary =
        cache_dir.with_extension(format!("ocs-building-{}-{nonce}", std::process::id()));
    let mut temporary_guard = TemporaryDirectory::create(temporary.clone())?;
    let mut reader = Reader::from_path(source)?;
    let metadata = CloudMetadata::from_header(reader.header())
        .map_err(|error| TileCacheError::InvalidCache(error.to_string()))?;
    let leaf_level = leaf_level(
        metadata.point_count,
        options.target_leaf_points,
        options.max_depth,
    );
    let strides: Vec<u64> = (0..=leaf_level)
        .map(|level| level_stride(metadata.point_count, options.target_leaf_points, level))
        .collect();
    let mut writers = WriterPool::new(temporary.clone(), MAX_OPEN_TILE_FILES);
    let mut stats: BTreeMap<TileKey, TileStats> = BTreeMap::new();
    let mut source_index = 0_u64;

    while source_index < metadata.point_count {
        let remaining = metadata.point_count - source_index;
        let data = reader.read_points(remaining.min(options.read_chunk_size))?;
        if data.is_empty() {
            break;
        }
        for point in data.points() {
            let point = SamplePoint::from_point(source_index, point?);
            for level in 0..=leaf_level {
                if level != leaf_level && source_index % strides[level as usize] != 0 {
                    continue;
                }
                let key = tile_key(&metadata, point.position, level);
                writers.write(key, &point)?;
                stats.entry(key).or_default().include(point.position);
            }
            source_index += 1;
        }
        if !continue_building(IndexProgress {
            points_read: source_index,
            total_points: metadata.point_count,
            tiles_created: stats.len(),
        }) {
            return Err(TileCacheError::Cancelled);
        }
    }
    writers.finish()?;

    let tiles = stats
        .into_iter()
        .map(|(key, stats)| TileEntry {
            key,
            file_name: tile_file_name(key),
            point_count: stats.point_count,
            bounds_min: stats.bounds_min,
            bounds_max: stats.bounds_max,
        })
        .collect();
    let manifest = TileCacheManifest {
        format_version: CACHE_FORMAT_VERSION,
        source_path: source.to_owned(),
        source_fingerprint: SourceFingerprint::from_path(source)?,
        source_metadata: metadata,
        leaf_level,
        target_leaf_points: options.target_leaf_points,
        record_size: RECORD_SIZE,
        tiles,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    fs::write(temporary.join("manifest.json"), manifest_bytes)?;
    fs::rename(&temporary, cache_dir)?;
    temporary_guard.commit();
    Ok(manifest)
}

/// Estimated on-disk size (bytes) of a cache built by [`build_tiled_cache`] for
/// `point_count` points. Every point is written once at the leaf level and again
/// at each coarser level whose stride it falls on.
pub fn estimate_cache_bytes(point_count: u64, target_leaf_points: u64, max_depth: u8) -> u64 {
    let leaf = leaf_level(point_count, target_leaf_points, max_depth);
    let mut total_points = 0_u64;
    for level in 0..=leaf {
        let stride = level_stride(point_count, target_leaf_points, level);
        total_points = total_points.saturating_add(point_count.div_ceil(stride));
    }
    total_points.saturating_mul(RECORD_SIZE as u64)
}

pub fn read_tile(
    cache_dir: impl AsRef<Path>,
    tile: &TileEntry,
) -> TileCacheResult<Vec<SamplePoint>> {
    let file = File::open(cache_dir.as_ref().join(&tile.file_name))?;
    let length = file.metadata()?.len();
    let expected = tile.point_count.saturating_mul(RECORD_SIZE as u64);
    if length != expected {
        return Err(TileCacheError::InvalidCache(format!(
            "{} has {length} bytes; expected {expected}",
            tile.file_name
        )));
    }
    let mut reader = BufReader::new(file);
    let mut points = Vec::with_capacity(usize::try_from(tile.point_count).unwrap_or(usize::MAX));
    for _ in 0..tile.point_count {
        points.push(read_point(&mut reader)?);
    }
    Ok(points)
}

/// Upper bound on concurrent tile readers: enough to keep fast NVMe busy
/// without swamping machines with modest page files.
pub const MAX_TILE_READ_WORKERS: usize = 8;

/// Reads several tiles in parallel across bounded worker threads.
///
/// Tiles are assigned round-robin so a batch holding one huge tile next to
/// many small ones still spreads its work evenly. Any failing tile fails the
/// whole batch, matching the sequential reader's semantics. `workers` is
/// clamped to 1..=[`MAX_TILE_READ_WORKERS`] and never exceeds the tile count.
pub fn read_tiles_parallel(
    cache_dir: impl AsRef<Path>,
    tiles: &[TileEntry],
    workers: usize,
) -> TileCacheResult<Vec<(TileKey, Vec<SamplePoint>)>> {
    if tiles.is_empty() {
        return Ok(Vec::new());
    }
    let workers = workers.clamp(1, MAX_TILE_READ_WORKERS).min(tiles.len());
    let cache_dir = cache_dir.as_ref();
    let mut shares: Vec<Vec<&TileEntry>> = vec![Vec::new(); workers];
    for (index, tile) in tiles.iter().enumerate() {
        shares[index % workers].push(tile);
    }
    let mut loaded = Vec::with_capacity(tiles.len());
    let outcome: TileCacheResult<()> = std::thread::scope(|scope| {
        let handles: Vec<_> = shares
            .into_iter()
            .map(|share| {
                scope.spawn(move || {
                    let mut results = Vec::with_capacity(share.len());
                    for tile in &share {
                        results.push((tile.key, read_tile(cache_dir, tile)?));
                    }
                    Ok::<_, TileCacheError>(results)
                })
            })
            .collect();
        for handle in handles {
            let results = handle
                .join()
                .map_err(|_| TileCacheError::InvalidCache("tile reader worker panicked".into()))?;
            loaded.extend(results?);
        }
        Ok(())
    });
    outcome?;
    Ok(loaded)
}

fn leaf_level(point_count: u64, target: u64, max_depth: u8) -> u8 {
    let mut level = 0_u8;
    let mut cells = 1_u64;
    while point_count.div_ceil(cells) > target && level < max_depth {
        level += 1;
        cells = cells.saturating_mul(8);
    }
    level
}

fn level_stride(point_count: u64, target: u64, level: u8) -> u64 {
    let cells = 8_u64.saturating_pow(level.into());
    point_count
        .div_ceil(target.saturating_mul(cells).max(1))
        .max(1)
}

fn tile_key(metadata: &CloudMetadata, position: [f64; 3], level: u8) -> TileKey {
    let cells = 1_u32.checked_shl(level.into()).unwrap_or(u32::MAX).max(1);
    let coordinate = |axis: usize| {
        let low = metadata.bounds_min[axis];
        let span = metadata.bounds_max[axis] - low;
        if !span.is_finite() || span <= f64::EPSILON {
            return 0;
        }
        (((position[axis] - low) / span * f64::from(cells)).floor() as i64)
            .clamp(0, i64::from(cells - 1)) as u32
    };
    TileKey {
        level,
        x: coordinate(0),
        y: coordinate(1),
        z: coordinate(2),
    }
}

fn tile_file_name(key: TileKey) -> String {
    format!("tile_{}_{}_{}_{}.bin", key.level, key.x, key.y, key.z)
}

#[derive(Default)]
struct TileStats {
    point_count: u64,
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
}

impl TileStats {
    fn include(&mut self, position: [f64; 3]) {
        if self.point_count == 0 {
            self.bounds_min = position;
            self.bounds_max = position;
        } else {
            for axis in 0..3 {
                self.bounds_min[axis] = self.bounds_min[axis].min(position[axis]);
                self.bounds_max[axis] = self.bounds_max[axis].max(position[axis]);
            }
        }
        self.point_count += 1;
    }
}

struct WriterPool {
    root: PathBuf,
    max_open: usize,
    writers: BTreeMap<TileKey, BufWriter<File>>,
    order: VecDeque<TileKey>,
}

impl WriterPool {
    fn new(root: PathBuf, max_open: usize) -> Self {
        Self {
            root,
            max_open: max_open.max(1),
            writers: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }

    fn write(&mut self, key: TileKey, point: &SamplePoint) -> io::Result<()> {
        if !self.writers.contains_key(&key) {
            if self.writers.len() >= self.max_open {
                if let Some(oldest) = self.order.pop_front() {
                    if let Some(mut writer) = self.writers.remove(&oldest) {
                        writer.flush()?;
                    }
                }
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.root.join(tile_file_name(key)))?;
            self.writers
                .insert(key, BufWriter::with_capacity(WRITER_BUFFER_BYTES, file));
        }
        self.order.retain(|candidate| *candidate != key);
        self.order.push_back(key);
        write_point(
            self.writers.get_mut(&key).expect("tile writer exists"),
            point,
        )
    }

    fn finish(mut self) -> io::Result<()> {
        for writer in self.writers.values_mut() {
            writer.flush()?;
        }
        Ok(())
    }
}

fn write_point(writer: &mut impl Write, point: &SamplePoint) -> io::Result<()> {
    writer.write_all(&point.source_index.to_le_bytes())?;
    for value in point.position {
        writer.write_all(&value.to_le_bytes())?;
    }
    writer.write_all(&point.intensity.to_le_bytes())?;
    writer.write_all(&[
        point.classification,
        point.return_number,
        point.number_of_returns,
    ])?;
    writer.write_all(&point.scan_angle.to_le_bytes())?;
    writer.write_all(&[point.user_data])?;
    writer.write_all(&point.point_source_id.to_le_bytes())?;
    writer.write_all(&point.gps_time.unwrap_or(f64::NAN).to_le_bytes())?;
    let color = point.color.unwrap_or([0; 3]);
    for value in color {
        writer.write_all(&value.to_le_bytes())?;
    }
    writer.write_all(&point.nir.unwrap_or(0).to_le_bytes())?;
    let flags = u8::from(point.color.is_some())
        | (u8::from(point.nir.is_some()) << 1)
        | (u8::from(point.is_synthetic) << 2)
        | (u8::from(point.is_key_point) << 3)
        | (u8::from(point.is_withheld) << 4)
        | (u8::from(point.is_overlap) << 5);
    writer.write_all(&[flags])
}

fn read_point(reader: &mut impl Read) -> io::Result<SamplePoint> {
    let source_index = read_u64(reader)?;
    let position = [read_f64(reader)?, read_f64(reader)?, read_f64(reader)?];
    let intensity = read_u16(reader)?;
    let classification = read_u8(reader)?;
    let return_number = read_u8(reader)?;
    let number_of_returns = read_u8(reader)?;
    let scan_angle = read_f32(reader)?;
    let user_data = read_u8(reader)?;
    let point_source_id = read_u16(reader)?;
    let gps_time = read_f64(reader)?;
    let color = [read_u16(reader)?, read_u16(reader)?, read_u16(reader)?];
    let nir = read_u16(reader)?;
    let flags = read_u8(reader)?;
    Ok(SamplePoint {
        source_index,
        position,
        intensity,
        classification,
        return_number,
        number_of_returns,
        scan_angle,
        user_data,
        point_source_id,
        gps_time: gps_time.is_finite().then_some(gps_time),
        color: (flags & 1 != 0).then_some(color),
        nir: (flags & 2 != 0).then_some(nir),
        is_synthetic: flags & 4 != 0,
        is_key_point: flags & 8 != 0,
        is_withheld: flags & 16 != 0,
        is_overlap: flags & 32 != 0,
    })
}

fn read_u8(reader: &mut impl Read) -> io::Result<u8> {
    let mut bytes = [0; 1];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn read_u16(reader: &mut impl Read) -> io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f32(reader: &mut impl Read) -> io::Result<f32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

fn read_f64(reader: &mut impl Read) -> io::Result<f64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(f64::from_le_bytes(bytes))
}

struct TemporaryDirectory {
    path: PathBuf,
    committed: bool,
}

impl TemporaryDirectory {
    fn create(path: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
