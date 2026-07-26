// Mesh GPU buffers — TriangleList rendering for solid objects.
//
// Vertex layout (40 bytes):
//   position   [f32; 3]   offset  0   12 B
//   normal     [f32; 3]   offset 12   12 B
//   color      [f32; 4]   offset 24   16 B
//                                ------
//                                 40 B / vertex

use crate::scene::model::mesh_model::{MeshLodSet, MeshModel};
use iced::wgpu;
use iced::wgpu::util::DeviceExt;

// ── Vertex layout ─────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub position_low: [f32; 3],
    /// gloss, reflectivity, self-illumination, luminance
    pub material: [f32; 4],
    /// specular RGB and refraction index
    pub specular: [f32; 4],
    pub uv_diffuse: [f32; 2],
    /// ambient RGB and translucence
    pub ambient: [f32; 4],
    /// normal strength, bump scale, reflectance scale, transmittance scale
    pub advanced: [f32; 4],
    /// illumination model, channel flags, material mode, luminance mode
    pub flags: [u32; 4],
    pub uv_specular: [f32; 2],
    pub uv_reflection: [f32; 2],
    pub uv_opacity: [f32; 2],
    pub uv_bump: [f32; 2],
    pub uv_refraction: [f32; 2],
    pub uv_normal: [f32; 2],
}

impl MeshVertex {
    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        const ATTRS: &[wgpu::VertexAttribute] = &[
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, position) as u64,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, normal) as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, color) as u64,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, position_low) as u64,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, material) as u64,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, specular) as u64,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_diffuse) as u64,
                shader_location: 6,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, ambient) as u64,
                shader_location: 7,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, advanced) as u64,
                shader_location: 8,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, flags) as u64,
                shader_location: 9,
                format: wgpu::VertexFormat::Uint32x4,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_specular) as u64,
                shader_location: 10,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_reflection) as u64,
                shader_location: 11,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_opacity) as u64,
                shader_location: 12,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_bump) as u64,
                shader_location: 13,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_refraction) as u64,
                shader_location: 14,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MeshVertex, uv_normal) as u64,
                shader_location: 15,
                format: wgpu::VertexFormat::Float32x2,
            },
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: ATTRS,
        }
    }
}

// ── GPU handle ────────────────────────────────────────────────────────────

pub struct MeshGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    /// Line-list index buffer: every triangle `(a, b, c)` from the
    /// solid index buffer is expanded into three segments
    /// `(a,b)(b,c)(c,a)`. Used by the wireframe-mode render path so 3D
    /// solids draw as their triangle edges without needing the
    /// `POLYGON_MODE_LINE` device feature.
    #[allow(dead_code)] // only the highlight overlay builds MeshGpu now (fill only)
    pub wire_index_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    pub wire_index_count: u32,
}

/// GPU-side bundle of MeshLodSet — one MeshGpu per available LOD plus
/// the world-XY AABB needed to pick a level per frame.
pub struct MeshLodGpu {
    pub lods: Vec<MeshGpu>,
    pub world_aabb: [f32; 4],
}

/// How a solid mesh is highlighted this frame.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Highlight {
    #[allow(dead_code)] // the highlight overlay only builds Selected / Hover
    None,
    /// Hovered — light orange wash.
    Hover,
    /// Selected — stronger blue wash.
    Selected,
}

impl Highlight {
    /// Blend colour and mix factor, or `None` when the mesh keeps its colour.
    fn tint(self) -> Option<([f32; 4], f32)> {
        match self {
            Highlight::None => None,
            Highlight::Hover => Some(([0.95, 0.55, 0.10, 1.0], 0.35)),
            Highlight::Selected => Some(([0.15, 0.55, 1.0, 1.0], 0.60)),
        }
    }
}

// ── Batched mesh buffers ──────────────────────────────────────────────────
//
// One MeshGpu per solid means one vertex/index bind + draw call per solid —
// ~10k draw calls a frame on a heavy 3D model, which strangles the GPU front
// end. The batch concatenates every solid's LOD0 geometry into a handful of
// large buffers (split only to stay under the 256 MB per-buffer cap), so the
// whole mesh set draws in a few calls. Vertices already carry their own colour,
// so no per-mesh state is needed between draws. Built once per geometry epoch —
// selection/hover no longer rebuild it (that tint is dropped in the batch path).

