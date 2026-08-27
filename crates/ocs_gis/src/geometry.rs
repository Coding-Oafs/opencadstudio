//! Simple-feature geometry model with WKB and GeoJSON coding.
//!
//! The model intentionally matches OGC simple features (2D, XY only for
//! now): it is what GeoPackage stores, what GeoJSON carries, and what the
//! viewer needs for CAD/GIS conversion.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub type Point2 = [f64; 2];

/// OGC simple-feature geometry, 2D.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Geometry {
    Point(Point2),
    MultiPoint(Vec<Point2>),
    LineString(Vec<Point2>),
    MultiLineString(Vec<Vec<Point2>>),
    Polygon(Vec<Vec<Point2>>),
    MultiPolygon(Vec<Vec<Vec<Point2>>>),
}

impl Geometry {
    /// WKB geometry type codes (2D variants).
    pub fn wkb_code(&self) -> u32 {
        match self {
            Self::Point(_) => 1,
            Self::LineString(_) => 2,
            Self::Polygon(_) => 3,
            Self::MultiPoint(_) => 4,
            Self::MultiLineString(_) => 5,
            Self::MultiPolygon(_) => 6,
        }
    }

    pub fn from_wkb_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => Self::Point([0.0, 0.0]),
            2 => Self::LineString(Vec::new()),
            3 => Self::Polygon(Vec::new()),
            4 => Self::MultiPoint(Vec::new()),
            5 => Self::MultiLineString(Vec::new()),
            6 => Self::MultiPolygon(Vec::new()),
            _ => return None,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Point(_) => "POINT",
            Self::LineString(_) => "LINESTRING",
            Self::Polygon(_) => "POLYGON",
            Self::MultiPoint(_) => "MULTIPOINT",
            Self::MultiLineString(_) => "MULTILINESTRING",
            Self::MultiPolygon(_) => "MULTIPOLYGON",
        }
    }

    /// Axis-aligned bounding box `[min_x, min_y, max_x, max_y]`.
    pub fn envelope(&self) -> Option<[f64; 4]> {
        let mut envelope: Option<[f64; 4]> = None;
        let mut visit = |point: &Point2| {
            let bounds = envelope.get_or_insert([f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        };
        match self {
            Self::Point(point) => visit(point),
            Self::MultiPoint(points) => points.iter().for_each(|point| visit(point)),
            Self::LineString(points) => points.iter().for_each(|point| visit(point)),
            Self::MultiLineString(lines) => lines
                .iter()
                .for_each(|line| line.iter().for_each(|point| visit(point))),
            Self::Polygon(rings) => rings
                .iter()
                .for_each(|ring| ring.iter().for_each(|point| visit(point))),
            Self::MultiPolygon(polygons) => polygons.iter().for_each(|rings| {
                rings
                    .iter()
                    .for_each(|ring| ring.iter().for_each(|point| visit(point)))
            }),
        }
        envelope
    }

    pub fn point_count(&self) -> usize {
        match self {
            Self::Point(_) => 1,
            Self::MultiPoint(points) => points.len(),
            Self::LineString(points) => points.len(),
            Self::MultiLineString(lines) => lines.iter().map(|line| line.len()).sum(),
            Self::Polygon(rings) => rings.iter().map(|ring| ring.len()).sum(),
            Self::MultiPolygon(polygons) => polygons
                .iter()
                .map(|rings| rings.iter().map(|ring| ring.len()).sum::<usize>())
                .sum(),
        }
    }

    /// Boundary-inclusive point-in-polygon for polygonal geometry.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        fn in_ring(ring: &[Point2], x: f64, y: f64) -> Option<bool> {
            let mut inside = false;
            let count = ring.len();
            for index in 0..count {
                let a = ring[index];
                let b = ring[(index + 1) % count];
                let cross = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
                let length_sq = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
                if cross.abs() <= 1e-12 * length_sq.max(1.0)
                    && x >= a[0].min(b[0]) - 1e-12
                    && x <= a[0].max(b[0]) + 1e-12
                    && y >= a[1].min(b[1]) - 1e-12
                    && y <= a[1].max(b[1]) + 1e-12
                {
                    return Some(true); // on boundary
                }
                if (a[1] > y) != (b[1] > y) {
                    let intersect_x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    if x < intersect_x {
                        inside = !inside;
                    }
                }
            }
            Some(inside)
        }
        fn in_polygon(rings: &[Vec<Point2>], x: f64, y: f64) -> bool {
            let Some(exterior) = rings.first() else {
                return false;
            };
            match in_ring(exterior, x, y) {
                Some(true) => {}
                _ => return false,
            }
            for hole in rings.iter().skip(1) {
                if in_ring(hole, x, y) == Some(true) {
                    return false;
                }
            }
            true
        }
        match self {
            Self::Polygon(rings) => in_polygon(rings, x, y),
            Self::MultiPolygon(polygons) => polygons.iter().any(|rings| in_polygon(rings, x, y)),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// WKB (little endian, 2D)
// ---------------------------------------------------------------------------

struct WkbWriter {
    bytes: Vec<u8>,
}

impl WkbWriter {
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn f64s(&mut self, values: &[f64]) {
        for value in values {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn ring(&mut self, points: &[Point2]) {
        self.u32(points.len() as u32);
        for point in points {
            self.f64s(point);
        }
    }
}

/// Encode geometry as little-endian 2D WKB.
pub fn geometry_to_wkb(geometry: &Geometry) -> Vec<u8> {
    let mut writer = WkbWriter { bytes: Vec::new() };
    writer.bytes.push(1); // little endian
    writer.u32(geometry.wkb_code());
    match geometry {
        Geometry::Point(point) => writer.f64s(point),
        Geometry::MultiPoint(points) => {
            writer.u32(points.len() as u32);
            for point in points {
                writer.bytes.push(1);
                writer.u32(1);
                writer.f64s(point);
            }
        }
        Geometry::LineString(points) => writer.ring(points),
        Geometry::MultiLineString(lines) => {
            writer.u32(lines.len() as u32);
            for line in lines {
                writer.bytes.push(1);
                writer.u32(2);
                writer.ring(line);
            }
        }
        Geometry::Polygon(rings) => {
            writer.u32(rings.len() as u32);
            for ring in rings {
                writer.ring(ring);
            }
        }
        Geometry::MultiPolygon(polygons) => {
            writer.u32(polygons.len() as u32);
            for rings in polygons {
                writer.bytes.push(1);
                writer.u32(3);
                writer.u32(rings.len() as u32);
                for ring in rings {
                    writer.ring(ring);
                }
            }
        }
    }
    writer.bytes
}

struct WkbReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> WkbReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.position + count > self.bytes.len() {
            return Err("WKB buffer is truncated".to_string());
        }
        let slice = &self.bytes[self.position..self.position + count];
        self.position += count;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let slice = self.take(4)?;
        Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    fn f64s(&mut self, count: usize) -> Result<Vec<f64>, String> {
        let slice = self.take(8 * count)?;
        Ok((0..count)
            .map(|index| {
                let mut buffer = [0u8; 8];
                buffer.copy_from_slice(&slice[index * 8..index * 8 + 8]);
                f64::from_le_bytes(buffer)
            })
            .collect())
    }

    fn point(&mut self) -> Result<Point2, String> {
        let values = self.f64s(2)?;
        Ok([values[0], values[1]])
    }

    fn ring(&mut self) -> Result<Vec<Point2>, String> {
        let count = self.u32()? as usize;
        (0..count).map(|_| self.point()).collect()
    }

    /// Reads a nested element with its own byte order + type header.
    fn nested(&mut self) -> Result<u32, String> {
        let order = self.u8()?;
        if order != 1 {
            return Err("only little-endian WKB is supported".to_string());
        }
        let code = self.u32()?;
        if code > 1000 {
            return Err("WKB Z/M variants are not supported yet".to_string());
        }
        Ok(code)
    }
}

/// Decode little-endian 2D WKB into geometry.
pub fn geometry_from_wkb(bytes: &[u8]) -> Result<Geometry, String> {
    let mut reader = WkbReader { bytes, position: 0 };
    let order = reader.u8()?;
    if order != 1 {
        return Err("only little-endian WKB is supported".to_string());
    }
    let code = reader.u32()?;
    if code > 1000 {
        return Err("WKB Z/M variants are not supported yet".to_string());
    }
    let geometry = match code {
        1 => Geometry::Point(reader.point()?),
        2 => Geometry::LineString(reader.ring()?),
        3 => {
            let count = reader.u32()? as usize;
            let mut rings = Vec::with_capacity(count);
            for _ in 0..count {
                rings.push(reader.ring()?);
            }
            Geometry::Polygon(rings)
        }
        4 => {
            let count = reader.u32()? as usize;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                let nested = reader.nested()?;
                if nested != 1 {
                    return Err("MULTIPOINT parts must be points".to_string());
                }
                points.push(reader.point()?);
            }
            Geometry::MultiPoint(points)
        }
        5 => {
            let count = reader.u32()? as usize;
            let mut lines = Vec::with_capacity(count);
            for _ in 0..count {
                let nested = reader.nested()?;
                if nested != 2 {
                    return Err("MULTILINESTRING parts must be linestrings".to_string());
                }
                lines.push(reader.ring()?);
            }
            Geometry::MultiLineString(lines)
        }
        6 => {
            let count = reader.u32()? as usize;
            let mut polygons = Vec::with_capacity(count);
            for _ in 0..count {
                let nested = reader.nested()?;
                if nested != 3 {
                    return Err("MULTIPOLYGON parts must be polygons".to_string());
                }
                let ring_count = reader.u32()? as usize;
                let mut rings = Vec::with_capacity(ring_count);
                for _ in 0..ring_count {
                    rings.push(reader.ring()?);
                }
                polygons.push(rings);
            }
            Geometry::MultiPolygon(polygons)
        }
        other => return Err(format!("unsupported WKB geometry code {other}")),
    };
    Ok(geometry)
}

// ---------------------------------------------------------------------------
// GeoJSON
// ---------------------------------------------------------------------------

fn ring_to_json(ring: &[Point2]) -> Value {
    Value::Array(
        ring.iter()
            .map(|point| json!([point[0], point[1]]))
            .collect(),
    )
}

fn ring_from_json(value: &Value) -> Result<Vec<Point2>, String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_array().and_then(|pair| {
                        let x = pair.first().and_then(Value::as_f64)?;
                        let y = pair.get(1).and_then(Value::as_f64)?;
                        Some([x, y])
                    })
                })
                .collect::<Vec<Point2>>()
        })
        .ok_or_else(|| "coordinates must be numeric arrays".to_string())
}

