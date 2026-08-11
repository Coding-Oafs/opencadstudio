// Image GPU buffers — renders raster images as textured quads.
//
// Group 1 bindings per image:
//   binding 0 — texture_2d<f32>   (RGBA image texture)
//   binding 1 — sampler           (bilinear filtering)
//   binding 2 — ImageParams       (opacity uniform, 16 bytes)

use crate::scene::model::image_model::ImageModel;
use iced::wgpu;
use iced::wgpu::util::DeviceExt;

// ── Vertex ────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImageVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub pos_low: [f32; 3],
}

impl ImageVertex {
    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        const ATTRS: &[wgpu::VertexAttribute] = &[
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(ImageVertex, pos) as u64,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(ImageVertex, uv) as u64,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(ImageVertex, pos_low) as u64,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x3,
            },
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: ATTRS,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImageInstance {
    pub translation: [f32; 3],
    pub translation_low: [f32; 3],
    pub draw_depth: f32,
    pub _pad: [f32; 3],
}

impl ImageInstance {
    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        const ATTRS: &[wgpu::VertexAttribute] = &[
            wgpu::VertexAttribute { offset: std::mem::offset_of!(ImageInstance, translation) as u64, shader_location: 3, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: std::mem::offset_of!(ImageInstance, translation_low) as u64, shader_location: 4, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: std::mem::offset_of!(ImageInstance, draw_depth) as u64, shader_location: 5, format: wgpu::VertexFormat::Float32 },
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: ATTRS,
        }
    }
}

// ── Uniform ───────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ImageParams {
    opacity: f32,
    /// Signed draw-order depth (-1,1); applied as a clip-z bias in the shader
    /// so the raster orders against other entity types. 0.0 = neutral.
    draw_depth: f32,
    _pad: [f32; 2],
} // 16 bytes

// ── Per-image GPU handle ──────────────────────────────────────────────────

pub struct ImageGpu {
    pub vertex_buffer: wgpu::Buffer,
    pub instance_buffer: wgpu::Buffer,
    /// Number of triangle vertices in `vertex_buffer` — 6 for a plain quad, or
    /// more when the raster is clipped to a triangulated polygon.
    pub vertex_count: u32,
    pub instance_count: u32,
    pub bind_group: wgpu::BindGroup,
    _texture: wgpu::Texture,
    _sampler: wgpu::Sampler,
    _params_buf: wgpu::Buffer,
}

impl ImageGpu {
    pub fn from_models(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        models: &[ImageModel],
        bgl1: &wgpu::BindGroupLayout,
    ) -> Vec<Self> {
        let mut slots = rustc_hash::FxHashMap::default();
        let mut groups: Vec<Vec<&ImageModel>> = Vec::new();
        for (index, model) in models.iter().enumerate() {
            let key = model
                .render_instance
                .map(|instance| (true, instance.source_id))
                .unwrap_or((false, index as u64));
            let slot = *slots.entry(key).or_insert_with(|| {
                let slot = groups.len();
                groups.push(Vec::new());
                slot
            });
            groups[slot].push(model);
        }
        groups
            .into_iter()
            .filter_map(|group| Self::new(device, queue, &group, bgl1))
            .collect()
    }

    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        models: &[&ImageModel],
        bgl1: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        let &model = models.first()?;
        if model.pixels.is_empty() || model.width == 0 || model.height == 0 {
            return None;
        }

        // ── Upload texture ────────────────────────────────────────────────
        let tex_label = format!("image.texture:{}", model.file_path);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&tex_label),
            size: wgpu::Extent3d {
                width: model.width,
                height: model.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &model.pixels[..],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * model.width),
                rows_per_image: Some(model.height),
            },
            wgpu::Extent3d {
                width: model.width,
                height: model.height,
                depth_or_array_layers: 1,
            },
        );
        let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Sampler ───────────────────────────────────────────────────────
        let _sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image.sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // ── Opacity uniform ───────────────────────────────────────────────
        let params = ImageParams {
            opacity: model.opacity.clamp(0.0, 1.0),
            draw_depth: 0.0,
            _pad: [0.0; 2],
        };
        let _params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // ── Bind group ────────────────────────────────────────────────────
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image.bind_group1"),
            layout: bgl1,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: _params_buf.as_entire_binding(),
                },
            ],
        });

        // ── Vertex buffer — the image's visible triangles ─────────────────
        // `model.verts` already holds either the full quad or the triangulated
        // clip polygon, each vertex carrying an RTE-split position and its UV.
        let verts: Vec<ImageVertex> = model
            .verts
            .iter()
            .map(|v| ImageVertex {
                pos: v.pos,
                uv: v.uv,
                pos_low: v.pos_low,
            })
            .collect();
        if verts.is_empty() {
            return None;
        }
        let vertex_count = verts.len() as u32;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image.vbuf"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let base = model
            .render_instance
            .map_or([0.0; 3], |instance| instance.translation);
        let instances: Vec<ImageInstance> = models
            .iter()
            .map(|model| {
                let translation = model
                    .render_instance
                    .map_or([0.0; 3], |instance| instance.translation);
                let delta = [
                    translation[0] - base[0],
                    translation[1] - base[1],
                    translation[2] - base[2],
                ];
                let high = delta.map(|value| value as f32);
                ImageInstance {
                    translation: high,
                    translation_low: [
                        (delta[0] - high[0] as f64) as f32,
                        (delta[1] - high[1] as f64) as f32,
                        (delta[2] - high[2] as f64) as f32,
                    ],
                    draw_depth: model.draw_depth,
                    _pad: [0.0; 3],
                }
            })
            .collect();
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image.instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Some(Self {
            vertex_buffer,
            instance_buffer,
            vertex_count,
            instance_count: instances.len() as u32,
            bind_group,
            _texture: texture,
            _sampler,
            _params_buf,
        })
    }
}
