//! Fixed-screen-size, depth-tested LiDAR point renderer.
//!
//! Instances carry position plus the attributes the shader colors from; the
//! colorization state itself lives in a small style uniform. Behind the
//! `point-arena` feature the instance buffer is a persistent arena paged by
//! chunk: streamed tiles enter and leave with one ranged write each instead
//! of rebuilding the whole buffer. The planning core is pure so it is
//! unit-testable without a device.

use crate::scene::model::point_cloud_model::{PointChunk, PointCloudModel};
use iced::wgpu;
use iced::wgpu::util::DeviceExt;

/// Bytes per GPU point instance: two position vec4s (relative-to-eye high and
/// low) plus one attribute vec4 and one color/flag vec4.
pub const POINT_INSTANCE_BYTES: usize = 64;

/// Smallest arena capacity (instances) so tiny clouds do not thrash the
/// buffer between recreations.
const MIN_ARENA_CAPACITY: u32 = 1 << 16;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointInstance {
    position_high: [f32; 4],
    position_low: [f32; 4],
    attributes: [f32; 4],
    color_selected: [f32; 4],
}

fn build_instances(
    points: &[crate::scene::PointCloudPoint],
    point_size: f32,
) -> Vec<PointInstance> {
    points
        .iter()
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
        .collect()
}

// ── Style uniform ──────────────────────────────────────────────────────────

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
    // Cross-section band: p0.xy, p1.xy, and (half_width_px, mode). mode: 0 =
    // off, 1 = dim, 2 = discard. `half_width_px` is in screen pixels (the
    // shader scales by world_per_pixel); a degenerate p0==p1 (mode != 0) is
    // treated as "off".
    section_p0: [f32; 2],
    _pad3: [f32; 2],
    section_p1: [f32; 2],
    _pad4: [f32; 2],
    section_params: [f32; 2],
    _pad5: [f32; 2],
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
        // Encode the active section (dim or discard). A zero-length segment
        // degrades to "off" so a half-formed section never blanks the cloud.
        let (p0, p1, params) = match style.section {
            Some(section) if section.p0 != section.p1 && section.half_width_px > 0.0 => (
                [section.p0[0] as f32, section.p0[1] as f32],
                [section.p1[0] as f32, section.p1[1] as f32],
                [
                    section.half_width_px as f32,
                    match section.mode {
                        crate::scene::model::point_cloud_model::SectionMode::Dim => 1.0,
                        crate::scene::model::point_cloud_model::SectionMode::Discard => 2.0,
                    },
                ],
            ),
            _ => ([0.0; 2], [0.0; 2], [0.0; 2]),
        };
        Self {
            color_mode: style.color_mode,
            point_size: model.point_size_px.clamp(1.0, 32.0),
            _pad0: [0; 2],
            intensity_range: style.intensity_range,
            _pad1: [0.0; 2],
            elevation_range: style.elevation_range,
            _pad2: [0.0; 2],
            section_p0: p0,
            _pad3: [0.0; 2],
            section_p1: p1,
            _pad4: [0.0; 2],
            section_params: params,
            _pad5: [0.0; 2],
            class_visible,
            class_colors: style.class_colors,
        }
    }
}

// ── Arena planning (pure) ──────────────────────────────────────────────────

