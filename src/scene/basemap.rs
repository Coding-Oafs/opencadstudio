//! Georeferenced basemap underlay: XYZ slippy-map tiles placed in world space.
//!
//! The pure math lives here so it is unit-testable without a GPU or network:
//! Web-Mercator tile bounds, the tile URL for a provider, and the reprojection
//! of a source envelope into the drawing's CRS. The renderer only consumes the
//! resulting world-space quads.

use serde::{Deserialize, Serialize};

/// Runtime + persisted configuration for the basemap underlay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BasemapSettings {
    pub provider: BasemapProvider,
    pub projection: BasemapProjection,
    /// When true, the underlay tracks the viewport (camera) and re-plans tiles
    /// at higher resolution as the user zooms in, instead of covering the whole
    /// drawing extent at a fixed zoom.
    #[serde(default)]
    pub follow_camera: bool,
    /// Source EPSG to reproject from (used when `projection` is `Epsg` or
    /// `FromLas`; `FromLas` fills this from the attached cloud).
    pub source_epsg: Option<u16>,
    /// Custom XYZ template when `provider` is `Custom`.
    pub custom_template: String,
    /// Slippy zoom level (0–22).
    pub zoom: u32,
    /// Band half-width around the drawing bounds, in meters (or target units),
    /// used to decide which tiles to fetch. Tiles are clamped to the world.
    pub opacity: f32,
}

impl Default for BasemapSettings {
    fn default() -> Self {
        Self {
            provider: BasemapProvider::Off,
            projection: BasemapProjection::FromDrawing,
            follow_camera: false,
            source_epsg: None,
            custom_template: String::new(),
            zoom: 16,
            opacity: 1.0,
        }
    }
}

impl BasemapSettings {
    /// Normalize the zoom and opacity into safe ranges.
    pub fn normalized(mut self) -> Self {
        self.zoom = self.zoom.clamp(0, 22);
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self
    }
}

/// Which basemap imagery to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BasemapProvider {
    #[default]
    Off,
    /// Esri World Imagery (no key required).
    ArcGisImagery,
    /// Esri World Street Map (no key required).
    ArcGisStreets,
    /// Google Maps Hybrid imagery — requires an API key, resolved from the
    /// `OCS_GOOGLE_MAPS_KEY` environment variable or the per-user key file
    /// (`<config>/OpenCADStudio/google_maps_key.txt`), never from source.
    GoogleHybrid,
    /// A user-supplied XYZ template (e.g. a different provider).
    Custom,
}

impl BasemapProvider {
    pub fn cache_namespace(self, custom_template: &str) -> String {
        match self {
            Self::ArcGisImagery => "arcgis-imagery".to_string(),
            Self::ArcGisStreets => "arcgis-streets".to_string(),
            Self::GoogleHybrid => "google-hybrid".to_string(),
            Self::Off => "off".to_string(),
            Self::Custom => {
                use sha2::{Digest, Sha256};
                let digest = Sha256::digest(custom_template.as_bytes());
                let hex = format!("{digest:x}");
                format!("custom-{}", &hex[..16])
            }
        }
    }

    /// The XYZ URL template for `z`/`x`/`y` placeholders. `{custom}` is filled
    /// from the user's stored template string (for `BasemapProvider::Custom`);
    /// `{key}` is filled from the resolved Google API key (for `GoogleHybrid`).
    pub fn url_template(&self) -> &'static str {
        match self {
            BasemapProvider::ArcGisImagery => {
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"
            }
            BasemapProvider::ArcGisStreets => {
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}"
            }
            BasemapProvider::GoogleHybrid => {
                "https://mt1.google.com/vt/lyrs=y&x={x}&y={y}&z={z}&key={key}"
            }
            BasemapProvider::Off | BasemapProvider::Custom => "",
        }
    }
}

/// Resolve the Google Maps API key for the `GoogleHybrid` provider.
///
/// The key is a secret and never lives in the repository. It is read from the
/// `OCS_GOOGLE_MAPS_KEY` environment variable, or — when unset — from a
/// per-user file `<config>/OpenCADStudio/google_maps_key.txt` (one line, the
/// key, optionally with surrounding whitespace).
#[cfg(not(target_arch = "wasm32"))]
pub fn google_api_key() -> Option<String> {
    if let Some(key) = std::env::var("OCS_GOOGLE_MAPS_KEY")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        return Some(key);
    }
    let path = crate::config::config_dir()?.join("google_maps_key.txt");
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(target_arch = "wasm32")]
pub fn google_api_key() -> Option<String> {
    None
}

