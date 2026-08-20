# LAS/LAZ point-cloud workflow

OpenCADStudio has a native Windows workflow for attaching, indexing, GPU viewing,
selecting, reclassifying, and exporting LAS/LAZ point clouds. The implementation
keeps the full cloud outside the CAD entity graph: a rebuildable disk cache and
bounded GPU working set provide display data, sparse edits retain stable source
indices, and export streams every source record into a new file.

## Build prerequisites

- Rust 1.97.1 through rustup, using `x86_64-pc-windows-msvc` (pinned in
  `rust-toolchain.toml`).
- Visual Studio Build Tools or Visual Studio with Desktop development with C++
  and a Windows 11 SDK.

From a Developer PowerShell or a shell where `%USERPROFILE%\.cargo\bin` is on
`PATH`:

```powershell
cargo test --workspace --jobs 1 -- --test-threads=1
cargo build --release --bin OpenCADStudio --jobs 1
.\target\release\OpenCADStudio.exe
```

The single-job settings are recommended on machines where a parallel native
build can exhaust the Windows paging file.

## Click-first workflow

The native build has a **LiDAR** ribbon tab and a **LiDAR Point Cloud Manager**.
Use **Attach**, then **Build / Open LOD** for dense production data. The manager
provides color and point-size controls, direct viewport selection tools, an
editable class/statistics grid, CRS safeguards, common ASPRS point edits,
class-table interchange, audit history, export/reprojection progress/cancel,
and detach. `POINTCLOUDMANAGER` opens it from the command line or a function key.

The source LAS/LAZ is read-only during attachment and editing. A revised cloud
is created only by **Export LAS/LAZ**.

