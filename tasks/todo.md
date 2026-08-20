# v1.0 — Basemap, spatial settings, measurement, navigation, and release

## Part A — CRS reprojection fix
- [x] Add `proj4: Option<String>` to `CrsInfo` (crates/ocs_pointcloud/src/crs.rs)
- [x] Add `proj4_from_wkt()` WKT→PROJ.4 parser (LCC 2SP, TMerc, geographic)
- [x] Add `reproject_from_proj4()` + proj4-preferring wrapper
- [x] Thread `CrsInfo` through `world_bounds_from_source` / `reproject_bounds_3857` (src/scene/basemap.rs)
- [x] Pass CRS from `refresh_basemap` (src/app/basemap.rs)
- [x] Unit test proj4_from_wkt + round-trip

## Part B — Basemap ribbon controls
- [x] Add `BASEMAP ZOOMIN`/`ZOOMOUT` subcommands (src/app/basemap.rs)
- [x] Add Basemap group to View tab (src/modules/view/mod.rs)
- [x] Register BASEMAP + POINTCLOUDDENSITY in autocomplete (src/app/commands/mod.rs)
- [x] Seed ribbon dropdown from persisted basemap state

## Part C — LAS density controls
- [x] Add `stride: Option<u64>` to SampleOptions (crates/ocs_pointcloud/src/lib.rs)
- [x] Add `Density` to DisplaySettings (crates/ocs_pointcloud/src/display.rs)
- [x] Map Density → SampleOptions in start_point_cloud_load / folder load (src/app/point_cloud.rs)
- [x] Add POINTCLOUDDENSITY dispatch + re-sample path
- [x] Folder-too-big warning + fallback
- [x] Density dropdown in LiDAR tab (src/modules/lidar/mod.rs)

## Part D — v1 spatial and interaction hardening

- [x] Bounded-parallel cached basemap jobs with progress and cancellation
- [x] Empty-drawing `BASEMAP BOUNDS` and first-load auto-fit
- [x] Empty-drawing world/CRS-area bootstrap and `BASEMAP CENTER` site workflow
- [x] Drawing-owned CRS and compatible working units in sidecar schema v4
- [x] Dedicated Select and ArcGIS-style Navigator tools
- [x] Length, area, closed-mesh volume, and point-cloud distance tools
- [x] Fix point-arena zero-shard assertion and sphere tessellation fallback
- [x] Update roadmap and LiDAR workflow documentation

## Build / ship
- [x] `cargo check --all-targets`
- [x] Full test suite and release build
- [x] Rebuild/install MSI (WiX candle/light), validate 1.0.0
- [x] Fast-forward default branch and push the release commit
- [ ] Move the unpublished tag to the verified basemap fix and publish GitHub release
- [x] Record durable prevention lessons in `tasks/lessons.md`

## Verification
- [x] Reprojection unit test passes
- [ ] Basemap places for Boston LAS (no "cannot reproject")
- [x] Empty drawing loads a bounded overview and accepts a CRS/site center without LAS
- [ ] Density re-sample + folder warning
