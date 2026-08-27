//! Typed catalog of coordinate reference systems.
//!
//! The catalog layers axis order, linear units, and CRS kind on top of the
//! bundled EPSG definitions. Transformations consult it before running so
//! unit and axis mistakes surface as explicit errors instead of silent
//! 30-centimeter-per-foot drift (the classic US-survey-foot trap).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Kind of a coordinate reference system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsKind {
    /// Geographic latitude/longitude (degrees).
    Geographic2d,
    /// Projected easting/northing.
    Projected2d,
    /// Vertical height reference (heights above a datum).
    Vertical,
    /// Horizontal + vertical pair (an EPSG compound CRS).
    Compound,
}

/// Linear (or angular) unit of a CRS axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisUnit {
    Metre,
    /// US survey foot: exactly 1200/3937 metres.
    UsSurveyFoot,
    /// International foot: exactly 0.3048 metres.
    InternationalFoot,
    Degree,
}

impl AxisUnit {
    /// Exact metres-per-unit factor; `Degree` has no linear factor.
    pub fn metres_per_unit(self) -> Option<f64> {
        match self {
            Self::Metre => Some(1.0),
            Self::UsSurveyFoot => Some(1200.0 / 3937.0),
            Self::InternationalFoot => Some(0.3048),
            Self::Degree => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Metre => "metre",
            Self::UsSurveyFoot => "US survey foot",
            Self::InternationalFoot => "international foot",
            Self::Degree => "degree",
        }
    }
}

/// Storage order of the horizontal axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisOrder {
    /// EPSG-defined order for geographic CRSs: latitude, longitude.
    LatLon,
    /// Mapping/GIS order: easting (x), northing (y) / lon, lat.
    EastingNorthing,
}

/// Catalog entry for one CRS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrsDefinition {
    pub epsg: u16,
    pub name: String,
    pub kind: CrsKind,
    pub unit: AxisUnit,
    pub axis_order: AxisOrder,
    /// Area of use `[west, south, east, north]` in degrees, when known.
    pub area_of_use: Option<[f64; 4]>,
    /// Vertical datum name for vertical/compound entries.
    pub vertical_datum: Option<String>,
}

/// Vertical reference model attached to a CRS or output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum VerticalReference {
    /// Heights are ellipsoidal (no geoid applied).
    Ellipsoidal,
    /// Orthometric heights above a named vertical datum (e.g. NAVD88).
    Orthometric { datum: String },
    /// A named geoid model relates ellipsoidal and orthometric heights.
    /// Grid evaluation is a future backend; the model name is recorded now
    /// so provenance is honest about what has and has not run.
    GeoidModel { name: String, applied: bool },
}

/// Registry of CRS metadata. Start from [`CrsCatalog::well_known`], extend
/// with [`CrsCatalog::register`].
#[derive(Clone, Debug, Default)]
pub struct CrsCatalog {
    entries: BTreeMap<u16, CrsDefinition>,
}

