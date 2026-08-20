//! `BASEMAP` command: georeferenced XYZ-slippy imagery underlay.
//!
//! Settings (provider, projection, zoom) live on the app; the command fetches
//! the tiles covering the drawing (or attached point-cloud) bounds and installs
//! them as session-only `ImageModel`s in world space, drawn behind content.

use super::{Message, OpenCADStudio};
use crate::command::CadCommand;
use crate::scene::basemap::{self, BasemapProjection, BasemapProvider, Tile};
use crate::scene::{resolve_image, ImageModel};
use iced::Task;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

static NEXT_BASEMAP_JOB: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct BasemapJob {
    pub id: u64,
    pub tab_id: u64,
    pub total: usize,
    pub completed: Arc<AtomicUsize>,
    pub failed: Arc<AtomicUsize>,
    pub cancel: Arc<AtomicBool>,
}

impl BasemapJob {
    fn new(tab_id: u64, total: usize) -> Self {
        Self {
            id: NEXT_BASEMAP_JOB.fetch_add(1, Ordering::Relaxed),
            tab_id,
            total,
            completed: Arc::new(AtomicUsize::new(0)),
            failed: Arc::new(AtomicUsize::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn snapshot(&self) -> (usize, usize, usize) {
        (
            self.completed.load(Ordering::Relaxed).min(self.total),
            self.total,
            self.failed.load(Ordering::Relaxed),
        )
    }
}

/// Offload a closure to a worker thread and map its result to a `Message`.
fn background_task<T, F, M>(work: F, map: M) -> Task<Message>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    M: FnOnce(T) -> Message + Send + 'static,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(work());
        });
        Task::perform(
            async move { receiver.await.expect("basemap worker dropped") },
            map,
        )
    }
    #[cfg(target_arch = "wasm32")]
    {
        Task::perform(async move { work() }, map)
    }
}

/// The result of a background tile fetch: the tile list + decoded quads.
#[derive(Debug, Clone)]
pub struct BasemapLoaded {
    pub job_id: u64,
    pub tab_id: u64,
    pub images: Vec<ImageModel>,
    pub requested: usize,
    pub failed: usize,
    /// Fit only when a manual envelope is the sole spatial extent in an empty
    /// drawing; normal drawings retain the user's current view.
    pub fit_bounds: Option<[f64; 4]>,
}

impl OpenCADStudio {
    /// `BASEMAP` (no args): toggle the underlay on/off at the stored settings.
    pub(super) fn basemap_toggle(&mut self) -> Task<Message> {
        let i = self.active_tab;
        let tab_id = self.tabs[i].id;
        if self.basemap.provider == BasemapProvider::Off {
            self.basemap.provider = BasemapProvider::ArcGisImagery;
        } else {
            self.basemap.provider = BasemapProvider::Off;
        }
        self.sync_basemap_dropdown();
        self.command_line.push_output(
            crate::t!(if self.basemap.provider == BasemapProvider::Off {
                "Basemap: off."
            } else {
                "Basemap: on (ArcGIS World Imagery)."
            })
            .as_ref(),
        );
        self.save_config();
        self.refresh_basemap(tab_id)
    }

