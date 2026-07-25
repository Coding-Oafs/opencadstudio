# OpenCADStudio patch

This directory vendors `iced_wgpu` 0.14.0 under its MIT license.

OpenCADStudio changes only the WASM device-limit selection in
`src/window/compositor.rs`: Browser WebGPU first requests normal WebGPU limits
so storage-buffer pipelines are available, then falls back to WebGL2 limits.
An actual WebGL adapter continues to request WebGL2 limits directly.
