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
provides color and point-size controls, selection prompts, common ASPRS classes,
point flags, statistics, class-table interchange, export progress/cancel, and
detach. `POINTCLOUDMANAGER` opens it from the command line or a function key.

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
| `POINTCLOUDSELECTPOINT` | Prompt for `X Y Z search-radius` and create the active source-index selection. |
| `POINTCLOUDSELECTBOX` | Prompt for `minX minY minZ maxX maxY maxZ` and create the active selection. |
| `POINTCLOUDSELECTBRUSH` | Prompt for `centerX centerY centerZ radius` and create the active selection. |
| `POINTCLOUDSELECTSLICE` | Select the resident points between two survey elevations. |
| `POINTCLOUDSELECTFILTER` | Set/clear persistent class, return, source, elevation, synthetic, key, withheld, or overlap filters used by spatial selections. |
| `POINTCLOUDSELECTCLEAR` | Clear the active highlighted selection without changing saved edits. |
| `POINTCLOUDBRUSHCLASSIFY <class>` | Function-key-friendly world-space brush prompt that selects and classifies in one transaction. |
| `POINTCLOUDCLASSIFYSELECTION <class>` | Reclassify the active selection as one transaction. |
| `POINTCLOUDFLAGSELECTION <flag> <ON/OFF>` | Change `WITHHELD`, `OVERLAP`, `KEY`, or `SYNTHETIC` on the active selection. |
| `POINTCLOUDELEVATIONSELECTION <z>` | Set an elevation patch on the active selection. |
| `POINTCLOUDUNDO` | Undo the most recent sparse point edit transaction. This is separate from CAD entity undo. |
| `POINTCLOUDPTCIMPORT` / `POINTCLOUDPTCEXPORT` | Pick and import/export an editable `.ptc` class/color table. A path argument is also accepted. |
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

## Important current limits

This is a production-oriented foundation and file-integrity path, not yet a
complete TerraScan replacement.

- The cache currently loads the best whole-cloud LOD under the point budget.
  Camera-frustum tile streaming, GPU/CPU residency eviction, and continuously
  adaptive screen-error LOD selection are the next renderer increment.
- Point/box/brush selection currently uses survey-coordinate prompts and the
  resident sample/LOD. Direct viewport click, screen fence/lasso/polygon,
  screen-space brush strokes, and graphical filter widgets are not connected
  yet. Elevation slices and class/return/source/flag filters are available from
  the manager's command prompts.
- Saved drawings persist sidecars automatically. An unsaved drawing cannot have
  an adjacent durable sidecar until it is first saved.
- Class tables are editable through `.ptc` interchange and class visibility
  commands; an in-app row editor for names/colors/locks is still pending.
- There is no automatic ground/building/vegetation classification, noise
  detection, flight-line processing, tiling, thinning, or batch macro engine.
- CRS metadata is preserved and reported, but coordinates are not reprojected.
- COPC, E57, PTS/PTX, raster surfaces, contours, and point-cloud-to-CAD feature
  extraction are not implemented.

## Next production increments

1. Connect frustum-aware tile streaming and budgeted CPU/GPU LRU residency to
   camera movement.
2. Connect viewport point picking, fence/lasso/polygon, brush strokes, slice
   planes, and graphical filter widgets to the implemented source-index query
   and edit model.
3. Add an in-app class-table grid, audit-log viewer, selection-set manager, and
   continuously updating job progress bars.
4. Compatibility-test `.ptc` and `.mnu` parsing against representative files
   from the user's MicroStation/TerraScan production environment.
5. Add CRS inspection/reprojection and survey-coordinate safeguards before
   surface generation, contours, breaklines, and automated classifiers.

The `ocs_pointcloud` workspace crate is intentionally UI-independent so these
increments can be tested against real LAS/LAZ fixtures without loading the CAD
application.
