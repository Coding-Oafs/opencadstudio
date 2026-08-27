//! GeoPackage feature storage.
//!
//! GeoPackage is the primary editable open GIS format: one portable SQLite
//! container per the OGC standard. This module creates and reads feature
//! tables through the required `gpkg_*` metadata tables, storing geometries
//! as GeoPackage binary blobs (WKB payloads) with per-layer CRS.

use crate::feature::{Feature, FeatureLayer, FieldValue};
use crate::geometry::{geometry_from_wkb, geometry_to_wkb, Geometry};
use rusqlite::{params, Connection};
use std::path::Path;

/// GeoPackage blob header: magic, version, flags, srs_id, envelope.
/// Flags nibble: byte order (bit 0), envelope indicator (bits 1-3),
/// empty geometry (bit 4).
fn encode_gpkg_blob(geometry: &Geometry, srs_id: i32) -> Vec<u8> {
    let wkb = geometry_to_wkb(geometry);
    let envelope = geometry.envelope();
    let mut blob = Vec::with_capacity(8 + 32 + wkb.len());
    blob.extend_from_slice(b"GP");
    blob.push(0); // version
    blob.push(1); // flags: little endian, no envelope
    blob.extend_from_slice(&srs_id.to_le_bytes());
    if let Some(envelope) = envelope {
        blob[3] |= 1 << 1; // envelope indicator 001: xy envelope
        for value in envelope {
            blob.extend_from_slice(&value.to_le_bytes());
        }
    }
    blob.extend_from_slice(&wkb);
    blob
}

fn decode_gpkg_blob(blob: &[u8]) -> Result<Geometry, String> {
    if blob.len() < 8 || &blob[0..2] != b"GP" {
        return Err("not a GeoPackage geometry blob".to_string());
    }
    let flags = blob[3];
    if flags & (1 << 4) != 0 {
        return Err("empty GeoPackage geometry".to_string());
    }
    let envelope_code = (flags >> 1) & 0x07;
    let envelope_bytes = match envelope_code {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 48,
        _ => 64,
    };
    let wkb_start = 8 + envelope_bytes;
    if blob.len() <= wkb_start {
        return Err("GeoPackage geometry blob is truncated".to_string());
    }
    geometry_from_wkb(&blob[wkb_start..])
}