pub struct MeshBatchChunk {
    pub vertex_buffer: wgpu::Buffer,
    /// Opaque triangle indices (mesh colour alpha ≈ 1). Drawn with depth write.
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    /// Transparent triangle indices (mesh colour alpha < 1). Drawn after the
    /// opaque fills with depth write disabled so they blend over — rather than
    /// erase — the geometry behind them.
    pub transp_index_buffer: wgpu::Buffer,
    pub transp_index_count: u32,
    /// Triangle-edge line list (into `vertex_buffer`) for plain meshes that
    /// carry no B-rep edges — the tessellation wireframe.
    pub wire_index_buffer: wgpu::Buffer,
    pub wire_index_count: u32,
    /// B-rep feature edges of ACIS solids, as a standalone LineList vertex
    /// buffer (pairs of endpoints), drawn non-indexed. Empty for plain meshes.
    pub edge_vertex_buffer: wgpu::Buffer,
    pub edge_vertex_count: u32,
    pub material: Option<crate::scene::model::material_model::MeshMaterial>,
    pub material_bind_group: Option<wgpu::BindGroup>,
}

fn make_chunk(
    device: &wgpu::Device,
    verts: &[MeshVertex],
    indices: &[u32],
    transp_indices: &[u32],
    wire_indices: &[u32],
    edge_verts: &[MeshVertex],
    material: Option<&crate::scene::model::material_model::MeshMaterial>,
) -> MeshBatchChunk {
    // `create_buffer_init` with an empty slice yields a zero-sized buffer that
    // some backends reject for INDEX usage; a chunk can legitimately hold only
    // opaque or only transparent tris, so fall back to a 1-index stub (count
    // stays 0, so the draw loop skips it).
    let mk_index = |data: &[u32], label: &'static str| {
        let stub = [0u32];
        let src = if data.is_empty() { &stub[..] } else { data };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(src),
            usage: wgpu::BufferUsages::INDEX,
        })
    };
    let mk_vertex = |data: &[MeshVertex], label: &'static str| {
        let stub = [MeshVertex {
            position: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            color: [0.0; 4],
            position_low: [0.0; 3],
            material: [0.0; 4],
            specular: [1.0, 1.0, 1.0, 1.0],
            uv_diffuse: [0.0; 2],
            ambient: [0.3, 0.3, 0.3, 0.0],
            advanced: [1.0; 4],
            flags: [0, 127, 0, 0],
            uv_specular: [0.0; 2],
            uv_reflection: [0.0; 2],
            uv_opacity: [0.0; 2],
            uv_bump: [0.0; 2],
            uv_refraction: [0.0; 2],
            uv_normal: [0.0; 2],
        }];
        let src = if data.is_empty() { &stub[..] } else { data };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(src),
            usage: wgpu::BufferUsages::VERTEX,
        })
    };
    MeshBatchChunk {
        vertex_buffer: mk_vertex(verts, "mesh.batch.vbuf"),
        index_buffer: mk_index(indices, "mesh.batch.ibuf"),
        index_count: indices.len() as u32,
        transp_index_buffer: mk_index(transp_indices, "mesh.batch.transp_ibuf"),
        transp_index_count: transp_indices.len() as u32,
        wire_index_buffer: mk_index(wire_indices, "mesh.batch.wire_ibuf"),
        wire_index_count: wire_indices.len() as u32,
        edge_vertex_buffer: mk_vertex(edge_verts, "mesh.batch.edge_vbuf"),
        edge_vertex_count: edge_verts.len() as u32,
        material: material.cloned(),
        material_bind_group: None,
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialMapParams {
    /// diffuse, specular, reflection and opacity blend factors.
    blends0: [f32; 4],
    /// Presence bits for the same four maps.
    present0: [u32; 4],
    /// bump, refraction, normal and reserved blend factors.
    blends1: [f32; 4],
    /// Presence bits for the same four maps.
    present1: [u32; 4],
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    image: Option<&crate::scene::model::material_model::MaterialImage>,
    fallback: [u8; 4],
    srgb: bool,
) -> wgpu::TextureView {
    let (width, height, pixels) = image.map_or((1, 1, fallback.as_slice()), |image| {
        (image.width, image.height, image.rgba.as_slice())
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

pub fn create_material_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    material: Option<&crate::scene::model::material_model::MeshMaterial>,
) -> wgpu::BindGroup {
    let diffuse = material.and_then(|material| material.diffuse_map.image.as_deref());
    let specular = material.and_then(|material| material.specular_map.image.as_deref());
    let reflection = material.and_then(|material| material.reflection_map.image.as_deref());
    let opacity = material.and_then(|material| material.opacity_map.image.as_deref());
    let bump = material.and_then(|material| material.bump_map.image.as_deref());
    let refraction = material.and_then(|material| material.refraction_map.image.as_deref());
    let normal = material.and_then(|material| material.normal_map.image.as_deref());
    let diffuse_view =
        upload_rgba_texture(device, queue, "mesh.material.diffuse", diffuse, [255; 4], true);
    let specular_view =
        upload_rgba_texture(device, queue, "mesh.material.specular", specular, [255; 4], true);
    let reflection_view =
        upload_rgba_texture(device, queue, "mesh.material.reflection", reflection, [0, 0, 0, 255], true);
    let opacity_view =
        upload_rgba_texture(device, queue, "mesh.material.opacity", opacity, [255; 4], false);
    let bump_view =
        upload_rgba_texture(device, queue, "mesh.material.bump", bump, [128, 128, 128, 255], false);
    let refraction_view =
        upload_rgba_texture(device, queue, "mesh.material.refraction", refraction, [255; 4], true);
    let normal_view = upload_rgba_texture(
        device,
        queue,
        "mesh.material.normal",
        normal,
        [128, 128, 255, 255],
        false,
    );
    let sampler = |label: &'static str, tiling: u8| {
        let address = match tiling {
            1 => wgpu::AddressMode::Repeat,
            4 => wgpu::AddressMode::MirrorRepeat,
            _ => wgpu::AddressMode::ClampToEdge,
        };
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        })
    };
    let tiling = |channel: usize| {
        material.map_or(1, |material| {
            [
                material.diffuse_map.tiling,
                material.specular_map.tiling,
                material.reflection_map.tiling,
                material.opacity_map.tiling,
                material.bump_map.tiling,
                material.refraction_map.tiling,
                material.normal_map.tiling,
            ][channel]
        })
    };
    let diffuse_sampler = sampler("mesh.material.diffuse_sampler", tiling(0));
    let specular_sampler = sampler("mesh.material.specular_sampler", tiling(1));
    let reflection_sampler = sampler("mesh.material.reflection_sampler", tiling(2));
    let opacity_sampler = sampler("mesh.material.opacity_sampler", tiling(3));
    let bump_sampler = sampler("mesh.material.bump_sampler", tiling(4));
    let refraction_sampler = sampler("mesh.material.refraction_sampler", tiling(5));
    let normal_sampler = sampler("mesh.material.normal_sampler", tiling(6));
    let params = MaterialMapParams {
        blends0: material.map_or([0.0; 4], |material| {
            [
                material.diffuse_map.blend_factor,
                material.specular_map.blend_factor,
                material.reflection_map.blend_factor,
                material.opacity_map.blend_factor,
            ]
        }),
        present0: material.map_or([0; 4], |material| {
            [
                material.diffuse_map.image.is_some() as u32,
                material.specular_map.image.is_some() as u32,
                material.reflection_map.image.is_some() as u32,
                material.opacity_map.image.is_some() as u32,
            ]
        }),
        blends1: material.map_or([0.0; 4], |material| {
            [
                material.bump_map.blend_factor,
                material.refraction_map.blend_factor,
                material.normal_map.blend_factor,
                0.0,
            ]
        }),
        present1: material.map_or([0; 4], |material| {
            [
                material.bump_map.image.is_some() as u32,
                material.refraction_map.image.is_some() as u32,
                material.normal_map.image.is_some() as u32,
                0,
            ]
        }),
    };
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh.material.params"),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh.material.bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&diffuse_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&specular_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&reflection_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&opacity_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&bump_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&refraction_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&normal_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(&diffuse_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::Sampler(&specular_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::Sampler(&reflection_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::Sampler(&opacity_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::Sampler(&bump_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::Sampler(&refraction_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 13,
                resource: wgpu::BindingResource::Sampler(&normal_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 14,
                resource: params_buffer.as_entire_binding(),
            },
        ],
    })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MaterialBatchKey([u32; 16]);