/// Build the Google Hybrid tile URL for a given key and tile. Pure and
/// unit-testable so the key-substitution path is verified without a network.
pub fn google_tile_url(key: &str, z: u32, x: u32, y: u32) -> String {
    BasemapProvider::GoogleHybrid
        .url_template()
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
        .replace("{key}", key)
}

/// Projection for the basemap. Tiles are always served in Web Mercator
/// (EPSG:3857); this chooses how the drawing's coordinates map to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BasemapProjection {
    /// Reproject using the CRS stored on the drawing, independent of LiDAR.
    #[default]
    FromDrawing,
    /// Assume the drawing is already in Web Mercator (no reprojection).
    WebMercator,
    /// Reproject using the EPSG code taken from the attached LAS cloud.
    FromLas,
    /// Reproject using a user-supplied EPSG code.
    Epsg(u16),
}

/// A single XYZ tile request: its coordinates plus the world-space (Web
/// Mercator, EPSG:3857) extent it covers, so the caller can place a fetched
/// image quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub z: u32,
    pub x: u32,
    pub y: u32,
    /// Web-Mercator meters: [min_x, min_y, max_x, max_y].
    pub bounds: [f64; 4],
}

/// Web-Mercator world half-extent in meters (EPSG:3857).
const MERCATOR_HALF: f64 = 20_037_508.342_789_244;

/// The Web-Mercator bounds of tile (z, x, y).
pub fn tile_bounds(z: u32, x: u32, y: u32) -> [f64; 4] {
    let n = (1_u64 << z) as f64;
    let x = x as f64;
    let y = y as f64;
    [
        x / n * 2.0 * MERCATOR_HALF - MERCATOR_HALF,
        MERCATOR_HALF - (y + 1.0) / n * 2.0 * MERCATOR_HALF,
        (x + 1.0) / n * 2.0 * MERCATOR_HALF - MERCATOR_HALF,
        MERCATOR_HALF - y / n * 2.0 * MERCATOR_HALF,
    ]
}

/// The tile URL for a provider at (z, x, y), using `custom_template` when the
/// provider is `Custom`, and the resolved API key when the provider is
/// `GoogleHybrid`. Returns `None` when the provider is off, the custom template
/// is empty, or the Google key is unavailable.
pub fn tile_url(
    provider: BasemapProvider,
    z: u32,
    x: u32,
    y: u32,
    custom_template: &str,
) -> Option<String> {
    match provider {
        BasemapProvider::Off => None,
        BasemapProvider::Custom if custom_template.is_empty() => None,
        BasemapProvider::Custom => Some(
            custom_template
                .replace("{z}", &z.to_string())
                .replace("{x}", &x.to_string())
                .replace("{y}", &y.to_string()),
        ),
        BasemapProvider::GoogleHybrid => {
            let key = google_api_key()?;
            Some(google_tile_url(&key, z, x, y))
        }
        other => Some(
            other
                .url_template()
                .replace("{z}", &z.to_string())
                .replace("{x}", &x.to_string())
                .replace("{y}", &y.to_string()),
        ),
    }
}

/// Compute the Web-Mercator world bounds of a drawing envelope expressed in
/// `crs`. Returns `None` on an unavailable projection.
#[cfg(not(target_arch = "wasm32"))]
pub fn world_bounds_from_source(
    min: [f64; 2],
    max: [f64; 2],
    crs: &ocs_pointcloud::CrsInfo,
) -> Option<[f64; 4]> {
    if crs.horizontal_epsg == Some(3857) && crs.proj4.is_none() {
        return Some([min[0], min[1], max[0], max[1]]);
    }
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    const STEPS: usize = 4;
    for iy in 0..=STEPS {
        for ix in 0..=STEPS {
            let x = min[0] + (max[0] - min[0]) * ix as f64 / STEPS as f64;
            let y = min[1] + (max[1] - min[1]) * iy as f64 / STEPS as f64;
            let (tx, ty) = ocs_pointcloud::reproject_from_crs(crs, 3857, x, y)?;
            out[0] = out[0].min(tx);
            out[1] = out[1].min(ty);
            out[2] = out[2].max(tx);
            out[3] = out[3].max(ty);
        }
    }
    Some(out)
}

