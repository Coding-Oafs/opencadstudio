# OpenCADStudio v0.9.7 → v1.0 roadmap: production LiDAR + scripting

## v1.0 implementation update — 20 August 2026

The original phase estimates below are retained as historical planning context.
The v1 hardening pass is now implemented on `v0.9.7-lidar-platform`:

- basemap requests use bounded parallel workers, cancellation/stale-job guards,
  a persistent provider/XYZ disk cache, visible progress/failure counts, manual
  empty-drawing bounds, and first-load auto-fit;
- drawing-owned `CRS`, `WORKINGUNITS`, and basemap bounds persist in sidecar
  schema v4 without requiring LAS/LAZ;
- View/Annotate/LiDAR/tool-palette entries expose Select, Navigator, distance,
  area, closed-mesh volume, and snapped point-cloud distance;
- the v0.9.7 point-arena zero-shard assertion and seam-bounded sphere
  tessellation release blockers are fixed and regression-tested;
- remaining release work is versioning, full verification, fast-forwarding the
  default branch, producing/installing the WiX MSI, and publishing `v1.0.0`.

See `docs/v1.0-release-plan.md` for the reviewed implementation record and
release acceptance gates.

Status: Phases 0–2 complete on `v0.9.7-lidar-platform` (August 2026) —
the multi-source dataset engine, folder ingest, merged export, parallel
tile IO, shader-side colorization, GPU point arena, Rhai macro scripting
with a batch classifier script, automated ground/noise/rule
classification, and DTM contour generation are all landed and validated
against a 418M-point USGS dataset. Remaining for v1.0: the optional PyO3
engine and release packaging (GUI smoke pass, version notes).
Baseline: v0.9.6 (`adaptive LiDAR workflows`)

## Vision

MicroStation + TerraScan in one standalone open-source program. OpenCADStudio
keeps its native drafting/design edge (DWG/DXF R13–2018, layouts, ribbon UI)
and puts all new capacity into what FreeCAD and friends cannot do: folder-scale
LAS/LAZ streaming, TerraScan-style classification and batch production
workflows, and a dual-engine scripting layer (embedded Python + Rhai) with
built-in macros.

This cycle deliberately does **not** chase FreeCAD. No parametric feature
tree, no constraint sketcher, no STEP/IGES import, no assemblies, CAM, or FEM.
Those ideas live in the earlier v1 comparison write-up; they are not this
roadmap.

## Release strategy

- **v0.9.7 — the platform.** Multi-file/folder dataset engine, parallel
  streaming, incremental GPU paging, shader colorization, embedded Python +
  Rhai scripting core, macro runner, built-in script library.
- **v1.0 — the production toolset.** Automated classifiers (ground, noise,
  rule-based), batch folder processing, DTM/contour generation, interaction
  refinements, hardening and scale soak.

Estimated duration: ~18 weeks of focused work. Phases 0 and 1 overlap.

---

## Phase 0 — Multi-source dataset foundation (weeks 1–4) → v0.9.7

### 0.1 Dataset refactor

Replace the single attachment (`point_cloud: Option<PointCloudAttachment>` in
`src/app/document.rs`) with:

```text
PointCloudDataset { sources: Vec<CloudSource> }
```

Each `CloudSource` wraps today's attachment state: manifest, tile cache,
sparse edits, display flags. A composite `PointId { source: SourceId,
index: u64 }` flows through selections, edit transactions, and point-cloud
undo, preserving today's stable per-source record indices.

Sidecar v3 (`crates/ocs_pointcloud/src/sidecar.rs`, SCHEMA_VERSION 2 → 3):
the `attachments`, `selection_sets`, `audit_log`, and `export_jobs` tables
already key on `attachment_id`, so multi-source is mostly a matter of writing
and reading multiple rows, plus a `collections` table for saved folder sets.
v0.9.6 sidecars migrate by promoting the `id="primary"` row to a one-source
dataset.

### 0.2 Folder ingest

- `POINTCLOUDATTACHFOLDER <path>`: recursive scan for `.las`/`.laz`,
  multi-select file dialog, and OS folder drag-drop.
- Per-source fingerprinting (reuse the existing fingerprint logic) and
  staggered initial sampling so attaching 50 files does not spike RAM.
- Source tree list in the LiDAR Point Cloud Manager with per-source
  attach/collapse state, stats, and CRS badges.
