// Mesh shader — renders triangle meshes (truck Shell/Solid tessellation).
//
// Vertex layout: position [f32;3], normal [f32;3], color [f32;4]  (40 bytes)
//
// Lighting: simple half-Lambert with a fixed directional light. Two
// shading paths share this shader, picked per-frame via `u.flat_shade`:
//   - 0.0 → per-vertex normals interpolated to the fragment (Gouraud).
//   - 1.0 → per-triangle face normal from screen-space derivatives
//     `cross(dpdx(pos), dpdy(pos))`, so each triangle reads as a single
//     flat shade (FlatShaded).

struct Uniforms {
    viewport_size:       vec2<f32>,
    world_per_pixel:     f32,
    lwdisplay_enable:    f32,
    flat_shade:          f32,
    transparency_enable: f32,
    _pad:                vec2<f32>,
    // Relative-to-eye (double-single): see wire.wgsl.
    view_rot:            mat4x4<f32>,
    eye_high:            vec3<f32>,
    _pad_eh:             f32,
    eye_low:             vec3<f32>,
    _pad_el:             f32,
    light_position_type: array<vec4<f32>, 4>,
    light_direction_intensity: array<vec4<f32>, 4>,
    light_color_hotspot: array<vec4<f32>, 4>,
    light_attenuation: array<vec4<f32>, 4>,
    lighting: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct MaterialMaps {
    blends0:  vec4<f32>,
    present0: vec4<u32>,
    blends1:  vec4<f32>,
    present1: vec4<u32>,
};

@group(1) @binding(0) var diffuse_map: texture_2d<f32>;
@group(1) @binding(1) var specular_map: texture_2d<f32>;
@group(1) @binding(2) var reflection_map: texture_2d<f32>;
@group(1) @binding(3) var opacity_map: texture_2d<f32>;
@group(1) @binding(4) var bump_map: texture_2d<f32>;
@group(1) @binding(5) var refraction_map: texture_2d<f32>;
@group(1) @binding(6) var normal_map: texture_2d<f32>;
@group(1) @binding(7) var diffuse_sampler: sampler;
@group(1) @binding(8) var specular_sampler: sampler;
@group(1) @binding(9) var reflection_sampler: sampler;
@group(1) @binding(10) var opacity_sampler: sampler;
@group(1) @binding(11) var bump_sampler: sampler;
@group(1) @binding(12) var refraction_sampler: sampler;
@group(1) @binding(13) var normal_sampler: sampler;
@group(1) @binding(14) var<uniform> maps: MaterialMaps;

struct MeshInstance {
    model_row_0:     vec4<f32>,
    model_row_1:     vec4<f32>,
    model_row_2:     vec4<f32>,
    translation_low: vec4<f32>,
    normal_row_0:    vec4<f32>,
    normal_row_1:    vec4<f32>,
    normal_row_2:    vec4<f32>,
};

@group(1) @binding(15)
var<storage, read> mesh_instances: array<MeshInstance>;

struct VertexIn {
    @location(0) position:     vec3<f32>,
    @location(1) normal:       vec3<f32>,
    @location(2) color:        vec4<f32>,
    @location(3) position_low: vec3<f32>,
    // gloss, reflectivity, self illumination, luminance
    @location(4) material:     vec4<f32>,
    // specular RGB, refraction index
    @location(5) specular:     vec4<f32>,
    @location(6) uv_diffuse:   vec2<f32>,
    // ambient RGB, translucence
    @location(7) ambient:      vec4<f32>,
    // normal strength, bump scale, reflectance scale, transmittance scale
    @location(8) advanced:     vec4<f32>,
    // illumination model, channel flags, material mode, luminance mode
    @location(9) flags:        vec4<u32>,
    @location(10) uv_specular:   vec2<f32>,
    @location(11) uv_reflection: vec2<f32>,
    @location(12) uv_opacity:    vec2<f32>,
    @location(13) uv_bump:       vec2<f32>,
    @location(14) uv_refraction: vec2<f32>,
    @location(15) uv_normal:     vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos:  vec4<f32>,
    @location(0)       color:     vec4<f32>,
    @location(1)       normal:    vec3<f32>,
    @location(2)       world_pos: vec3<f32>,
    @location(3)       material:  vec4<f32>,
    @location(4)       specular:  vec4<f32>,
    @location(5)       eye_vec:   vec3<f32>,
    @location(6)       uv_diffuse: vec2<f32>,
    @location(7)       ambient:   vec4<f32>,
    @location(8)       advanced:  vec4<f32>,
    @location(9) @interpolate(flat) flags: vec4<u32>,
    @location(10)      uv_specular:   vec2<f32>,
    @location(11)      uv_reflection: vec2<f32>,
    @location(12)      uv_opacity:    vec2<f32>,
    @location(13)      uv_bump:       vec2<f32>,
    @location(14)      uv_refraction: vec2<f32>,
    @location(15)      uv_normal:     vec2<f32>,
};

@vertex
fn vs_main(
    v: VertexIn,
    @builtin(instance_index) instance_index: u32,
) -> VertexOut {
    var out: VertexOut;
    let instance = mesh_instances[instance_index];
    let world_high = vec3<f32>(
        dot(instance.model_row_0.xyz, v.position) + instance.model_row_0.w,
        dot(instance.model_row_1.xyz, v.position) + instance.model_row_1.w,
        dot(instance.model_row_2.xyz, v.position) + instance.model_row_2.w,
    );
    let world_low = vec3<f32>(
        dot(instance.model_row_0.xyz, v.position_low),
        dot(instance.model_row_1.xyz, v.position_low),
        dot(instance.model_row_2.xyz, v.position_low),
    ) + instance.translation_low.xyz;
    let rel = (world_high - u.eye_high) + (world_low - u.eye_low);
    out.clip_pos  = u.view_rot * vec4<f32>(rel, 1.0);
    out.color     = v.color;
    out.normal    = normalize(vec3<f32>(
        dot(instance.normal_row_0.xyz, v.normal),
        dot(instance.normal_row_1.xyz, v.normal),
        dot(instance.normal_row_2.xyz, v.normal),
    ));
    out.world_pos = rel;
    out.material  = v.material;
    out.specular  = v.specular;
    out.eye_vec   = -rel;
    out.uv_diffuse = v.uv_diffuse;
    out.ambient   = v.ambient;
    out.advanced  = v.advanced;
    out.flags     = v.flags;
    out.uv_specular = v.uv_specular;
    out.uv_reflection = v.uv_reflection;
    out.uv_opacity = v.uv_opacity;
    out.uv_bump = v.uv_bump;
    out.uv_refraction = v.uv_refraction;
    out.uv_normal = v.uv_normal;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    var n: vec3<f32>;
    if (u.flat_shade > 0.5) {
        // Per-triangle face normal: derivatives of the interpolated
        // world position are constant across the primitive, so every
        // fragment in the same triangle sees the same normal.
        n = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    } else {
        n = normalize(in.normal);
    }

    // Build a derivative tangent frame from the generated material UVs. This
    // works for planar/box/cylindrical/spherical AcDbMaterial projections and
    // does not require ACIS to carry explicit texture-coordinate vertices.
    let dp1 = dpdx(in.world_pos);
    let dp2 = dpdy(in.world_pos);
    let tangent_uv = select(in.uv_bump, in.uv_normal, maps.present1.z != 0u);
    let duv1 = dpdx(tangent_uv);
    let duv2 = dpdy(tangent_uv);
    let det = duv1.x * duv2.y - duv1.y * duv2.x;
    var tangent = normalize(dp1);
    var bitangent = normalize(cross(n, tangent));
    if (abs(det) > 1e-8) {
        tangent = normalize((dp1 * duv2.y - dp2 * duv1.y) / det);
        bitangent = normalize((-dp1 * duv2.x + dp2 * duv1.x) / det);
    }
    if (maps.present1.z != 0u) {
        let sampled = textureSample(normal_map, normal_sampler, in.uv_normal).xyz * 2.0 - 1.0;
        let mapped = normalize(
            tangent * sampled.x + bitangent * sampled.y + n * sampled.z
        );
        let strength = maps.blends1.z * max(in.advanced.x, 0.0);
        n = normalize(mix(n, mapped, clamp(strength, 0.0, 1.0)));
    } else if (maps.present1.x != 0u) {
        let height = dot(
            textureSample(bump_map, bump_sampler, in.uv_bump).rgb,
            vec3<f32>(0.2126, 0.7152, 0.0722),
        );
        let slope = tangent * dpdx(height) + bitangent * dpdy(height);
        n = normalize(n - slope * maps.blends1.x * max(in.advanced.y, 0.0) * 4.0);
    }

    // Native AcDbLight / Sun lighting. When a drawing has no active lights,
    // retain the neutral three-point editor rig so ordinary models remain
    // readable.
    let l0 = normalize(vec3<f32>( 0.5,  0.8,  0.6)); // key (upper front)
    let l1 = normalize(vec3<f32>(-0.7,  0.3,  0.4)); // fill (left)
    let l2 = normalize(vec3<f32>( 0.2, -0.6, -0.8)); // back/under
    var direct_light = vec3<f32>(
        0.45 * abs(dot(n, l0))
            + 0.30 * abs(dot(n, l1))
            + 0.25 * abs(dot(n, l2))
    );
    var key_light = l0;
    if (u.lighting.x > 0.5) {
        direct_light = vec3<f32>(0.0);
        let count = min(u32(u.lighting.x), 4u);
        for (var index = 0u; index < count; index = index + 1u) {
            let position_type = u.light_position_type[index];
            let direction_intensity = u.light_direction_intensity[index];
            let color_hotspot = u.light_color_hotspot[index];
            let attenuation_data = u.light_attenuation[index];
            var light_vector = -normalize(direction_intensity.xyz);
            var attenuation = 1.0;
            if (position_type.w > 1.5) {
                let delta = position_type.xyz - in.world_pos;
                let distance = max(length(delta), 1e-5);
                light_vector = delta / distance;
                if (attenuation_data.x > 1.5) {
                    attenuation /= max(distance * distance, 1.0);
                } else if (attenuation_data.x > 0.5) {
                    attenuation /= max(distance, 1.0);
                }
                if (attenuation_data.z > attenuation_data.y) {
                    attenuation *= 1.0 - smoothstep(
                        attenuation_data.y,
                        attenuation_data.z,
                        distance,
                    );
                }
                if (position_type.w > 2.5) {
                    let source_to_fragment = -light_vector;
                    let cone = dot(
                        source_to_fragment,
                        normalize(direction_intensity.xyz),
                    );
                    attenuation *= smoothstep(
                        attenuation_data.w,
                        color_hotspot.w,
                        cone,
                    );
                }
            }
            let strength = max(direction_intensity.w, 0.0) * attenuation;
            direct_light += color_hotspot.rgb
                * max(dot(n, light_vector), 0.0)
                * strength;
            if (index == 0u) {
                key_light = light_vector;
            }
        }
    }
    let view = normalize(in.eye_vec);
    let half_vec = normalize(key_light + view);
    let gloss_exp = mix(2.0, 128.0, clamp(in.material.x, 0.0, 1.0));
    let fresnel0 = pow(
        (max(in.specular.w, 1.0) - 1.0) / (max(in.specular.w, 1.0) + 1.0),
        2.0,
    );
    let fresnel = fresnel0
        + (1.0 - fresnel0) * pow(1.0 - abs(dot(n, view)), 5.0);
    let specular_strength = clamp(0.08 + in.material.y + fresnel, 0.0, 1.5);
    var specular_color = in.specular.rgb;
    if (maps.present0.y != 0u) {
        let texel = textureSample(specular_map, specular_sampler, in.uv_specular).rgb;
        specular_color = mix(specular_color, texel, clamp(maps.blends0.y, 0.0, 1.0));
    }
    let half_response = select(
        abs(dot(n, half_vec)),
        max(dot(n, half_vec), 0.0),
        u.lighting.x > 0.5,
    );
    let specular = specular_color
        * pow(half_response, gloss_exp)
        * specular_strength
        * max(in.advanced.z, 0.0);
    var albedo = in.color.rgb;
    if (maps.present0.x != 0u) {
        let texel = textureSample(diffuse_map, diffuse_sampler, in.uv_diffuse).rgb;
        albedo = mix(albedo, texel, clamp(maps.blends0.x, 0.0, 1.0));
    }
    let ambient_light = albedo * clamp(in.ambient.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    var lit = ambient_light + albedo * clamp(direct_light, vec3<f32>(0.0), vec3<f32>(2.0)) + specular;
    if (maps.present0.z != 0u) {
        let reflected = textureSample(
            reflection_map,
            reflection_sampler,
            in.uv_reflection,
        ).rgb;
        lit = mix(
            lit,
            reflected,
            clamp(maps.blends0.z * in.material.y * max(in.advanced.z, 0.0), 0.0, 1.0),
        );
    }
    if (maps.present1.y != 0u) {
        let transmitted = textureSample(
            refraction_map,
            refraction_sampler,
            in.uv_refraction,
        ).rgb;
        lit = mix(
            lit,
            transmitted,
            clamp(maps.blends1.y * in.ambient.a * max(in.advanced.w, 0.0), 0.0, 1.0),
        );
    }
    let emission = clamp(in.material.z + in.material.w, 0.0, 1.0);
    let rgb = mix(lit, albedo, emission);
    var material_alpha = in.color.a;
    if (maps.present0.w != 0u) {
        let opacity = textureSample(opacity_map, opacity_sampler, in.uv_opacity).r;
        material_alpha *= mix(1.0, opacity, clamp(maps.blends0.w, 0.0, 1.0));
    }
    material_alpha *= 1.0 - clamp(in.ambient.a * max(in.advanced.w, 0.0), 0.0, 0.95);
    let alpha = select(1.0, material_alpha, u.transparency_enable > 0.5);
    return vec4<f32>(rgb, alpha);
}

// Edge fragment (LineList): flat entity colour, no lighting. Used for the
// wireframe and hidden-line edge passes so lines read at their true colour.
@fragment
fn fs_edge(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb, 1.0);
}

// Edge fragment for filled render modes: force black so edges frame the shaded
// fill regardless of the solid's colour.
@fragment
fn fs_edge_black(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_highlight_selected(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(
        mix(in.color.rgb, vec3<f32>(0.15, 0.55, 1.0), 0.60),
        0.90,
    );
}

@fragment
fn fs_highlight_hover(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(
        mix(in.color.rgb, vec3<f32>(0.95, 0.55, 0.10), 0.35),
        0.82,
    );
}
