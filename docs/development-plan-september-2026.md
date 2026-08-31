# OpenCADStudio development plan — September 2026

Plan window: 1–25 September 2026

This plan assumes one primary maintainer with roughly three focused engineering
days per week. Each week reserves one day for testing, triage, documentation,
and release operations. Scope is intentionally limited to proving v2 in real
production workflows before starting another broad feature cycle.

## Outcomes for the month

1. Prove v2.0.1 on large, multi-source LiDAR projects and supported desktop
   packages.
2. Turn the new LOD behavior into measured performance contracts enforced by
   CI where practical.
3. Remove the highest-signal renderer and point-cloud maintenance debt without
   mixing in a repository-wide rewrite.
4. Define and start the smallest valuable v2.1 workflow increment from user
   evidence rather than the historical backlog alone.

## Week 1 — release soak and production evidence (1–4 September)

Deliverables:

- Watch the v2.0.1 release workflow through Windows, Linux, macOS, and Snap
  packaging; record any signing, notarization, or store-publish exceptions.
- Install the Windows MSI on a clean user profile and verify launch, update
  detection, DWG/DXF association, thumbnail provider, bundled classifier, and
  bundled PROJ self-test.
- Run a scripted GUI smoke on at least one multi-GB project: attach multiple
  LAS/LAZ sources, build v3 caches, orbit/zoom in multiple view frames, change
  section width/mode, recolor classes, reopen the project, and export a subset.
- Capture peak process RAM, estimated GPU point bytes, cache-build duration,
  time to first stable frontier, and visible regressions at three camera
  distances.
- Triage release reports daily. Ship v2.0.2 only for data loss, crashes,
  packaging failure, or a repeatable rendering blocker; otherwise queue fixes.

Exit gate: one clean packaged-install smoke and one documented large-project
soak with no unbounded memory growth or persistent viewport holes.

## Week 2 — performance contracts and CI hardening (7–11 September)

Deliverables:

- Add deterministic tests for the multi-source publication barrier, point-quota
  fairness, section-compaction limits, stale camera generations, and cache-v3
  upgrade/reuse paths that are not already covered.
- Add lightweight counters or debug logging for frontier generation, selected
  mixed-level nodes, resident points/bytes, uploaded ranges, and deferred stale
  batches. Keep telemetry local and opt-in.
- Add a synthetic multi-viewport/multi-source stress target with explicit
  ceilings: no more than the configured GPU byte budget, no duplicate
  ancestor/descendant nodes, and no stale frontier publication.
- Make the release workflow report those measurements in its job summary.
- Resolve the current high-signal Rust warnings in touched point-cloud paths:
  ineffective reference drops, visibility mismatches, and now-unused point-GPU
  helpers. Do not combine this with the 300+ file rustfmt backlog.

Exit gate: repeatable measurements exist locally and in CI, and a budget or
frontier regression fails a test rather than relying on visual inspection.

## Week 3 — workflow usability and failure recovery (14–18 September)

Deliverables:

- Improve the Point Cloud Manager's state reporting for cache format, indexing
  progress, active frontier generation, memory budget, and the reason a source
  is waiting or degraded.
- Add a deliberate “rebuild v3 cache” action with confirmation and clear disk
  space/time estimates; never delete older caches implicitly.
- Make interrupted indexing/reopen scenarios explicit and testable, including
  partial cache directories, source fingerprint changes, corrupt tile records,
  and one failed source in a multi-source batch.
- Run an accessibility and keyboard pass over the LiDAR manager and section
  controls, then update `docs/lidar-point-clouds.md` from the observed workflow.
- Interview or collect structured feedback from 2–3 real workflows if users are
  available; otherwise use issue/release telemetry and the large-project soak.

Exit gate: users can tell whether a cloud is sampled, indexing, tiled, stale,
or failed, and can recover without manually deleting cache directories.

## Week 4 — choose and start v2.1 (21–25 September)

Spend no more than two days on discovery and sizing. Rank candidate increments
by user frequency, correctness risk, and ability to ship independently. The
default candidates are:

1. Continuous brush/lasso selection and a named selection-set organizer.
2. Batch workflow queue UX for existing processing tools.
3. Surface/contour production polish and export validation.

Choose one vertical slice with a two-week implementation ceiling. Before code,
write its acceptance examples, performance budget, persistence impact, and
rollback strategy. Use the remaining time for the first end-to-end slice behind
an opt-in flag; do not start all three candidates.

Exit gate: a reviewed v2.1 mini-spec, named owner, acceptance tests, and one
working vertical slice or prototype with a go/no-go decision.

## Operating rhythm and guardrails

- Keep `main` releasable; use short-lived `codex/` or issue-named branches and
  delete them after merge.
- Require formatting checks on changed files, focused unit tests, and desktop
  `cargo check` for every point-cloud/renderer change.
- Run the real LAS/LAZ fixtures and two-million-point stress gate before each
  release candidate; run the large local dataset before stable tags.
- Reserve 20% of capacity for release support and regressions. If that reserve
  is consumed, cut v2.1 scope instead of compressing verification.
- Avoid schema or cache-format changes during the soak unless a correctness bug
  cannot be fixed compatibly.

## End-of-plan decision

On 25 September, review crash/blocker count, large-project memory measurements,
release downloads/issues, and v2.1 prototype evidence. The next milestone is
either v2.0.x hardening (if stability thresholds are missed) or the single
selected v2.1 vertical slice. No second v2.1 theme starts until the first is
shippable.
