// LiDAR points as camera-facing fixed-pixel quads with circular fragments.
// Colorization happens on the GPU from the per-frame style uniform, so color
// mode, class visibility and class-table edits never rebuild the instances.

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

// Mirrors `PointStyle` on the CPU. Keep offsets in sync with StyleUniforms.
struct Style {
    color_mode: u32,
    point_size: f32,
    _pad0: vec2<u32>,
    intensity_range: vec2<f32>,
    _pad1: vec2<f32>,
    elevation_range: vec2<f32>,
    _pad2: vec2<f32>,
    class_visible: array<vec4<u32>, 8>,
    class_colors: array<vec4<f32>, 256>,
}
@group(1) @binding(0) var<uniform> style: Style;

struct InstanceIn {
    @location(0) position_high_size: vec4<f32>,
    @location(1) position_low: vec4<f32>,
    @location(2) attributes: vec4<f32>,
    @location(3) color_selected: vec4<f32>,
}
struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
}

fn class_is_visible(classification: u32) -> bool {
    let shift = classification % 32u;
    let word = style.class_visible[classification / 32u][shift];
    return ((word >> shift) & 1u) == 1u;
}

fn class_color(classification: u32) -> vec4<f32> {
    return style.class_colors[classification];
}

fn normalize_range(value: f32, range: vec2<f32>) -> f32 {
    if (!(range.y > range.x)) {
        return 0.5;
    }
    return clamp((value - range.x) / (range.y - range.x), 0.0, 1.0);
}

fn elevation_gradient(value: f32) -> vec4<f32> {
    let red = clamp(value * 1.5, 0.0, 1.0);
    let blue = clamp((1.0 - value) * 1.5, 0.0, 1.0);
    let green = clamp(1.0 - abs(value - 0.5) * 2.0, 0.0, 1.0);
    return vec4<f32>(red, green, blue, 1.0);
}

fn categorical_hash(value: u32) -> vec4<f32> {
    let hash = (value * 0x9e3779b9u << 13u) | (value * 0x9e3779b9u >> 19u);
    return vec4<f32>(
        0.25 + f32(hash & 0xffu) / 510.0,
        0.25 + f32((hash >> 8u) & 0xffu) / 510.0,
        0.25 + f32((hash >> 16u) & 0xffu) / 510.0,
        1.0,
    );
}

fn point_color(classification: u32, intensity: f32, return_number: u32,
               source_id: u32, rgb: vec3<f32>, elevation: f32) -> vec4<f32> {
    if (style.color_mode == 0u) {
        return class_color(classification);
    } else if (style.color_mode == 1u) {
        // A zero RGB triple marks points whose source has no color band.
        if (rgb.r > 0.0 || rgb.g > 0.0 || rgb.b > 0.0) {
            return vec4<f32>(rgb, 1.0);
        }
        return class_color(classification);
    } else if (style.color_mode == 2u) {
        let value = normalize_range(intensity, style.intensity_range);
        return vec4<f32>(value, value, value, 1.0);
    } else if (style.color_mode == 3u) {
        return elevation_gradient(normalize_range(elevation, style.elevation_range));
    } else if (style.color_mode == 4u) {
        return categorical_hash(return_number);
    }
    return categorical_hash(source_id);
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
    let classification = u32(point.position_low.w);
    // Hidden classes collapse to a zero-size quad: no fragments, no cost.
    let visible = class_is_visible(classification);
    var half_size_px = select(0.0, style.point_size * 0.5, visible);
    let ndc_offset = local * half_size_px / (u.viewport_size * 0.5);
    var output: VertexOut;
    output.clip_position = center + vec4<f32>(ndc_offset * center.w, 0.0, 0.0);
    output.local = local;
    var color = point_color(
        classification,
        point.attributes.x,
        u32(point.attributes.y),
        u32(point.attributes.z),
        point.color_selected.xyz,
        point.position_high_size.z,
    );
    if (point.color_selected.w > 0.5) {
        color = vec4<f32>(1.0, 0.82, 0.05, 1.0);
    }
    output.color = color;
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