/// Reproject a WGS 84 longitude/latitude envelope into drawing coordinates.
/// The edges are densified because state-plane and other projected CRS edges
/// are generally curved in geographic space.
#[cfg(not(target_arch = "wasm32"))]
pub fn source_bounds_from_wgs84_area(
    area: [f64; 4],
    crs: &ocs_pointcloud::CrsInfo,
) -> Option<[f64; 4]> {
    let [west, south, east, north] = area;
    if !area.iter().all(|value| value.is_finite())
        || west >= east
        || south >= north
        || west < -180.0
        || east > 180.0
        || south < -90.0
        || north > 90.0
    {
        return None;
    }
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    const STEPS: usize = 8;
    for iy in 0..=STEPS {
        for ix in 0..=STEPS {
            if ix != 0 && ix != STEPS && iy != 0 && iy != STEPS {
                continue;
            }
            let longitude = west + (east - west) * ix as f64 / STEPS as f64;
            let latitude = south + (north - south) * iy as f64 / STEPS as f64;
            let (x, y) = ocs_pointcloud::reproject_to_crs(4326, crs, longitude, latitude)?;
            out[0] = out[0].min(x);
            out[1] = out[1].min(y);
            out[2] = out[2].max(x);
            out[3] = out[3].max(y);
        }
    }
    (out.iter().all(|value| value.is_finite()) && out[0] < out[2] && out[1] < out[3]).then_some(out)
}

/// Build a WGS 84 envelope around a longitude/latitude site center. `radius_km`
/// is approximate ground distance and is intended for initial basemap framing,
/// not survey measurement.
pub fn wgs84_radius_area(longitude: f64, latitude: f64, radius_km: f64) -> Option<[f64; 4]> {
    if !longitude.is_finite()
        || !latitude.is_finite()
        || !radius_km.is_finite()
        || !(-180.0..=180.0).contains(&longitude)
        || !(-85.0..=85.0).contains(&latitude)
        || radius_km <= 0.0
    {
        return None;
    }
    let latitude_delta = radius_km / 111.32;
    let longitude_scale = (latitude.to_radians().cos().abs() * 111.32).max(1.0);
    let longitude_delta = radius_km / longitude_scale;
    let west = (longitude - longitude_delta).max(-180.0);
    let east = (longitude + longitude_delta).min(180.0);
    let south = (latitude - latitude_delta).max(-85.051_128_779_806_6);
    let north = (latitude + latitude_delta).min(85.051_128_779_806_6);
    (west < east && south < north).then_some([west, south, east, north])
}

/// Inclusive XYZ tile range covering a Web-Mercator envelope.
fn tile_range(bounds: [f64; 4], zoom: u32) -> Option<(u32, u32, u32, u32)> {
    if zoom > 22
        || !bounds.iter().all(|value| value.is_finite())
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
    {
        return None;
    }
    let n = (1_u64 << zoom) as f64;
    let clamp_x = |x: f64| -> u32 {
        (((x + MERCATOR_HALF) / (2.0 * MERCATOR_HALF)) * n)
            .floor()
            .clamp(0.0, n - 1.0) as u32
    };
    let clamp_y = |y: f64| -> u32 {
        ((1.0 - (y + MERCATOR_HALF) / (2.0 * MERCATOR_HALF)) * n)
            .floor()
            .clamp(0.0, n - 1.0) as u32
    };
    let (x0, x1) = (clamp_x(bounds[0]), clamp_x(bounds[2]));
    let (y0, y1) = (clamp_y(bounds[3]), clamp_y(bounds[1]));
    Some((x0, x1, y0, y1))
}

/// Number of tiles at `zoom` covering `bounds`, computed without allocating.
/// A world envelope at zoom 22 covers over 17 trillion tiles, so this count
/// must always be checked before materializing a request vector.
pub fn tile_count_covering(bounds: [f64; 4], zoom: u32) -> u64 {
    let Some((x0, x1, y0, y1)) = tile_range(bounds, zoom) else {
        return 0;
    };
    u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1)
}

/// Lower `requested_zoom` until the envelope fits within `max_tiles`.
/// Returns the effective zoom and exact tile count without allocating.
pub fn zoom_for_tile_limit(
    bounds: [f64; 4],
    requested_zoom: u32,
    max_tiles: u64,
) -> Option<(u32, u64)> {
    if max_tiles == 0 {
        return None;
    }
    let mut zoom = requested_zoom.min(22);
    loop {
        let count = tile_count_covering(bounds, zoom);
        if count == 0 {
            return None;
        }
        if count <= max_tiles || zoom == 0 {
            return Some((zoom, count));
        }
        zoom -= 1;
    }
}