fn material_key(
    material: Option<&crate::scene::model::material_model::MeshMaterial>,
    color: [f32; 4],
) -> MaterialBatchKey {
    let Some(material) = material else {
        return MaterialBatchKey([
            0,
            0,
            color[0].to_bits(),
            color[1].to_bits(),
            color[2].to_bits(),
            color[3].to_bits(),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]);
    };
    let handle = material.handle.map_or(0, |handle| handle.value());
    MaterialBatchKey([
        handle as u32,
        (handle >> 32) as u32,
        material.diffuse[0].to_bits(),
        material.diffuse[1].to_bits(),
        material.diffuse[2].to_bits(),
        material.diffuse[3].to_bits(),
        material.specular[0].to_bits(),
        material.specular[1].to_bits(),
        material.specular[2].to_bits(),
        material.gloss.to_bits(),
        material.reflectivity.to_bits(),
        material.self_illumination.to_bits(),
        material.luminance.to_bits(),
        material.refraction_index.to_bits(),
        material.diffuse_map.projection as u32,
        material.diffuse_map.tiling as u32,
    ])
}

struct MeshBatchPart<'a> {
    set: &'a MeshLodSet,
    mesh: &'a MeshModel,
    material: Option<&'a crate::scene::model::material_model::MeshMaterial>,
    color: [f32; 4],
    indices: Vec<u32>,
    include_faces: bool,
    include_edges: bool,
}

