# Open CAD Studio 3D development plan

This plan builds on the 2.0.1 foundation: persistent CAD mesh entities,
view-dependent mesh LODs, 3MF import, solid/mesh measurements, 3D transforms,
LiDAR display and editing, ground classification, TIN contours, and raster
surface generation. The goal is one coherent 3D workflow rather than separate
model-viewer and point-cloud features.

## Status

- The v2.1.0 release candidate now contains the stable production slice of
  Weeks 1–5. Week 1 provides hardened 3MF import, mesh LODs, and atomic terrain
  draping. Week 2 adds topology diagnostics and undoable mesh repair commands.
  Week 3 adds atomic 3MF Core export and tested material/unit/topology round
  trips.
- Terrain work now records full-density DTM/DSM/hillshade outputs as
  fingerprinted, CRS-aware project sources with regeneration recipes.
  `POINTCLOUDBREAKLINECHECK` validates selected 3D breaklines,
  `POINTCLOUDSURFACEAT` inspects an interpolated elevation, and
  `POINTCLOUDDRAPE` supports arcs, circles, ellipses, splines, closed
  footprints, 2D/3D polylines, and mesh copies.
- The existing spatial-project CRS/local-origin model, bounded point budgets,
  tile-backed full-density raster jobs, cross-section manager, LOD telemetry,
  installer associations, and large real-model gates provide the Week 5
  integration and release baseline. See
  [the compatibility matrix](v2.1.0-3d-compatibility.md).
- Advanced interactive sub-object overlays, hierarchy-to-block preservation,
  constrained-Delaunay breakline rebuilding, image/corridor draping, surface
  difference heat maps, occlusion culling, and signed-release infrastructure
  remain explicitly post-v2.1.0. They are not silently represented as
  production-ready features.

## Product principles

- Preserve source coordinates, units, materials, and spatial-reference
  provenance. Never silently flatten or reproject geometry.
- Keep large source data external and immutable. Store edits, derived products,
  and audit history separately until the user explicitly exports.
- Make expensive operations cancellable and move them off the UI thread.
- Use exact source geometry for editing and measurements; use generated LODs
  only for display.
- Every geometry mutation must be undoable and covered by a deterministic test.

## Week 1 — shared 3D and terrain foundation

Deliverables:

- Add a reusable surface-sampling and path-draping API with input validation,
  output-size limits, vertical offsets, and atomic failure outside the surface.
- Add `POINTCLOUDDRAPE [spacing] [vertical-offset]` to turn selected CAD lines
  and polylines into terrain-following 3D polylines.
- Use class-2 ground points when available and clearly report fallback to all
  resident points.
- Add 3MF import diagnostics for object counts, units, bounds, materials,
  components, and skipped package features.
- Establish representative 3MF and LiDAR performance fixtures and record
  import, TIN-build, drape, render-cache, and peak-memory baselines.

Exit criteria: a selected path can be draped without altering its source; all
new geometry is undoable; invalid spacing, excessive subdivision, and coverage
gaps fail without partial output.

## Week 2 — practical mesh editing

Deliverables:

- Add a topology cache for vertex/edge/face adjacency, boundaries, manifold
  status, connected components, and consistently oriented normals.
- Add vertex, edge, face, and connected-component selection modes with visible
  hover/selection overlays that do not duplicate the full mesh on the CPU.
- Implement move, rotate, scale, delete face, flip normals, weld-by-tolerance,
  fill small hole, and recalculate normals.
- Add object-level 3D transform handles aligned to WCS, UCS, or local axes.
- Keep imported multi-million-triangle models object-editable while restricting
  sub-object editing to an explicit working region or decimated proxy.

Exit criteria: common repairs and local changes are undoable, save to DWG/DXF,
and do not invalidate unrelated mesh LODs or scene caches.

## Week 3 — 3MF round-trip and model organization

Deliverables:

- Implement 3MF export for meshes, units, build items, component transforms,
  base materials, colors, names, and package relationships.
- Preserve the imported object/component hierarchy instead of presenting every
  component only as an unrelated CAD mesh.
- Add an import report and options for model centering, unit override, component
  merging, material preservation, and display-LOD generation.
- Add background import/export progress, cancellation, memory budgeting, and
  package security/complexity diagnostics.