/// Slippy zoom whose tiles approximately match the screen at the given viewport
/// width (one tile pixel ≈ one screen pixel). `world_bounds` is the visible
/// Web-Mercator envelope in metres. Returns a value clamped to 0..22.
pub fn zoom_for_pixel_scale(world_bounds: [f64; 4], viewport_width_px: f32) -> u32 {
    const METERS_PER_PX_Z0: f64 = 156_543.033_928_040_97;
    let world_width = (world_bounds[2] - world_bounds[0]).abs();
    if world_width <= 0.0 || !world_width.is_finite() || viewport_width_px <= 0.0 {
        return 0;
    }
    let meters_per_px = world_width / viewport_width_px as f64;
    let zoom = (METERS_PER_PX_Z0 / meters_per_px).log2().floor();
    zoom.clamp(0.0, 22.0) as u32
}

/// Materialize the tiles covering `bounds` only after enforcing `max_tiles`.
/// Returns the exact required count on overflow and never performs a large
/// allocation. This is the OOM boundary for every basemap request.
pub fn tiles_covering_bounded(
    bounds: [f64; 4],
    zoom: u32,
    max_tiles: u64,
) -> Result<Vec<Tile>, u64> {
    let Some((x0, x1, y0, y1)) = tile_range(bounds, zoom) else {
        return Ok(Vec::new());
    };
    let count = u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1);
    if count > max_tiles || count > usize::MAX as u64 {
        return Err(count);
    }
    let mut tiles = Vec::with_capacity(count as usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            tiles.push(Tile {
                z: zoom,
                x,
                y,
                bounds: tile_bounds(zoom, x, y),
            });
        }
    }
    Ok(tiles)
}