    /// `BASEMAP <subcommand> ...` parser.
    pub(super) fn basemap_command(&mut self, cmd: &str) -> Task<Message> {
        let rest = cmd.trim_start_matches("BASEMAP").trim();
        let mut parts = rest.split_whitespace();
        let Some(sub) = parts.next() else {
            return self.basemap_toggle();
        };
        let i = self.active_tab;
        let tab_id = self.tabs[i].id;
        match sub.to_ascii_uppercase().as_str() {
            "OFF" | "NONE" => {
                self.basemap.provider = BasemapProvider::Off;
                self.sync_basemap_dropdown();
                self.command_line
                    .push_output(crate::t!("Basemap: off.").as_ref());
                self.save_config();
                self.clear_basemap();
                return Task::none();
            }
            "ARCGIS" | "IMAGERY" => {
                self.basemap.provider = BasemapProvider::ArcGisImagery;
                self.command_line
                    .push_output(crate::t!("Basemap: ArcGIS World Imagery.").as_ref());
            }
            "STREETS" => {
                self.basemap.provider = BasemapProvider::ArcGisStreets;
                self.command_line
                    .push_output(crate::t!("Basemap: ArcGIS World Street Map.").as_ref());
            }
            "GOOGLE" | "HYBRID" => {
                // Google Hybrid needs an API key (never stored in source). If
                // none is resolvable, tell the user where to put it instead of
                // silently fetching nothing.
                if crate::scene::basemap::google_api_key().is_none() {
                    self.command_line.push_error(
                        "Basemap: Google Hybrid needs an API key. Set OCS_GOOGLE_MAPS_KEY or write it to the OpenCADStudio config google_maps_key.txt file.",
                    );
                    return Task::none();
                }
                self.basemap.provider = BasemapProvider::GoogleHybrid;
                self.command_line
                    .push_output(crate::t!("Basemap: Google Hybrid.").as_ref());
            }
            "CUSTOM" => {
                let template = parts.collect::<Vec<_>>().join(" ");
                if template.is_empty() {
                    self.command_line.push_error(
                        "BASEMAP CUSTOM <template> — an XYZ template with {z} {x} {y}.",
                    );
                    return Task::none();
                }
                self.basemap.provider = BasemapProvider::Custom;
                self.basemap.custom_template = template;
                self.command_line
                    .push_output(crate::t!("Basemap: custom XYZ template.").as_ref());
            }
            "PROJ" | "PROJECTION" => {
                let arg = parts.next().unwrap_or("").to_ascii_uppercase();
                match arg.as_str() {
                    "DRAWING" | "CRS" => {
                        self.basemap.projection = BasemapProjection::FromDrawing;
                        self.command_line.push_output(
                            crate::t!("Basemap projection: from drawing CRS.").as_ref(),
                        );
                    }
                    "DEFAULT" | "3857" | "WEBMERCATOR" | "MERCATOR" => {
                        self.basemap.projection = BasemapProjection::WebMercator;
                        self.basemap.source_epsg = None;
                        self.command_line.push_output(
                            crate::t!("Basemap projection: default (Web Mercator).").as_ref(),
                        );
                    }
                    "LAS" | "CLOUD" | "POINTCLOUD" => {
                        self.basemap.projection = BasemapProjection::FromLas;
                        self.command_line.push_output(
                            crate::t!("Basemap projection: from the attached LAS cloud.").as_ref(),
                        );
                    }
                    other => {
                        if let Ok(epsg) = other.parse::<u16>() {
                            self.basemap.projection = BasemapProjection::Epsg(epsg);
                            self.basemap.source_epsg = Some(epsg);
                            self.command_line.push_output(
                                crate::tf!("Basemap projection: EPSG:{epsg}.").as_ref(),
                            );
                        } else {
                            self.command_line
                                .push_error("BASEMAP PROJ <DRAWING|DEFAULT|LAS|<epsg>>.");
                            return Task::none();
                        }
                    }
                }
            }
            "ZOOM" => {
                if let Some(z) = parts.next().and_then(|v| v.parse::<u32>().ok()) {
                    self.basemap.zoom = z.clamp(0, 22);
                    self.command_line
                        .push_output(crate::tf!("Basemap zoom: {}.", self.basemap.zoom).as_ref());
                } else {
                    self.command_line.push_error("BASEMAP ZOOM <0-22>.");
                    return Task::none();
                }
            }
            "ZOOMIN" | "ZOOMOUT" => {
                let delta: i32 = if sub.eq_ignore_ascii_case("ZOOMIN") {
                    1
                } else {
                    -1
                };
                self.basemap.zoom = (self.basemap.zoom as i32 + delta).clamp(0, 22) as u32;
                self.command_line
                    .push_output(crate::tf!("Basemap zoom: {}.", self.basemap.zoom).as_ref());
            }
            "CENTER" | "LOCATION" | "LOCATE" => {
                let values = parts.collect::<Vec<_>>();
                if values.is_empty() {
                    use crate::command::ValuePromptCommand;
                    let command = ValuePromptCommand::new(
                        "BASEMAP CENTER",
                        "BASEMAP CENTER  Enter longitude latitude [radius-km]:",
                    );
                    self.command_line.push_info(&command.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(command));
                    return Task::none();
                }
                let parsed = values
                    .iter()
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>();
                let Ok(values) = parsed else {
                    self.command_line.push_error(
                        "BASEMAP CENTER <longitude> <latitude> [radius-km] (example: BASEMAP CENTER -71.0589 42.3601 5).",
                    );
                    return Task::none();
                };
                if !(2..=3).contains(&values.len()) {
                    self.command_line.push_error(
                        "BASEMAP CENTER <longitude> <latitude> [radius-km] (example: BASEMAP CENTER -71.0589 42.3601 5).",
                    );
                    return Task::none();
                }
                let radius_km = values.get(2).copied().unwrap_or(5.0);
                let Some(area) = basemap::wgs84_radius_area(values[0], values[1], radius_km) else {
                    self.command_line.push_error(
                        "BASEMAP CENTER requires longitude -180..180, latitude -85..85, and a positive radius in km.",
                    );
                    return Task::none();
                };
                let Some(crs) = self.tabs[i].spatial.drawing_crs.as_ref() else {
                    self.command_line.push_error(
                        "BASEMAP CENTER needs a drawing CRS first; use CRS <epsg>, then enter the site longitude/latitude.",
                    );
                    return Task::none();
                };
                #[cfg(not(target_arch = "wasm32"))]
                let drawing_bounds = basemap::source_bounds_from_wgs84_area(
                    area,
                    &ocs_pointcloud::CrsInfo {
                        horizontal_epsg: Some(crs.epsg),
                        ..Default::default()
                    },
                );
                #[cfg(target_arch = "wasm32")]
                let drawing_bounds = (crs.epsg == 4326).then_some(area);
                let Some(bounds) = drawing_bounds else {
                    self.command_line.push_error(
                        "BASEMAP CENTER could not transform that site into the drawing CRS.",
                    );
                    return Task::none();
                };
                self.tabs[i].spatial.basemap_bounds = Some(bounds);
                self.basemap.projection = BasemapProjection::FromDrawing;
                self.persist_spatial_settings(i);
                self.command_line.push_output(
                    format!(
                        "Basemap: site centered at {:.6}, {:.6} with {:.2} km radius; loading imagery.",
                        values[0], values[1], radius_km
                    )
                    .as_str(),
                );
            }
            "BOUNDS" => {
                let values = parts.collect::<Vec<_>>();
                if values.len() == 1
                    && matches!(values[0].to_ascii_uppercase().as_str(), "CLEAR" | "UNSET")
                {
                    self.tabs[i].spatial.basemap_bounds = None;
                    self.persist_spatial_settings(i);
                    self.command_line
                        .push_output("Basemap: manual drawing bounds cleared.");
                    return Task::none();
                }
                let parsed = values
                    .iter()
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>();
                let Ok(values) = parsed else {
                    self.command_line.push_error(
                        "BASEMAP BOUNDS <min-x> <min-y> <max-x> <max-y> (drawing CRS coordinates).",
                    );
                    return Task::none();
                };
                if values.len() != 4
                    || values.iter().any(|value| !value.is_finite())
                    || values[0] >= values[2]
                    || values[1] >= values[3]
                {
                    self.command_line
                        .push_error("BASEMAP BOUNDS requires four finite values with min < max.");
                    return Task::none();
                }
                self.tabs[i].spatial.basemap_bounds =
                    Some([values[0], values[1], values[2], values[3]]);
                self.persist_spatial_settings(i);
                self.command_line
                    .push_output("Basemap: manual bounds stored on the drawing; loading imagery.");
            }
            _ => {
                self.command_line.push_error(
                    "BASEMAP [ARCGIS|STREETS|GOOGLE|CUSTOM <t>|PROJ <drawing|default|las|epsg>|CENTER <lon lat [radius-km]>|BOUNDS <minx miny maxx maxy>|ZOOM <z>|OFF].",
                );
                return Task::none();
            }
        }
        self.sync_basemap_dropdown();
        self.save_config();
        self.refresh_basemap(tab_id)
    }