/// Create a new GeoPackage with the OGC-required metadata tables.
pub fn create_geopackage(path: &Path) -> Result<Connection, String> {
    if path.exists() {
        return Err(format!(
            "refusing to overwrite existing GeoPackage: {}",
            path.display()
        ));
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "PRAGMA application_id = 0x47504B47;
             CREATE TABLE gpkg_spatial_ref_sys (
               srs_name TEXT NOT NULL,
               srs_id INTEGER PRIMARY KEY,
               organization TEXT NOT NULL,
               organization_coordsys_id INTEGER NOT NULL,
               definition TEXT NOT NULL,
               description TEXT
             );
             CREATE TABLE gpkg_contents (
               table_name TEXT PRIMARY KEY,
               data_type TEXT NOT NULL,
               identifier TEXT UNIQUE,
               description TEXT DEFAULT '',
               last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
               min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE,
               srs_id INTEGER REFERENCES gpkg_spatial_ref_sys(srs_id)
             );
             CREATE TABLE gpkg_geometry_columns (
               table_name TEXT NOT NULL,
               column_name TEXT NOT NULL,
               geometry_type_name TEXT NOT NULL,
               srs_id INTEGER NOT NULL,
               z TINYINT NOT NULL,
               m TINYINT NOT NULL,
               PRIMARY KEY (table_name, column_name)
             );
             INSERT INTO gpkg_spatial_ref_sys
               (srs_name, srs_id, organization, organization_coordsys_id, definition)
             VALUES
               ('Undefined Cartesian', -1, 'NONE', -1, 'undefined'),
               ('Undefined geographic', 0, 'NONE', 0, 'undefined'),
               ('WGS 84 geodetic', 4326, 'EPSG', 4326,
                'GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\"],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]');",
        )
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

/// Open an existing GeoPackage read/write.
pub fn open_geopackage(path: &Path) -> Result<Connection, String> {
    if !path.exists() {
        return Err(format!("GeoPackage not found: {}", path.display()));
    }
    Connection::open(path).map_err(|error| error.to_string())
}

/// Write a feature layer as a new table in the GeoPackage.
pub fn write_layer(connection: &Connection, layer: &FeatureLayer) -> Result<(), String> {
    let table = sanitize_identifier(&layer.name);
    let srs_id = layer.epsg as i32;
    connection
        .execute(
            "INSERT OR IGNORE INTO gpkg_spatial_ref_sys
               (srs_name, srs_id, organization, organization_coordsys_id, definition)
             VALUES (?1, ?2, 'EPSG', ?2, 'EPSG:?2')",
            params![format!("EPSG:{}", layer.epsg), srs_id],
        )
        .map_err(|error| error.to_string())?;
    let mut attribute_columns = String::new();
    for field in &layer.fields {
        attribute_columns.push_str(&format!(", \"{}\" {}", sanitize_identifier(field), "TEXT"));
    }
    connection
        .execute_batch(&format!(
            "CREATE TABLE \"{table}\" (
               fid INTEGER PRIMARY KEY AUTOINCREMENT,
               geom BLOB NOT NULL{attribute_columns}
             );
             INSERT INTO gpkg_contents
               (table_name, data_type, identifier, description, srs_id)
             VALUES ('{table}', 'features', '{table}', '', {srs_id});
             INSERT INTO gpkg_geometry_columns
               (table_name, column_name, geometry_type_name, srs_id, z, m)
             VALUES ('{table}', 'geom', 'GEOMETRY', {srs_id}, 0, 0);"
        ))
        .map_err(|error| error.to_string())?;
    let columns = layer.fields.join(", ");
    let insert_sql = if layer.fields.is_empty() {
        format!("INSERT INTO \"{table}\" (geom) VALUES (?1)")
    } else {
        format!(
            "INSERT INTO \"{table}\" (geom, {columns}) VALUES (?1, {})",
            (2..=layer.fields.len() + 1)
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut statement = connection.prepare(&insert_sql).map_err(|e| e.to_string())?;
    for feature in &layer.features {
        let blob = encode_gpkg_blob(&feature.geometry, srs_id);
        let mut parameter_index = 2usize;
        for field in &layer.fields {
            let value = feature
                .properties
                .get(field)
                .map(FieldValue::to_sql_text)
                .unwrap_or_default();
            statement
                .raw_bind_parameter(parameter_index, value)
                .map_err(|e| e.to_string())?;
            parameter_index += 1;
        }
        statement
            .raw_bind_parameter(1, blob)
            .map_err(|e| e.to_string())?;
        statement.raw_execute().map_err(|e| e.to_string())?;
    }
    // Update the contents envelope from the actual layer bounds.
    if let Some(envelope) = layer.envelope() {
        connection
            .execute(
                "UPDATE gpkg_contents SET min_x = ?1, min_y = ?2, max_x = ?3, max_y = ?4
                 WHERE table_name = ?5",
                params![envelope[0], envelope[1], envelope[2], envelope[3], table],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Read one feature table back into a layer.
pub fn read_layer(connection: &Connection, table: &str) -> Result<FeatureLayer, String> {
    let table = sanitize_identifier(table);
    let (epsg, geometry_column): (i32, String) = connection
        .query_row(
            "SELECT srs_id, column_name FROM gpkg_geometry_columns WHERE table_name = ?1",
            params![table],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let mut columns_statement = connection
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(|error| error.to_string())?;
    let mut fields: Vec<String> = Vec::new();
    let mut rows = columns_statement
        .query([])
        .map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(1).map_err(|e| e.to_string())?;
        if name != "fid" && name != geometry_column {
            fields.push(name);
        }
    }
    let field_list = if fields.is_empty() {
        String::new()
    } else {
        format!(
            ", {}",
            fields
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT \"{geometry_column}\"{field_list} FROM \"{table}\" ORDER BY fid"
        ))
        .map_err(|error| error.to_string())?;
    let column_count = statement.column_count();
    let mut rows = statement.query([]).map_err(|error| error.to_string())?;
    let mut features = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let blob: Vec<u8> = row.get(0).map_err(|e| e.to_string())?;
        let geometry = decode_gpkg_blob(&blob)?;
        let mut properties = std::collections::BTreeMap::new();
        for (index, field) in fields.iter().enumerate() {
            let value: Option<String> = row.get(index + 1).map_err(|e| e.to_string())?;
            if let Some(value) = value {
                properties.insert(field.clone(), FieldValue::from_text(&value));
            }
        }
        let _ = column_count;
        features.push(Feature {
            id: features.len() as u64 + 1,
            geometry,
            properties,
        });
    }
    Ok(FeatureLayer {
        name: table,
        epsg: epsg.max(0) as u16,
        fields,
        features,
    })
}

/// Names of every feature table in the container.
pub fn feature_tables(connection: &Connection) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT table_name FROM gpkg_contents WHERE data_type = 'features' ORDER BY table_name",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement.query([]).map_err(|error| error.to_string())?;
    let mut tables = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        tables.push(row.get::<_, String>(0).map_err(|e| e.to_string())?);
    }
    Ok(tables)
}

fn sanitize_identifier(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "layer".to_string()
    } else {
        cleaned
    }
}