/// Encode geometry as a GeoJSON geometry object.
pub fn geometry_to_geojson(geometry: &Geometry) -> Value {
    match geometry {
        Geometry::Point(point) => json!({"type": "Point", "coordinates": [point[0], point[1]]}),
        Geometry::MultiPoint(points) => json!({
            "type": "MultiPoint",
            "coordinates": points.iter().map(|point| json!([point[0], point[1]])).collect::<Vec<_>>()
        }),
        Geometry::LineString(points) => json!({
            "type": "LineString",
            "coordinates": ring_to_json(points)
        }),
        Geometry::MultiLineString(lines) => json!({
            "type": "MultiLineString",
            "coordinates": lines.iter().map(|line| ring_to_json(line)).collect::<Vec<_>>()
        }),
        Geometry::Polygon(rings) => json!({
            "type": "Polygon",
            "coordinates": rings.iter().map(|ring| ring_to_json(ring)).collect::<Vec<_>>()
        }),
        Geometry::MultiPolygon(polygons) => json!({
            "type": "MultiPolygon",
            "coordinates": polygons.iter().map(|rings| Value::Array(
                rings.iter().map(|ring| ring_to_json(ring)).collect::<Vec<_>>()
            )).collect::<Vec<_>>()
        }),
    }
}

/// Decode a GeoJSON geometry object.
pub fn geometry_from_geojson(value: &Value) -> Result<Geometry, String> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "geometry needs a type".to_string())?;
    match kind {
        "Point" => {
            let coordinates = value
                .get("coordinates")
                .and_then(Value::as_array)
                .ok_or_else(|| "point needs coordinates".to_string())?;
            let x = coordinates
                .first()
                .and_then(Value::as_f64)
                .ok_or_else(|| "point needs numeric x".to_string())?;
            let y = coordinates
                .get(1)
                .and_then(Value::as_f64)
                .ok_or_else(|| "point needs numeric y".to_string())?;
            Ok(Geometry::Point([x, y]))
        }
        "MultiPoint" => Ok(Geometry::MultiPoint(ring_from_json(
            value.get("coordinates").unwrap_or(&Value::Null),
        )?)),
        "LineString" => Ok(Geometry::LineString(ring_from_json(
            value.get("coordinates").unwrap_or(&Value::Null),
        )?)),
        "MultiLineString" => {
            let parts = value
                .get("coordinates")
                .and_then(Value::as_array)
                .ok_or_else(|| "MultiLineString needs coordinates".to_string())?;
            Ok(Geometry::MultiLineString(
                parts
                    .iter()
                    .map(ring_from_json)
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "Polygon" => {
            let parts = value
                .get("coordinates")
                .and_then(Value::as_array)
                .ok_or_else(|| "Polygon needs coordinates".to_string())?;
            Ok(Geometry::Polygon(
                parts
                    .iter()
                    .map(ring_from_json)
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "MultiPolygon" => {
            let polygons = value
                .get("coordinates")
                .and_then(Value::as_array)
                .ok_or_else(|| "MultiPolygon needs coordinates".to_string())?;
            Ok(Geometry::MultiPolygon(
                polygons
                    .iter()
                    .map(|rings| {
                        rings
                            .as_array()
                            .ok_or_else(|| "polygon rings must be arrays".to_string())
                            .and_then(|rings| {
                                rings
                                    .iter()
                                    .map(ring_from_json)
                                    .collect::<Result<Vec<_>, _>>()
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        other => Err(format!("unsupported GeoJSON geometry type {other}")),
    }
}