- `POINTCLOUDEXPORTALL <out.las|.laz>`: merge export streaming each source in
  order, writing a distinct `point_source_id` per source file. A union-CRS
  check (existing survey-readiness gate) blocks mixed-CRS merges with a clear
  error.

### 0.3 Parallel streaming

Replace the single reader thread (`std::thread::spawn` in
`src/app/point_cloud.rs`) with a bounded worker pool (min(cores, 8)) and
per-source tile queues with round-robin fairness so one huge file cannot
starve the rest. Keep camera-generation-keyed request invalidation.

### 0.4 Incremental GPU paging — highest-risk item

Today `PointGpu::upload` (`src/scene/pipeline/point_gpu.rs`) recreates the
whole instance buffer on every working-set change (~192 MB at 4M points;
unusable at 100M). Target design:

- Persistent GPU point arena with fixed tile slots; staging-buffer uploads
  only for tiles entering or leaving the working set.
- Relative-to-eye high/low coordinates move to per-frame uniforms so camera
  motion costs approximately zero buffer traffic.
- Slot LRU eviction mirroring the CPU tile LRU; residency generation counter
  drops stale uploads.

**Prototype in weeks 1–2 behind a feature flag**, with fallback to the
current full-upload path. Gate before any UI work depends on it:

> 8 files / ~100M total points attached; camera move moves <5 MB of buffer
> traffic; class visibility toggle <16 ms; tile pop-in comparable to v0.9.6.

### 0.5 Shader-side colorization

Move the class-color table, color mode, and class visibility into bind-group
uniforms (classification/intensity are already vertex attributes). This
removes the CPU per-point color rebuild (`src/app/point_cloud.rs`) so
`POINTCLOUDCOLOR` and class-visibility changes become free uniform updates.

---

## Phase 1 — Scripting core (weeks 5–8, overlaps Phase 0) → v0.9.7

### 1.1 New workspace crate `ocs_scripting`

An engine-agnostic `OcsScriptApi` facade with two engines:

- **Python via PyO3** — embedded CPython, desktop only. Cargo feature
  `python`, enabled for native builds, never for wasm.
- **Rhai** — pure Rust; works in both native and web builds.

The facade reuses the request implementations already proven in
`src/app/automation.rs` (`new`, `open`, `run`, `entities`, `query`, `layers`,
`header`, `select`, `undo`, `redo`, `save`). Headless JSON, embedded Python,
and Rhai therefore share one code path, and `run` coverage grows with the
app's own command system.

Threading model: scripts execute on a dedicated worker thread; document and
scene access marshals through the app thread via a channel (the same pattern
as tile IO). Progress and stdout stream to a console pane; long-running
operations are cancellable.

### 1.2 Point-cloud API surface (both engines)

- `attach(path)`, `attach_folder(path)`, `detach(source)`
- Dataset and per-source stats, metadata, CRS
- Filters (class/return/source/elevation/synthetic/key/withheld/overlap)
- Selections: box/fence/slice/nearest + named selection sets
- Edits: `classify`, `flag`, `elevate`, `undo`
- Output: `export`, `export_status`, `reproject`

Scripts drive the same audited transactions as UI commands
(`ClassificationEdits` + sidecar `audit_log`). There is no side door.

### 1.3 Macro runner and UI

- Script Manager pane: list/edit/run/reload `scripts/library/*.py` and
  `*.rhai`, pin-to-toolbar.
- Extend the existing MNU `$FK` function-key system so a script binds to a
  function key — TerraScan-style keyboard macros.
- Console output pane with script stdout, progress, and errors.

### 1.4 Built-in script library

Shipped, documented, and used as integration tests:

- `elevate_by_z.py` — TerraScan "elevate points" analog
- `classify_by_intensity.py` — rule-based classification example
- `export_per_class.py` — split a cloud into per-class LAS files
- `class_table_apply.py` — `.ptc` class-table round trip across a dataset
- `folder_health_check.py` — CRS/fingerprint/overlap report across a folder

### v0.9.7 release gate

- A folder of ≥8 real production LAS/LAZ files attaches, streams, edits, and
  exports merged.
- A Python macro runs the full attach → filter → classify → export pipeline
  unattended.
