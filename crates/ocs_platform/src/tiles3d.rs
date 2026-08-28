//! 3D Tiles 1.0 point export with deterministic octree tiling.
//!
//! The single-tile entry point remains available for small products. Large
//! point sets use [`export_point_octree_tileset`], whose internal nodes carry
//! bounded overview samples and whose children progressively replace them.
//! A server can expose the generated files lazily through [`TilesetStream`].

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OctreeOptions {
    /// Maximum overview/leaf content size before a node is subdivided.
    pub max_points_per_tile: usize,
    /// Hard recursion bound for extremely concentrated or coincident points.
    pub max_depth: u8,
}

impl Default for OctreeOptions {
    fn default() -> Self {
        Self {
            max_points_per_tile: 50_000,
            max_depth: 12,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OctreeTilesetExport {
    pub tileset: PathBuf,
    pub content_directory: PathBuf,
    pub point_count: usize,
    pub tile_count: usize,
    pub byte_length: u64,
    pub max_depth: u8,
}

/// Incremental, disk-backed 3D Tiles octree writer. Source records are spooled
/// once, then partitioned in bounded chunks, so export memory is proportional
/// to one tile rather than to the source point count.
pub struct PointOctreeWriter {
    directory: PathBuf,
    staging: PathBuf,
    spool_path: PathBuf,
    spool: Option<BufWriter<fs::File>>,
    geometric_error: f64,
    options: OctreeOptions,
    overwrite: bool,
    point_count: usize,
    minimum: [f64; 3],
    maximum: [f64; 3],
    finished: bool,
}

impl PointOctreeWriter {
    pub fn create(
        directory: impl AsRef<Path>,
        geometric_error: f64,
        options: OctreeOptions,
        overwrite: bool,
    ) -> io::Result<Self> {
        if !geometric_error.is_finite() || geometric_error < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "geometric error must be finite and non-negative",
            ));
        }
        if options.max_points_per_tile == 0 || options.max_depth == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "octree limits must be greater than zero",
            ));
        }
        let directory = directory.as_ref().to_path_buf();
        if directory.exists() && !overwrite {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "3D Tiles output directory already exists",
            ));
        }
        let parent = directory.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let leaf_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tileset");
        let staging = parent.join(format!(".{leaf_name}.partial-{}", std::process::id()));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(staging.join("tiles"))?;
        fs::create_dir_all(staging.join("work"))?;
        let spool_path = staging.join("work").join("root.bin");
        let spool = BufWriter::new(fs::File::create(&spool_path)?);
        Ok(Self {
            directory,
            staging,
            spool_path,
            spool: Some(spool),
            geometric_error,
            options,
            overwrite,
            point_count: 0,
            minimum: [f64::INFINITY; 3],
            maximum: [f64::NEG_INFINITY; 3],
            finished: false,
        })
    }

    pub fn write_point(&mut self, point: [f64; 3]) -> io::Result<()> {
        if point.iter().any(|value| !value.is_finite()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "3D Tiles point coordinates must be finite",
            ));
        }
        let spool = self
            .spool
            .as_mut()
            .ok_or_else(|| io::Error::other("octree writer is already finished"))?;
        for axis in 0..3 {
            spool.write_all(&point[axis].to_le_bytes())?;
            self.minimum[axis] = self.minimum[axis].min(point[axis]);
            self.maximum[axis] = self.maximum[axis].max(point[axis]);
        }
        self.point_count = self
            .point_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("point count overflow"))?;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<OctreeTilesetExport> {
        let mut spool = self
            .spool
            .take()
            .ok_or_else(|| io::Error::other("octree writer is already finished"))?;
        spool.flush()?;
        drop(spool);
        if self.point_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one point is required",
            ));
        }
        let geometric_error = if self.geometric_error == 0.0 {
            (0..3)
                .map(|axis| (self.maximum[axis] - self.minimum[axis]).powi(2))
                .sum::<f64>()
                .sqrt()
                / 32.0
        } else {
            self.geometric_error
        };
        let mut stats = OctreeStats::default();
        let root = write_spooled_node(
            &self.staging,
            &self.spool_path,
            self.point_count,
            self.minimum,
            self.maximum,
            0,
            "r",
            geometric_error,
            self.options,
            &mut stats,
        )?;
        let tileset_json = serde_json::to_vec_pretty(&serde_json::json!({
            "asset": {"version": "1.0", "generator": "OpenCADStudio 2 disk octree"},
            "geometricError": geometric_error,
            "root": root,
        }))?;
        fs::write(self.staging.join("tileset.json"), tileset_json)?;
        let _ = fs::remove_dir(self.staging.join("work"));
        replace_directory(&self.staging, &self.directory, self.overwrite)?;
        self.finished = true;
        Ok(OctreeTilesetExport {
            tileset: self.directory.join("tileset.json"),
            content_directory: self.directory.join("tiles"),
            point_count: self.point_count,
            tile_count: stats.tile_count,
            byte_length: stats.byte_length,
            max_depth: stats.max_depth,
        })
    }
}

