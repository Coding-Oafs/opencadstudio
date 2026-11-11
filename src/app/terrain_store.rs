//! Immutable, bounded elevation snapshots. Drawing sidecars hold only the key.
use super::terrain::{TerrainDataset, TerrainRequest};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 8] = b"OCSTERR1";
const MAX_CELLS: usize = 16_000_000;
static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedTerrain {
    pub key: String,
    pub request: TerrainRequest,
    pub crs: super::spatial::DrawingCrs,
}

#[derive(Serialize, Deserialize)]
struct Header {
    origin: [f64; 2],
    cell_size: f64,
    columns: usize,
    rows: usize,
    no_data: f64,
    zoom: u32,
    tiles: usize,
    failed_tiles: usize,
    crs_label: String,
    elevation_unit: String,
    source: String,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn root() -> io::Result<PathBuf> {
    crate::config::config_dir()
        .map(|p| p.join("terrain_models"))
        .ok_or_else(|| invalid("application data directory is unavailable"))
}

fn cells(h: &Header) -> io::Result<usize> {
    let n = h
        .columns
        .checked_mul(h.rows)
        .filter(|n| *n > 0 && *n <= MAX_CELLS)
        .ok_or_else(|| invalid("terrain dimensions exceed the supported limit"))?;
    if !h.cell_size.is_finite()
        || h.cell_size <= 0.0
        || !h.origin.iter().all(|v| v.is_finite())
        || !h.no_data.is_finite()
    {
        return Err(invalid("invalid terrain coordinates"));
    }
    Ok(n)
}

pub(super) fn save(dataset: &TerrainDataset) -> io::Result<String> {
    save_in(&root()?, dataset)
}
pub(super) fn load(key: &str) -> io::Result<TerrainDataset> {
    load_in(&root()?, key)
}

fn save_in(root: &Path, d: &TerrainDataset) -> io::Result<String> {
    let s = &d.surface;
    let h = Header {
        origin: s.origin,
        cell_size: s.cell_size,
        columns: s.columns,
        rows: s.rows,
        no_data: s.no_data,
        zoom: d.zoom,
        tiles: d.tiles,
        failed_tiles: d.failed_tiles,
        crs_label: d.crs_label.clone(),
        elevation_unit: d.elevation_unit.clone(),
        source: d.source.clone(),
    };
    let n = cells(&h)?;
    if s.values.len() != n || s.counts.len() != n || s.values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("terrain arrays do not match the grid"));
    }
    let json = serde_json::to_vec(&h).map_err(io::Error::other)?;
    let mut hash = Sha256::new();
    hash.update(&json);
    for (z, count) in s.values.iter().zip(&s.counts) {
        hash.update(z.to_le_bytes());
        hash.update(count.to_le_bytes());
    }
    let key = format!("{:x}", hash.finalize());
    fs::create_dir_all(root)?;
    let target = root.join(format!("{key}.ocsterrain"));
    // A complete immutable snapshot can be shared by multiple drawings.
    if target.exists() && load_in(root, &key).is_ok() {
        return Ok(key);
    }
    let serial = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = root.join(format!("{key}.{}.{serial}.tmp", std::process::id()));
    let result = (|| {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut out = ZlibEncoder::new(io::BufWriter::new(file), Compression::fast());
        out.write_all(MAGIC)?;
        out.write_all(&(json.len() as u32).to_le_bytes())?;
        out.write_all(&json)?;
        for (z, count) in s.values.iter().zip(&s.counts) {
            out.write_all(&z.to_le_bytes())?;
            out.write_all(&count.to_le_bytes())?;
        }
        let mut file = out.finish()?;
        file.flush()?;
        file.get_ref().sync_all()?;
        drop(file);
        // Never replace a valid snapshot. A corrupt cache can be rebuilt.
        if target.exists() {
            fs::remove_file(&target)?;
        }
        fs::rename(&temp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map(|_| key)
}

fn load_in(root: &Path, key: &str) -> io::Result<TerrainDataset> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid terrain snapshot key"));
    }
    let file = fs::File::open(root.join(format!("{key}.ocsterrain")))?;
    let mut input = ZlibDecoder::new(io::BufReader::new(file));
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("unsupported terrain snapshot version"));
    }
    let mut size = [0; 4];
    input.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > 64 * 1024 {
        return Err(invalid("terrain header exceeds the supported limit"));
    }
    let mut json = vec![0; size];
    input.read_exact(&mut json)?;
    let h: Header = serde_json::from_slice(&json).map_err(io::Error::other)?;
    let n = cells(&h)?;
    let mut hash = Sha256::new();
    hash.update(&json);
    let mut values = Vec::with_capacity(n);
    let mut counts = Vec::with_capacity(n);
    for _ in 0..n {
        let mut z = [0; 8];
        let mut count = [0; 4];
        input.read_exact(&mut z)?;
        input.read_exact(&mut count)?;
        hash.update(z);
        hash.update(count);
        let value = f64::from_le_bytes(z);
        if !value.is_finite() {
            return Err(invalid("non-finite terrain elevation"));
        }
        values.push(value);
        counts.push(u32::from_le_bytes(count));
    }
    let mut extra = [0; 1];
    if input.read(&mut extra)? != 0 || format!("{:x}", hash.finalize()) != key {
        return Err(invalid("terrain snapshot checksum mismatch"));
    }
    Ok(TerrainDataset {
        surface: ocs_pointcloud::RasterSurface {
            origin: h.origin,
            cell_size: h.cell_size,
            columns: h.columns,
            rows: h.rows,
            no_data: h.no_data,
            values,
            counts,
        },
        zoom: h.zoom,
        tiles: h.tiles,
        failed_tiles: h.failed_tiles,
        crs_label: h.crs_label,
        elevation_unit: h.elevation_unit,
        source: h.source,
        cache_key: Some(key.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_preserves_exact_elevations_counts_and_rejects_corruption() {
        let root = std::env::temp_dir().join(format!("ocs-terrain-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let d = TerrainDataset {
            surface: ocs_pointcloud::RasterSurface {
                origin: [400000., 4700000.],
                cell_size: 0.5,
                columns: 2,
                rows: 2,
                no_data: -9999.,
                values: vec![1.123456789, -9999., 3., 4.],
                counts: vec![2, 0, 5, 1],
            },
            zoom: 12,
            tiles: 1,
            failed_tiles: 0,
            crs_label: "EPSG:26986".into(),
            elevation_unit: "meters".into(),
            source: "fixture".into(),
            cache_key: None,
        };
        let key = save_in(&root, &d).unwrap();
        assert_eq!(load_in(&root, &key).unwrap().surface, d.surface);
        assert!(load_in(&root, "../other").is_err());
        fs::write(root.join(format!("{key}.ocsterrain")), b"corrupt").unwrap();
        assert!(load_in(&root, &key).is_err());
        assert_eq!(save_in(&root, &d).unwrap(), key);
        assert_eq!(load_in(&root, &key).unwrap().surface, d.surface);
        fs::remove_dir_all(root).unwrap();
    }
}
