//! ArcGIS FeatureServer reference providers for urban classification.
//!
//! `BostonArcGisProvider` mirrors the Python oracle's endpoints, query
//! envelope, paging, retry, and exact-response cache so native runs are
//! reproducible against the same authoritative layers. `CustomArcGisProvider`
//! exposes the same machinery for user-configured servers, field maps, and
//! source CRS with reprojection into the cloud CRS before fusion.

use crate::urban::{
    layer_cache_path, parse_geojson_collection, ReferenceCollection, ReferenceGeometry, UrbanLayer,
    UrbanReferenceProvider,
};
use crate::Error;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    thread::sleep,
    time::Duration,
};

const USER_AGENT: &str = "OpenCADStudio-UPCP-Adapter/1.0";
const OBJECT_ID_PAGE: usize = 1000;

/// HTTP transport seam so query/paging/retry logic is testable offline.
pub trait ArcGisTransport: Send {
    /// POST a form-encoded request and return the response body.
    fn post_form(&mut self, url: &str, form: &[(String, String)]) -> Result<String, Error>;
}

/// Production transport over `ureq` with a 120-second per-request timeout.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl ArcGisTransport for UreqTransport {
    fn post_form(&mut self, url: &str, form: &[(String, String)]) -> Result<String, Error> {
        let pairs: Vec<(&str, &str)> = form
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let mut response = self
            .agent
            .post(url)
            .header("User-Agent", USER_AGENT)
            .send_form(pairs)
            .map_err(|error| Error::Urban(format!("ArcGIS request failed: {error}")))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| Error::Urban(format!("ArcGIS response read failed: {error}")))
    }
}

/// Retry shape shared with the oracle: 5 attempts, exponential backoff
/// capped at 15 seconds.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub backoff_cap_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 5,
            backoff_cap_secs: 15,
        }
    }
}

impl RetryPolicy {
    fn delay_secs(&self, attempt: u32) -> u64 {
        (1u64 << attempt.min(4)).min(self.backoff_cap_secs)
    }
}

/// One FeatureServer layer endpoint and its query profile.
#[derive(Clone, Debug)]
pub struct ArcGisEndpoint {
    pub query_url: String,
    pub out_fields: String,
    /// Envelope margin in CRS units (survey feet for EPSG:6492).
    pub margin: f64,
    pub where_clause: String,
    pub source_wkid: u16,
}

/// Authoritative Boston endpoints, matching the Python oracle exactly.
pub fn boston_endpoint(layer: UrbanLayer) -> ArcGisEndpoint {
    match layer {
        UrbanLayer::Buildings => ArcGisEndpoint {
            query_url: "https://gis.bostonplans.org/hosting/rest/services/Boston_Buildings/FeatureServer/9/query".to_string(),
            out_fields: "OBJECTID,GRND_ELEV_2010,ROOF_ELEV_2010,BLDG_HGT_2010".to_string(),
            margin: 10.0,
            where_clause: "1=1".to_string(),
            source_wkid: 6492,
        },
        UrbanLayer::Roads => ArcGisEndpoint {
            query_url: "https://services.arcgis.com/sFnw0xNflSi8J0uh/arcgis/rest/services/All_Boston_Roads/FeatureServer/0/query".to_string(),
            out_fields: "OBJECTID,CLASS,SURFACE_WD,NUM_LANES,F_CLASS_STR".to_string(),
            margin: 100.0,
            where_clause: "1=1".to_string(),
            source_wkid: 6492,
        },
        UrbanLayer::Trees => ArcGisEndpoint {
            query_url: "https://services.arcgis.com/sFnw0xNflSi8J0uh/arcgis/rest/services/Primary_Street_Trees_Public/FeatureServer/0/query".to_string(),
            out_fields: "FID,Species,Alive,TreeThere".to_string(),
            margin: 12.0,
            where_clause: "TreeThere IN ('Y','Yes') AND (Alive IS NULL OR Alive <> 'N')".to_string(),
            source_wkid: 6492,
        },
    }
}

