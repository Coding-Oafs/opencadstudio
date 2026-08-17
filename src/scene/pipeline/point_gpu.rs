//! Fixed-screen-size, depth-tested LiDAR point renderer.
//!
//! Instances carry position plus the attributes the shader colors from; the
//! colorization state itself lives in a small style uniform. Rebuilding the
//! instance buffer is reserved for membership and per-point-attribute changes
//! (tile loads, edits, selections) — color mode, class visibility, and class
//! table edits rewrite only the style uniform.

use crate::scene::model::point_cloud_model::PointCloudModel;
use iced::wgpu;
use iced::wgpu::util::DeviceExt;

/// Bytes per GPU point instance: two position vec4s (relative-to-eye high and
/// low) plus one attribute vec4 and one color/flag vec4.
pub const POINT_INSTANCE_BYTES: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointInstance {
    position_high: [f32; 4],
    position_low: [f32; 4],
    attributes: [f32; 4],
    color_selected: [f32; 4],
}

/// CPU mirror of the `Style` uniform in `point_cloud.wgsl`. Layout must match
/// WGSL uniform rules exactly; `bytemuck` casts it into one buffer write.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StyleUniforms {
    color_mode: u32,
    point_size: f32,
    _pad0: [u32; 2],
    intensity_range: [f32; 2],
    _pad1: [f32; 2],
    elevation_range: [f32; 2],
    _pad2: [f32; 2],
    class_visible: [[u32; 4]; 8],
    class_colors: [[f32; 4]; 256],
}

impl StyleUniforms {
    fn new(model: &PointCloudModel) -> Self {
        let style = &model.style;
        let mut class_visible = [[0_u32; 4]; 8];
        for (word_index, word) in style.class_visible.iter().enumerate() {
            class_visible[word_index / 4][word_index % 4] = *word;
        }
        Self {
            color_mode: style.color_mode,
            point_size: model.point_size_px.clamp(1.0, 32.0),
            _pad0: [0; 2],
            intensity_range: style.intensity_range,
            _pad1: [0.0; 2],
            elevation_range: style.elevation_range,
            _pad2: [0.0; 2],
            class_visible,
            class_colors: style.class_colors,
        }
    }
}

pub struct PointGpu {
    pipeline: wgpu::RenderPipeline,
    instances: Option<wgpu::Buffer>,
    count: u32,
    style_buffer: wgpu::Buffer,
    style_bind_group: wgpu::BindGroup,
    style_generation: u64,
    geometry_generation: u64,
    source_id: usize,
}

impl PointGpu {
    pub fn reset(&mut self) {
        self.instances = None;
        self.count = 0;
        self.geometry_generation = u64::MAX;
        self.style_generation = u64::MAX;
        self.source_id = 0;
    }

    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        frame_bgl: &wgpu::BindGroupLayout,
        stencil: wgpu::StencilState,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point_cloud.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "../../shaders/point_cloud.wgsl"
            ))),
        });
        let style_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point_cloud.style_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<StyleUniforms>() as u64
                    ),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point_cloud.pipeline_layout"),
            bind_group_layouts: &[Some(frame_bgl), Some(&style_bgl)],
            immediate_size: 0,
        });
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PointInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 3,
                },
            ],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point_cloud.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil,
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: super::MSAA_SAMPLES,
                mask: !0,
                alpha_to_coverage_enabled: true,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let style_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point_cloud.style"),
            contents: bytemuck::bytes_of(&StyleUniforms {
                color_mode: 0,
                point_size: 3.0,
                _pad0: [0; 2],
                intensity_range: [0.0, 65535.0],
                _pad1: [0.0; 2],
                elevation_range: [0.0, 0.0],
                _pad2: [0.0; 2],
                class_visible: [[u32::MAX; 4]; 8],
                class_colors: [[0.92, 0.92, 0.92, 1.0]; 256],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let style_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point_cloud.style_bind_group"),
            layout: &style_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: style_buffer.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            instances: None,
            count: 0,
            style_buffer,
            style_bind_group,
            style_generation: u64::MAX,
            geometry_generation: u64::MAX,
            source_id: 0,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &PointCloudModel,
    ) {
        let source_id = std::sync::Arc::as_ptr(&model.points) as usize;
        // Style-only changes (color mode, class visibility, class colors)
        // rewrite the uniform and skip the instance buffer entirely.
        if self.style_generation != model.style_generation
            || self.geometry_generation == u64::MAX
        {
            queue.write_buffer(
                &self.style_buffer,
                0,
                bytemuck::bytes_of(&StyleUniforms::new(model)),
            );
            self.style_generation = model.style_generation;
        }
        if self.geometry_generation == model.geometry_generation
            && self.source_id == source_id
            && self.instances.is_some()
        {
            return;
        }
        let point_size = model.point_size_px.clamp(1.0, 32.0);
        let instances: Vec<_> = model
            .points
            .iter()
            .take(u32::MAX as usize)
            .map(|point| {
                let (high, low) = split_f64(point.position);
                PointInstance {
                    position_high: [high[0], high[1], high[2], point_size],
                    position_low: [low[0], low[1], low[2], point.classification as f32],
                    attributes: [
                        point.intensity as f32,
                        point.return_number as f32,
                        point.point_source_id as f32,
                        0.0,
                    ],
                    color_selected: [
                        point.color.map_or(0.0, |color| color[0] as f32 / 65_535.0),
                        point.color.map_or(0.0, |color| color[1] as f32 / 65_535.0),
                        point.color.map_or(0.0, |color| color[2] as f32 / 65_535.0),
                        f32::from(point.selected),
                    ],
                }
            })
            .collect();
        self.count = instances.len() as u32;
        self.instances = (!instances.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point_cloud.instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        self.geometry_generation = model.geometry_generation;
        self.source_id = source_id;
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        frame_bind_group: &'a wgpu::BindGroup,
        stencil_reference: u32,
    ) {
        let Some(instances) = &self.instances else {
            return;
        };
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, &self.style_bind_group, &[]);
        pass.set_stencil_reference(stencil_reference);
        pass.set_vertex_buffer(0, instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

fn split_f64(position: [f64; 3]) -> ([f32; 3], [f32; 3]) {
    let high = [position[0] as f32, position[1] as f32, position[2] as f32];
    let low = [
        (position[0] - high[0] as f64) as f32,
        (position[1] - high[1] as f64) as f32,
        (position[2] - high[2] as f64) as f32,
    ];
    (high, low)
}
