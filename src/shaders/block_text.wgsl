struct Uniforms {
    viewport_size: vec2<f32>,
    world_per_pixel: f32,
    lwdisplay_enable: f32,
    flat_shade: f32,
    transparency_enable: f32,
    _pad: vec2<f32>,
    view_rot: mat4x4<f32>,
    eye_high: vec3<f32>,
    _pad_eh: f32,
    eye_low: vec3<f32>,
    _pad_el: f32,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_samp: sampler;

struct VertIn {
    @location(0) pos: vec3<f32>,
    @location(1) pos_low: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) source_depth: f32,
    @location(5) translation: vec3<f32>,
    @location(6) translation_low: vec3<f32>,
    @location(7) instance_depth: f32,
}

struct VertOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

const DRAW_ORDER_BIAS: f32 = 0.001;

@vertex
fn vs_main(v: VertIn) -> VertOut {
    var out: VertOut;
    let rel = (v.pos + v.translation - u.eye_high)
        + (v.pos_low + v.translation_low - u.eye_low);
    out.clip_pos = u.view_rot * vec4<f32>(rel, 1.0);
    out.clip_pos.z = out.clip_pos.z
        - (v.source_depth + v.instance_depth) * DRAW_ORDER_BIAS * out.clip_pos.w;
    out.uv = v.uv;
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    let sd = textureSample(atlas_tex, atlas_samp, in.uv).r;
    let aa = max(fwidth(sd), 1e-4);
    let alpha = smoothstep(0.5 - aa, 0.5 + aa, sd);
    if alpha <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
