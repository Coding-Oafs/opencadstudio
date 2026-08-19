# Plan: TOOLPALETTES, SHEETSET, and LiDAR cross-sections

Status: **first pass landed** (2026-08-19). Decisions from the user:

1. Build **TOOLPALETTES + cross-section together** (one pass), then SHEETSET.
2. Sheet set persistence = **JSON** (not AutoCAD `.dst`).
3. Section runs over the **streamed working set** first.
4. Tool palette = **docked side panel** (like the Properties dock).

Landed in this pass (commits on `v0.9.7-lidar-platform`):
- `942de8a6` — docked TOOLPALETTES panel with seeded LiDAR/edit palettes.
- `02bbd02d` — LiDAR vertical cross-section (shader-side band) + `POINTCLOUDSECTION*` commands + section/view presets.
- `afb9cace` — docked SHEETSET manager (JSON sheet-set model).

**Known follow-ups (not in this pass):** JSON persistence of user-authored
palette/sheet-set edits; applying a sheet's layout after an async open; and the
continuous brush / full-density section-tile streaming noted in §3.4.

Two goals from the request:

1. Real **TOOLPALETTES** and **SHEETSET** commands (both are stubs today that
   print "not yet implemented").
2. A **LiDAR cross-section** tool — "View Laser: Draw Vertical Section" from
   TerraScan — plus plan/side/top view presets and point/vector/polygon editing.

---

## 1. TOOLPALETTES

Today `TOOLPALETTES` is a two-line stub in `src/app/commands/display.rs:341`.
A tool palette is a docked/floating panel of buttons that each run a command —
reusing the existing `ModuleEvent::Command(String)` dispatch, so a palette
button is zero new command plumbing.

**Implementation (scoped, no new subsystems):**

1. **Data model** — `src/ui/window/tool_palettes.rs` (new): a small,
   JSON-persisted list of palettes. Each palette = `{ name, tools: Vec<{label,
   command, icon}> }`. Persist beside user settings (the same location the
   app already uses for `settings.rs`); a default built-in palette ships
   in-tree and seeds the file on first run.
2. **UI** — a dockable panel like the existing properties dock (see
   `src/app/view/mod.rs` `properties_divider()` / `DockSide`), or a modal in
   `ModalKind`. A palette tab strip + a grid of buttons. Clicking a button
   emits `Message::Command("<command>")` — the same path the ribbon uses.
3. **Seed content** — the default palette is the natural home for the LiDAR
   and editing tools in §3/§4 (select fence/brush, classify, flag, section,
   view presets), which is exactly what the request asks for.

**Out of scope (explicitly deferred):** drag-and-drop palette authoring, AutoCAD
`.xtp`/`.atc` import/export. A text-JSON editor + in-app add/remove is enough
for v1.0.

---

## 2. SHEETSET

Today `SHEETSET` is a stub at `src/app/commands/display.rs:348`. A full AutoCAD
sheet-set manager (`.dst` files, cross-drawing sheet lists, field-based title
blocks) is a large project. The request is about *utilities*, so scope it to a
**sheet set manager that drives layouts** — the app already has a layout
manager and named views.

**Implementation (scoped):**

1. **Model** — `src/ui/window/sheetset.rs` (new): a sheet set = an ordered list
   of `{ name, drawing path, layout }` plus a `.dst`-like JSON file for
   persistence (a simple, documented JSON format — not AutoCAD `.dst` binary).
