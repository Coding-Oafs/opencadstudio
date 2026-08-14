// LiDAR points as camera-facing fixed-pixel quads with circular fragments.

struct Uniforms {
    viewport_size: vec2<f32>,
    world_per_pixel: f32,
    lwdisplay_enable: f32,
    flat_shade: f32,
    transparency_enable: f32,
    linetype_scale: f32,
    _pad: f32,
    view_rot: mat4x4<f32>,
    eye_high: vec3<f32>,
    _pad_eh: f32,
    eye_low: vec3<f32>,
    _pad_el: f32,
}
@group(0) @binding(0) var<uniform> u: Uniforms;

struct InstanceIn {
    @location(0) position_high_size: vec4<f32>,
    @location(1) position_low: vec4<f32>,
    @location(2) color: vec4<f32>,
}
struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex fn vs_main(@builtin(vertex_index) vertex: u32, point: InstanceIn) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let local = corners[vertex];
    let relative = (point.position_high_size.xyz - u.eye_high)
        + (point.position_low.xyz - u.eye_low);
    let center = u.view_rot * vec4<f32>(relative, 1.0);
    let half_size_px = point.position_high_size.w * 0.5;
    let ndc_offset = local * half_size_px / (u.viewport_size * 0.5);
    var output: VertexOut;
    output.clip_position = center + vec4<f32>(ndc_offset * center.w, 0.0, 0.0);
    output.local = local;
    output.color = point.color;
    return output;
}

@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let radius_sq = dot(input.local, input.local);
    if radius_sq > 1.0 {
        discard;
    }
    let edge = 1.0 - smoothstep(0.78, 1.0, radius_sq);
    let alpha = input.color.a * edge;
    return vec4<f32>(input.color.rgb * alpha, alpha);
}