fn material_map_uv(
    map: &crate::scene::model::material_model::MeshTextureMap,
    position: [f32; 3],
    normal: [f32; 3],
) -> [f32; 2] {
    let m = &map.transform;
    let p = [
        position[0] * m[0] + position[1] * m[1] + position[2] * m[2] + m[3],
        position[0] * m[4] + position[1] * m[5] + position[2] * m[6] + m[7],
        position[0] * m[8] + position[1] * m[9] + position[2] * m[10] + m[11],
    ];
    match map.projection {
        3 => [
            p[1].atan2(p[0]) / std::f32::consts::TAU + 0.5,
            p[2],
        ],
        4 => {
            let radius = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            if radius <= f32::EPSILON {
                [0.0; 2]
            } else {
                [
                    p[1].atan2(p[0]) / std::f32::consts::TAU + 0.5,
                    (p[2] / radius).clamp(-1.0, 1.0).acos() / std::f32::consts::PI,
                ]
            }
        }
        2 => {
            let n = [normal[0].abs(), normal[1].abs(), normal[2].abs()];
            if n[0] >= n[1] && n[0] >= n[2] {
                [p[1], p[2]]
            } else if n[1] >= n[2] {
                [p[0], p[2]]
            } else {
                [p[0], p[1]]
            }
        }
        _ => [p[0], p[1]],
    }
}

fn material_uvs(
    material: Option<&crate::scene::model::material_model::MeshMaterial>,
    position: [f32; 3],
    normal: [f32; 3],
) -> [[f32; 2]; 7] {
    let Some(material) = material else {
        return [[0.0; 2]; 7];
    };
    [
        material_map_uv(&material.diffuse_map, position, normal),
        material_map_uv(&material.specular_map, position, normal),
        material_map_uv(&material.reflection_map, position, normal),
        material_map_uv(&material.opacity_map, position, normal),
        material_map_uv(&material.bump_map, position, normal),
        material_map_uv(&material.refraction_map, position, normal),
        material_map_uv(&material.normal_map, position, normal),
    ]
}

