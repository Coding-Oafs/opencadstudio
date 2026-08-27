# v1.1 to v2 development audit

Audit date: 2026-08-27

## Starting point

The repository was beyond the v1.1 benchmark, but not yet at the v1.2 product
benchmark. The v1.1 LiDAR workstation core was integrated and its focused suite
passed. GIS, geodesy, and CPython had promising standalone crates, but GIS and
geodesy were not used by the desktop application and Python exposed only the
older point-cloud command surface.

## Stage assessment after this development pass

| Stage | Status | Evidence | Remaining release work |
|---|---|---|---|
| v1.1 LiDAR workstation | Implemented core | `.ocsproj`, fixed sections, full-density processing, durable jobs, COPC/E57, classifiers, surfaces, measurements, breakline validation | Run opt-in real-data and scale suites on release fixtures; complete installer smoke |
| v1.2 native GIS | Integrated foundation | GeoPackage/GeoJSON, typed attributes, table operations, topology, desktop commands, project catalog, reprojection provenance | Attribute-table/map rendering UI, labeling/symbology, CSV/COG, OGC services, COGO, CAD conversion |
| v1.2 geodesy | Safe foundation | Explicit horizontal/compound plans, units, epoch, vertical policy, fail-closed grid operations | Bundle and validate a PROJ grid backend for survey-grade horizontal/vertical transformations |
| v1.2 Python | Integrated foundation | Worker process, live project/GIS/section/tool APIs, script manifests, health and digest trust | Project virtual-environment creation/package UI, Arrow/shared-memory bulk transfer, editor/console UI, cancellation handles |
| v1.3 reality to model | Algorithm core | Plane/sphere/line fitting, stationing, surface comparison, change detection, LOD1 reconstruction | Interactive extraction/refinement, strip alignment, roof segmentation/LOD2, mesh/solid and corridor production tools |
| v2 integrated platform | Alpha core | Atomic cross-domain transactions, workflow DAGs, signed standards, provenance, schema-2 persistence, 3D Tiles PNTS export | Visual workflow/standards UI, adapters applying transactions to every live model, tiled octree streaming, signed plugin enforcement, reproducible environment locks, collaboration |

## Verification snapshot

- Focused core suites: 94 tests passed.
- Desktop `cargo check`: passed.
- Python worker end-to-end test: passed against a real CPython interpreter.
- Real-data LiDAR smoke tests: present but skipped unless
  `OCS_LIDAR_SMOKE_DIR` points at release data.
- Two-million-point scale test: present but intentionally opt-in and should run
  under `--release` before packaging.

## Release recommendation

Treat the current tree as `2.0.0-alpha.1`, not a final v2 release. It now has a
coherent path through every roadmap stage and durable v2 contracts, while the
table above keeps the remaining UI, format, backend, scale, and installer gates
explicit.
