# v0.9.7 — Basemap fix + UI, LAS density controls

## Part A — CRS reprojection fix
- [ ] Add `proj4: Option<String>` to `CrsInfo` (crates/ocs_pointcloud/src/crs.rs)
- [ ] Add `proj4_from_wkt()` WKT→PROJ.4 parser (LCC 2SP, TMerc, geographic)
- [ ] Add `reproject_from_proj4()` + proj4-preferring wrapper
- [ ] Thread `CrsInfo` through `world_bounds_from_source` / `reproject_bounds_3857` (src/scene/basemap.rs)
- [ ] Pass CRS from `refresh_basemap` (src/app/basemap.rs)
- [ ] Unit test proj4_from_wkt + round-trip

## Part B — Basemap ribbon controls
- [ ] Add `BASEMAP ZOOMIN`/`ZOOMOUT` subcommands (src/app/basemap.rs)
- [ ] Add Basemap group to View tab (src/modules/view/mod.rs)
- [ ] Register BASEMAP + POINTCLOUDDENSITY in autocomplete (src/app/commands/mod.rs)
- [ ] Seed ribbon dropdown from persisted basemap state

## Part C — LAS density controls
- [ ] Add `stride: Option<u64>` to SampleOptions (crates/ocs_pointcloud/src/lib.rs)
- [ ] Add `Density` to DisplaySettings (crates/ocs_pointcloud/src/display.rs)
- [ ] Map Density → SampleOptions in start_point_cloud_load / folder load (src/app/point_cloud.rs)
- [ ] Add POINTCLOUDDENSITY dispatch + re-sample path
- [ ] Folder-too-big warning + fallback
- [ ] Density dropdown in LiDAR tab (src/modules/lidar/mod.rs)

## Build / ship
- [ ] cargo check + release build
- [ ] Rebuild MSI (candle/light), validate 0.9.7
- [ ] Commit + push on v0.9.7-lidar-platform
- [ ] Record lesson in tasks/lessons.md

## Verification
- [ ] Reprojection unit test passes
- [ ] Basemap places for Boston LAS (no "cannot reproject")
- [ ] Density re-sample + folder warning
