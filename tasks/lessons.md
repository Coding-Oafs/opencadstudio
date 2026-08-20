# Lessons Learned

Durable prevention rules — not "be more careful", but "always check X before Y".

## Ribbon tool `id` is user-visible: keep it in sync with the command it dispatches

**What happened:** The LiDAR ribbon module (`src/modules/lidar/mod.rs`) set each tool's
`id` to a `LIDAR_*` value (e.g. `LIDAR_ATTACH`) while the `event` carried the real command
(`POINTCLOUDATTACH`). The ribbon tooltip renders `t.id` as the "Command:" line, so users saw
"Command: LIDAR_ATTACH" — a command that does not exist. Typing it yields "Unknown command",
so the LiDAR attach commands were reported as broken even though the dispatch was correct.

**Why:** `ToolDef.id` is documented as "Unique command id, e.g. 'LINE'", and every other
module sets `id == command`. `id` is surfaced to users (the tooltip) and used for tool
activation / last-used state, so decoupling it from the command breaks continuity and
discoverability.

**How to apply:**
- When adding a ribbon tool, set `id` equal to the command string its `event` carries.
  Never invent a separate `id` namespace for one module's tools.
- Prefer a helper that derives `id` from `command`, so the two cannot drift.
- Root-cause fix landed in `src/ui/ribbon/widgets.rs`: the tooltip's "Command:" line now
  derives from `event` via `tool_command()`, so a mismatched `id` can no longer mislead users.

## Command strings are the stable user-facing contract

**What happened:** Users remembered `PointCloudAttach` / `PointCloudAttachFolder`, but the
canonical form is uppercase `POINTCLOUDATTACH` / `POINTCLOUDATTACHFOLDER`. The camelCase
form exists only as a Rust `Message` enum variant (`Message::PointCloudAttach`), never as a
dispatchable command string.

**How to apply:**
- A command name must stay consistent across exactly three places: the
  `ModuleEvent::Command(...)` a tool emits, the dispatch arm in `src/app/commands/display.rs`
  (or a registered `CadCommand`), and the autocomplete registry in `src/app/commands/mod.rs`.
  When adding or renaming a command, update all three plus the ribbon tool `id`.
- The command line is case-insensitive (verbs are uppercased), so "pointcloudattach" and
  "POINTCLOUDATTACH" resolve the same — but the tooltip and any docs must show the canonical
  uppercase form.

## A transform helper must be wired into the runtime path, not just unit-tested

**What happened:** `reproject_bounds_3857` in `src/scene/basemap.rs` was written and unit-tested to
reproject Web-Mercator tile bounds back into the source CRS, but `refresh_basemap` never called it.
Tiles were placed at raw Web-Mercator meters while UTM / projected content sat at its own
coordinates, so the underlay landed millions of metres off — with no error, because every step
"succeeded". A unit-tested-but-unwired helper is the worst kind of bug: the test proves the helper
correct, yet the feature is still broken.

**How to apply:**
- When you add a helper for a transform (reprojection, coordinate mapping, unit conversion), trace
  the runtime path and confirm it actually calls the helper — a green unit test is not proof the
  feature works end to end.
- For CRS-aware features (basemap, georeferenced overlays), place geometry in the same CRS the
  scene uses; keep the "reproject back" step adjacent to the "reproject forward" step so the pair
  is visibly symmetric.

## A projected WKT with no PROJCS EPSG authority must not fall back to the geographic EPSG

**What happened:** The Boston USGS LAS declared `NAD83(2011) / Massachusetts Mainland (ft)` — a
Lambert Conformal Conic state-plane CRS in International feet — but its WKT carried EPSG authority
ids only on the base `GEOGCS` (6318), datum (1116) and spheroid (7019), not on the `PROJCS`
itself. `epsg_from_wkt` returned the last authority (the geographic 6318), so `reproject_xy` fed
foot-based state-plane coordinates into a degree-based projection, `transform` failed, and the
basemap reported "cannot reproject the drawing bounds into Web Mercator".

**Why:** "No EPSG on the projected CRS" is common in the wild, but silently falling back to the
*geographic* base CRS is never the right guess — it changes the unit (feet→degrees) and fails
mysteriously downstream.

**How to apply:**
- When parsing CRS WKT, if a `PROJECTION[...]` element is present but no projected-CRS EPSG
  authority resolves, build a PROJ.4 string from the WKT (`+proj=lcc/tmerc/... + params + units`)
  and reproject with that — never fall back to the geographic EPSG. (See `proj4_from_wkt` in
  `crates/ocs_pointcloud/src/crs.rs`.)
- Prefer the WKT-derived PROJ.4 string over a geographic `horizontal_epsg` fallback whenever both
  are present (`reproject_from_crs` / `reproject_to_crs`).

## Remote map tiles need bounded concurrency, durable caching, and stale-job rejection together

**What happened:** The v0.9.7 basemap loop fetched hundreds of tiles serially, downloaded them
again on every refresh, and could install the result of an obsolete request after the user had
changed provider or projection. A correct tile URL was therefore still practically unusable.

**How to apply:**
- Treat a tiled underlay as a cancellable generation-keyed job, not a loop of blocking requests.
- Bound parallelism independently of tile count, publish completed/failed counts, and ignore any
  result whose job id or document tab is no longer active.
- Decode cache files by signature, write downloads through an adjacent temporary file, and rename
  only after a complete response so a failed request never poisons future warm loads.

## Drawing CRS belongs to the drawing, not to whichever spatial attachment happens to be loaded

**What happened:** v0.9.7 could infer a coordinate system only from LAS/LAZ. An empty or ordinary
CAD drawing could not establish its own CRS, working unit, or basemap envelope, so unrelated
features were accidentally coupled to point-cloud attachment state.

**How to apply:**
- Persist drawing spatial metadata in its own singleton sidecar record and migrate the schema even
  when the attachment table is empty.
- Derive the coordinate unit from the CRS database and normalize saved settings on load; do not
  trust a stale sidecar to keep an incompatible unit/CRS pair.
- Keep INSUNITS for DWG insertion scaling. Drawing working units are the user-facing survey/query
  contract and must not silently rescale geometry.

## A CRS is not a drawing extent

**What happened:** The first v1 release candidate allowed a drawing-owned CRS
but still rejected an empty drawing with “no bounds to place the underlay.” A
CRS defines how coordinates are interpreted; it does not say which project site
the user wants to see.

**How to apply:**
- Treat an empty spatial document as a supported bootstrap state, not an error.
- Use a low-cost world overview before CRS selection and the EPSG definition's
  area of use afterward, with a strict tile ceiling.
- Provide a location-first control that accepts familiar longitude/latitude and
  transforms a small site envelope into drawing coordinates.
- Clear manual bounds when changing between CRSs so stale coordinates cannot be
  silently interpreted in a different reference system.

## Enforce cardinality limits before allocation

**What happened:** v1.0.0 computed every XYZ tile into a `Vec` and only then
checked the 16,384-tile limit. The new empty-drawing world overview at zoom 16
therefore attempted more than four billion entries, froze the UI, and ended in
Rust's native out-of-memory abort (`0xc0000409`).

**How to apply:**
- Compute range cardinality with checked fixed-width arithmetic before creating
  a collection or starting network work.
- Make the bounded materializer enforce its own limit so callers cannot bypass
  the memory boundary accidentally.
- Test pathological-but-valid inputs (full world at maximum zoom), not only
  small happy-path tile envelopes.
- Prefer automatically reducing overview detail to rejecting or attempting an
  impractical request; preserve the user's configured detail level for smaller
  extents.