impl Drop for PointOctreeWriter {
    fn drop(&mut self) {
        if !self.finished {
            let _ = fs::remove_dir_all(&self.staging);
        }
    }
}

/// Read-only, traversal-safe access to a generated tileset directory. HTTP
/// adapters can map request paths directly to `read_asset`, so PNTS payloads
/// are opened only when a 3D Tiles client asks for them.
#[derive(Clone, Debug)]
pub struct TilesetStream {
    root: PathBuf,
}

impl TilesetStream {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !root.join("tileset.json").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "tileset.json is missing",
            ));
        }
        Ok(Self { root })
    }

    pub fn read_asset(&self, uri: &str) -> io::Result<Vec<u8>> {
        let relative = Path::new(uri);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "3D Tiles asset path escapes the tileset",
            ));
        }
        let path = self.root.join(relative);
        let canonical = fs::canonicalize(&path)?;
        if !canonical.starts_with(&self.root) || !canonical.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "3D Tiles asset path is outside the tileset",
            ));
        }
        fs::read(canonical)
    }
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
    let pnts = encode_pnts(&tile.positions, center)?;
    let byte_length = pnts.len();
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

/// Export a progressively streamable point-cloud hierarchy. Internal nodes
/// contain deterministic overview samples capped by `max_points_per_tile`;
/// leaf nodes retain every input point. `REPLACE` refinement prevents overview
/// samples from being double-rendered once finer children arrive.
pub fn export_point_octree_tileset(
    directory: impl AsRef<Path>,
    tile: &PointTile,
    options: OctreeOptions,
    overwrite: bool,
) -> io::Result<OctreeTilesetExport> {
    validate_tile(tile)?;
    let mut writer = PointOctreeWriter::create(
        directory,
        tile.geometric_error,
        options,
        overwrite,
    )?;
    for point in &tile.positions {
        writer.write_point(*point)?;
    }
    writer.finish()
}