Drawing CRS and working units no longer depend on a LiDAR attachment. On an
empty drawing, turning on a basemap shows a small Web Mercator world overview;
`CRS <EPSG>` immediately replaces it with the CRS area-of-use overview. Use the
View ribbon's **Set Location** tool (or `BASEMAP CENTER <longitude> <latitude>
[radius-km]`) to choose the project site without loading LAS or CAD geometry.
`BASEMAP BOUNDS <minx> <miny> <maxx> <maxy>` remains available for an exact
drawing-coordinate envelope. Drawing-owned values persist in the adjacent
`.ocspc` sidecar for a saved DWG/DXF.

## Commands

| Command | Purpose |
| --- | --- |
| `POINTCLOUDATTACH` | Pick a `.las` or `.laz`, inspect it, and attach a bounded initial GPU display sample. |
| `POINTCLOUDATTACHFOLDER [path]` | Recursively attach every `.las`/`.laz` under a folder (picker, or a path argument). Files attach one at a time so a large folder cannot exhaust memory; already-attached or queued files are skipped, and the folder is recorded as a sidecar collection. |
| `POINTCLOUDMANAGER` | Open the click-first LiDAR manager. |
| `POINTCLOUDRESTORE` | Resolve the saved attachment from the drawing sidecar, validating its fingerprint and repairing a moved relative path. |
| `POINTCLOUDINFO` | Report source path, source/display point counts, sample stride, pending edits, CRS-VLR presence, and VLR/EVLR counts. |
| `POINTCLOUDINDEX` / `POINTCLOUDINDEXCANCEL` | Build or open the adjacent disk-backed `.ocstiles` hierarchy, or cancel a build. |
| `POINTCLOUDCOLOR <mode>` | Use `CLASS`, `RGB`, `INTENSITY`, `ELEVATION`, `RETURN`, or `SOURCE` GPU coloration. |
| `POINTCLOUDPOINTSIZE <1-32>` | Set the fixed physical-pixel point diameter. |
| `POINTCLOUDCLASSVISIBLE <class> <ON/OFF>` | Show or hide one class. |
| `POINTCLOUDSTATS` | Report per-class counts for the current full/sample/LOD working set. |
| `POINTCLOUDCLASSIFY <class> <indices>` | Queue an ASPRS class for source indices. Indices accept comma-separated values and inclusive ranges, for example `POINTCLOUDCLASSIFY 2 10-25,40`. |
| `POINTCLOUDSELECTPOINT` | Pick the nearest displayed point within a fixed screen-pixel aperture. |
| `POINTCLOUDMEASURE` | Pick two displayed cloud points and report snapped 3D distance, horizontal distance, and elevation delta in the drawing working unit. |
| `POINTCLOUDSELECTBOX` | Pick two viewport corners for a screen-space window selection. Coordinate arguments remain available for scripts. |
| `POINTCLOUDSELECTFENCE` | Click a screen-space polygon fence and press Enter to close it. |
| `POINTCLOUDSELECTBRUSH` | Apply a repeating 32-pixel viewport selection brush; press Enter to finish. Coordinate arguments remain available for scripts. |
| `POINTCLOUDSELECTSLICE` | Select the resident points between two survey elevations. |
| `POINTCLOUDSELECTFILTER` | Set/clear persistent class, return, source, elevation, synthetic, key, withheld, or overlap filters used by spatial selections. |
| `POINTCLOUDSELECTCLEAR` | Clear the active highlighted selection without changing saved edits. |
| `POINTCLOUDBRUSHCLASSIFY <class>` | Function-key-friendly repeating fixed-pixel viewport brush that selects and classifies source points. |
| `POINTCLOUDCLASSIFYSELECTION <class>` | Reclassify the active selection as one transaction. |
| `POINTCLOUDGROUND [cell] [distance] [angle]` | Densify bare-earth ground (class 2) over the display working set with a simplified progressive TIN: each grid cell seeds with its lowest point, the surface interpolates from neighbouring-cell triangles, and every iteration accepts the nearest-to-surface candidate under the distance (default 0.75) and angle (default 30°) thresholds. Points far above the surface (roofs) never join. Results are audited, undoable sparse edits. |
| `POINTCLOUDNOISE <radius> <min-neighbors> [class]` | Flag isolated points with fewer than `min-neighbors` neighbours inside `radius` (voxel-hash k-NN) as noise (default class 7). Scale `radius` to the working set's own spacing: a strided 1-in-N sample is N times sparser than the source, so either raise the radius to match (≈3× the average point spacing) or build the LOD index first so the classifiers run over dense tiles. |
| `POINTCLOUDRULE <field> <op> <a> [b] <class>` | Rule classification over any attribute (`ELEVATION`, `INTENSITY`, `RETURN`, `SOURCE`) with `LT`, `GT`, `BETWEEN`, or `EQ`. |
| `POINTCLOUDCONTOUR [interval]` | Build a Delaunay TIN over the class-2 ground points (or every point when no ground exists yet) and write chained contour polylines at `interval` onto the `LIDAR-CONTOURS` layer as CAD entities. |
| `POINTCLOUDFLAGSELECTION <flag> <ON/OFF>` | Change `WITHHELD`, `OVERLAP`, `KEY`, or `SYNTHETIC` on the active selection. |
| `POINTCLOUDELEVATIONSELECTION <z>` | Set an elevation patch on the active selection. |
| `POINTCLOUDUNDO` | Undo the most recent sparse point edit transaction. This is separate from CAD entity undo. |
| `POINTCLOUDPTCIMPORT` / `POINTCLOUDPTCEXPORT` | Pick and import/export an editable `.ptc` class/color table. A path argument is also accepted. |
| `POINTCLOUDCLASSADD` | Add the next available class code to the manager's editable class grid. |
| `POINTCLOUDCRS` | Report WKT/GeoTIFF CRS source, horizontal/vertical EPSG identifiers, and survey-product readiness. |
| `POINTCLOUDREPROJECT <EPSG>` | Pick an output LAS/LAZ and stream a reprojected copy. Sparse edits are applied, XY transforms, and Z is deliberately preserved. |
| `MNUIMPORT` / `MNUEXPORT` | Pick and import/export `$FK5.0$` function-key `.mnu` files. A path argument is also accepted. |
| `POINTCLOUDEXPORT` | Pick a new `.las`/`.laz` path and stream the full source cloud with pending sparse edits applied. |
| `POINTCLOUDEXPORTALL [path]` | Stream every attached source into one merged `.las`/`.laz` (picker, or a path argument). Sources must share LAS version, point format and horizontal CRS; each point's `point_source_id` records which file it came from (1..=N). |
| `POINTCLOUDEXPORTSTATUS` / `POINTCLOUDEXPORTCANCEL` | Report or cancel a background export. |
| `POINTCLOUDDETACH` | Remove the session attachment without modifying the source file. |

`POINTCLOUDCLASSIFY` uses zero-based physical record indices from the source
LAS/LAZ. A display point retains this index even when the viewer uses a stride,
so edits are applied to the correct full-resolution record during export.
The active selection is highlighted amber in the GPU view before editing.

## Storage, safety, and fidelity

- LAS and compressed LAZ input/output through `las-rs` with parallel LAZ
  support.
- Header-only metadata inspection and a bounded initial sample (up to one
  million points).
- Native `wgpu` instanced point quads with depth testing, fixed screen size,
  circular antialiasing, and high/low relative-to-eye coordinates for survey
  coordinate precision.
- GPU-side colorization: instances carry source attributes (class, intensity,
  returns, point source, RGB) and the shader computes classification, RGB,
  intensity, elevation, return-number and point-source coloring from a style
  uniform. Changing color mode, class visibility, class colors or point size
  rewrites only that uniform — the instance buffer is rebuilt solely when the
  point set or per-point attributes change (tile loads, edits, selections).
- A versioned, rebuildable tiled LOD cache adjacent to the source:
  `<cloud>.las.ocstiles` or `<cloud>.laz.ocstiles`. It retains full leaf records,
  deterministic coarser levels, caps simultaneously open tile files, and
  rejects a cache when the source fingerprint has changed.
- Camera-frustum-driven tile selection chooses the finest visible level that
  fits the point, CPU-memory, and GPU-memory budgets. Missing tiles load on
  worker threads — one source streams at a time (round-robin per camera tick)
  with up to `min(cores, 8)` parallel tile readers per batch — and an LRU
  retains recently used CPU tiles; the GPU model holds only the active
  visible working set.
- Direct viewport point, window, polygon-fence, and fixed-pixel brush queries
  use a camera-generation-keyed screen spatial grid. Attribute filters and all
  resulting edit transactions retain stable source indices.
- Sparse source-indexed edit patches, transaction audit data, undo, and compact
  selection ranges rather than an in-memory copy of every edited source point.
- A versioned SQLite sidecar adjacent to a saved drawing: `<drawing>.ocspc`.
  It stores the attachment fingerprint/path, display settings, class table,
  sparse edits, selection sets, audit log, job schema, drawing CRS, working
  units, and manual basemap bounds. Spatial settings exist without a point-cloud
  attachment. Relative-path repair allows a drawing, source, cache, and sidecar
  to move together.
- LAS class 12 overlap handling using the LAS 1.4 overlap flag convention.
- Streaming export preserves the source header, CRS VLRs, point format,
  coordinate transforms, extra bytes, GPS time, color, intensity, returns, and
  other attributes supported by the source format.
- Export refuses an existing destination or the source path. It writes an
  adjacent temporary file and renames it only after a successful close; cancel
  removes the unpublished temporary result.
- `.ptc` parsing accepts header-aware CSV, semicolon, tab, and whitespace forms.
  `.mnu` support reads/writes `$FK5.0$` function keys and preserves unsupported
  VBA/MDL/Scan key-ins with visible compatibility warnings.
- WKT and GeoTIFF CRS records are inspected into horizontal/vertical EPSG
  identifiers when possible. Survey-product readiness blocks missing,
  unresolved, geographic, or invalid coordinate systems and warns when the
  vertical datum is unresolved.
- Copy reprojection uses a bundled pure-Rust EPSG/PROJ pipeline, densifies the
  transformed source envelope, writes safe LAS XY offsets/scales, updates the
  output WKT, applies sparse edits, and preserves Z rather than silently
  applying an unverified vertical-datum conversion.

## Important current limits

This is a production-oriented foundation and file-integrity path, not yet a
complete TerraScan replacement.

- LOD adapts by visible tile count and configured memory/point budgets. It does
  not yet use per-tile projected spacing/error metrics or direct COPC HTTP range
  streaming.
- Viewport point, box, fence, lasso, and continuous mouse-down brush selection
  operate on the resident LOD working set; queries against unloaded points
  require loading denser tiles first.
- Saved drawings persist sidecars automatically. An unsaved drawing cannot have
  an adjacent durable sidecar until it is first saved.
- Ground, isolated-noise, attribute-rule classification, and DTM contours are
  available. Building/vegetation classifiers, flight-line processing,
  thinning, and a watched-folder production queue remain future work.
- Horizontal reprojection supports EPSG definitions available in the bundled
  pure-Rust database. Grid-based and orthometric vertical datum transformations
  require a separately validated geodetic backend; v0.9.6 preserves Z and says
  so in the UI and audit log.
- COPC, E57, PTS/PTX, raster surface export, and point-cloud-to-CAD feature
  extraction are not implemented.

## Next production increments

1. Add projected-spacing screen-error refinement and native COPC/E57 readers.
2. Add a named selection-set organizer and point-to-plane/cloud-to-cloud
   measurement modes.
3. Add watched-folder processing, continuously updating in-modal job progress
   bars, and export queueing.
4. Compatibility-test `.ptc` and `.mnu` parsing against representative files
   from the user's MicroStation/TerraScan production environment.
5. Add validated raster surface export, breaklines, and specialized automated
   classifiers; every entry point must pass the survey-readiness gate.

The `ocs_pointcloud` workspace crate is intentionally UI-independent so these
increments can be tested against real LAS/LAZ fixtures without loading the CAD
application.
