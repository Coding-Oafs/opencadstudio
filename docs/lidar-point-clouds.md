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

## Commands

| Command | Purpose |
| --- | --- |
| `POINTCLOUDATTACH` | Pick a `.las` or `.laz`, inspect it, and attach a bounded initial GPU display sample. |
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
| `POINTCLOUDSELECTBOX` | Pick two viewport corners for a screen-space window selection. Coordinate arguments remain available for scripts. |
| `POINTCLOUDSELECTFENCE` | Click a screen-space polygon fence and press Enter to close it. |
| `POINTCLOUDSELECTBRUSH` | Apply a repeating 32-pixel viewport selection brush; press Enter to finish. Coordinate arguments remain available for scripts. |
| `POINTCLOUDSELECTSLICE` | Select the resident points between two survey elevations. |
| `POINTCLOUDSELECTFILTER` | Set/clear persistent class, return, source, elevation, synthetic, key, withheld, or overlap filters used by spatial selections. |
| `POINTCLOUDSELECTCLEAR` | Clear the active highlighted selection without changing saved edits. |
| `POINTCLOUDBRUSHCLASSIFY <class>` | Function-key-friendly repeating fixed-pixel viewport brush that selects and classifies source points. |
| `POINTCLOUDCLASSIFYSELECTION <class>` | Reclassify the active selection as one transaction. |
| `POINTCLOUDFLAGSELECTION <flag> <ON/OFF>` | Change `WITHHELD`, `OVERLAP`, `KEY`, or `SYNTHETIC` on the active selection. |
| `POINTCLOUDELEVATIONSELECTION <z>` | Set an elevation patch on the active selection. |
| `POINTCLOUDUNDO` | Undo the most recent sparse point edit transaction. This is separate from CAD entity undo. |
| `POINTCLOUDPTCIMPORT` / `POINTCLOUDPTCEXPORT` | Pick and import/export an editable `.ptc` class/color table. A path argument is also accepted. |
| `POINTCLOUDCLASSADD` | Add the next available class code to the manager's editable class grid. |
| `POINTCLOUDCRS` | Report WKT/GeoTIFF CRS source, horizontal/vertical EPSG identifiers, and survey-product readiness. |
| `POINTCLOUDREPROJECT <EPSG>` | Pick an output LAS/LAZ and stream a reprojected copy. Sparse edits are applied, XY transforms, and Z is deliberately preserved. |
| `MNUIMPORT` / `MNUEXPORT` | Pick and import/export `$FK5.0$` function-key `.mnu` files. A path argument is also accepted. |
| `POINTCLOUDEXPORT` | Pick a new `.las`/`.laz` path and stream the full source cloud with pending sparse edits applied. |
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
- Classification, RGB, intensity, elevation, return-number, and point-source
  GPU color modes.
- A versioned, rebuildable tiled LOD cache adjacent to the source:
  `<cloud>.las.ocstiles` or `<cloud>.laz.ocstiles`. It retains full leaf records,
  deterministic coarser levels, caps simultaneously open tile files, and
  rejects a cache when the source fingerprint has changed.
- Camera-frustum-driven tile selection chooses the finest visible level that
  fits the point, CPU-memory, and GPU-memory budgets. Missing tiles load on a
  worker thread; an LRU retains recently used CPU tiles and the GPU model holds
  only the active visible working set.
- Direct viewport point, window, polygon-fence, and fixed-pixel brush queries
  use a camera-generation-keyed screen spatial grid. Attribute filters and all
  resulting edit transactions retain stable source indices.
- Sparse source-indexed edit patches, transaction audit data, undo, and compact
  selection ranges rather than an in-memory copy of every edited source point.
- A versioned SQLite sidecar adjacent to a saved drawing: `<drawing>.ocspc`.
  It stores the attachment fingerprint/path, display settings, class table,
  sparse edits, selection sets, audit log, and job schema. Relative-path repair
  allows a drawing, source, cache, and sidecar to move together.
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
- The fixed-pixel brush is a repeating click brush in v0.9.6; a continuous
  mouse-down paint stroke and freehand lasso overlay are future refinements.
- Saved drawings persist sidecars automatically. An unsaved drawing cannot have
  an adjacent durable sidecar until it is first saved.
- There is no automatic ground/building/vegetation classification, noise
  detection, flight-line processing, tiling, thinning, or batch macro engine.
- Horizontal reprojection supports EPSG definitions available in the bundled
  pure-Rust database. Grid-based and orthometric vertical datum transformations
  require a separately validated geodetic backend; v0.9.6 preserves Z and says
  so in the UI and audit log.
- COPC, E57, PTS/PTX, raster surfaces, contours, and point-cloud-to-CAD feature
  extraction are not implemented.

## Next production increments

1. Add projected-spacing screen-error refinement and native COPC/E57 readers.
2. Add continuous mouse-down brush painting, a freehand lasso overlay, and a
   named selection-set organizer.
3. Add continuously updating in-modal job progress bars and export queueing.
4. Compatibility-test `.ptc` and `.mnu` parsing against representative files
   from the user's MicroStation/TerraScan production environment.
5. Add validated surface generation, contours, breaklines, and automated
   classifiers; every entry point must pass the v0.9.6 survey-readiness gate.

The `ocs_pointcloud` workspace crate is intentionally UI-independent so these
increments can be tested against real LAS/LAZ fixtures without loading the CAD
application.