fn material_vertex_params(
    material: Option<&crate::scene::model::material_model::MeshMaterial>,
) -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4], [u32; 4]) {
    let Some(material) = material else {
        return (
            [0.5, 0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.3, 0.3, 0.3, 0.0],
            [1.0; 4],
            [0, 127, 0, 0],
        );
    };
    (
        [
            material.gloss,
            material.reflectivity,
            material.self_illumination,
            material.luminance,
        ],
        [
            material.specular[0],
            material.specular[1],
            material.specular[2],
            material.refraction_index,
        ],
        [
            material.ambient[0],
            material.ambient[1],
            material.ambient[2],
            material.translucence,
        ],
        [
            material.normal_map_strength,
            material.indirect_bump_scale,
            material.reflectance_scale,
            material.transmittance_scale,
        ],
        [
            material.illumination_model as u32,
            material.channel_flags as u32,
            material.mode as u32,
            material.luminance_mode as u32,
        ],
    )
}

/// Concatenate every set's first non-empty LOD into a few large GPU buffers.
/// Returns the chunks plus the total triangle count drawn (for diagnostics).
///
/// Every emitted buffer stays under the device's `max_buffer_size` (default
/// 256 MB). Both the vertex buffer (`size_of::<MeshVertex>()` B/vert) and the
/// wire-index buffer (6 u32 = 24 B/triangle — the fattest index buffer) are
/// bounded; a single mesh too large for one chunk is split into triangle-soup
/// sub-chunks so an XREF-heavy model can never overflow a single buffer (#203).
pub fn build_mesh_batch(device: &wgpu::Device, sets: &[MeshLodSet]) -> (Vec<MeshBatchChunk>, u64) {
    // Derive the caps from the real device limit and vertex size. The previous
    // fixed 6 M-vertex cap assumed 40 B/vertex, but `position_low` (RTE) grew
    // MeshVertex to 52 B, so 6 M × 52 B = 312 MB blew past the 256 MB cap.
    let budget = (device.limits().max_buffer_size as usize / 10) * 9; // 10% headroom
    let vsize = std::mem::size_of::<MeshVertex>();
    let max_verts = (budget / vsize).max(3);
    let max_tris = (budget / (6 * 4)).max(1); // wire-index buffer: 6 u32 per tri

    let mut chunks = Vec::new();
    let mut verts: Vec<MeshVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut transp_indices: Vec<u32> = Vec::new();
    let mut wire_indices: Vec<u32> = Vec::new();
    let mut edge_verts: Vec<MeshVertex> = Vec::new();
    let mut total_tris = 0u64;
    let mut ordered: Vec<MeshBatchPart<'_>> = Vec::new();
    for set in sets {
        let Some(mesh) = set.lods.iter().find(|mesh| !mesh.indices.is_empty()) else {
            continue;
        };
        let triangle_count = mesh.indices.len() / 3;
        let has_face_materials = mesh.triangle_material_handles.len() == triangle_count
            && !set.face_materials.is_empty();
        let has_face_colors = mesh.triangle_colors.len() == triangle_count
            && mesh.triangle_colors.iter().any(Option::is_some);
        let include_faces = set
            .visual_style
            .as_ref()
            .map_or(true, |style| style.face_visible());
        let include_edges = set
            .visual_style
            .as_ref()
            .map_or(true, |style| style.edges_visible());
        if !has_face_materials && !has_face_colors {
            let base_color =
                set.material.as_ref().map_or(mesh.color, |material| material.diffuse);
            let color = set
                .visual_style
                .as_ref()
                .map_or(base_color, |style| style.face_color(base_color));
            ordered.push(MeshBatchPart {
                set,
                mesh,
                material: set.material.as_ref(),
                color,
                indices: mesh.indices.clone(),
                include_faces,
                include_edges,
            });
            continue;
        }
        let mut groups: std::collections::BTreeMap<
            MaterialBatchKey,
            (
                Option<&crate::scene::model::material_model::MeshMaterial>,
                [f32; 4],
                Vec<u32>,
            ),
        > = std::collections::BTreeMap::new();
        for (triangle, indices) in mesh.indices.chunks_exact(3).enumerate() {
            let material = if has_face_materials {
                mesh.triangle_material_handles[triangle]
                    .and_then(|handle| set.face_materials.get(&handle))
                    .or(set.material.as_ref())
            } else {
                set.material.as_ref()
            };
            let base_color = material.map_or(mesh.color, |material| material.diffuse);
            let base_color = if has_face_colors {
                mesh.triangle_colors[triangle].unwrap_or(base_color)
            } else {
                base_color
            };
            let color = set
                .visual_style
                .as_ref()
                .map_or(base_color, |style| style.face_color(base_color));
            groups
                .entry(material_key(material, color))
                .or_insert_with(|| (material, color, Vec::new()))
                .2
                .extend_from_slice(indices);
        }
        for (part_index, (_, (material, color, indices))) in groups.into_iter().enumerate() {
            ordered.push(MeshBatchPart {
                set,
                mesh,
                material,
                color,
                indices,
                include_faces,
                include_edges: include_edges && part_index == 0,
            });
        }
    }
    ordered.sort_by_key(|part| {
        material_key(part.material, part.color)
    });
    let mut active_key: Option<MaterialBatchKey> = None;
    let mut active_material: Option<&crate::scene::model::material_model::MeshMaterial> = None;
    for part in ordered {
        let set = part.set;
        let mesh = part.mesh;
        let material = part.material;
        let part_color = part.color;
        let key = material_key(material, part_color);
        if active_key.is_some_and(|active| active != key)
            && (!verts.is_empty() || !edge_verts.is_empty())
        {
            chunks.push(make_chunk(
                device,
                &verts,
                &indices,
                &transp_indices,
                &wire_indices,
                &edge_verts,
                active_material,
            ));
            verts.clear();
            indices.clear();
            transp_indices.clear();
            wire_indices.clear();
            edge_verts.clear();
        }
        active_key = Some(key);
        active_material = material;
        let has_normals = mesh.normals.len() == mesh.verts.len();
        let (material_params, specular, ambient, advanced, flags) =
            material_vertex_params(material);
        let edge_color = set
            .visual_style
            .as_ref()
            .map_or(mesh.color, |style| style.edge_color(mesh.color));
        let vtx = |vi: usize| {
            let normal = if has_normals {
                mesh.normals[vi]
            } else {
                [0.0, 1.0, 0.0]
            };
            let uv = material_uvs(material, mesh.verts[vi], normal);
            MeshVertex {
                position: mesh.verts[vi],
                normal,
                color: part_color,
                position_low: mesh.verts_low.get(vi).copied().unwrap_or([0.0; 3]),
                material: material_params,
                specular,
                uv_diffuse: uv[0],
                ambient,
                advanced,
                flags,
                uv_specular: uv[1],
                uv_reflection: uv[2],
                uv_opacity: uv[3],
                uv_bump: uv[4],
                uv_refraction: uv[5],
                uv_normal: uv[6],
            }
        };
        // A solid whose baked colour is not fully opaque routes into the
        // transparent index stream so it is drawn last, without depth writes.
        let is_transp = part_color[3] < 0.999;
        let mesh_tris = part.indices.len() / 3;
        if part.include_faces {
            total_tris += mesh_tris as u64;
        }

        // Feature edges present (ACIS solid) → emit the B-rep edges as a line
        // list and skip the triangulation wireframe. Absent (plain mesh) → keep
        // the triangle edges so the mesh still shows a wireframe.
        let has_feat = !set.edge_verts.is_empty();
        if has_feat && part.include_edges {
            // Feature edges use their own vertex buffer, so they need their
            // own cap. Large ACIS models can have relatively few faces but
            // millions of B-rep edge vertices; only checking `mesh.verts`
            // allowed `edge_vbuf` to exceed wgpu's max_buffer_size.
            let mut edge_start = 0;
            let edge_end = set.edge_verts.len() & !1usize;
            while edge_start < edge_end {
                let available = max_verts.saturating_sub(edge_verts.len());
                // LineList consumes pairs. Never split a segment between
                // chunks even when the vertex budget is odd.
                let take = available
                    .min(edge_end - edge_start)
                    & !1usize;
                if take == 0 {
                    chunks.push(make_chunk(
                        device,
                        &verts,
                        &indices,
                        &transp_indices,
                        &wire_indices,
                        &edge_verts,
                        active_material,
                    ));
                    verts.clear();
                    indices.clear();
                    transp_indices.clear();
                    wire_indices.clear();
                    edge_verts.clear();
                    continue;
                }
                for i in edge_start..edge_start + take {
                    edge_verts.push(MeshVertex {
                        position: set.edge_verts[i],
                        normal: [0.0, 1.0, 0.0],
                        color: edge_color,
                        position_low: set.edge_verts_low.get(i).copied().unwrap_or([0.0; 3]),
                        material: material_params,
                        specular,
                        uv_diffuse: [0.0; 2],
                        ambient,
                        advanced,
                        flags,
                        uv_specular: [0.0; 2],
                        uv_reflection: [0.0; 2],
                        uv_opacity: [0.0; 2],
                        uv_bump: [0.0; 2],
                        uv_refraction: [0.0; 2],
                        uv_normal: [0.0; 2],
                    });
                }
                edge_start += take;
                if edge_start < edge_end {
                    chunks.push(make_chunk(
                        device,
                        &verts,
                        &indices,
                        &transp_indices,
                        &wire_indices,
                        &edge_verts,
                        active_material,
                    ));
                    verts.clear();
                    indices.clear();
                    transp_indices.clear();
                    wire_indices.clear();
                    edge_verts.clear();
                }
            }
        }

        // A single mesh larger than a whole chunk: emit as triangle-soup
        // sub-chunks (corners expanded, no vertex sharing) so each buffer fits.
        if mesh.verts.len() > max_verts || mesh_tris > max_tris {
            if !verts.is_empty() || !edge_verts.is_empty() {
                chunks.push(make_chunk(
                    device,
                    &verts,
                    &indices,
                    &transp_indices,
                    &wire_indices,
                    &edge_verts,
                    active_material,
                ));
                verts.clear();
                indices.clear();
                transp_indices.clear();
                wire_indices.clear();
                edge_verts.clear();
            }
            let tris_per = (max_verts / 3).min(max_tris).max(1);
            let mut t = 0;
            while t < mesh_tris {
                let end = (t + tris_per).min(mesh_tris);
                let (mut sv, mut si, mut swi) = (Vec::new(), Vec::new(), Vec::new());
                for tri in t..end {
                    let ix = &part.indices[tri * 3..tri * 3 + 3];
                    let b = sv.len() as u32;
                    sv.push(vtx(ix[0] as usize));
                    sv.push(vtx(ix[1] as usize));
                    sv.push(vtx(ix[2] as usize));
                    if part.include_faces {
                        si.extend_from_slice(&[b, b + 1, b + 2]);
                    }
                    if part.include_edges && !has_feat {
                        swi.extend_from_slice(&[b, b + 1, b + 1, b + 2, b + 2, b]);
                    }
                }
                // The whole mesh shares one colour, so a sub-chunk is entirely
                // opaque or entirely transparent.
                if is_transp {
                    chunks.push(make_chunk(
                        device,
                        &sv,
                        &[],
                        &si,
                        &swi,
                        &[],
                        active_material,
                    ));
                } else {
                    chunks.push(make_chunk(
                        device,
                        &sv,
                        &si,
                        &[],
                        &swi,
                        &[],
                        active_material,
                    ));
                }
                t = end;
            }
            continue;
        }

        // Flush when adding this mesh would overflow either the vertex buffer
        // or the wire-index buffer.
        if !verts.is_empty()
            && (verts.len() + mesh.verts.len() > max_verts
                || wire_indices.len() / 6 + mesh_tris > max_tris)
        {
            chunks.push(make_chunk(
                device,
                &verts,
                &indices,
                &transp_indices,
                &wire_indices,
                &edge_verts,
                active_material,
            ));
            verts.clear();
            indices.clear();
            transp_indices.clear();
            wire_indices.clear();
            edge_verts.clear();
        }
        let base = verts.len() as u32;
        for i in 0..mesh.verts.len() {
            verts.push(vtx(i));
        }
        if part.include_faces {
            let fill = if is_transp { &mut transp_indices } else { &mut indices };
            for &idx in &part.indices {
                fill.push(base + idx);
            }
        }
        if part.include_edges && !has_feat {
            for tri in part.indices.chunks_exact(3) {
                let (a, b, c) = (base + tri[0], base + tri[1], base + tri[2]);
                wire_indices.extend_from_slice(&[a, b, b, c, c, a]);
            }
        }
    }
    if !indices.is_empty()
        || !transp_indices.is_empty()
        || !wire_indices.is_empty()
        || !edge_verts.is_empty()
    {
        chunks.push(make_chunk(
            device,
            &verts,
            &indices,
            &transp_indices,
            &wire_indices,
            &edge_verts,
            active_material,
        ));
    }
    (chunks, total_tris)
}