/// Boston Planning buildings, MassDOT roads, and public street trees.
pub struct BostonArcGisProvider {
    transport: Box<dyn ArcGisTransport>,
    retry: RetryPolicy,
}

impl BostonArcGisProvider {
    pub fn new() -> Self {
        Self {
            transport: Box::new(UreqTransport::new()),
            retry: RetryPolicy::default(),
        }
    }

    fn with_transport(transport: Box<dyn ArcGisTransport>, retry: RetryPolicy) -> Self {
        Self { transport, retry }
    }
}

impl Default for BostonArcGisProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl UrbanReferenceProvider for BostonArcGisProvider {
    fn load(
        &mut self,
        layer: UrbanLayer,
        tile_stem: &str,
        bounds: [f64; 4],
        references_dir: &Path,
        use_cache: bool,
    ) -> Result<ReferenceCollection, Error> {
        query_layer(
            &boston_endpoint(layer),
            layer,
            tile_stem,
            bounds,
            references_dir,
            use_cache,
            self.transport.as_mut(),
            self.retry,
            &WidthFieldMap::boston(),
            None,
        )
    }
}

/// Property names supplying road widths when the server fields differ from
/// MassDOT's `SURFACE_WD` / `NUM_LANES` / `CLASS`.
#[derive(Clone, Debug)]
pub struct WidthFieldMap {
    pub surface_wd: String,
    pub num_lanes: String,
    pub road_class: String,
}

impl WidthFieldMap {
    pub fn boston() -> Self {
        Self {
            surface_wd: "SURFACE_WD".to_string(),
            num_lanes: "NUM_LANES".to_string(),
            road_class: "CLASS".to_string(),
        }
    }

    fn canonicalize(&self, properties: &serde_json::Value) -> serde_json::Value {
        let Some(object) = properties.as_object() else {
            return properties.clone();
        };
        let mut mapped = serde_json::Map::new();
        for (key, value) in object {
            let canonical = if key == &self.surface_wd {
                "SURFACE_WD"
            } else if key == &self.num_lanes {
                "NUM_LANES"
            } else if key == &self.road_class {
                "CLASS"
            } else {
                mapped.insert(key.clone(), value.clone());
                continue;
            };
            mapped.insert(canonical.to_string(), value.clone());
        }
        serde_json::Value::Object(mapped)
    }
}

/// User-configured endpoints plus CRS policy for non-Boston servers.
#[derive(Clone, Debug, Default)]
pub struct CustomArcGisConfig {
    pub buildings: Option<ArcGisEndpoint>,
    pub roads: Option<ArcGisEndpoint>,
    pub trees: Option<ArcGisEndpoint>,
    pub field_map: Option<WidthFieldMap>,
    /// Cloud CRS the masks must be expressed in; `None` keeps the source CRS.
    pub target_wkid: Option<u16>,
}

pub struct CustomArcGisProvider {
    config: CustomArcGisConfig,
    transport: Box<dyn ArcGisTransport>,
    retry: RetryPolicy,
}

impl CustomArcGisProvider {
    pub fn new(config: CustomArcGisConfig) -> Self {
        Self {
            config,
            transport: Box::new(UreqTransport::new()),
            retry: RetryPolicy::default(),
        }
    }

    fn with_transport(
        config: CustomArcGisConfig,
        transport: Box<dyn ArcGisTransport>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            config,
            transport,
            retry,
        }
    }
}

