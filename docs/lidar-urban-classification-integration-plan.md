# Urban point-cloud classification integration plan

## Decision

OpenCADStudio should adopt the ordered-fuser model from
`Coding-Oafs/Urban_PointCloud_Processing`, not its original runtime unchanged.
The upstream code is tied to Dutch Rijksdriehoek tiles and AHN/BGT/BAG/CycloMedia
services, Open3D 0.16, and separately built CloudCompare Python bindings. The
Boston batch proves the portable part of the design:

1. stream every physical source point;
2. seed trusted labels from existing ASPRS classes;
3. run ordered spatial fusers against versioned reference layers;
4. preserve the source `classification` byte;
5. write a separate uint8 `label` extra dimension and provenance VLR; and
6. publish the result only after a complete, validated write.

The production implementation should be native Rust in `ocs_pointcloud`. The
Python batch adapter in `scripts/lidar/boston_upcp_classifier.py` remains a
reference implementation and regression oracle, not an application dependency.

## User workflow

Add an **Urban Classify** button to the LiDAR ribbon's **Classify** group and an
**Urban classification** section to the LiDAR Point Cloud Manager.

The button opens the manager with these settings:

| Setting | Default | Purpose |
| --- | --- | --- |
| Scope | Attached source | Current file, every attachment, or source folder |
| Profile | Auto-detect | Boston ArcGIS for EPSG:6492; local/custom profile otherwise |
| Output folder | `<source>/classified` | Never write into or replace a source LAZ |
| Seed source classes | On | ASPRS 2→UPCP 9, 17→14, 18→99 |
| Building fuser | On | Class-1 points inside building footprints → UPCP 10 |
| Road fuser | On | Class-2 points inside width-buffered road centerlines → UPCP 1 |
| Road edge allowance | 1 survey foot | Added to half of `SURFACE_WD`; lane/class fallback is visible |
| Reference cache | On | Store the exact GeoJSON responses used by each tile |
| Preserve ASPRS classification | On, locked | Source provenance is never destroyed |
| Write UPCP `label` | On, locked | Portable uint8 extra-byte output |
| Overwrite outputs | Off | Existing outputs are skipped or require explicit confirmation |
| Attach completed output | On | Replace the viewer attachment after successful validation |

The pane must show a reference-epoch warning. The current Boston building and
road services do not necessarily describe conditions at the 2013–2014 LiDAR
epoch. A future profile can point at era-matched local GeoJSON/GeoPackage data.

The job area shows current tile, points processed/total, fuser stage, output
path, reference feature counts, elapsed time, and **Cancel**. Classification
must always stream the complete LAS/LAZ. It must never run on the initial GPU
sample or only the currently resident LOD tiles.

## Commands and macro API

Add the commands:

```text
POINTCLOUDURBANCLASSIFY
POINTCLOUDURBANCLASSIFYFOLDER [source-folder] [output-folder]
POINTCLOUDURBANSTATUS
POINTCLOUDURBANCANCEL
```

`POINTCLOUDURBANCLASSIFY` opens settings when invoked without arguments. A
settings preset name or JSON path makes it non-interactive.

Extend the Rhai bridge with:

```text
ocs.cloud_urban_classify(settings_json) -> job_id
ocs.cloud_urban_status(job_id) -> object
ocs.cloud_urban_cancel(job_id) -> bool
```

A library macro can then submit a folder job and wait on structured status,
without busy-spinning the UI thread. The existing
`scripts/library/batch_classify.rhai` should gain an optional urban-fusion step
after ground/noise work and before export.

## Native architecture

### 1. Core processor

Create `crates/ocs_pointcloud/src/urban.rs` with UI-independent types:

```text
UrbanClassificationSettings
UrbanProfile
UrbanReferenceProvider
UrbanFuser
UrbanJobProgress
UrbanTileStats
UrbanBatchManifest
```

