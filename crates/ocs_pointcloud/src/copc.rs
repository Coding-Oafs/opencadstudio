//! Local and HTTP-range COPC access using the native `las` COPC reader.

use crate::{CrsInfo, Result, SamplePoint};
use las::{Bounds, BoundsSelection, CopcReader, LodSelection, PointData, Vector};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CopcLod {
    All,
    Resolution(f64),
    Level(i32),
    LevelRange([i32; 2]),
}

impl From<CopcLod> for LodSelection {
    fn from(value: CopcLod) -> Self {
        match value {
            CopcLod::All => Self::All,
            CopcLod::Resolution(value) => Self::Resolution(value),
            CopcLod::Level(value) => Self::Level(value),
            CopcLod::LevelRange([minimum, maximum]) => Self::LevelMinMax(minimum, maximum),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopcQuery {
    pub lod: CopcLod,
    pub bounds: Option<([f64; 3], [f64; 3])>,
}

impl Default for CopcQuery {
    fn default() -> Self {
        Self {
            lod: CopcLod::All,
            bounds: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopcMetadata {
    pub point_count: u64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub root_spacing: f64,
    pub hierarchy_entries: usize,
    pub crs: CrsInfo,
}

pub fn inspect_copc(path: impl AsRef<Path>) -> Result<CopcMetadata> {
    let reader = CopcReader::from_path(path)?;
    Ok(metadata(&reader))
}

pub fn query_copc(path: impl AsRef<Path>, query: &CopcQuery) -> Result<Vec<SamplePoint>> {
    let mut reader = CopcReader::from_path(path)?;
    query_reader(&mut reader, query)
}

pub fn inspect_copc_http(url: &str) -> Result<CopcMetadata> {
    let stream = HttpRangeReader::open(url, 1024 * 1024)?;
    let reader = CopcReader::new(stream)?;
    Ok(metadata(&reader))
}

pub fn query_copc_http(url: &str, query: &CopcQuery) -> Result<Vec<SamplePoint>> {
    let stream = HttpRangeReader::open(url, 1024 * 1024)?;
    let mut reader = CopcReader::new(stream)?;
    query_reader(&mut reader, query)
}

fn metadata<R: Read + Seek>(reader: &CopcReader<'_, R>) -> CopcMetadata {
    let header = reader.header();
    let bounds = header.bounds();
    let root_spacing = header
        .copc_info_vlr()
        .map_or(0.0, |information| information.spacing);
    CopcMetadata {
        point_count: header.number_of_points(),
        bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z],
        bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z],
        root_spacing,
        hierarchy_entries: reader.hierarchy_entries().count(),
        crs: CrsInfo::from_header(header),
    }
}

fn query_reader<R: Read + Seek>(
    reader: &mut CopcReader<'_, R>,
    query: &CopcQuery,
) -> Result<Vec<SamplePoint>> {
    let bounds = query.bounds.map_or(BoundsSelection::All, |(min, max)| {
        BoundsSelection::Within(Bounds {
            min: Vector {
                x: min[0],
                y: min[1],
                z: min[2],
            },
            max: Vector {
                x: max[0],
                y: max[1],
                z: max[2],
            },
        })
    });
    point_data_to_sample(reader.query(query.lod.into(), bounds)?)
}

fn point_data_to_sample(data: PointData) -> Result<Vec<SamplePoint>> {
    data.points()
        .enumerate()
        .map(|(index, point)| Ok(SamplePoint::from_point(index as u64, point?)))
        .collect()
}

/// Seekable HTTP byte source. COPC hierarchy and chunk seeks become bounded
/// `Range` requests, so remote data is not downloaded as one monolithic file.
pub struct HttpRangeReader {
    agent: ureq::Agent,
    url: String,
    length: u64,
    position: u64,
    chunk_size: usize,
    cache_start: u64,
    cache: Vec<u8>,
}

impl HttpRangeReader {
    pub fn open(url: &str, chunk_size: usize) -> io::Result<Self> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "COPC URL must use http or https",
            ));
        }
        let agent = ureq::Agent::new_with_defaults();
        let response = agent
            .get(url)
            .header("Range", "bytes=0-0")
            .header("Accept-Encoding", "identity")
            .header(
                "User-Agent",
                concat!("OpenCADStudio/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(http_error)?;
        let length = response
            .headers()
            .get("Content-Range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|value| value.parse::<u64>().ok())
            .or_else(|| {
                response
                    .headers()
                    .get("Content-Length")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "server did not report the remote COPC length",
                )
            })?;
        if response.status().as_u16() != 206 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server does not support HTTP byte ranges",
            ));
        }
        Ok(Self {
            agent,
            url: url.to_string(),
            length,
            position: 0,
            chunk_size: chunk_size.max(64 * 1024),
            cache_start: 0,
            cache: Vec::new(),
        })
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    fn refill(&mut self) -> io::Result<()> {
        if self.position >= self.length {
            self.cache.clear();
            return Ok(());
        }
        let end = self
            .position
            .saturating_add(self.chunk_size as u64)
            .saturating_sub(1)
            .min(self.length - 1);
        let range = format!("bytes={}-{}", self.position, end);
        let mut response = self
            .agent
            .get(&self.url)
            .header("Range", &range)
            .header("Accept-Encoding", "identity")
            .header(
                "User-Agent",
                concat!("OpenCADStudio/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(http_error)?;
        if response.status().as_u16() != 206 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server stopped honoring HTTP byte ranges",
            ));
        }
        self.cache_start = self.position;
        self.cache = response
            .body_mut()
            .with_config()
            .limit(self.chunk_size as u64 + 1)
            .read_to_vec()
            .map_err(http_error)?;
        Ok(())
    }
}

impl Read for HttpRangeReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.cache.is_empty() || self.position < self.cache_start || self.position >= cache_end {
            self.refill()?;
        }
        let offset = usize::try_from(self.position - self.cache_start).unwrap_or(usize::MAX);
        let available = self.cache.len().saturating_sub(offset);
        let count = output.len().min(available);
        if count == 0 {
            return Ok(0);
        }
        output[..count].copy_from_slice(&self.cache[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for HttpRangeReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
            SeekFrom::End(delta) => i128::from(self.length) + i128::from(delta),
        };
        if !(0..=i128::from(self.length)).contains(&position) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek is outside the remote COPC",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

fn http_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copc_lod_maps_to_native_selection() {
        assert_eq!(LodSelection::All, CopcLod::All.into());
        assert_eq!(
            LodSelection::LevelMinMax(2, 5),
            CopcLod::LevelRange([2, 5]).into()
        );
    }

    #[test]
    fn non_http_range_source_is_rejected_without_network_access() {
        assert!(HttpRangeReader::open("file:///tile.copc.laz", 1).is_err());
    }
}