impl UrbanReferenceProvider for CustomArcGisProvider {
    fn load(
        &mut self,
        layer: UrbanLayer,
        tile_stem: &str,
        bounds: [f64; 4],
        references_dir: &Path,
        use_cache: bool,
    ) -> Result<ReferenceCollection, Error> {
        let endpoint = match layer {
            UrbanLayer::Buildings => self.config.buildings.clone(),
            UrbanLayer::Roads => self.config.roads.clone(),
            UrbanLayer::Trees => self.config.trees.clone(),
        };
        let Some(endpoint) = endpoint else {
            return Ok(ReferenceCollection::default());
        };
        let field_map = self
            .config
            .field_map
            .clone()
            .unwrap_or_else(WidthFieldMap::boston);
        query_layer(
            &endpoint,
            layer,
            tile_stem,
            bounds,
            references_dir,
            use_cache,
            self.transport.as_mut(),
            self.retry,
            &field_map,
            self.config.target_wkid,
        )
    }
}

#[derive(Serialize)]
struct CachedCrsName {
    name: String,
}

#[derive(Serialize)]
struct CachedCrs {
    #[serde(rename = "type")]
    kind: &'static str,
    properties: CachedCrsName,
}

#[derive(Serialize)]
struct CachedMetadata {
    query_url: String,
    queried_utc: String,
    bounds: [f64; 4],
    margin: f64,
    #[serde(rename = "where")]
    where_clause: String,
    object_id_count: usize,
}

#[derive(Serialize)]
struct CachedCollection {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
    crs: CachedCrs,
    features: Vec<serde_json::Value>,
    metadata: CachedMetadata,
}