2. **UI** — a tree/table: sheets grouped by drawing, with open/activate/rename.
   Activating a sheet opens the drawing and switches to its layout (reuse
   `layout_manager.rs`'s switch logic). A "publish to PDF/plot" action reuses
   the existing plot/print path (`plot.rs` / `print_all.rs`) per sheet.
3. **Cross-drawing opens** reuse `Message::OpenExternal` / the tab system —
   already multi-tab.

**Out of scope:** `.dst` compatibility, sheet-index fields, automatic
title-block numbering.

---

## 3. LiDAR cross-section ("View Laser: Draw Vertical Section")

This is the core new capability. TerraScan's "View Laser: Draw Vertical
Section" cuts the cloud along a user-drawn line (in plan/top view) and shows a
vertical slice (elevation vs. along-section distance) in a second view, where
the user classifies points.

### 3.1 Current architecture (what it builds on)

- The whole point cloud is drawn from **one shared** `PointCloudModel`
  (`src/scene/mod.rs` `point_cloud: Arc<PointCloudModel>`), cloned into every
  viewport (`src/scene/view/render.rs:3359`). There is one `PointGpu` arena per
  viewport, but the *data* is shared.
- The cloud is a merged, bounded **display sample / streamed LOD** — NOT the
  full-resolution cloud. `sample.stride == 0` means "streamed active tiles".
- Points carry `source_index` (stable LAS record index); edits are sparse
  `(source_index, patch)` in `EditStore`, applied on export. Selections are
  `SelectionSet` ranges over source indices.
- Camera is a single struct with `snap_to_face` / `snap_to_direction`
  (`src/scene/view/camera.rs`); the multi-pane model layout has one `ModelTile`
  per pane (`src/scene/mod.rs:1113`), each with its own `camera`.
- Point picking/brush/fence already exist screen-space in
  `src/app/point_cloud.rs` (`ensure_screen_spatial_index`, `select_brush`,
  `select_polygon`, `screen_candidates`).

### 3.2 The design — a "section" view + a clip filter

The clean way to get a vertical section without a second copy of the pipeline:

1. **Define a section plane** (the cut). A user draws a polyline (the section
   line) in plan view — this already exists as `POINTCLOUDSELECTFENCE` /
   screen-fence machinery. Store it as the *active section*: a center line +
   a half-thickness (`POINTCLOUDSECTION <width>`), or just the fence vertices.

2. **Section membership is a shader/clip concern, not a data copy.** Add two
   uniforms to `Style` (`point_cloud.wgsl`) plus the CPU `PointStyle`
   (`point_cloud_model.rs`): a `section_enable` flag, a section plane
   (or a distance-to-segment function encoded as a few vec4s for a straight
   cut, with a fuller signed-distance-to-polyline for multi-segment cuts as a
   stretch). Points outside the band are either **discarded** (section-only
   view) or **dimmed** (context view). This is one uniform write — the arena
   and instances are untouched, so it is O(1) to move/rotate the section.

3. **The section view is a second Model pane** with an orthographic camera
   looking *along* the section line (side-on). The existing multi-pane model
   layout (`model_tiles`) already supports N panes each with its own camera, and
   `VPORTS 2V` already makes two side-by-side panes. So "section view" =
   (a) split to two panes, (b) snap pane 1's camera to the section's side-on
   direction (`snap_to_direction`), (c) enable the section filter so only the
   band shows.

   - **Plan/top view** and **side view** presets are the same mechanism as
     `VIEW TOP` / `VIEW FRONT` (already dispatch to `Message::ViewCubeSnapWorld`
     in `layerprops.rs:645`). A `VIEW SECTION` / `POINTCLOUDSECTIONVIEW` command
     snaps a pane to the section plane normal.

4. **Classify points in the section view.** Because both panes render the same
   shared point set with the same source indices, the existing
   `POINTCLOUDSELECTBRUSH` / `POINTCLOUDSELECTFENCE` / `POINTCLOUDCLASSIFYSELECTION`
   already work in *any* pane — the screen-space pick runs against the active
   tile's camera. The section view just makes it ergonomic: you brush points
   in the vertical slice and they classify, and the result is identical to
   brushing them in plan view (same source indices).

5. **Rotate/advance the section.** "Rotate and classify points" maps to:
   - `POINTCLOUDSECTION` sets the cut (draw the line).
   - `POINTCLOUDSECTIONWIDTH <w>` sets the band thickness.
   - `POINTCLOUDSECTIONSTEP` / `POINTCLOUDSECTIONMOVE <d>` advances the cut
     perpendicular to itself by `d` (walk the corridor, TerraScan-style).
   - Rotating = re-orient the section line (redraw it), or snap the side-on
     camera; the filter uniform updates.

### 3.3 What this requires in code (work items)

- **CPU**: add section state to `PointCloudDataset`/`PointStyle`
  (`point_cloud_model.rs`), a `section: Option<Section>` field; write it into
  `StyleUniforms`.
- **Shader**: extend `Style` + `vs_main` with the section discard/dim branch
  (`point_cloud.wgsl`).
- **Commands**: `POINTCLOUDSECTION` (draw), `POINTCLOUDSECTIONWIDTH`,
  `POINTCLOUDSECTIONMOVE`, `POINTCLOUDSECTIONVIEW` (snap side-on),
  `POINTCLOUDSECTIONCLEAR`. Wire them in `display.rs` next to the other
  `POINTCLOUD*` commands.
- **Ribbon/tool palette**: add a "Section" group to `src/modules/lidar/mod.rs`
  and the new default tool palette.

### 3.4 Honest limitation to flag

The section operates on the **display/streamed** points, not the full cloud —
so a section is only as dense as the current LOD/sample. For true full-density
sections the section band would need to *pull its own tile set* (a second
streaming query filtered by the section bounds), which is a follow-up. v1 does
the section over the existing working set (correct source indices, coarse when
zoomed out, densifying as the LOD streams in), which is a solid, ship-worthy
first cut.

---

## 4. Editing utilities (points / vectors / polygons)

Mostly *existing* commands; the ask is surface them coherently (tool palette +
ribbon) and fill a couple of gaps:

**Already present (surfacing only):**
- Vector/polygon edit: LINE, PLINE, PEDIT, OFFSET, TRIM, EXTEND, FILLET,
  MOVE/COPY/ROTATE/SCALE/MIRROR, grip editing, PROPERTIES.
- Point edit: `POINTCLOUDCLASSIFYSELECTION`, `POINTCLOUDFLAGSELECTION`,
  `POINTCLOUDELEVATIONSELECTION`, `POINTCLOUDGROUND`/`NOISE`/`RULE`,
  select fence/brush/box/slice/nearest.

**Gaps to fill (small, high-value):**
- `POINTCLOUDSECTION*` (above).
- Continuous brush painting (today it is a repeating click brush; a
  mouse-down drag stroke is a known roadmap item — `docs/lidar-point-clouds.md`
  "Next production increments" #2).
- A "class by elevation band in the section" shortcut — already expressible as
  `POINTCLOUDRULE ELEVATION BETWEEN`, just add a palette button.

---

## 5. Sequencing (recommended order)

1. **TOOLPALETTES** (smallest, unblocks everything else as a home for new
   tools) — model + docked panel + seed palette.
2. **View presets** for point clouds — `POINTCLOUDSECTIONVIEW` / top/side
   snapping reusing `VIEW TOP`/`VIEW FRONT` + a "Section" ribbon group.
3. **Cross-section core** — section state + shader clip + `POINTCLOUDSECTION*`
   commands + `POINTCLOUDSECTIONWIDTH/MOVE`.
4. **Section classification loop** — brush/classify in the section pane
   (mostly works already via shared indices; verify + fix).
5. **SHEETSET** — model + UI + layout switch + publish (plot reuse).
6. **Polish** — continuous brush, palette persistence round-trip.

Each step is independently shippable and testable (`cargo test` + the existing
`tests/point_cloud_shader.rs` and `crates/ocs_pointcloud` fixtures, plus the
`--serve` headless runner for command-level checks).

---

## 6. Open questions for the user (before implementation)

1. **Sheet set persistence** — is a simple JSON format (not AutoCAD `.dst`)
   acceptable for v1? (Recommended: yes.)
2. **Section density** — accept that sections run over the streamed working
   set first (dense-tile section streaming as a follow-up)?
3. **Tool palette placement** — docked side panel (like Properties) vs. a
   floating/modal window? (Recommended: docked side panel.)
4. **Priority** — is the cross-section the thing to build first (ahead of
   TOOLPALETTES/SHEETSET UI), given it's the TerraScan workflow you cited?