    /// Keep the ribbon's Basemap dropdowns in sync with `self.basemap` so they
    /// reflect the persisted state, not just the last clicked item.
    pub(super) fn sync_basemap_dropdown(&mut self) {
        use crate::scene::basemap::{BasemapProjection, BasemapProvider};
        let provider = match self.basemap.provider {
            BasemapProvider::ArcGisImagery => "BASEMAP ARCGIS",
            BasemapProvider::ArcGisStreets => "BASEMAP STREETS",
            BasemapProvider::GoogleHybrid => "BASEMAP GOOGLE",
            BasemapProvider::Off => "BASEMAP OFF",
            // Custom has no dropdown entry; show Imagery as the nearest.
            BasemapProvider::Custom => "BASEMAP ARCGIS",
        };
        self.ribbon
            .select_dropdown_item("BASEMAP_PROVIDER", provider);
        let projection = match self.basemap.projection {
            BasemapProjection::FromDrawing => "BASEMAP PROJ DRAWING",
            BasemapProjection::FromLas => "BASEMAP PROJ LAS",
            _ => "BASEMAP PROJ DEFAULT",
        };
        self.ribbon
            .select_dropdown_item("BASEMAP_PROJECTION", projection);
        let working_units = match self.tabs[self.active_tab].spatial.working_unit {
            super::spatial::WorkingUnit::Meters => "WORKINGUNITS METERS",
            super::spatial::WorkingUnit::Centimeters => "WORKINGUNITS CENTIMETERS",
            super::spatial::WorkingUnit::Feet => "WORKINGUNITS FEET",
            super::spatial::WorkingUnit::Inches => "WORKINGUNITS INCHES",
            // Geographic CRSs lock to degrees, which is intentionally absent
            // from the CRS-free length-unit picker.
            super::spatial::WorkingUnit::Degrees => "WORKINGUNITS METERS",
        };
        self.ribbon
            .select_dropdown_item("WORKING_UNITS", working_units);
    }