fn post_json_with_retries(
    transport: &mut dyn ArcGisTransport,
    url: &str,
    form: &[(String, String)],
    retry: RetryPolicy,
) -> Result<serde_json::Value, Error> {
    let mut last_error = String::new();
    for attempt in 1..=retry.attempts {
        if attempt > 1 {
            sleep(Duration::from_secs(retry.delay_secs(attempt - 1)));
        }
        match transport.post_form(url, form) {
            Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(value) => {
                    if let Some(error) = value.get("error") {
                        return Err(Error::Urban(format!("ArcGIS error response: {error}")));
                    }
                    return Ok(value);
                }
                Err(error) => last_error = format!("invalid JSON response: {error}"),
            },
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(Error::Urban(format!(
        "ArcGIS query failed after {} attempts: {}",
        retry.attempts, last_error
    )))
}

/// Query one layer by envelope, page by object id, cache the exact response,
/// and return features expressed in `target_wkid` (or the source CRS).
#[allow(clippy::too_many_arguments)]
fn query_layer(
    endpoint: &ArcGisEndpoint,
    layer: UrbanLayer,
    tile_stem: &str,
    bounds: [f64; 4],
    references_dir: &Path,
    use_cache: bool,
    transport: &mut dyn ArcGisTransport,
    retry: RetryPolicy,
    field_map: &WidthFieldMap,
    target_wkid: Option<u16>,
) -> Result<ReferenceCollection, Error> {
    let cache_path = layer_cache_path(references_dir, tile_stem, layer);
    if use_cache && cache_path.is_file() {
        let text = fs::read_to_string(&cache_path).map_err(Error::Io)?;
        let mut collection = parse_geojson_collection(&text).map_err(Error::Urban)?;
        collection.from_cache = true;
        return Ok(collection);
    }

    let envelope = serde_json::json!({
        "xmin": bounds[0] - endpoint.margin,
        "ymin": bounds[1] - endpoint.margin,
        "xmax": bounds[2] + endpoint.margin,
        "ymax": bounds[3] + endpoint.margin,
        "spatialReference": {"wkid": endpoint.source_wkid},
    })
    .to_string();
    let spatial: Vec<(String, String)> = vec![
        ("where".to_string(), endpoint.where_clause.clone()),
        ("geometry".to_string(), envelope),
        (
            "geometryType".to_string(),
            "esriGeometryEnvelope".to_string(),
        ),
        ("inSR".to_string(), endpoint.source_wkid.to_string()),
        (
            "spatialRel".to_string(),
            "esriSpatialRelIntersects".to_string(),
        ),
    ];

    let ids_result = post_json_with_retries(
        transport,
        &endpoint.query_url,
        &[
            spatial.clone(),
            vec![
                ("returnIdsOnly".to_string(), "true".to_string()),
                ("returnGeometry".to_string(), "false".to_string()),
                ("f".to_string(), "json".to_string()),
            ],
        ]
        .concat(),
        retry,
    )?;
    let mut object_ids: Vec<i64> = ids_result
        .get("objectIds")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_i64())
                .collect::<Vec<i64>>()
        })
        .unwrap_or_default();
    object_ids.sort_unstable();

    let mut features: Vec<serde_json::Value> = Vec::new();
    for group in object_ids.chunks(OBJECT_ID_PAGE) {
        let ids_csv = group
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<String>>()
            .join(",");
        let result = post_json_with_retries(
            transport,
            &endpoint.query_url,
            &[
                spatial.clone(),
                vec![
                    ("objectIds".to_string(), ids_csv),
                    ("outFields".to_string(), endpoint.out_fields.clone()),
                    ("returnGeometry".to_string(), "true".to_string()),
                    ("outSR".to_string(), endpoint.source_wkid.to_string()),
                    ("f".to_string(), "geojson".to_string()),
                ],
            ]
            .concat(),
            retry,
        )?;
        if let Some(batch) = result.get("features").and_then(|value| value.as_array()) {
            features.extend(batch.iter().cloned());
        }
    }

    let cached = CachedCollection {
        kind: "FeatureCollection",
        name: format!("{tile_stem}.{}", layer.file_suffix()),
        crs: CachedCrs {
            kind: "name",
            properties: CachedCrsName {
                name: format!("EPSG:{}", endpoint.source_wkid),
            },
        },
        features: features.clone(),
        metadata: CachedMetadata {
            query_url: endpoint.query_url.clone(),
            queried_utc: crate::urban::iso_utc(crate::urban::unix_ms_now()),
            bounds,
            margin: endpoint.margin,
            where_clause: endpoint.where_clause.clone(),
            object_id_count: object_ids.len(),
        },
    };
    let text = serde_json::to_string(&cached)
        .map_err(|error| Error::Urban(format!("cannot serialize reference cache: {error}")))?;
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).map_err(Error::Io)?;
    }
    let temporary = cache_path.with_extension("geojson.partial");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)
        .map_err(Error::Io)?;
    file.write_all(text.as_bytes()).map_err(Error::Io)?;
    drop(file);
    fs::rename(&temporary, &cache_path).map_err(Error::Io)?;

    let mut collection = parse_geojson_collection(&text).map_err(Error::Urban)?;
    if let Some(target_wkid) = target_wkid {
        if target_wkid != endpoint.source_wkid {
            reproject_collection(&mut collection, endpoint.source_wkid, target_wkid)?;
        }
    }
    for feature in &mut collection.features {
        feature.properties = field_map.canonicalize(&feature.properties);
    }
    Ok(collection)
}