#[allow(clippy::too_many_arguments)]
fn write_spooled_node(
    root: &Path,
    spool_path: &Path,
    point_count: usize,
    minimum: [f64; 3],
    maximum: [f64; 3],
    depth: u8,
    key: &str,
    root_error: f64,
    options: OctreeOptions,
    stats: &mut OctreeStats,
) -> io::Result<serde_json::Value> {
    if point_count <= options.max_points_per_tile {
        let points = read_spooled_points(spool_path, point_count)?;
        fs::remove_file(spool_path)?;
        return write_point_node(
            root,
            key,
            &points,
            minimum,
            maximum,
            0.0,
            Vec::new(),
            depth,
            stats,
        );
    }

    if depth >= options.max_depth {
        return write_spill_node(
            root,
            spool_path,
            point_count,
            minimum,
            maximum,
            depth,
            key,
            root_error,
            options.max_points_per_tile,
            stats,
        );
    }

    let midpoint = center(minimum, maximum);
    let work = root.join("work");
    let mut child_paths: [Option<PathBuf>; 8] = std::array::from_fn(|_| None);
    let mut child_writers: [Option<BufWriter<fs::File>>; 8] = std::array::from_fn(|_| None);
    let mut child_counts = [0_usize; 8];
    let mut child_minimum = [[f64::INFINITY; 3]; 8];
    let mut child_maximum = [[f64::NEG_INFINITY; 3]; 8];
    let mut overview = Vec::with_capacity(options.max_points_per_tile);
    let mut reader = BufReader::new(fs::File::open(spool_path)?);
    for index in 0..point_count {
        let point = read_raw_point(&mut reader)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "octree spool ended early")
        })?;
        if overview.len() < options.max_points_per_tile
            && index >= overview.len() * point_count / options.max_points_per_tile
        {
            overview.push(point);
        }
        let octant = usize::from(point[0] >= midpoint[0])
            | (usize::from(point[1] >= midpoint[1]) << 1)
            | (usize::from(point[2] >= midpoint[2]) << 2);
        if child_writers[octant].is_none() {
            let path = work.join(format!("{key}{octant}.bin"));
            child_writers[octant] = Some(BufWriter::new(fs::File::create(&path)?));
            child_paths[octant] = Some(path);
        }
        write_raw_point(child_writers[octant].as_mut().unwrap(), point)?;
        child_counts[octant] += 1;
        update_bounds(
            &mut child_minimum[octant],
            &mut child_maximum[octant],
            point,
        );
    }
    for writer in child_writers.iter_mut().flatten() {
        writer.flush()?;
    }
    drop(child_writers);
    drop(reader);
    fs::remove_file(spool_path)?;

    let mut children = Vec::new();
    for octant in 0..8 {
        if let Some(path) = child_paths[octant].as_ref() {
            children.push(write_spooled_node(
                root,
                path,
                child_counts[octant],
                child_minimum[octant],
                child_maximum[octant],
                depth + 1,
                &format!("{key}{octant}"),
                root_error,
                options,
                stats,
            )?);
        }
    }
    write_point_node(
        root,
        key,
        &overview,
        minimum,
        maximum,
        root_error / 2_f64.powi(depth as i32),
        children,
        depth,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_spill_node(
    root: &Path,
    spool_path: &Path,
    point_count: usize,
    minimum: [f64; 3],
    maximum: [f64; 3],
    depth: u8,
    key: &str,
    root_error: f64,
    max_points: usize,
    stats: &mut OctreeStats,
) -> io::Result<serde_json::Value> {
    let mut reader = BufReader::new(fs::File::open(spool_path)?);
    let mut overview = Vec::with_capacity(max_points);
    let mut chunk = Vec::with_capacity(max_points);
    let mut children = Vec::new();
    let mut chunk_index = 0_usize;
    for index in 0..point_count {
        let point = read_raw_point(&mut reader)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "octree spool ended early")
        })?;
        if overview.len() < max_points && index >= overview.len() * point_count / max_points {
            overview.push(point);
        }
        chunk.push(point);
        if chunk.len() == max_points || index + 1 == point_count {
            let (chunk_minimum, chunk_maximum) = bounds(&chunk);
            children.push(write_point_node(
                root,
                &format!("{key}s{chunk_index}"),
                &chunk,
                chunk_minimum,
                chunk_maximum,
                0.0,
                Vec::new(),
                depth.saturating_add(1),
                stats,
            )?);
            chunk.clear();
            chunk_index += 1;
        }
    }
    drop(reader);
    fs::remove_file(spool_path)?;
    write_point_node(
        root,
        key,
        &overview,
        minimum,
        maximum,
        root_error / 2_f64.powi(depth as i32),
        children,
        depth,
        stats,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_point_node(
    root: &Path,
    key: &str,
    points: &[[f64; 3]],
    minimum: [f64; 3],
    maximum: [f64; 3],
    geometric_error: f64,
    children: Vec<serde_json::Value>,
    depth: u8,
    stats: &mut OctreeStats,
) -> io::Result<serde_json::Value> {
    let uri = format!("tiles/{key}.pnts");
    let bytes = encode_pnts(points, center(minimum, maximum))?;
    fs::write(root.join("tiles").join(format!("{key}.pnts")), &bytes)?;
    stats.tile_count += 1;
    stats.byte_length += bytes.len() as u64;
    stats.max_depth = stats.max_depth.max(depth);
    let mut value = serde_json::json!({
        "boundingVolume": bounding_box(minimum, maximum),
        "geometricError": geometric_error,
        "refine": "REPLACE",
        "content": {"uri": uri},
    });
    if !children.is_empty() {
        value["children"] = serde_json::Value::Array(children);
    }
    Ok(value)
}

fn read_spooled_points(path: &Path, count: usize) -> io::Result<Vec<[f64; 3]>> {
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        points.push(read_raw_point(&mut reader)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "octree spool ended early")
        })?);
    }
    Ok(points)
}

