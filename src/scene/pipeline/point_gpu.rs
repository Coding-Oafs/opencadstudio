//! Fixed-screen-size, depth-tested LiDAR point renderer.

use crate::scene::model::point_cloud_model::PointCloudModel;
use iced::wgpu;
use iced::wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointInstance {
    position_high: [f32; 4],
    position_low: [f32; 4],
    color: [f32; 4],
}

pub struct PointGpu {
    pipeline: wgpu::RenderPipeline,
    instances: Option<wgpu::Buffer>,
    count: u32,
    generation: u64,
    source_id: usize,
}

impl PointGpu {
    pub fn reset(&mut self) {
        self.instances = None;
        self.count = 0;
        self.generation = u64::MAX;
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point_cloud.pipeline_layout"),
            bind_group_layouts: &[Some(frame_bgl)],
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
        Self {
            pipeline,
            instances: None,
            count: 0,
            generation: u64::MAX,
            source_id: 0,
        }
    }

    pub fn upload(&mut self, device: &wgpu::Device, model: &PointCloudModel) {
        let source_id = std::sync::Arc::as_ptr(&model.points) as usize;
        if self.generation == model.generation && self.source_id == source_id {
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
                    position_low: [low[0], low[1], low[2], 0.0],
                    color: point.color,
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
        self.generation = model.generation;
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