Implement a streaming reader/writer using the existing LAZ path. Preserve the
header, CRS records, point format, coordinates, GPS time, RGB, flags, and every
existing extra byte. Add `label` through an Extra Bytes VLR, plus an
`OpenCADStudio` provenance VLR. Write `<name>.laz.partial`, validate point count,
point format, scale/offset, CRS, source-class histogram, `label`, and provenance,
then atomically rename to `<name>_classified.laz`.

The first native fusers should exactly match the proven Boston rules:

```text
SeedFuser:       ASPRS 2 -> label 9; ASPRS 17 -> 14; ASPRS 18 -> 99
BuildingFuser:   ASPRS 1 + footprint intersection -> label 10
RoadFuser:       ASPRS 2 + buffered centerline intersection -> label 1
```

Water (ASPRS 9) and rail (ASPRS 10) remain in the preserved classification and
receive UPCP label 0 because the upstream label table has no direct equivalent.
Do not invent vegetation, car, pole, cable, or sign labels without a suitable
reference layer or a validated geometric/model classifier.

### 2. Reference providers

Implement provider adapters behind one trait:

- `BostonArcGisProvider`: Boston Planning building polygons and Boston/MassDOT
  road centerlines in EPSG:6492.
- `LocalVectorProvider`: GeoJSON first; GeoPackage after GDAL-free reading is
  selected or a controlled geospatial backend is bundled.
- `CustomArcGisProvider`: user-configurable FeatureServer layer URL, field map,
  source CRS, and width rules.

Query by the real LAS header envelope with a small profile-defined margin. Page
by object ID, retry transient failures, cache exact responses beside the batch
manifest, and allow a fully offline rerun from that cache. Reproject reference
geometry to the cloud CRS before fusion; block missing or unresolved source CRS.

### 3. Viewer and class tables

Teach the point reader and `.ocstiles` cache to retain a selected uint8 extra
dimension named `label`. Add `POINTCLOUDCOLOR LABEL` and a **UPCP label** color
mode. Statistics and visibility use the selected classification scheme:

- **ASPRS**: existing `classification` behavior;
- **UPCP**: the `label` extra dimension when present;
- **Auto**: UPCP when a valid `label` and provenance record exist, otherwise
  ASPRS.

Keep edits explicit. Manual ASPRS edits continue to target `classification`;
future UPCP-label edits need a separate command and sidecar patch type so one
scheme cannot silently overwrite the other.

### 4. Application job integration

Reuse the existing atomic progress/cancel pattern used by export and
reprojection, but give urban classification a dataset-wide queue. Add message
variants for settings changes, start, progress tick, cancel, and completion.
Persist settings presets, completed manifests, and resumable per-tile states in
the `.ocspc` sidecar job tables.

Primary code touch points:

- `crates/ocs_pointcloud/src/urban.rs` and `lib.rs`
- `crates/ocs_pointcloud/src/tile_cache.rs` and display/style types
- `crates/ocs_scripting/src/lib.rs`
- `src/app/scripting.rs`, `point_cloud.rs`, `commands/display.rs`, and update messages
- `src/ui/window/point_cloud_manager.rs`
- `src/modules/lidar/mod.rs`
- `docs/lidar-point-clouds.md`

## Delivery sequence and acceptance tests

1. **Extra-byte fidelity:** read, display, stream-write, and round-trip a fixture
   containing `label`; prove all other dimensions and CRS/VLRs are unchanged.
2. **Native Boston fusers:** reproduce the Python oracle's label histogram on a
   fixed test subset and agree point-for-point at polygon boundaries.
3. **Background job:** full-density multi-tile run with progress, cancel,
   partial-file cleanup, safe resume, no-overwrite behavior, and manifest audit.
4. **Button and macro:** the ribbon, manager settings, commands, and Rhai API all
   submit the same typed job configuration.
5. **Regression and real-data QA:** unit tests for width fallback, CRS mismatch,
   missing services/cache, invalid geometry, and class priority; ignored Boston
   smoke test gated by `OCS_LIDAR_SMOKE_DIR`.

Acceptance requires identical source point count and ASPRS histogram, a readable
UPCP `label`, no source modifications, deterministic label counts from cached
references, cancellation without a published partial LAZ, and successful attach
of the completed output in OpenCADStudio.