- Add mesh repair-on-copy options rather than mutating source data implicitly.

Exit criteria: supported 3MF models can be imported, transformed or repaired,
exported, and reopened with equivalent bounds, topology, units, and materials.

## Week 4 — production LiDAR terrain editing

Deliverables:

- Promote TIN/raster outputs to persistent drawing-linked surface objects with
  source fingerprints, CRS, class filter, resolution, and regeneration recipe.
- Add breakline ingestion and constrained surface rebuilds; validate crossings,
  duplicate vertices, spikes, and surface-boundary gaps.
- Extend draping to arcs, splines, closed footprints, meshes, imagery, and
  corridors, with replace/copy and vertical-offset options.
- Add terrain-aware snapping, point-to-surface inspection, cut/fill profiles,
  cross sections, and surface difference heat maps.
- Run TIN construction and draping against full-density indexed tiles in a
  bounded background job rather than only the resident display set.

Exit criteria: a user can classify ground, build a traceable surface, add
breaklines, drape design geometry, inspect elevations, and regenerate results
after source edits.

## Week 5 — integration, performance, and release hardening

Deliverables:

- Add shared spatial-reference and local-origin handling across CAD, 3MF,
  meshes, point clouds, surfaces, and basemaps.
- Add GPU frustum/occlusion culling, bounded upload queues, mesh and point-cloud
  memory telemetry, and graceful degradation before allocation failure.
- Add end-to-end fixtures for large 3MF, LAS/LAZ, mixed CAD/LiDAR drawings,
  save/reopen, undo/redo, and installer file associations.
- Add a 3D workspace panel for render mode, selection mode, transform space,
  mesh diagnostics, surface source, and active LOD/budget information.
- Publish a signed installer candidate with migration notes and a documented
  compatibility matrix.

Exit criteria: the complete workflow is measurable, recoverable, documented,
and stable on both integrated and discrete GPUs at the supported memory floor.

## Deferred until the foundation is stable

- Sculpting and brush-based mesh deformation.
- Volumetric voxel editing.
- Texture painting and UV unwrapping.
- Parametric history for arbitrary imported triangle meshes.
- Distributed or cloud-hosted point-cloud processing.

These are valuable, but they should not displace topology correctness, 3MF
round-trip, surface provenance, or bounded-memory processing.

## Next three weeks after v2.1.0

### Week 1 — interaction and import/export jobs

- Add a bounded working-region model for vertex/edge/face selection and visible
  hover/selection overlays. Keep whole-object transforms as the fallback above
  the supported sub-object budget.
- Move 3MF import/export to cancellable jobs with progress, peak-memory
  telemetry, explicit centering/unit/material controls, and a repair-on-copy
  export option.
- Persist a small assembly graph so 3MF components can round-trip as CAD blocks
  without changing the exact mesh entities used by the renderer.

Gate: deterministic selection/edit tests, cancellation at every job phase, and
component-transform round trips on nested fixtures and the Boston HIGH model.

### Week 2 — editable terrain surfaces

- Introduce a persistent full-density TIN surface object backed by indexed
  source tiles and the existing regeneration recipe.
- Apply validated breaklines through a constrained-Delaunay rebuild, with
  preview, cancel, undo, and explicit handling for boundary gaps.
- Add terrain-aware snapping, cut/fill profiles, and surface-difference rasters;
  keep image and corridor draping behind an experimental flag until coverage
  and coordinate-space tests are complete.

Gate: regenerate after source/classification changes with identical provenance,
bounded memory, and numeric elevation/volume fixtures.

### Week 3 — workspace, performance, and signed release automation

- Consolidate render mode, transform space, mesh diagnostics, active surface,
  LOD, and memory budgets into one 3D workspace panel.
- Add conservative GPU occlusion culling behind telemetry and automatic
  fallback, then test integrated-GPU and discrete-GPU memory floors.
- Exercise mixed CAD/3MF/LAS save-reopen and undo-redo flows in CI, install the
  MSI in a clean Windows VM, and enable Azure signing once repository signing
  credentials are available.

Gate: no regression in exact geometry or large-fixture tests, graceful fallback
under memory pressure, clean upgrade/uninstall, and signed/hash-published
artifacts from the release workflow.