/// Reproject a Web-Mercator envelope into `crs` (or the same envelope when the
/// CRS is already Web Mercator). Returns the densified target bounds, or `None`
/// when the projection is unavailable.
#[cfg(not(target_arch = "wasm32"))]
pub fn reproject_bounds_3857(bounds: [f64; 4], crs: &ocs_pointcloud::CrsInfo) -> Option<[f64; 4]> {
    if crs.horizontal_epsg == Some(3857) && crs.proj4.is_none() {
        return Some(bounds);
    }
    let [min_x, min_y, max_x, max_y] = bounds;
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    // Densify the envelope edges so curved projection boundaries are respected.
    const STEPS: usize = 8;
    for iy in 0..=STEPS {
        for ix in 0..=STEPS {
            if ix != 0 && ix != STEPS && iy != 0 && iy != STEPS {
                continue;
            }
            let x = min_x + (max_x - min_x) * ix as f64 / STEPS as f64;
            let y = min_y + (max_y - min_y) * iy as f64 / STEPS as f64;
            let (tx, ty) = ocs_pointcloud::reproject_to_crs(3857, crs, x, y)?;
            out[0] = out[0].min(tx);
            out[1] = out[1].min(ty);
            out[2] = out[2].max(tx);
            out[3] = out[3].max(ty);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_0_bounds_cover_the_whole_world() {
        let b = tile_bounds(0, 0, 0);
        assert!((b[0] + MERCATOR_HALF).abs() < 1e-6);
        assert!((b[2] - MERCATOR_HALF).abs() < 1e-6);
        assert!((b[1] + MERCATOR_HALF).abs() < 1e-6);
        assert!((b[3] - MERCATOR_HALF).abs() < 1e-6);
    }

    #[test]
    fn tile_bounds_are_contiguous_and_quarter_at_z1() {
        // The four z=1 tiles split the world in half in each axis.
        let nw = tile_bounds(1, 0, 0);
        let ne = tile_bounds(1, 1, 0);
        // x=0 is the z1 seam: west tile's right edge meets east tile's left.
        assert!(
            (nw[2] - ne[0]).abs() < 1e-6,
            "west/right edge meets east/left edge"
        );
        assert!((nw[3] - ne[3]).abs() < 1e-6, "same top edge");
        assert!(
            nw[2].abs() < 1e-6 && ne[0].abs() < 1e-6,
            "x=0 is the z1 seam"
        );
        // West tile spans the negative half, east the positive half.
        assert!((nw[0] + MERCATOR_HALF).abs() < 1e-6);
        assert!((ne[2] - MERCATOR_HALF).abs() < 1e-6);
    }

    #[test]
    fn tile_url_supports_builtin_and_custom() {
        let url = tile_url(BasemapProvider::ArcGisImagery, 3, 2, 1, "").unwrap();
        assert!(url.contains("/tile/3/1/2"), "url = {url}");
        let url = tile_url(
            BasemapProvider::Custom,
            3,
            2,
            1,
            "https://x/{z}/{x}/{y}?k=a",
        )
        .unwrap();
        assert_eq!(url, "https://x/3/2/1?k=a");
        assert!(tile_url(BasemapProvider::Off, 1, 0, 0, "").is_none());
        assert!(tile_url(BasemapProvider::Custom, 1, 0, 0, "").is_none());
    }

    #[test]
    fn site_radius_and_crs_area_bootstrap_empty_drawings() {
        let site = wgs84_radius_area(-71.0589, 42.3601, 5.0).expect("Boston site");
        assert!(site[0] < -71.0589 && site[2] > -71.0589);
        assert!(site[1] < 42.3601 && site[3] > 42.3601);

        #[cfg(not(target_arch = "wasm32"))]
        {
            let crs = ocs_pointcloud::CrsInfo {
                horizontal_epsg: Some(3857),
                ..Default::default()
            };
            let bounds = source_bounds_from_wgs84_area(site, &crs).expect("Web Mercator");
            assert!(bounds[0] < bounds[2] && bounds[1] < bounds[3]);
            let round_trip =
                world_bounds_from_source([bounds[0], bounds[1]], [bounds[2], bounds[3]], &crs)
                    .expect("round trip");
            assert!((round_trip[0] - bounds[0]).abs() < 1e-6);
            assert!((round_trip[3] - bounds[3]).abs() < 1e-6);
        }
    }

    #[test]
    fn google_tile_url_substitutes_key_and_coords() {
        let url = google_tile_url("SECRET123", 3, 2, 1);
        assert!(url.contains("lyrs=y"), "url = {url}");
        assert!(url.contains("x=2"), "url = {url}");
        assert!(url.contains("y=1"), "url = {url}");
        assert!(url.contains("z=3"), "url = {url}");
        assert!(url.contains("key=SECRET123"), "url = {url}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn reproject_3857_to_3857_is_identity() {
        let b = tile_bounds(1, 0, 0);
        let crs = ocs_pointcloud::CrsInfo {
            horizontal_epsg: Some(3857),
            ..Default::default()
        };
        assert_eq!(reproject_bounds_3857(b, &crs), Some(b));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn reproject_to_geographic_lands_in_degrees() {
        // Web Mercator origin (0,0) is roughly (0°, 0°) in EPSG:4326.
        let b = tile_bounds(0, 0, 0);
        let crs = ocs_pointcloud::CrsInfo {
            horizontal_epsg: Some(4326),
            ..Default::default()
        };
        let geo = reproject_bounds_3857(b, &crs).unwrap();
        assert!(
            geo[0] <= -170.0 && geo[1] <= -80.0,
            "SW corner in degrees: {geo:?}"
        );
        assert!(
            geo[2] >= 170.0 && geo[3] >= 80.0,
            "NE corner in degrees: {geo:?}"
        );
    }

    #[test]
    fn tiles_covering_clamps_to_world() {
        // The whole world at z=0 is exactly one tile.
        let world = tile_bounds(0, 0, 0);
        assert_eq!(tiles_covering_bounded(world, 0, 1).unwrap().len(), 1);
        // An envelope strictly inside one tile at z=2 maps to just that tile.
        let b = tile_bounds(2, 1, 1);
        let interior = [b[0] + 1.0, b[1] + 1.0, b[2] - 1.0, b[3] - 1.0];
        let covered = tiles_covering_bounded(interior, 2, 1).unwrap();
        assert_eq!(
            covered.len(),
            1,
            "interior envelope = one tile: {covered:?}"
        );
        assert_eq!((covered[0].x, covered[0].y), (1, 1));
        // A degenerate bound yields nothing.
        assert!(tiles_covering_bounded([1.0, 1.0, 0.0, 0.0], 2, 1)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn huge_tile_requests_are_counted_and_bounded_before_allocation() {
        let world = tile_bounds(0, 0, 0);
        assert_eq!(tile_count_covering(world, 22), 17_592_186_044_416);
        assert_eq!(
            tiles_covering_bounded(world, 22, 256),
            Err(17_592_186_044_416)
        );
        assert_eq!(zoom_for_tile_limit(world, 16, 64), Some((3, 64)));
        assert_eq!(tiles_covering_bounded(world, 3, 64).unwrap().len(), 64);
    }
}