    /// Clear the underlay immediately (used by BASEMAP OFF).
    pub(super) fn clear_basemap(&mut self) {
        if let Some(job) = self.basemap_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        let i = self.active_tab;
        self.tabs[i].scene.set_basemap_images(Vec::new());
    }

    /// Resolve the source EPSG (from the attached cloud when `FromLas`), compute
    /// the covered tiles, fetch and decode them on a worker, then install the
    /// world-space quads.
    pub(super) fn refresh_basemap(&mut self, tab_id: u64) -> Task<Message> {
        let i = self.active_tab;
        let settings = self.basemap.clone().normalized();
        if settings.provider == BasemapProvider::Off {
            self.clear_basemap();
            return Task::none();
        }

        // Resolve drawing/cloud extents before the CRS. An entirely empty tab
        // is a supported spatial bootstrap case: it can use either a global
        // Web-Mercator overview or the selected EPSG definition's area of use.
        let manual_bounds = self.tabs[i]
            .spatial
            .basemap_bounds
            .map(|bounds| ([bounds[0], bounds[1], 0.0], [bounds[2], bounds[3], 0.0]));
        let drawing_bounds = self.tabs[i].scene.model_space_extents().map(|(min, max)| {
            (
                [min.x as f64, min.y as f64, min.z as f64],
                [max.x as f64, max.y as f64, max.z as f64],
            )
        });
        let cloud_bounds = self.tabs[i].point_cloud.bounds();
        let no_spatial_bounds =
            manual_bounds.is_none() && drawing_bounds.is_none() && cloud_bounds.is_none();

        // Resolve the source CRS used to interpret drawing coordinates. A full
        // `CrsInfo` (not just the EPSG) lets projected LAS WKT continue using
        // its PROJ.4 definition. With no content and no CRS, a temporary 3857
        // world overview is safe because it does not claim to geolocate CAD
        // coordinates; selecting a drawing CRS replaces it.
        #[cfg(not(target_arch = "wasm32"))]
        let source_crs: ocs_pointcloud::CrsInfo = match settings.projection {
            BasemapProjection::FromDrawing => match self.tabs[i].spatial.drawing_crs.as_ref() {
                Some(crs) => ocs_pointcloud::CrsInfo {
                    horizontal_epsg: Some(crs.epsg),
                    ..Default::default()
                },
                None if no_spatial_bounds => ocs_pointcloud::CrsInfo {
                    horizontal_epsg: Some(3857),
                    ..Default::default()
                },
                None => {
                    self.command_line.push_error(
                            "Basemap: drawing CRS is unset; use CRS <epsg> to align the underlay with drawing coordinates.",
                        );
                    return Task::none();
                }
            },
            BasemapProjection::WebMercator => ocs_pointcloud::CrsInfo {
                horizontal_epsg: Some(3857),
                ..Default::default()
            },
            BasemapProjection::FromLas => {
                let crs = self.tabs[i]
                    .point_cloud
                    .active()
                    .map(|s| s.sample.metadata.crs.clone())
                    .filter(|crs| crs.horizontal_epsg.is_some() || crs.proj4.is_some());
                match crs {
                    Some(crs) => crs,
                    None => {
                        self.command_line.push_error(
                            "Basemap: no attached LAS cloud provides a CRS; use BASEMAP PROJ <epsg>.",
                        );
                        return Task::none();
                    }
                }
            }
            BasemapProjection::Epsg(epsg) => ocs_pointcloud::CrsInfo {
                horizontal_epsg: Some(epsg),
                ..Default::default()
            },
        };

        let preferred_bounds: Option<([f64; 3], [f64; 3])> = match settings.projection {
            BasemapProjection::FromLas => cloud_bounds.or(manual_bounds).or(drawing_bounds),
            _ => manual_bounds.or(drawing_bounds).or(cloud_bounds),
        };
        let mut automatic_bootstrap = false;
        let (min, max) = if let Some(bounds) = preferred_bounds {
            bounds
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            let source_bounds = source_crs
                .horizontal_epsg
                .and_then(ocs_pointcloud::epsg_area_of_use)
                .and_then(|area| basemap::source_bounds_from_wgs84_area(area, &source_crs));
            #[cfg(target_arch = "wasm32")]
            let source_bounds = match settings.projection {
                BasemapProjection::WebMercator => Some(basemap::tile_bounds(0, 0, 0)),
                BasemapProjection::FromDrawing if self.tabs[i].spatial.drawing_crs.is_none() => {
                    Some(basemap::tile_bounds(0, 0, 0))
                }
                _ => None,
            };
            let Some(bounds) = source_bounds else {
                self.command_line.push_error(
                    "Basemap: no drawing extent is available. Set CRS <epsg>, then use BASEMAP CENTER <longitude> <latitude> [radius-km].",
                );
                return Task::none();
            };
            automatic_bootstrap = true;
            ([bounds[0], bounds[1], 0.0], [bounds[2], bounds[3], 0.0])
        };

        // World bounds (Web Mercator meters) for the drawing envelope.
        #[cfg(not(target_arch = "wasm32"))]
        let world_bounds =
            basemap::world_bounds_from_source([min[0], min[1]], [max[0], max[1]], &source_crs);
        #[cfg(target_arch = "wasm32")]
        let world_bounds = Some([min[0], min[1], max[0], max[1]]);

        let Some(world_bounds) = world_bounds else {
            self.command_line
                .push_error("Basemap: cannot reproject the drawing bounds into Web Mercator.");
            return Task::none();
        };

        // Count before allocation. A world envelope at zoom 16 covers more
        // than four billion tiles; building that Vec first caused the v1.0.0
        // basemap freeze followed by Rust's native out-of-memory abort.
        const MAX_BOOTSTRAP_TILES: u64 = 64;
        const MAX_INTERACTIVE_TILES: u64 = 256;
        let tile_limit = if automatic_bootstrap {
            MAX_BOOTSTRAP_TILES
        } else {
            MAX_INTERACTIVE_TILES
        };
        let Some((effective_zoom, planned_count)) =
            basemap::zoom_for_tile_limit(world_bounds, settings.zoom, tile_limit)
        else {
            self.command_line
                .push_error("Basemap: no tiles cover the drawing bounds.");
            return Task::none();
        };
        let tiles = match basemap::tiles_covering_bounded(world_bounds, effective_zoom, tile_limit)
        {
            Ok(tiles) => tiles,
            Err(required) => {
                self.command_line.push_error(
                    format!(
                        "Basemap: request needs {required} tiles, above the safe {tile_limit}-tile limit."
                    )
                    .as_str(),
                );
                return Task::none();
            }
        };
        if tiles.is_empty() {
            self.command_line
                .push_error("Basemap: no tiles cover the drawing bounds.");
            return Task::none();
        }
        if effective_zoom < settings.zoom && !automatic_bootstrap {
            self.command_line.push_info(
                format!(
                    "Basemap: drawing extent is too large for zoom {}; using zoom {effective_zoom} ({planned_count} tiles) to keep the viewport responsive.",
                    settings.zoom
                )
                .as_str(),
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        let worker_crs = source_crs.clone();
        let custom = settings.custom_template.clone();
        let provider = settings.provider;
        let worker_tiles: Vec<Tile> = tiles;
        let requested = worker_tiles.len();
        let fit_bounds = (automatic_bootstrap
            || (manual_bounds.is_some() && drawing_bounds.is_none() && cloud_bounds.is_none()))
        .then_some([min[0], min[1], max[0], max[1]]);
        if automatic_bootstrap {
            if self.tabs[i].spatial.drawing_crs.is_none() {
                self.command_line.push_info(
                    "Basemap: empty drawing; showing a Web Mercator world overview. Set CRS <epsg>, then use Set Location or BASEMAP CENTER for the project site.",
                );
            } else {
                self.command_line.push_info(
                    format!(
                        "Basemap: no drawing extent; showing the EPSG area-of-use overview at zoom {effective_zoom}. Use Set Location or BASEMAP CENTER for the project site."
                    )
                    .as_str(),
                );
            }
        }
        if let Some(previous) = self.basemap_job.take() {
            previous.cancel.store(true, Ordering::Relaxed);
        }
        let job = BasemapJob::new(tab_id, requested);
        let job_id = job.id;
        let completed = Arc::clone(&job.completed);
        let failed = Arc::clone(&job.failed);
        let failed_for_result = Arc::clone(&job.failed);
        let cancel = Arc::clone(&job.cancel);
        self.basemap_job = Some(job);
        self.command_line.push_info(
            format!(
                "Basemap: loading {requested} tile(s) at zoom {} (parallel workers + disk cache)...",
                effective_zoom
            )
            .as_str(),
        );
        #[cfg(not(target_arch = "wasm32"))]
        let cache_root = crate::config::config_dir().map(|path| path.join("basemap_cache"));
        let cache_namespace = provider.cache_namespace(&custom);
        background_task(
            move || {
                let fetch = |tile: &Tile| -> Option<ImageModel> {
                    if cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    let Some(url) = basemap::tile_url(provider, tile.z, tile.x, tile.y, &custom)
                    else {
                        failed.fetch_add(1, Ordering::Relaxed);
                        completed.fetch_add(1, Ordering::Relaxed);
                        return None;
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    let decoded = cache_root
                        .as_ref()
                        .and_then(|root| {
                            let path = root
                                .join(&cache_namespace)
                                .join(tile.z.to_string())
                                .join(tile.x.to_string())
                                .join(format!("{}.img", tile.y));
                            crate::scene::resolve_remote_image_cached(&url, &path)
                        })
                        .or_else(|| resolve_image(&url));
                    #[cfg(target_arch = "wasm32")]
                    let decoded = resolve_image(&url);
                    let Some(decoded) = decoded else {
                        failed.fetch_add(1, Ordering::Relaxed);
                        completed.fetch_add(1, Ordering::Relaxed);
                        return None;
                    };
                    // Each tile is a Web-Mercator quad; place it in the source
                    // CRS the drawing uses so the underlay lines up with UTM or
                    // other projected content instead of landing at Mercator
                    // meters (millions of metres off for non-3857 sources).
                    // A no-op when the source is already 3857.
                    #[cfg(not(target_arch = "wasm32"))]
                    let quad_bounds = basemap::reproject_bounds_3857(tile.bounds, &worker_crs)
                        .unwrap_or(tile.bounds);
                    #[cfg(target_arch = "wasm32")]
                    let quad_bounds = tile.bounds;
                    let image = ImageModel::from_world_quad(
                        &url,
                        decoded.pixels,
                        decoded.width,
                        decoded.height,
                        quad_bounds,
                        0.0,
                        0.0,
                    );
                    completed.fetch_add(1, Ordering::Relaxed);
                    Some(image)
                };

                #[cfg(not(target_arch = "wasm32"))]
                {
                    use rayon::prelude::*;
                    let worker_count = std::thread::available_parallelism()
                        .map(usize::from)
                        .unwrap_or(4)
                        .clamp(2, 8)
                        .min(worker_tiles.len().max(1));
                    match rayon::ThreadPoolBuilder::new()
                        .num_threads(worker_count)
                        .thread_name(|index| format!("ocs-basemap-{index}"))
                        .build()
                    {
                        Ok(pool) => pool.install(|| {
                            worker_tiles
                                .par_iter()
                                .filter_map(fetch)
                                .collect::<Vec<_>>()
                        }),
                        Err(_) => worker_tiles.iter().filter_map(fetch).collect(),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    worker_tiles.iter().filter_map(fetch).collect::<Vec<_>>()
                }
            },
            move |images| {
                Message::BasemapLoaded(BasemapLoaded {
                    job_id,
                    tab_id,
                    images,
                    requested,
                    failed: failed_for_result.load(Ordering::Relaxed),
                    fit_bounds,
                })
            },
        )
    }

    /// Install the fetched basemap quads on the active tab.
    pub(super) fn install_basemap(&mut self, loaded: BasemapLoaded) {
        if self
            .basemap_job
            .as_ref()
            .is_none_or(|job| job.id != loaded.job_id || job.tab_id != loaded.tab_id)
        {
            return;
        }
        self.basemap_job = None;
        let Some(i) = self.tabs.iter().position(|tab| tab.id == loaded.tab_id) else {
            return;
        };
        // Dim the underlay to the configured opacity by scaling each image.
        let opacity = self.basemap.opacity.clamp(0.0, 1.0);
        let count = loaded.images.len();
        let mut images = loaded.images;
        for image in &mut images {
            image.opacity = opacity;
        }
        self.tabs[i].scene.set_basemap_images(images);
        if let Some(bounds) = loaded.fit_bounds {
            self.tabs[i]
                .scene
                .fit_external_bounds([bounds[0], bounds[1], 0.0], [bounds[2], bounds[3], 0.0]);
        }
        if count == 0 {
            self.command_line.push_error(
                format!(
                    "Basemap: all {} tile requests failed; check the network/provider and try again.",
                    loaded.requested
                )
                .as_str(),
            );
        } else {
            self.command_line.push_output(
                format!(
                    "Basemap: {count}/{} tile(s) placed{}.",
                    loaded.requested,
                    if loaded.failed > 0 {
                        format!("; {} failed", loaded.failed)
                    } else {
                        String::new()
                    }
                )
                .as_str(),
            );
        }
    }
}
