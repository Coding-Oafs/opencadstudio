//! Feature layers: geometry plus attributes, with GeoJSON coding.

use crate::geometry::{geometry_from_geojson, geometry_to_geojson, Geometry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Typed attribute value stored per feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldValue {
    Text(String),
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Null,
}

impl FieldValue {
    /// SQLite stores all attributes as TEXT in this first iteration; the
    /// typed parse keeps numbers round-tripping through the JSON layer.
    pub fn to_sql_text(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Integer(value) => value.to_string(),
            Self::Real(value) => value.to_string(),
            Self::Boolean(value) => value.to_string(),
            Self::Null => String::new(),
        }
    }

    pub fn from_text(value: &str) -> Self {
        if value.is_empty() {
            return Self::Null;
        }
        if let Some(integer) = value.parse::<i64>().ok() {
            return Self::Integer(integer);
        }
        if let Some(real) = value.parse::<f64>().ok() {
            return Self::Real(real);
        }
        match value {
            "true" => Self::Boolean(true),
            "false" => Self::Boolean(false),
            _ => Self::Text(value.to_string()),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Text(value) => Value::String(value.clone()),
            Self::Integer(value) => Value::from(*value),
            Self::Real(value) => Value::from(*value),
            Self::Boolean(value) => Value::Bool(*value),
            Self::Null => Value::Null,
        }
    }

    pub fn from_json(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(flag) => Self::Boolean(*flag),
            Value::Number(number) => {
                if let Some(integer) = number.as_i64() {
                    Self::Integer(integer)
                } else {
                    Self::Real(number.as_f64().unwrap_or_default())
                }
            }
            Value::String(text) => Self::Text(text.clone()),
            other => Self::Text(other.to_string()),
        }
    }
}

/// One feature: geometry plus attribute values keyed by field name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: u64,
    pub geometry: Geometry,
    pub properties: BTreeMap<String, FieldValue>,
}

/// A named layer of features in one CRS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureLayer {
    pub name: String,
    pub epsg: u16,
    /// Ordered attribute field names; drives GeoPackage columns.
    pub fields: Vec<String>,
    pub features: Vec<Feature>,
}

impl FeatureLayer {
    pub fn new(name: impl Into<String>, epsg: u16) -> Self {
        Self {
            name: name.into(),
            epsg,
            fields: Vec::new(),
            features: Vec::new(),
        }
    }

    /// Layer-wide `[min_x, min_y, max_x, max_y]`.
    pub fn envelope(&self) -> Option<[f64; 4]> {
        let mut envelope: Option<[f64; 4]> = None;
        for feature in &self.features {
            if let Some(bounds) = feature.geometry.envelope() {
                let merged = envelope.get_or_insert([f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
                merged[0] = merged[0].min(bounds[0]);
                merged[1] = merged[1].min(bounds[1]);
                merged[2] = merged[2].max(bounds[2]);
                merged[3] = merged[3].max(bounds[3]);
            }
        }
        envelope
    }

    /// Register a field name if new, then push a feature.
    pub fn push(&mut self, geometry: Geometry, properties: BTreeMap<String, FieldValue>) {
        for field in properties.keys() {
            if !self.fields.contains(field) {
                self.fields.push(field.clone());
            }
        }
        let id = self.features.len() as u64 + 1;
        self.features.push(Feature {
            id,
            geometry,
            properties,
        });
    }

    /// Export as a GeoJSON FeatureCollection string (RFC 7946 shapes; the
    /// layer CRS is recorded in a `crs` member when it is not EPSG:4326).
    pub fn to_geojson(&self) -> String {
        let features: Vec<Value> = self
            .features
            .iter()
            .map(|feature| {
                let properties: Map<String, Value> = feature
                    .properties
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_json()))
                    .collect();
                json!({
                    "type": "Feature",
                    "id": feature.id,
                    "geometry": geometry_to_geojson(&feature.geometry),
                    "properties": Value::Object(properties),
                })
            })
            .collect();
        let mut collection = json!({
            "type": "FeatureCollection",
            "name": self.name,
            "features": features,
        });
        if self.epsg != 4326 {
            collection["crs"] = json!({
                "type": "name",
                "properties": { "name": format!("EPSG:{}", self.epsg) }
            });
        }
        serde_json::to_string(&collection).unwrap_or_default()
    }

    /// Import the first layer of a GeoJSON FeatureCollection.
    pub fn from_geojson(text: &str, default_name: &str, default_epsg: u16) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(default_name)
            .to_string();
        let epsg = value
            .pointer("/crs/properties/name")
            .and_then(Value::as_str)
            .and_then(|text| text.rsplit(':').next())
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(default_epsg);
        let mut layer = Self::new(name, epsg);
        let features = value
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| "expected a FeatureCollection with features".to_string())?;
        for feature in features {
            let Some(geometry_value) = feature.get("geometry") else {
                continue;
            };
            let geometry = geometry_from_geojson(geometry_value)?;
            let mut properties = BTreeMap::new();
            if let Some(object) = feature.get("properties").and_then(Value::as_object) {
                for (key, value) in object {
                    properties.insert(key.clone(), FieldValue::from_json(value));
                }
            }
            layer.push(geometry, properties);
        }
        Ok(layer)
    }
}