/// One live chunk placement inside the arena buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    key: u64,
    /// Content revision this placement holds; a changed generation reuses
    /// the range but rewrites its bytes.
    generation: u64,
    offset: u32,
    len: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct ArenaUpdate {
    /// Live placements after this update.
    slots: Vec<Slot>,
    /// Free ranges remaining after this update.
    free: Vec<(u32, u32)>,
    /// Instance capacity the buffer must have; differing from the current
    /// capacity means the buffer is recreated (and every chunk is written).
    capacity: u32,
    /// `(chunk_index, buffer_offset)` pairs whose bytes must be written.
    writes: Vec<(usize, u32)>,
    /// Contiguous instance ranges covering all live slots, for drawing.
    runs: Vec<(u32, u32)>,
    /// Total live instances.
    total: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ArenaState {
    slots: Vec<Slot>,
    free: Vec<(u32, u32)>,
    capacity: u32,
}

impl ArenaState {
    fn runs(&self) -> Vec<(u32, u32)> {
        runs_from_slots(&self.slots)
    }
}

fn runs_from_slots(slots: &[Slot]) -> Vec<(u32, u32)> {
    let mut sorted: Vec<Slot> = slots.to_vec();
    sorted.sort_by_key(|slot| slot.offset);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for slot in sorted {
        match runs.last_mut() {
            Some(last) if last.0 + last.1 == slot.offset => last.1 += slot.len,
            _ => runs.push((slot.offset, slot.len)),
        }
    }
    runs
}

/// Plans the transition from `state` to `chunks`. Chunks whose key, length
/// and generation all match keep their placement untouched; a generation
/// change reuses the range and rewrites only its bytes; vanished chunks free
/// their ranges; the buffer grows when the request does not fit and compacts
/// when the arena is mostly empty (both recreate with a fresh layout).
fn plan_arena(state: &ArenaState, chunks: &[PointChunk]) -> ArenaUpdate {
    let total: u32 = chunks
        .iter()
        .map(|chunk| chunk.len as u64)
        .sum::<u64>()
        .min(u32::MAX as u64) as u32;

    let recreate = total > state.capacity
        || (state.capacity > MIN_ARENA_CAPACITY && total * 4 < state.capacity);
    if recreate {
        let capacity = total.max(MIN_ARENA_CAPACITY).next_power_of_two();
        let mut slots = Vec::with_capacity(chunks.len());
        let mut writes = Vec::with_capacity(chunks.len());
        let mut offset = 0_u32;
        for (index, chunk) in chunks.iter().enumerate() {
            slots.push(Slot {
                key: chunk.key,
                generation: chunk.generation,
                offset,
                len: chunk.len,
            });
            writes.push((index, offset));
            offset += chunk.len;
        }
        return ArenaUpdate {
            runs: runs_from_slots(&slots),
            slots,
            free: Vec::new(),
            capacity,
            writes,
            total,
        };
    }

    // Incremental path: match by key, reuse placements whose length still
    // fits, allocate from the free list first, then from the bump area past
    // every live slot.
    let mut slots: Vec<Slot> = Vec::with_capacity(chunks.len());
    let mut writes = Vec::new();
    let mut free: Vec<(u32, u32)> = Vec::new();
    let mut bump = 0_u32;
    for slot in &state.slots {
        bump = bump.max(slot.offset + slot.len);
    }
    for slot in &state.slots {
        if !chunks.iter().any(|chunk| chunk.key == slot.key) {
            free.push((slot.offset, slot.len));
        }
    }
    for (index, chunk) in chunks.iter().enumerate() {
        let previous = state.slots.iter().find(|slot| slot.key == chunk.key);
        let mut needs_write = true;
        let slot = match previous {
            // Same content, same length: nothing to do at all.
            Some(slot) if slot.len == chunk.len && slot.generation == chunk.generation => {
                needs_write = false;
                *slot
            }
            // Same length, new content: reuse the range, rewrite its bytes.
            Some(slot) if slot.len == chunk.len => Slot {
                key: chunk.key,
                generation: chunk.generation,
                offset: slot.offset,
                len: chunk.len,
            },
            Some(slot) => {
                free.push((slot.offset, slot.len));
                allocate_slot(&mut free, &mut bump, chunk)
            }
            None => allocate_slot(&mut free, &mut bump, chunk),
        };
        if needs_write {
            writes.push((index, slot.offset));
        }
        slots.push(slot);
    }
    coalesce_free(&mut free);
    let capacity = bump.max(total).max(state.capacity);
    ArenaUpdate {
        runs: runs_from_slots(&slots),
        slots,
        free,
        capacity,
        writes,
        total,
    }
}

/// First-fit allocation: the smallest free range that fits, else the bump
/// area past every live slot.
fn allocate_slot(free: &mut Vec<(u32, u32)>, bump: &mut u32, chunk: &PointChunk) -> Slot {
    let mut best: Option<usize> = None;
    for (index, (offset, len)) in free.iter().enumerate() {
        if *len >= chunk.len
            && best.is_none_or(|current| {
                let (_, current_len) = free[current];
                *len < current_len
            })
        {
            best = Some(index);
        }
    }
    let offset = match best {
        Some(index) => {
            let (offset, len) = free[index];
            let remainder = len - chunk.len;
            if remainder > 0 {
                free[index] = (offset + chunk.len, remainder);
            } else {
                free.remove(index);
            }
            offset
        }
        None => {
            let offset = *bump;
            *bump += chunk.len;
            offset
        }
    };
    Slot {
        key: chunk.key,
        generation: chunk.generation,
        offset,
        len: chunk.len,
    }
}

fn coalesce_free(free: &mut Vec<(u32, u32)>) {
    free.sort_by_key(|(offset, _)| *offset);
    let mut coalesced: Vec<(u32, u32)> = Vec::with_capacity(free.len());
    for (offset, len) in free.drain(..) {
        match coalesced.last_mut() {
            Some(last) if last.0 + last.1 == offset => last.1 += len,
            _ => coalesced.push((offset, len)),
        }
    }
    *free = coalesced;
}

// ── GPU resources ──────────────────────────────────────────────────────────

pub struct PointGpu {
    pipeline: wgpu::RenderPipeline,
    instances: Option<wgpu::Buffer>,
    count: u32,
    style_buffer: wgpu::Buffer,
    style_bind_group: wgpu::BindGroup,
    style_generation: u64,
    geometry_generation: u64,
    source_id: usize,
    #[cfg(feature = "point-arena")]
    arena: ArenaGpu,
}

/// Persistent paged instance buffer plus its live-slot map.
///
/// The logical arena is a single contiguous range of instance slots, but its
/// storage is sharded across several GPU buffers so no one buffer exceeds the
/// device's `max_buffer_size` (wgpu's default is 256 MB; a merged folder
/// working set can need more than that). Shards are indexed by `slot /
/// shard_instances`; a chunk write or draw run is clipped to shard boundaries.
#[cfg(feature = "point-arena")]
struct ArenaGpu {
    buffers: Vec<wgpu::Buffer>,
    /// Instance capacity of each full shard (the last shard may be smaller).
    shard_instances: u32,
    state: ArenaState,
    runs: Vec<(u32, u32)>,
}

#[cfg(feature = "point-arena")]
impl Default for ArenaGpu {
    fn default() -> Self {
        Self {
            buffers: Vec::new(),
            shard_instances: 0,
            state: ArenaState::default(),
            runs: Vec::new(),
        }
    }
}

impl PointGpu {
    pub fn reset(&mut self) {
        self.instances = None;
        self.count = 0;
        self.geometry_generation = u64::MAX;
        self.style_generation = u64::MAX;
        self.source_id = 0;
        #[cfg(feature = "point-arena")]
        {
            self.arena = ArenaGpu::default();
        }
    }

    /// Rebuilds the arena shards for `capacity` instance slots, sized so no
    /// single buffer exceeds the device's `max_buffer_size` (with headroom).
    /// Returns `false` when even one instance would not fit (pathological:
    /// `max_buffer_size` < 64 B), leaving an empty arena so nothing is drawn.
    #[cfg(feature = "point-arena")]
    fn allocate_arena(&mut self, device: &wgpu::Device, capacity: u32) -> bool {
        let max_bytes = device.limits().max_buffer_size;
        let instance_bytes = POINT_INSTANCE_BYTES as u64;
        let max_instances = (max_bytes / instance_bytes) as u32;
        if max_instances == 0 || capacity == 0 {
            self.arena.buffers.clear();
            self.arena.shard_instances = 0;
            return false;
        }
        let shards = capacity.div_ceil(max_instances) as usize;
        self.arena.shard_instances = max_instances;
        self.arena.buffers = (0..shards)
            .map(|shard| {
                let len = ((shard as u32 + 1) * max_instances).min(capacity)
                    - shard as u32 * max_instances;
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("point_cloud.arena"),
                    size: u64::from(len) * POINT_INSTANCE_BYTES as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        true
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
                section_p0: [0.0; 2],
                _pad3: [0.0; 2],
                section_p1: [0.0; 2],
                _pad4: [0.0; 2],
                section_params: [0.0; 2],
                _pad5: [0.0; 2],
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
            #[cfg(feature = "point-arena")]
            arena: ArenaGpu::default(),
        }
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, model: &PointCloudModel) {
        let source_id = std::sync::Arc::as_ptr(&model.points) as usize;
        // Style-only changes (color mode, class visibility, class colors)
        // rewrite the uniform and skip the instance buffer entirely.
        if self.style_generation != model.style_generation || self.geometry_generation == u64::MAX {
            queue.write_buffer(
                &self.style_buffer,
                0,
                bytemuck::bytes_of(&StyleUniforms::new(model)),
            );
            self.style_generation = model.style_generation;
        }
        #[cfg(feature = "point-arena")]
        {
            if !model.chunks.is_empty() {
                self.upload_arena(device, queue, model);
                return;
            }
            // Models without chunk identity fall back to whole-buffer upload.
            self.arena = ArenaGpu::default();
        }
        if self.geometry_generation == model.geometry_generation
            && self.source_id == source_id
            && self.instances.is_some()
        {
            return;
        }
        let point_size = model.point_size_px.clamp(1.0, 32.0);
        let visible = &model.points[..model.points.len().min(u32::MAX as usize)];
        let instances = build_instances(visible, point_size);
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

    /// Pages chunks into the persistent arena: new or generation-changed
    /// chunks get one ranged write each; untouched chunks cost nothing.
    #[cfg(feature = "point-arena")]
    fn upload_arena(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &PointCloudModel,
    ) {
        let plan = plan_arena(&self.arena.state, &model.chunks);
        // Recreate the sharded buffers when the requested capacity changes.
        let recreate = self.arena.buffers.is_empty()
            || plan.capacity != self.arena.state.capacity
            || self.arena.shard_instances == 0;
        if recreate && !self.allocate_arena(device, plan.capacity) {
            // Even one instance cannot fit a buffer; clear and fall through to
            // the whole-buffer upload path in `upload`.
            self.arena = ArenaGpu::default();
            return;
        }
        if self.arena.buffers.is_empty() {
            return;
        }
        let point_size = model.point_size_px.clamp(1.0, 32.0);
        let shard_instances = self.arena.shard_instances;
        for (chunk_index, offset) in &plan.writes {
            let chunk = &model.chunks[*chunk_index];
            let start = chunk.offset as usize;
            let end = start.saturating_add(chunk.len as usize);
            let Some(slice) = model.points.get(start..end) else {
                continue;
            };
            let instances = build_instances(slice, point_size);
            if instances.is_empty() {
                continue;
            }
            // A chunk may straddle a shard boundary; clip the write per shard.
            let mut written = 0_u32;
            for (shard, local_start, local_len) in
                shard_segments(*offset, chunk.len, shard_instances)
            {
                let src_start = written as usize;
                let src_end = src_start + local_len as usize;
                let Some(buffer) = self.arena.buffers.get(shard) else {
                    break;
                };
                queue.write_buffer(
                    buffer,
                    u64::from(local_start) * POINT_INSTANCE_BYTES as u64,
                    bytemuck::cast_slice(&instances[src_start..src_end]),
                );
                written += local_len;
            }
        }
        self.arena.state = ArenaState {
            slots: plan.slots,
            free: plan.free,
            capacity: plan.capacity,
        };
        self.arena.runs = plan.runs;
        self.count = plan.total;
        // The arena path owns drawing while it is active.
        self.instances = None;
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        frame_bind_group: &'a wgpu::BindGroup,
        stencil_reference: u32,
    ) {
        #[cfg(feature = "point-arena")]
        if !self.arena.buffers.is_empty() {
            if self.count == 0 || self.arena.runs.is_empty() {
                return;
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, frame_bind_group, &[]);
            pass.set_bind_group(1, &self.style_bind_group, &[]);
            pass.set_stencil_reference(stencil_reference);
            // Each run lives in the logical slot range, but the storage is
            // sharded: emit one draw per (shard, run) overlap.
            let shard_instances = self.arena.shard_instances;
            for (start, len) in &self.arena.runs {
                for (shard, local_start, local_len) in shard_segments(*start, *len, shard_instances)
                {
                    let Some(buffer) = self.arena.buffers.get(shard) else {
                        break;
                    };
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..6, local_start..local_start + local_len);
                }
            }
            return;
        }
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

/// Splits a logical slot range `[start, start+len)` into `(shard, local_start,
/// local_len)` segments at the `shard_instances` boundaries. `shard_instances`
/// is the instance capacity of each shard; segments never cross a boundary, so
/// a single write/draw maps into exactly one backing buffer.
#[cfg(feature = "point-arena")]
fn shard_segments(start: u32, len: u32, shard_instances: u32) -> Vec<(usize, u32, u32)> {
    if len == 0 || shard_instances == 0 {
        return Vec::new();
    }
    debug_assert!(shard_instances > 0);
    let mut segments = Vec::new();
    let mut done = 0_u32;
    while done < len {
        let slot = start + done;
        let shard = (slot / shard_instances) as usize;
        let shard_base = shard as u32 * shard_instances;
        let local_start = slot - shard_base;
        let local_len = (len - done).min(shard_instances - local_start);
        segments.push((shard, local_start, local_len));
        done += local_len;
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(key: u64, len: u32) -> PointChunk {
        PointChunk {
            key,
            generation: 1,
            offset: 0,
            len,
        }
    }

    fn chunk_at(key: u64, len: u32, offset: u32, generation: u64) -> PointChunk {
        PointChunk {
            key,
            generation,
            offset,
            len,
        }
    }

    #[test]
    fn first_plan_lays_out_chunks_sequentially() {
        let plan = plan_arena(
            &ArenaState::default(),
            &[
                chunk_at(1, 10, 0, 1),
                chunk_at(2, 20, 10, 1),
                chunk_at(3, 5, 30, 1),
            ],
        );
        assert_eq!(MIN_ARENA_CAPACITY, plan.capacity);
        assert_eq!(35, plan.total);
        assert_eq!(vec![(0, 35)], plan.runs);
        assert_eq!(3, plan.writes.len());
        assert_eq!(
            vec![(1, 0), (2, 10), (3, 30)],
            plan.slots
                .iter()
                .map(|slot| (slot.key, slot.offset))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unchanged_chunks_keep_placements_and_skip_writes() {
        let first = plan_arena(
            &ArenaState::default(),
            &[chunk_at(1, 10, 0, 1), chunk_at(2, 20, 10, 1)],
        );
        let state = ArenaState {
            slots: first.slots.clone(),
            free: first.free.clone(),
            capacity: first.capacity,
        };
        // Same keys, lengths and generations: nothing is written.
        let again = plan_arena(&state, &[chunk_at(1, 10, 0, 1), chunk_at(2, 20, 10, 1)]);
        assert!(again.writes.is_empty(), "identical content must not write");
        assert_eq!(first.slots, again.slots);
    }

    #[test]
    fn generation_change_reuses_range_with_one_write() {
        let first = plan_arena(
            &ArenaState::default(),
            &[chunk_at(1, 10, 0, 1), chunk_at(2, 20, 10, 1)],
        );
        let state = ArenaState {
            slots: first.slots.clone(),
            free: first.free.clone(),
            capacity: first.capacity,
        };
        // Chunk 2's content changed (edit/selection); chunk 1 untouched.
        let second = plan_arena(&state, &[chunk_at(1, 10, 0, 1), chunk_at(2, 20, 10, 9)]);
        assert_eq!(vec![(1, 10)], second.writes);
        assert_eq!(first.slots[1].offset, second.slots[1].offset);
    }

    #[test]
    fn departed_chunks_free_ranges_and_new_chunks_reuse_them() {
        let first = plan_arena(
            &ArenaState::default(),
            &[chunk_at(1, 10, 0, 1), chunk_at(2, 20, 10, 1)],
        );
        let state = ArenaState {
            slots: first.slots.clone(),
            free: first.free.clone(),
            capacity: first.capacity,
        };
        // Chunk 2 departs; a smaller chunk 3 arrives and must fit its hole.
        let second = plan_arena(&state, &[chunk_at(1, 10, 0, 1), chunk_at(3, 8, 10, 1)]);
        let slot_three = second.slots.iter().find(|slot| slot.key == 3).unwrap();
        let departed_offset = first
            .slots
            .iter()
            .find(|slot| slot.key == 2)
            .unwrap()
            .offset;
        assert_eq!(departed_offset, slot_three.offset);
        assert_eq!(18, second.total);
        assert_eq!(vec![(0, 18)], second.runs);
    }

    #[test]
    fn oversized_requests_recreate_with_sequential_layout() {
        let state = ArenaState {
            slots: vec![Slot {
                key: 1,
                generation: 1,
                offset: 0,
                len: MIN_ARENA_CAPACITY,
            }],
            free: Vec::new(),
            capacity: MIN_ARENA_CAPACITY,
        };
        let plan = plan_arena(
            &state,
            &[
                chunk_at(1, MIN_ARENA_CAPACITY, 0, 1),
                chunk_at(2, 100, 0, 1),
            ],
        );
        assert!(plan.capacity > MIN_ARENA_CAPACITY);
        // A recreated buffer writes every chunk, laid out back to back.
        assert_eq!(2, plan.writes.len());
        assert_eq!(
            plan.slots[0].offset + plan.slots[0].len,
            plan.slots[1].offset
        );
        assert!(plan.free.is_empty());
    }

    #[test]
    fn mostly_empty_arenas_compact() {
        let big = MIN_ARENA_CAPACITY * 4;
        let state = ArenaState {
            slots: vec![Slot {
                key: 1,
                generation: 1,
                offset: 0,
                len: 100,
            }],
            free: Vec::new(),
            capacity: big,
        };
        let plan = plan_arena(&state, &[chunk_at(1, 100, 0, 1)]);
        assert!(plan.capacity < big, "arena must shrink when mostly empty");
        assert_eq!(1, plan.writes.len());
    }

    #[cfg(feature = "point-arena")]
    #[test]
    fn shard_segments_clip_ranges_at_shard_boundaries() {
        // A range wholly inside one shard stays a single segment.
        assert_eq!(shard_segments(0, 10, 64), vec![(0, 0, 10)]);
        // A range straddling a boundary splits exactly at the edge.
        assert_eq!(shard_segments(60, 8, 64), vec![(0, 60, 4), (1, 0, 4)]);
        // A range spanning many shards clips each one.
        assert_eq!(
            shard_segments(63, 130, 64),
            vec![(0, 63, 1), (1, 0, 64), (2, 0, 64), (3, 0, 1)]
        );
        // Zero length and a degenerate shard size are both safe.
        assert!(shard_segments(0, 0, 64).is_empty());
        assert!(shard_segments(0, 10, 0).is_empty());
    }
}