fn read_raw_point(reader: &mut impl Read) -> io::Result<Option<[f64; 3]>> {
    let mut bytes = [0_u8; 24];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(std::array::from_fn(|axis| {
            f64::from_le_bytes(bytes[axis * 8..axis * 8 + 8].try_into().unwrap())
        }))),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_raw_point(writer: &mut impl Write, point: [f64; 3]) -> io::Result<()> {
    for value in point {
        writer.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

fn update_bounds(minimum: &mut [f64; 3], maximum: &mut [f64; 3], point: [f64; 3]) {
    for axis in 0..3 {
        minimum[axis] = minimum[axis].min(point[axis]);
        maximum[axis] = maximum[axis].max(point[axis]);
    }
}

#[derive(Default)]
struct OctreeStats {
    tile_count: usize,
    byte_length: u64,
    max_depth: u8,
}

fn encode_pnts(points: &[[f64; 3]], rtc_center: [f64; 3]) -> io::Result<Vec<u8>> {
    let mut feature_json = serde_json::to_vec(&serde_json::json!({
        "POINTS_LENGTH": points.len(),
        "POSITION": {"byteOffset": 0},
        "RTC_CENTER": rtc_center,
    }))?;
    pad(&mut feature_json, 8, b' ');
    let binary_len = points
        .len()
        .checked_mul(12)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "PNTS tile is too large"))?;
    let mut feature_binary = Vec::with_capacity(binary_len);
    for point in points {
        for axis in 0..3 {
            feature_binary
                .extend_from_slice(&((point[axis] - rtc_center[axis]) as f32).to_le_bytes());
        }
    }
    pad(&mut feature_binary, 8, 0);
    let byte_length = 28_usize
        .checked_add(feature_json.len())
        .and_then(|length| length.checked_add(feature_binary.len()))
        .filter(|length| *length <= u32::MAX as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "PNTS tile is too large"))?;
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
    Ok(pnts)
}

fn validate_tile(tile: &PointTile) -> io::Result<()> {
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
    Ok(())
}

fn replace_directory(staging: &Path, destination: &Path, overwrite: bool) -> io::Result<()> {
    if !destination.exists() {
        return fs::rename(staging, destination);
    }
    if !overwrite {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "3D Tiles output directory already exists",
        ));
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("tileset");
    let backup = parent.join(format!(".{name}.backup-{}", std::process::id()));
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    fs::rename(destination, &backup)?;
    if let Err(error) = fs::rename(staging, destination) {
        let _ = fs::rename(&backup, destination);
        return Err(error);
    }
    let _ = fs::remove_dir_all(backup);
    Ok(())
}

fn center(minimum: [f64; 3], maximum: [f64; 3]) -> [f64; 3] {
    [
        (minimum[0] + maximum[0]) * 0.5,
        (minimum[1] + maximum[1]) * 0.5,
        (minimum[2] + maximum[2]) * 0.5,
    ]
}

fn bounding_box(minimum: [f64; 3], maximum: [f64; 3]) -> serde_json::Value {
    let center = center(minimum, maximum);
    serde_json::json!({"box": [
        center[0], center[1], center[2],
        ((maximum[0] - minimum[0]) * 0.5).max(1.0e-9), 0.0, 0.0,
        0.0, ((maximum[1] - minimum[1]) * 0.5).max(1.0e-9), 0.0,
        0.0, 0.0, ((maximum[2] - minimum[2]) * 0.5).max(1.0e-9)
    ]})
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

    #[test]
    fn exports_progressive_octree_and_streams_assets_safely() {
        let directory =
            std::env::temp_dir().join(format!("ocs-3dtiles-octree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let positions = (0..64)
            .map(|index| {
                let x = (index & 3) as f64;
                let y = ((index >> 2) & 3) as f64;
                let z = ((index >> 4) & 3) as f64;
                [x, y, z]
            })
            .collect();
        let export = export_point_octree_tileset(
            &directory,
            &PointTile {
                positions,
                geometric_error: 8.0,
            },
            OctreeOptions {
                max_points_per_tile: 4,
                max_depth: 4,
            },
            false,
        )
        .unwrap();
        assert!(export.tile_count > 8);
        assert!(export.max_depth >= 2);
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&export.tileset).unwrap()).unwrap();
        assert_eq!(json["root"]["refine"], "REPLACE");
        assert!(json["root"]["children"].as_array().unwrap().len() > 1);

        let stream = TilesetStream::open(&directory).unwrap();
        let uri = json["root"]["content"]["uri"].as_str().unwrap();
        assert_eq!(&stream.read_asset(uri).unwrap()[..4], b"pnts");
        assert_eq!(
            stream.read_asset("../secrets.txt").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let _ = fs::remove_dir_all(directory);
    }
}