impl MeshLodGpu {
    #[allow(dead_code)] // built by the bypassed per-mesh upload_meshes path
    pub fn new(device: &wgpu::Device, set: &MeshLodSet, highlight: Highlight) -> Self {
        Self {
            lods: set
                .lods
                .iter()
                .filter(|m| !m.indices.is_empty())
                .map(|m| MeshGpu::new(device, m, set.material.as_ref(), highlight))
                .collect(),
            world_aabb: set.world_aabb,
        }
    }
}

impl MeshGpu {
    pub fn new(
        device: &wgpu::Device,
        mesh: &MeshModel,
        material: Option<&crate::scene::model::material_model::MeshMaterial>,
        highlight: Highlight,
    ) -> Self {
        let has_normals = mesh.normals.len() == mesh.verts.len();
        // Blend the base colour toward the highlight so a selected / hovered
        // solid reads clearly while keeping some shape shading.
        let color = match highlight.tint() {
            Some((hl, t)) => {
                let mut c = [0.0f32; 4];
                for k in 0..4 {
                    c[k] = mesh.color[k] * (1.0 - t) + hl[k] * t;
                }
                c
            }
            None => mesh.color,
        };
        let (material_params, specular, ambient, advanced, flags) =
            material_vertex_params(material);
        let vertices: Vec<MeshVertex> = mesh
            .verts
            .iter()
            .enumerate()
            .map(|(i, &pos)| {
                let normal = if has_normals {
                    mesh.normals[i]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let uv = material_uvs(material, pos, normal);
                MeshVertex {
                    position: pos,
                    normal,
                    color,
                    position_low: mesh.verts_low.get(i).copied().unwrap_or([0.0; 3]),
                    material: material_params,
                    specular,
                    uv_diffuse: uv[0],
                    ambient,
                    advanced,
                    flags,
                    uv_specular: uv[1],
                    uv_reflection: uv[2],
                    uv_opacity: uv[3],
                    uv_bump: uv[4],
                    uv_refraction: uv[5],
                    uv_normal: uv[6],
                }
            })
            .collect();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("mesh.vbuf.{}", mesh.name)),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("mesh.ibuf.{}", mesh.name)),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Wireframe-mode index buffer: expand each triangle into its
        // three edge segments. Allocates ~2× the solid index count but
        // is cheap compared to mesh tessellation and only happens when
        // a new mesh is uploaded.
        let mut wire_indices: Vec<u32> = Vec::with_capacity(mesh.indices.len() * 2);
        for tri in mesh.indices.chunks_exact(3) {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            wire_indices.extend_from_slice(&[a, b, b, c, c, a]);
        }
        let wire_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("mesh.wire_ibuf.{}", mesh.name)),
            contents: bytemuck::cast_slice(&wire_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            wire_index_buffer,
            wire_index_count: wire_indices.len() as u32,
        }
    }
}