- v0.9.6 drawings and sidecars migrate cleanly.
- The wasm build is unaffected (`python` feature off; Rhai + JSON automation
  documented as the web scripting path).
- Docs updated: `docs/lidar-point-clouds.md` limits section, new
  `docs/scripting.md`.

---

## Phase 2 — Production toolset (weeks 9–15) → v1.0

All algorithms live in the UI-independent `ocs_pointcloud` crate (per its
stated design goal), are tested against LAS fixtures, are exposed as both
commands and script functions, and pass the v0.9.6 survey-readiness gate.

### 2.1 Automated classifiers

- **Ground classification** — progressive TIN densification, implemented
  in-house (no LGPL solver vendoring). Seed low points, iterate TIN +
  angle/distance thresholds, classify ground (class 2). Tunable parameters
  (tile size, iteration cap, angles); per-source and dataset-wide modes.
- **Noise/isolated point detection** — voxel-grid k-NN neighbor distance;
  classify to noise classes (7/18).
- **Classify-by-attribute rules** — elevation, intensity, RGB, return number,
  scan angle, class-range predicates composable into pipelines. A rule set is
  data a macro can build and batch-apply.
- Stretch: RANSAC plane detection → building/roof seed classification.

### 2.2 Batch folder processing

Persist a batch job queue by generalizing the existing `export_jobs` sidecar
schema into workflow jobs:

- Apply a saved script/workflow to a folder of LAS/LAZ files.
- Per-file status with continuously updating in-modal progress bars, cancel,
  and failure isolation (one bad file does not kill the batch).
- Per-output audit trail and output naming templates.

This is the TerraScan "batch run macros on a project" analog and the delivery
mechanism for production utility work.

### 2.3 DTM and contours

- Incremental Delaunay TIN from the ground class (delaunator-style,
  memory-bounded streaming over the tile cache).
- Marching-triangles contours at user intervals → CAD polylines with labels
  on layouts (reuses existing layout/drafting machinery — the CAD side
  benefits without new CAD features).
- GeoTIFF DEM export from the TIN (pure-Rust TIFF writer).
- Breaklines: stretch goal.

### 2.4 Interaction refinements (from the lidar doc's next-increments list)

Continuous mouse-down brush painting (replacing the repeating click brush),
freehand lasso overlay, and a named selection-set organizer UI.

### 2.5 LOD refinements

Per-tile projected-spacing screen-error metric (finer than visible-tile-count
adaptation). Stretch: COPC v2 read support and an E57 reader.

---

## Phase 3 — Hardening and v1.0 release (weeks 16–18)

- Scale soak: multi-billion-point folder datasets, 8h+ batch runs, memory
  budget assertions.
- Migration matrix tests v0.9.6 → v0.9.7 → v1.0.
- Windows packaging: CPython runtime strategy for the PyO3 build
  (python3x.dll handling in the installer), and verification that the new
  native dependencies respect the documented single-job paging-file build
  constraint.
- Licensing pass on new dependencies: `pyo3`, `rhai`, and a Delaunay crate
  are MIT/Apache-2.0 compatible; verify each at vendoring time.
- Web parity: wasm build green; Rhai + JSON automation documented as the web
  scripting path.
- Docs: rewrite the limits section of `docs/lidar-point-clouds.md`, publish
  `docs/scripting.md` with a macro cookbook, update README positioning.

---

## Explicitly out of scope this cycle

- Anything FreeCAD-clone: parametric feature tree, constraint sketcher,
  property-bound expressions, STEP import, IGES, assemblies, CAM/FEM,
  TechDraw-style drawing views.
- Sections/profiles → drawings (post-1.0; natural follow-on to DTM work).
- Python on the web build.

## Top risks and mitigations

1. **GPU arena rewrite (0.4)** — riskiest item; prototyped first, behind a
   feature flag, with a fallback to the current full-upload path; gated
   before UI work depends on it.
2. **PyO3/wasm leakage** — the `python` feature is strictly off for the wasm
   target; CI builds both targets from day one.
3. **Ground-classifier quality** — labeled fixture clouds are collected or
   synthesized starting week 1; ship tuned defaults plus exposed parameters
   rather than promising magic.
4. **Windows build fragility** — PyO3 adds native dependencies; verify the
   paging-file constraint early.
5. **Scope discipline** — CAD-side requests go to a backlog file, not this
   cycle.