/// Reproject every feature vertex from the source CRS into the cloud CRS.
fn reproject_collection(
    collection: &mut ReferenceCollection,
    source_epsg: u16,
    target_epsg: u16,
) -> Result<(), Error> {
    let project = |x: f64, y: f64| -> Result<(f64, f64), Error> {
        crate::reproject_xy(source_epsg, target_epsg, x, y).ok_or_else(|| {
            Error::Crs(format!(
                "cannot reproject reference geometry from EPSG:{source_epsg} to EPSG:{target_epsg}"
            ))
        })
    };
    for feature in &mut collection.features {
        match &mut feature.geometry {
            ReferenceGeometry::Point(point) => {
                let (x, y) = project(point[0], point[1])?;
                *point = [x, y];
            }
            ReferenceGeometry::LineString(points) => {
                for point in points.iter_mut() {
                    let (x, y) = project(point[0], point[1])?;
                    *point = [x, y];
                }
            }
            ReferenceGeometry::MultiLineString(lines) => {
                for points in lines.iter_mut() {
                    for point in points.iter_mut() {
                        let (x, y) = project(point[0], point[1])?;
                        *point = [x, y];
                    }
                }
            }
            ReferenceGeometry::Polygon(rings) => {
                for ring in rings.iter_mut() {
                    for point in ring.iter_mut() {
                        let (x, y) = project(point[0], point[1])?;
                        *point = [x, y];
                    }
                }
            }
            ReferenceGeometry::MultiPolygon(polygons) => {
                for rings in polygons.iter_mut() {
                    for ring in rings.iter_mut() {
                        for point in ring.iter_mut() {
                            let (x, y) = project(point[0], point[1])?;
                            *point = [x, y];
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// Fake transport serving scripted responses per request.
    struct ScriptedTransport {
        responses: Vec<Result<String, String>>,
        seen: Vec<(String, Vec<(String, String)>)>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<Result<String, String>>) -> Self {
            Self {
                responses,
                seen: Vec::new(),
            }
        }
    }

    impl ArcGisTransport for ScriptedTransport {
        fn post_form(&mut self, url: &str, form: &[(String, String)]) -> Result<String, Error> {
            let index = self.seen.len();
            self.seen.push((url.to_string(), form.to_vec()));
            match self.responses.get(index) {
                Some(Ok(body)) => Ok(body.clone()),
                Some(Err(error)) => Err(Error::Urban(error.clone())),
                None => panic!("unexpected request #{index}"),
            }
        }
    }

    fn retry_immediate() -> RetryPolicy {
        RetryPolicy {
            attempts: 3,
            backoff_cap_secs: 0,
        }
    }

    #[test]
    fn queries_pages_and_caches_exact_response() {
        let dir = std::env::temp_dir().join(format!("ocs-arcgis-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let endpoint = ArcGisEndpoint {
            query_url: "https://example.test/buildings/query".to_string(),
            out_fields: "OBJECTID".to_string(),
            margin: 10.0,
            where_clause: "1=1".to_string(),
            source_wkid: 6492,
        };
        // 1500 ids force two pages.
        let ids: Vec<i64> = (1..=1500).collect();
        let first_page: Vec<Value> = (1..=1000)
            .map(|id| {
                json!({"type":"Feature","id":id,"properties":{"OBJECTID":id},
                "geometry":{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,0]]]}})
            })
            .collect();
        let second_page: Vec<Value> = (1001..=1500)
            .map(|id| {
                json!({"type":"Feature","id":id,"properties":{"OBJECTID":id},
                "geometry":{"type":"Point","coordinates":[1,1]}})
            })
            .collect();
        let mut transport = ScriptedTransport::new(vec![
            Ok(json!({"objectIds": ids}).to_string()),
            Ok(json!({"type":"FeatureCollection","features": first_page}).to_string()),
            Ok(json!({"type":"FeatureCollection","features": second_page}).to_string()),
        ]);
        let retry = retry_immediate();
        let collection = query_layer(
            &endpoint,
            UrbanLayer::Buildings,
            "tile",
            [100.0, 200.0, 300.0, 400.0],
            &dir,
            false,
            &mut transport,
            retry,
            &WidthFieldMap::boston(),
            None,
        )
        .unwrap();
        assert_eq!(collection.features.len(), 1500);
        assert!(!collection.from_cache);

        // The probe carried the expanded envelope; pages carried objectIds.
        let probe = &transport.seen[0];
        assert!(probe.0.contains("/query"));
        let geometry = probe
            .1
            .iter()
            .find(|(key, _)| key == "geometry")
            .map(|(_, value)| value.clone())
            .unwrap();
        let envelope: Value = serde_json::from_str(&geometry).unwrap();
        assert_eq!(envelope["xmin"], json!(90.0));
        assert_eq!(envelope["ymax"], json!(410.0));
        assert_eq!(envelope["spatialReference"]["wkid"], json!(6492));
        let page = &transport.seen[1];
        assert!(page
            .1
            .iter()
            .any(|(key, value)| key == "f" && value == "geojson"));
        assert!(page.1.iter().any(|(key, _)| key == "objectIds"));

        // The cache exists and a rerun uses it without touching the network.
        let cache = dir.join("tile.buildings.geojson");
        assert!(cache.is_file());
        let cached_text = fs::read_to_string(&cache).unwrap();
        let cached: Value = serde_json::from_str(&cached_text).unwrap();
        assert_eq!(cached["type"], "FeatureCollection");
        assert_eq!(cached["metadata"]["object_id_count"], json!(1500));
        assert_eq!(cached["metadata"]["query_url"], json!(endpoint.query_url));
        assert_eq!(cached["crs"]["properties"]["name"], json!("EPSG:6492"));
        let mut empty_transport = ScriptedTransport::new(vec![]);
        let again = query_layer(
            &endpoint,
            UrbanLayer::Buildings,
            "tile",
            [0.0; 4],
            &dir,
            true,
            &mut empty_transport,
            retry_immediate(),
            &WidthFieldMap::boston(),
            None,
        )
        .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.features.len(), 1500);
    }

    #[test]
    fn retries_transient_failures() {
        let dir = std::env::temp_dir().join(format!("ocs-arcgis-retry-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let endpoint = ArcGisEndpoint {
            query_url: "https://example.test/roads/query".to_string(),
            out_fields: "OBJECTID".to_string(),
            margin: 100.0,
            where_clause: "1=1".to_string(),
            source_wkid: 6492,
        };
        let mut transport = ScriptedTransport::new(vec![
            Err("network down".to_string()),
            Err("still down".to_string()),
            Ok(json!({"objectIds": [7]}).to_string()),
            Ok(json!({"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"WIDTH_FEET":44.0},
                 "geometry":{"type":"LineString","coordinates":[[0,0],[10,0]]}}]})
            .to_string()),
        ]);
        let collection = query_layer(
            &endpoint,
            UrbanLayer::Roads,
            "tile",
            [0.0, 0.0, 1.0, 1.0],
            &dir,
            false,
            &mut transport,
            retry_immediate(),
            &WidthFieldMap {
                surface_wd: "WIDTH_FEET".to_string(),
                num_lanes: "LANES".to_string(),
                road_class: "RDCLASS".to_string(),
            },
            None,
        )
        .unwrap();
        assert_eq!(collection.features.len(), 1);
        // The custom field name was canonicalized to SURFACE_WD so the
        // width rules apply without further mapping.
        assert_eq!(
            collection.features[0].properties.get("SURFACE_WD"),
            Some(&json!(44.0))
        );
    }

    #[test]
    fn surfaces_arcgis_errors_after_retries() {
        let dir = std::env::temp_dir().join(format!("ocs-arcgis-err-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let endpoint = ArcGisEndpoint {
            query_url: "https://example.test/trees/query".to_string(),
            out_fields: "FID".to_string(),
            margin: 12.0,
            where_clause: "1=1".to_string(),
            source_wkid: 6492,
        };
        let mut transport = ScriptedTransport::new(vec![Ok(
            json!({"error": {"code": 500, "message": "boom"}}).to_string(),
        )]);
        let result = query_layer(
            &endpoint,
            UrbanLayer::Trees,
            "tile",
            [0.0; 4],
            &dir,
            false,
            &mut transport,
            retry_immediate(),
            &WidthFieldMap::boston(),
            None,
        );
        assert!(matches!(result, Err(Error::Urban(_))));
    }

    #[test]
    fn custom_provider_uses_disabled_layers() {
        let mut provider = CustomArcGisProvider::with_transport(
            CustomArcGisConfig::default(),
            Box::new(ScriptedTransport::new(vec![])),
            retry_immediate(),
        );
        let collection = provider
            .load(UrbanLayer::Trees, "tile", [0.0; 4], Path::new("refs"), true)
            .unwrap();
        assert!(collection.features.is_empty());
    }
}
