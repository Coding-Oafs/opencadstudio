# LAS/LAZ point-cloud workflow

OpenCADStudio has an initial native Windows workflow for attaching, inspecting,
viewing, reclassifying, and exporting LAS/LAZ point clouds. The implementation
keeps the full cloud outside the CAD entity graph: a document tab retains a
bounded display sample and a sparse edit map, while export streams every source
record into a new file.

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

## Current commands

| Command | Purpose |
| --- | --- |
| `POINTCLOUDATTACH` | Pick a `.las` or `.laz`, read its metadata, and attach a bounded display sample. The Insert ribbon's Point Cloud > Attach button uses this command. |
| `POINTCLOUDINFO` | Report source path, source/display point counts, sample stride, pending edits, CRS-VLR presence, and VLR/EVLR counts. |
| `POINTCLOUDCLASSIFY <class> <indices>` | Queue an ASPRS class for source indices. Indices accept comma-separated values and inclusive ranges, for example `POINTCLOUDCLASSIFY 2 10-25,40`. |
| `POINTCLOUDUNDO` | Undo the most recent point-cloud classification transaction. This is separate from CAD entity undo. |
| `POINTCLOUDEXPORT` | Pick a new `.las`/`.laz` path and stream the full source cloud with pending edits applied. |
| `POINTCLOUDDETACH` | Remove the session attachment without modifying the source file. |

`POINTCLOUDCLASSIFY` uses zero-based physical record indices from the source
LAS/LAZ. A display point retains this index even when the viewer uses a stride,
so edits are applied to the correct full-resolution record during export.

## Implemented safety and fidelity

- LAS and compressed LAZ input/output through `las-rs` with parallel LAZ
  support.
- Header-only metadata inspection and bounded-memory sampling.
- A default display cap of 50,000 approximately uniform source records, colored
  by classification.
- Sparse, transactional edits rather than an in-memory copy of every point.
- LAS class 12 overlap handling using the LAS 1.4 overlap flag convention.
- Streaming export preserves the source header, CRS VLRs, point format,
  coordinate transforms, extra bytes, GPS time, color, intensity, returns, and
  other attributes supported by the source format.
- Export refuses an existing destination or the source path. It writes an
  adjacent temporary file and renames it only after a successful close.

## Important current limits

This is a usable foundation and file-integrity path, not yet a TerraScan
replacement.

- The display uses small high-precision wire crosses, not a dedicated GPU point
  primitive, octree, or screen-space level-of-detail renderer.
- Attachments and edit maps are session-only; they are not yet persisted in DWG
  data or an OpenCADStudio sidecar.
- Reclassification currently targets explicit source indices. Viewport picking,
  fence/polygon selection, brush tools, profiles, and class/elevation/return
  filters are not yet connected.
- There is no automatic ground/building/vegetation classification, noise
  detection, flight-line processing, tiling, thinning, or batch macro engine.
- CRS metadata is preserved and reported, but coordinates are not reprojected.
- COPC, E57, PTS/PTX, raster surfaces, contours, and point-cloud-to-CAD feature
  extraction are not implemented.

## Recommended next increments

1. Add a native GPU point pipeline with fixed screen-size points, depth testing,
   classification/RGB/intensity color modes, and an adaptive octree or tiled
   level of detail.
2. Add spatial picking and selection sets (single point, screen fence, polygon,
   brush, elevation slice, class/return/source filters) that emit source-index
   edit transactions.
3. Persist attachment metadata and sparse edits in a versioned sidecar adjacent
   to the drawing, with relative-path repair for moved projects.
4. Add classification statistics, editable class tables, withheld/overlap/key
   point flag tools, edit audit logs, and background export progress/cancel.
5. Add CRS inspection/reprojection and survey-coordinate safeguards before
   surface generation, contours, breaklines, and automated classifiers.

The `ocs_pointcloud` workspace crate is intentionally UI-independent so these
increments can be tested against real LAS/LAZ fixtures without loading the CAD
application.
