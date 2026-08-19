//! `BASEMAP` command: georeferenced XYZ-slippy imagery underlay.
//!
//! Settings (provider, projection, zoom) live on the app; the command fetches
//! the tiles covering the drawing (or attached point-cloud) bounds and installs
//! them as session-only `ImageModel`s in world space, drawn behind content.

use super::{Message, OpenCADStudio};
use crate::scene::basemap::{self, BasemapProvider, BasemapProjection, Tile};
use crate::scene::{resolve_image, ImageModel};
use iced::Task;

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
    pub tab_id: u64,
    pub images: Vec<ImageModel>,
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
                self.command_line.push_output(crate::t!("Basemap: off.").as_ref());
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
                    self.command_line
                        .push_error("BASEMAP CUSTOM <template> — an XYZ template with {z} {x} {y}.");
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
                    "DEFAULT" | "3857" | "WEBMERCATOR" | "MERCATOR" => {
                        self.basemap.projection = BasemapProjection::WebMercator;
                        self.basemap.source_epsg = None;
                        self.command_line
                            .push_output(crate::t!("Basemap projection: default (Web Mercator).").as_ref());
                    }
                    "LAS" | "CLOUD" | "POINTCLOUD" => {
                        self.basemap.projection = BasemapProjection::FromLas;
                        self.command_line
                            .push_output(crate::t!("Basemap projection: from the attached LAS cloud.").as_ref());
                    }
                    other => {
                        if let Ok(epsg) = other.parse::<u16>() {
                            self.basemap.projection = BasemapProjection::Epsg(epsg);
                            self.basemap.source_epsg = Some(epsg);
                            self.command_line
                                .push_output(crate::tf!("Basemap projection: EPSG:{epsg}.").as_ref());
                        } else {
                            self.command_line.push_error(
                                "BASEMAP PROJ <DEFAULT|LAS|<epsg>>.",
                            );
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
                    self.command_line
                        .push_error("BASEMAP ZOOM <0-22>.");
                    return Task::none();
                }
            }
            _ => {
                self.command_line.push_error(
                    "BASEMAP [ARCGIS|STREETS|GOOGLE|CUSTOM <t>|PROJ <d|las|epsg>|ZOOM <z>|OFF].",
                );
                return Task::none();
            }
        }
        self.save_config();
        self.refresh_basemap(tab_id)
    }

    /// Clear the underlay immediately (used by BASEMAP OFF).
    pub(super) fn clear_basemap(&mut self) {
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

        // Resolve the source EPSG used to interpret drawing coordinates.
        let source_epsg: u16 = match settings.projection {
            BasemapProjection::WebMercator => 3857,
            BasemapProjection::FromLas => {
                let cloud = self.tabs[i]
                    .point_cloud
                    .active()
                    .and_then(|s| s.sample.metadata.crs.horizontal_epsg);
                match cloud {
                    Some(epsg) => epsg,
                    None => {
                        self.command_line.push_error(
                            "Basemap: no attached LAS cloud provides a CRS; use BASEMAP PROJ <epsg>.",
                        );
                        return Task::none();
                    }
                }
            }
            BasemapProjection::Epsg(epsg) => epsg,
        };

        // Drawing bounds in the source CRS: prefer the attached cloud (f64),
        // else the model-space extents (f32, widened to f64).
        let bounds: Option<([f64; 3], [f64; 3])> = self.tabs[i]
            .point_cloud
            .bounds()
            .or_else(|| {
                self.tabs[i].scene.model_space_extents().map(|(min, max)| {
                    (
                        [min.x as f64, min.y as f64, min.z as f64],
                        [max.x as f64, max.y as f64, max.z as f64],
                    )
                })
            });
        let Some((min, max)) = bounds else {
            self.command_line
                .push_error("Basemap: the drawing has no bounds to place the underlay.");
            return Task::none();
        };

        // World bounds (Web Mercator meters) for the drawing envelope.
        #[cfg(not(target_arch = "wasm32"))]
        let world_bounds = basemap::world_bounds_from_source(
            [min[0], min[1]],
            [max[0], max[1]],
            source_epsg,
        );
        #[cfg(target_arch = "wasm32")]
        let world_bounds = Some([min[0], min[1], max[0], max[1]]);

        let Some(world_bounds) = world_bounds else {
            self.command_line
                .push_error("Basemap: cannot reproject the drawing bounds into Web Mercator.");
            return Task::none();
        };

        let tiles = basemap::tiles_covering(world_bounds, settings.zoom);
        if tiles.is_empty() {
            self.command_line
                .push_error("Basemap: no tiles cover the drawing bounds.");
            return Task::none();
        }

        let custom = settings.custom_template.clone();
        let provider = settings.provider;
        let worker_tiles: Vec<Tile> = tiles;
        background_task(
            move || {
                let mut images = Vec::new();
                for tile in &worker_tiles {
                    let Some(url) = basemap::tile_url(provider, tile.z, tile.x, tile.y, &custom)
                    else {
                        continue;
                    };
                    let Some(decoded) = resolve_image(&url) else {
                        continue;
                    };
                    images.push(ImageModel::from_world_quad(
                        &url,
                        decoded.pixels,
                        decoded.width,
                        decoded.height,
                        tile.bounds,
                        0.0,
                        0.0,
                    ));
                }
                images
            },
            move |images| {
                Message::BasemapLoaded(BasemapLoaded {
                    tab_id,
                    images,
                })
            },
        )
    }

    /// Install the fetched basemap quads on the active tab.
    pub(super) fn install_basemap(&mut self, loaded: BasemapLoaded) {
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
        self.command_line
            .push_output(crate::tf!("Basemap: {count} tile(s) placed.").as_ref());
    }
}
