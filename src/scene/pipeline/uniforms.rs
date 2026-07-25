use crate::scene::view::camera::Camera;
use iced::Rectangle;

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Uniforms {
    pub viewport_size: [f32; 2],
    /// World units per screen pixel at the current zoom. Used by the
    /// hatch shader to substitute solid fill when pattern line spacing
    /// falls below ~2 px (Phase 3.3 LOD).
    pub world_per_pixel: f32,
    /// LWDISPLAY toggle (1.0 = show lineweights, 0.0 = force 1 px).
    /// Read by the wire shader so the toggle does not require a retessellate.
    pub lwdisplay_enable: f32,
    /// 1.0 → mesh fragment shader replaces the interpolated vertex
    /// normal with `normalize(cross(dpdx(pos), dpdy(pos)))` so each
    /// triangle gets a uniform shade (FlatShaded mode); 0.0 → keeps the
    /// per-vertex normal interpolation (GouraudShaded-style).
    pub flat_shade: f32,
    /// Transparency-display toggle (1.0 = honour entity transparency,
    /// 0.0 = force opaque). Read by the wire shader so the toggle does not
    /// require a retessellate.
    pub transparency_enable: f32,
    /// Per-viewport PSLTSCALE factor. Kept in the shared frame uniform so
    /// zooming inside MSPACE changes one scalar instead of re-tessellating and
    /// re-uploading every dashed wire.
    pub linetype_scale: f32,
    /// Pads the struct to the uniform alignment required by wgpu.
    pub _pad: f32,

    // ── Relative-to-eye (double-single) additions ───────────────────────────
    // Appended at the end so existing field offsets are unchanged; shaders that
    // still read only the legacy fields keep working. Pipelines migrate to RTE
    // one at a time. `view_rot` is the rotation-only view-projection; vertices
    // pre-subtract the eye (via `eye_high`/`eye_low`, two f32 emulating f64) so
    // the large eye translation never enters the f32 matrix → no large-coord
    // jitter.
    pub view_rot: glam::Mat4,
    pub eye_high: [f32; 3],
    pub _pad_eh: f32,
    pub eye_low: [f32; 3],
    pub _pad_el: f32,
}

impl Uniforms {
    pub fn new(camera: &Camera, bounds: Rectangle, lwdisplay_enable: bool) -> Self {
        let half_h = camera.ortho_size();
        let world_per_pixel = if bounds.height > 0.0 {
            (2.0 * half_h) / bounds.height
        } else {
            0.0
        };
        let (eye_high, eye_low) = camera.eye_high_low();
        Self {
            viewport_size: [bounds.width, bounds.height],
            world_per_pixel,
            lwdisplay_enable: if lwdisplay_enable { 1.0 } else { 0.0 },
            flat_shade: 0.0,
            transparency_enable: 1.0,
            linetype_scale: 1.0,
            _pad: 0.0,
            view_rot: camera.view_proj_rte(bounds),
            eye_high,
            _pad_eh: 0.0,
            eye_low,
            _pad_el: 0.0,
        }
    }
}
