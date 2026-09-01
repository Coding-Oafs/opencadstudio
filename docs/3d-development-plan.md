# Open CAD Studio 3D development plan

This plan builds on the 2.0.1 foundation: persistent CAD mesh entities,
view-dependent mesh LODs, 3MF import, solid/mesh measurements, 3D transforms,
LiDAR display and editing, ground classification, TIN contours, and raster
surface generation. The goal is one coherent 3D workflow rather than separate
model-viewer and point-cloud features.

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
