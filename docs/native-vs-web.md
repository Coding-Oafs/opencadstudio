# Native vs. Web (WebAssembly)

Open CAD Studio ships as a native desktop app and as a WebAssembly build that
runs in the browser (https://hakanseven12.github.io/OpenCADStudio/). Both are
built from the same source; the web target drops or shims the pieces that a
browser can't provide. This page lists the differences.

## At a glance

| Area | Native (desktop) | Web (wasm) |
|------|------------------|------------|
| Windowing | Multi-window (`iced::daemon`) | Single window (`iced::application`); dialogs are in-canvas modals |
| 3D solid modeling | Yes | **No** — `solid3d` feature off |
| Hatch rendering | Yes | **No** — WebGL2 has no vertex-stage storage |
| Fonts | Embedded stroke fonts + system TrueType + shaping + fallback | Embedded stroke fonts only |
| File open | Native file dialog → path | Browser picker → bytes |
| File save | Path / Save dialog → write to disk | Save dialog → direct browser download |
| Parallelism | Multi-threaded (rayon) | Single-threaded |
| GPU backend | Vulkan / DX12 / Metal (wgpu) | WebGL2 (WebGPU where available) |
| Update check / external links | `ureq`, `open` | skipped / `window.open` |
| External plugins | Native packages + Plugin Manager | **No** — native libraries cannot run in a browser |

## Details

### 3D solid modeling — web: disabled
The `solid3d` Cargo feature (default on) gates `truck-meshalgo` and
`truck-shapeops`, which pull `vtkio → xz2 → lzma-sys`, a C library that can't
cross-compile to `wasm32`. The web build compiles with `--no-default-features`,
so on the web:
- The Model tab's solid primitives and boolean operations are no-ops.
- Solid tessellation and ACIS (SAT) import produce no geometry.

2-D CAD is unaffected.

### Hatch rendering — web: not drawn
The batched hatch pipeline binds a read-only storage buffer in the vertex
stage. WebGL2 lacks `VERTEX_STORAGE`, so that pipeline is skipped on wasm and
hatches simply don't render. Everything else (lines, arcs, polylines, text,
images) draws normally.

### Fonts — web: bundled stroke fonts only
- **Native** discovers installed system fonts (fontdb), extracts TrueType
  outlines (ttf-parser), shapes runs with cosmic-text (ligatures, Arabic
  joining, kerning, bidi), and falls back to a system font for glyphs a stroke
  font lacks.
- **Web** has no system fonts, and cosmic-text panics with "no default font
  found" on an empty font set, so shaping and fallback are disabled. Only the
  embedded LFF stroke fonts render; a glyph missing from them is skipped.
  (A future option is to fetch a font file from the server at startup.)

### File I/O — web: bytes and downloads
- **Open**: the desktop returns a filesystem path and reads it (streaming).
  The web reads the picked file's bytes via the browser and parses them in
  memory (`io::load_bytes`).
- **Save**: both show the in-app Save dialog (filename + format). The desktop
  writes to the chosen path; the web serializes to bytes and triggers a direct
  browser download (a Blob + a programmatic `<a download>` click — no
  intermediate "click to download" link). The unsaved-changes prompt's *Save*
  routes through the same dialog on the web.
- There is no persistent filesystem path on the web, so a name-only path stands
  in for document tracking.

### Windowing — web: single window, in-canvas modals
The desktop uses `iced::daemon` and opens secondary OS windows for every
manager and style dialog. The browser has only the canvas, so the web uses
`iced::application` and renders all dialogs (layer/layout/plot managers, the
style editors, the colour picker, Save / unsaved prompts, About, shortcuts,
plugins, …) as in-canvas modal overlays. As of the Plan-B work this path is
shared: native renders the same modals too, so the desktop is effectively
single-window now. A modal's backdrop dims and blocks clicks but does not
dismiss; the ✕ button closes it.

### Parallelism — web: single-threaded
`wasm32` has no threads without SharedArrayBuffer (which needs COOP/COEP
headers GitHub Pages can't set). `crate::par::prelude` re-exports `rayon` on
native and sequential `std` iterators on wasm, so the same call sites run in
parallel on the desktop and serially in the browser. Large drawings are slower
on the web.

### External plugins — web: unavailable

Marketplace plugins are native `.dll`, `.so`, or `.dylib` packages launched in
an isolated child process. Browsers cannot load those libraries or spawn the
plugin runner, so the web build does not show the Plugin Manager or register
its commands. Installed desktop plugins remain in the per-user plugins folder
and load again on the next desktop launch.

### Platform shims (`src/sys.rs`)
- `open_url`: `open::that` on native, `window.open(_blank)` on web.
- `download_bytes`: web only (Blob + anchor download).
- `handle_path`: real path on native, a name-only `PathBuf` on web.
- `platform_info`: OS + arch on native, the browser user-agent on web (used to
  pre-fill the Send Feedback issue).
- The self-update check (`ureq`) is a no-op on the web.

## Build & deploy

- Native: `cargo build --release --bin OpenCADStudio`.
- Web: `trunk build --release --public-url /OpenCADStudio/` (the rust `<link>`
  in `index.html` sets `data-cargo-no-default-features`, dropping `solid3d`).
  `.github/workflows/pages.yml` builds and deploys to GitHub Pages on every
  release. No COOP/COEP headers are needed because the web build is
  single-threaded.
