# v1.1 to v2 development audit

Audit closed: 2026-08-27

## Final stage assessment

| Stage | v2.0.0 status | Release evidence |
|---|---|---|
| v1.1 LiDAR workstation | Complete production core | `.ocsproj`, fixed sections, full-density tools, restart-safe jobs, COPC/E57, classifiers, surfaces, measurements, breakline validation, real LAS/LAZ and 2-million-point gates |
| v1.2 native GIS | Integrated | GeoPackage/GeoJSON, typed attributes, table/topology operations, desktop commands, project catalog, and reprojection provenance |
| v1.2 geodesy | Integrated with survey grid execution | Explicit horizontal/compound plans, units, epochs, checksum-pinned NOAA GEOID18, and an isolated bundled PROJ 9 worker |
| v1.2 Python | Integrated | Isolated CPython worker, live project/GIS/section/tool APIs, manifests, health checks, digest trust, and bundled classifier |
| v1.3 reality to model | Integrated algorithm/tool layer | Plane/sphere/line fitting, stationing, surface comparison, change detection, and LOD1 reconstruction registered as stable tools |
| v2 integrated platform | Complete final scope | Unified cross-domain transactions, registry-validated workflow DAGs, visual workflow/standards manager, signed standards, provenance, schema-2 persistence, and disk-backed octree 3D Tiles streaming/export |

## v2 plan closure

- Stable project, plugin, tool, and Python API contracts are versioned and release-tested.
- The Manage ribbon exposes a visual workflow graph/editor backed by the shared
  tool registry, plus company standards import, export, validation, SHA-256
  sealing, Ed25519 verification, and explicit signer trust.
- Project templates and validation rules persist in `.ocsproj` schema 2.
- 3D Tiles export scans full source density, transforms to WGS84 Earth-centered
  coordinates, spools to disk, creates bounded octree PNTS tiles, and exposes a
  traversal-safe lazy asset stream.
- Desktop packages include isolated, pinned processing runtimes. The PROJ grid
  is SHA-256 checked before every use; Rust dependencies are locked.
- Full provenance and deliverable validation remain local-first. Enterprise
  collaboration is intentionally optional, as specified by the roadmap, and is
  not required to use any desktop capability.

## Verification snapshot

- Desktop application check: passed.
- Platform octree/workflow/standards suite: passed.
- Bundled PROJ worker self-test and checksum-pinned GEOID18 transform: passed.
- Real-data LiDAR smokes: passed against PDAL Autzen LAS and LAZ fixtures,
  including 220,000-record merge/reimport and native urban classification.
- The release workflow now downloads the same public fixtures, runs the
  ignored real-data suite, and runs the two-million-point release stress test
  before packaging.

## Upstream networking note

The narrow multi-address fallback behavior is tracked upstream in ureq issue
1184 and pull request 1195. OpenCADStudio retains its IPv4-first resolver
workaround and does not vendor an unreleased HTTP dependency patch. The updater
also compares semantic versions and cannot offer a downgrade.
