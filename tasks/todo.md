# v1.0 — Basemap, spatial settings, measurement, navigation, and release

## v2.1.0 — 3D plan Week 1: 3MF import + terrain draping (docs/3d-development-plan.md)

- [x] Surface-sampling/path-draping API in ocs_pointcloud (measurement.rs): validation, 1M-point ceiling, vertical offset, atomic failure; 4 unit tests
- [x] `Tin: SurfaceSampler` bridge (dtm.rs)
- [x] `POINTCLOUDDRAPE [spacing] [vertical-offset]` command: lines/2D/3D polylines via curve tessellation, class-2 ground TIN with reported fallback, single undoable transaction, atomic across the selection
- [x] LiDAR manager "Terrain and CAD draping" section (ground/contours/drape/DTM/DSM/hillshade)
- [x] Streaming 3MF Core importer (io/three_mf.rs): OPC StartPart discovery, quick-xml forward parse, components/build items/transforms, base materials → per-color layers, units → header
- [x] Package hardening: entries/expansion/path-safety/ratio limits, vertex+triangle ceilings, cycle/depth limits, required-extension rejection
- [x] Import diagnostics on the command line: objects, units, bounds, materials, components, build items, skipped package parts (threaded io → DerivedCaches → finish_open)
- [x] 3MF open path: unsaved-drawing semantics (no QSAVE over source), shaded+fit default view, OPEN/recents/file-dialog/boot argv, .3mf installer association (main.wxs)
- [x] Mesh display LOD fast path (entities/mesh.rs): indexed-triangle direct path, +2 border-locked LODs over 50k triangles; 3 lod_tests incl. large-mesh monotonicity
- [x] Opt-in fixtures: OCS_3MF_STRESS_FILE import + render-cache + OCS_3MF_STRESS_DIR audit tests
- [x] Baseline on real fixture (release build, this machine): 48 MB Palo Alto model → 86 objects, 2.27M vertices, 4.37M triangles imported in ~4 s
- [x] Gates: cargo check, app suite 493, ocs_pointcloud 74, platform 5, spatial 10, shader contract 5, rustfmt on changed files
- [x] Release: version 2.1.0, release notes, README line, tag + GitHub release (CI MSI/portable/AppImage/snap/dmg)

## v1.0.4 — full-density LiDAR OOM fix

- [x] Root-cause: `Density::Full` materializes the whole file (unbounded `sample()`); auto cache-activation adds a second streaming copy; resample leaks streamed tiles
- [x] Add `full_density_over_budget()` helper (src/app/point_cloud.rs)
- [x] Guard single-file Full attach in `start_point_cloud_load` (stream via LOD cache, else fall back to Auto + hint)
- [x] Guard `set_point_cloud_density` (skip already-streamed sources; activate cache when available; else Auto + hint)
- [x] Clear streamed tiles/state in `install_point_cloud_resample` (no sustained double copy)
- [x] `cargo check --bin OpenCADStudio` — passes (pre-existing warnings only)
- [x] `cargo build --release --bin OpenCADStudio --jobs 1` + `-p dwg-thumbnailer-win`
- [x] Rebuild the WiX MSI and refresh `dist/v1.0.4` artifacts (msi + portable exe + local-install)
- [ ] Runtime smoke-test (attach a large tile at Full → streams, no OOM) — not run; needs the GUI against the multi-GB tiles on this memory-constrained box. Repro: `POINTCLOUDDENSITY FULL` then `POINTCLOUDATTACH D:\MA_Lidar\...\tR0_C0.laz`


## v1.0.2 basemap imagery/alignment patch

- [x] Reproduce the renderer-cache blank-frame failure
- [x] Correct foot-unit WKT false easting/northing conversion
- [x] Verify the Boston LAS center resolves to Boston imagery tiles
- [x] Add regression coverage for renderer invalidation and CRS conversion
- [x] Build and validate the v1.0.2 portable executable and WiX MSI
- [x] Install v1.0.2 and smoke-test the installed binary
- [x] Receive authorization to commit, push, tag, and publish v1.0.2

## v1.0.1 urgent basemap crash patch

- [x] Symbolize repeated v1.0.0 Windows crash dumps as Rust OOM aborts
- [x] Count huge tile coverage before allocation
- [x] Automatically lower effective zoom to 64 bootstrap / 256 drawing tiles
- [x] Add full-world zoom-22 no-allocation regression coverage
- [x] Run full regression suite and release build
- [x] Build/install/verify the 1.0.1 MSI
- [x] Push, tag, and publish v1.0.1 as Latest

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

## v1.0.4-local — Palo Alto LiDAR: CRS-agnostic + LOD + UX

### Part A — CRS-agnostic reprojection
- [x] A1 crs.rs: `projection_from_crs` (proj4-first) + refactor `reproject_with_patches_progress`
- [x] A1 crs.rs: CRS label helper (no geographic-fallback EPSG when proj4 present) + unit tests
- [x] A2 point_cloud.rs: reprojection guard/message proj4-aware; export-all CRS identity; crs_info label
- [x] A3 spatial.rs: `drawing_crs_command` LAS inference rejects geographic fallback w/ guidance

### Part B — LOD progress + efficiency
- [x] B1 progress: `index_job` + `POINTCLOUDINDEXSTATUS` + finish reports cache size
- [x] B2 tile_cache.rs: 1 MiB BufWriter buffer + `estimate_cache_bytes` + upfront warning

### Part C — coordinate copy cartesian/decimal
- [x] C viewport.rs decimal copy (lon/lat + CRS note); Message variant; overlay menu item

### Part D — pixel-width slices (1–1024 px)
- [x] D model/shader/pipeline px->world; stream band conversion; setters validation
- [x] D UI slider + replace Wider/Narrower buttons/palette

### Part E — point size up to 10 px
- [x] E extend 3 UI option lists to 1–10 px

### Part F — basemap camera-driven resolution
- [x] F slippy-zoom-from-pixels helper + viewport-mode refresh + debounced camera hook

### Verify
- [x] cargo check --bin OpenCADStudio
- [x] cargo build --release --bin OpenCADStudio --jobs 1
