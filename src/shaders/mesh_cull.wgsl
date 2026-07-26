struct CullUniform {
    view_rot: mat4x4<f32>,
    eye: vec4<f32>,
    count: vec4<u32>,
};

struct CullItem {
    min: vec4<f32>,
    max: vec4<f32>,
    counts: vec4<u32>,
    info: vec4<u32>,
};

struct DrawIndexed {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: u32,
    first_instance: u32,
};

struct Draw {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
};

@group(0) @binding(0) var<uniform> u: CullUniform;
@group(0) @binding(1) var<storage, read> items: array<CullItem>;
@group(0) @binding(2) var<storage, read_write> opaque: array<DrawIndexed>;
@group(0) @binding(3) var<storage, read_write> transparent: array<DrawIndexed>;
@group(0) @binding(4) var<storage, read_write> wire: array<DrawIndexed>;
@group(0) @binding(5) var<storage, read_write> edge: array<Draw>;

fn chunk_visible(item: CullItem) -> bool {
    if (item.info.y == 0u) {
        return false;
    }
    var min_ndc = vec2<f32>(1e30, 1e30);
    var max_ndc = vec2<f32>(-1e30, -1e30);
    for (var corner = 0u; corner < 8u; corner++) {
        let point = vec3<f32>(
            select(item.min.x, item.max.x, (corner & 1u) != 0u),
            select(item.min.y, item.max.y, (corner & 2u) != 0u),
            select(item.min.z, item.max.z, (corner & 4u) != 0u),
        );
        let clip = u.view_rot * vec4<f32>(point - u.eye.xyz, 1.0);
        // A box touching or crossing the eye plane is conservatively retained.
        if (clip.w <= 1e-6) {
            return true;
        }
        let ndc = clip.xy / clip.w;
        min_ndc = min(min_ndc, ndc);
        max_ndc = max(max_ndc, ndc);
    }
    // 25% viewport margin on each side is 0.5 in NDC.
    return !(
        max_ndc.x < -1.5
        || min_ndc.x > 1.5
        || max_ndc.y < -1.5
        || min_ndc.y > 1.5
    );
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= u.count.x) {
        return;
    }
    let item = items[index];
    let instances = select(0u, item.info.x, chunk_visible(item));
    opaque[index] = DrawIndexed(item.counts.x, instances, 0u, 0u, 0u);
    transparent[index] = DrawIndexed(item.counts.y, instances, 0u, 0u, 0u);
    wire[index] = DrawIndexed(item.counts.z, instances, 0u, 0u, 0u);
    edge[index] = Draw(item.counts.w, instances, 0u, 0u);
}
