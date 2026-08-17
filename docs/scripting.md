# Scripting and macros

Open CAD Studio runs Rhai macros that drive the same audited operations as
the ribbon and the command line. Scripts live on a worker thread and call
into the application through a request bridge, so the UI stays responsive
while the script reads live state.

## Running a script

| Command | Purpose |
| --- | --- |
| `SCRIPT` | Pick a `.rhai` file and run it. |
| `SCRIPT <path>` | Run the script at `path`. |

Output lines appear in the command line prefixed with `[script]`. Only one
script runs at a time; long operations (attach, export) start their usual
background jobs and return immediately — scripts observe progress by polling
(`cloud_sources`, `cloud_export_status`).

## The `ocs` API

| Call | Returns |
| --- | --- |
| `ocs.cloud_attach(path)` | source id (queued) |
| `ocs.cloud_attach_folder(path)` | queued count |
| `ocs.cloud_sources()` | `[{id, path, points, displayed, edits}]` |
| `ocs.cloud_stats()` | map of ASPRS class → point count |
| `ocs.cloud_filter(json)` | sets the attribute filter used by selections |
| `ocs.cloud_select_slice(low, high)` | selects an elevation band |
| `ocs.cloud_select_clear()` | clears the selection |
| `ocs.cloud_classify_selection(class)` | reclassifies the selection |
| `ocs.cloud_classify(source, class, "10,25-40")` | classifies explicit indices |
| `ocs.cloud_undo()` | undoes the last edit action |
| `ocs.cloud_export_all(path)` | merged export of every source (queued) |
| `ocs.cloud_export_status()` | `{running, completed, total}` |
| `ocs.cloud_detach()` | detaches every source (session only) |
| `ocs.cloud_list_folder(path)` | LAS/LAZ file paths directly under a folder |
| `ocs.command("LAYER Walls")` | runs any command-line command |
| `ocs.log(message)` | prints to the script console |

Selections and edits apply across every attached source, exactly like the
interactive tools, and undo steps the last action as one transaction.

## Built-in library

`scripts/library/` ships ready-to-edit production macros:

- `folder_workflow.rhai` — attach a folder, report class statistics,
  classify an elevation band as ground, export one merged delivery file, and
  wait for the export to complete.
- `class_report.rhai` — per-source inventory with point counts, pending
  edits, and class totals.
- `batch_classify.rhai` — batch production over a folder: for each LAS/LAZ,
  attach, run `POINTCLOUDNOISE` and `POINTCLOUDGROUND`, export a
  `<name>_classified.laz` beside the source, detach, and continue.

Copy them, edit the constants at the top, and run with `SCRIPT <path>`.

## Engine notes

The engine-agnostic host lives in the `ocs_scripting` crate: the
`OcsScriptApi` trait is the application-side contract and `ScriptRequest`
is the wire format, so additional engines (Python via PyO3 is planned behind
a `python` feature) present the same API without new app plumbing. The Rhai
engine is sandboxed with operation and call-depth limits; a runaway macro
errors out instead of hanging the app.