impl CrsCatalog {
    /// Catalog with curated metadata for the CRSs OpenCADStudio meets in
    /// survey and LiDAR work. Unknown EPSG codes can still be transformed
    /// (via the bundled proj4 database) but carry no curated metadata.
    pub fn well_known() -> Self {
        let mut catalog = Self::default();
        // WGS 84 geographic.
        catalog.register(CrsDefinition {
            epsg: 4326,
            name: "WGS 84 (geographic)".to_string(),
            kind: CrsKind::Geographic2d,
            unit: AxisUnit::Degree,
            axis_order: AxisOrder::LatLon,
            area_of_use: Some([-180.0, -90.0, 180.0, 90.0]),
            vertical_datum: None,
        });
        // Web Mercator (metres; non-conformal but ubiquitous for basemaps).
        catalog.register(CrsDefinition {
            epsg: 3857,
            name: "WGS 84 / Pseudo-Mercator".to_string(),
            kind: CrsKind::Projected2d,
            unit: AxisUnit::Metre,
            axis_order: AxisOrder::EastingNorthing,
            area_of_use: Some([-180.0, -85.06, 180.0, 85.06]),
            vertical_datum: None,
        });
        // Massachusetts mainland, NAD83 (1983), metres.
        catalog.register(CrsDefinition {
            epsg: 26986,
            name: "NAD83 / Massachusetts Mainland (metres)".to_string(),
            kind: CrsKind::Projected2d,
            unit: AxisUnit::Metre,
            axis_order: AxisOrder::EastingNorthing,
            area_of_use: Some([-73.5, 41.46, -69.86, 42.88]),
            vertical_datum: None,
        });
        // Massachusetts mainland, NAD83 (2011), US survey feet — the Boston
        // LiDAR delivery grid.
        catalog.register(CrsDefinition {
            epsg: 6492,
            name: "NAD83(2011) / Massachusetts Mainland (US survey feet)".to_string(),
            kind: CrsKind::Projected2d,
            unit: AxisUnit::UsSurveyFoot,
            axis_order: AxisOrder::EastingNorthing,
            area_of_use: Some([-73.5, 41.46, -69.86, 42.88]),
            vertical_datum: None,
        });
        // Older NAD83 foot grid used by many MassGIS layers.
        catalog.register(CrsDefinition {
            epsg: 2249,
            name: "NAD83 / Massachusetts Mainland (US survey feet)".to_string(),
            kind: CrsKind::Projected2d,
            unit: AxisUnit::UsSurveyFoot,
            axis_order: AxisOrder::EastingNorthing,
            area_of_use: Some([-73.5, 41.46, -69.86, 42.88]),
            vertical_datum: None,
        });
        // UTM 19N (NAD83), common for New England collections.
        catalog.register(CrsDefinition {
            epsg: 26919,
            name: "NAD83 / UTM zone 19N".to_string(),
            kind: CrsKind::Projected2d,
            unit: AxisUnit::Metre,
            axis_order: AxisOrder::EastingNorthing,
            area_of_use: Some([-72.0, 0.0, -66.0, 84.0]),
            vertical_datum: None,
        });
        catalog.register(CrsDefinition {
            epsg: 5703,
            name: "NAVD88 height (orthometric)".to_string(),
            kind: CrsKind::Vertical,
            unit: AxisUnit::Metre,
            axis_order: AxisOrder::EastingNorthing,
            area_of_use: Some([-172.0, 18.0, -66.0, 72.0]),
            vertical_datum: Some("NAVD88".to_string()),
        });
        catalog
    }

    pub fn register(&mut self, definition: CrsDefinition) {
        self.entries.insert(definition.epsg, definition);
    }

    pub fn get(&self, epsg: u16) -> Option<&CrsDefinition> {
        self.entries.get(&epsg)
    }

    /// Metadata lookup with a fallback built from the bundled proj4
    /// database: the unit string of the projection decides the axis unit.
    pub fn get_or_infer(&self, epsg: u16) -> Result<CrsDefinition, String> {
        if let Some(definition) = self.entries.get(&epsg) {
            return Ok(definition.clone());
        }
        let definition = crs_definitions::from_code(epsg)
            .ok_or_else(|| format!("EPSG:{epsg} is not in the bundled CRS database"))?;
        let projection =
            proj4rs::Proj::from_epsg_code(epsg).map_err(|error| format!("EPSG:{epsg}: {error}"))?;
        let geographic = projection.is_latlong();
        let unit = if geographic {
            AxisUnit::Degree
        } else {
            match projection.units() {
                "us-ft" => AxisUnit::UsSurveyFoot,
                "ft" => AxisUnit::InternationalFoot,
                _ => AxisUnit::Metre,
            }
        };
        Ok(CrsDefinition {
            epsg,
            name: format!("EPSG:{epsg}"),
            kind: if geographic {
                CrsKind::Geographic2d
            } else {
                CrsKind::Projected2d
            },
            unit,
            axis_order: if geographic {
                AxisOrder::LatLon
            } else {
                AxisOrder::EastingNorthing
            },
            area_of_use: None,
            vertical_datum: None,
        })
        .map(|inferred| {
            let _ = &definition;
            inferred
        })
    }
}
