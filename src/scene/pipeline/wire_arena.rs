// Persistent per-entity wire instance arena (native, behind OCS_WIRE_GPU_PATCH).
//
// The normal wire path re-emits EVERY wire into a fresh instance buffer whenever
// the resident set's content id changes — so any edit on a drawing whose wires
// expand to millions of segments re-uploads the whole (hundreds-of-MB) buffer,
// which stalls for ~1s on a shared-memory GPU. This arena instead keeps one
// persistent instance buffer (plus its shared WireConst storage) laid out as
// per-entity *slabs*, so an edit only writes what changed:
//
//   * Modify in place — a move / rotate / scale / colour change keeps the
//     entity's segment count, so its slab is overwritten where it sits.
//   * Add / a Modify whose segment count changed — bump-allocate a fresh slab at
//     the tail (tombstone the old one); the instance buffer only grows by that
//     entity.
//   * Erase — tombstone the slab (blank instances that render nothing).
//
// Two correctness points make add/remove safe:
//   * draw_depth_map normalises each entity's draw-order z-bias by the block's
//     entity count, so ANY add/remove re-scales EVERY entity's bias. We keep a
//     CPU mirror of the WireConst array and, on a structural change, refresh every
//     slab's draw_depth and re-upload the (small, ~1 MB) const buffer — the huge
//     instance buffer is untouched.
//   * A tail-appended entity draws last, which only mis-orders alpha-blended /
//     coincident wires. So when the set contains ANY transparent wire we bail to a
//     full rebuild instead of appending. Opaque overlap resolves by the z-bias, so
//     it is order-independent and safe to relocate.
//
// A tombstoned instance points at const slot 0 (a blank WireConst, half_width 0),
// so the shader expands it to a zero-area quad — no pixels. When tombstone waste
// or capacity is exceeded, `patch` returns false and the caller compacts via a
// full rebuild. Because a full rebuild is always the fallback, correctness never
// rides on the fast path.
//
// Scope: a SINGLE batch — the set must have no mesh-edge fills (which force the
// draw-order-preserving multi-batch split) and no per-wire scissor (paper content
// viewports). Mixed 2D/3D or scissored sets fall back to the batched path.

#![cfg(not(target_arch = "wasm32"))]

use super::wire_gpu::{emit_wire_native, wire_draw_depth, WireConst, WireGpu, WireInstance};
use crate::scene::model::wire_model::WireModel;
use crate::scene::ChangeKind;
use acadrust::Handle;
use iced::wgpu;
use rustc_hash::FxHashMap;

/// Spare capacity multiplier when (re)allocating, so a run of adds appends
/// without reallocating each time.
const HEADROOM_NUM: u64 = 3;
const HEADROOM_DEN: u64 = 2;
const MIN_INST_CAP: u64 = 4096;
const MIN_CONST_CAP: u64 = 1024;
/// wgpu caps a single buffer at 256 MB. An arena keeps ONE instance buffer per
/// batch, so a batch whose instances exceed this can't be an arena — build
/// returns None and the caller falls back to the chunked batched path. The
/// buffer (with headroom) is also clamped here so it never exceeds the limit.
const MAX_INSTANCES: u64 = 268_435_456 / std::mem::size_of::<WireInstance>() as u64;
const MAX_CONSTS: u64 = 268_435_456 / std::mem::size_of::<WireConst>() as u64;

struct Slab {
    inst_off: u32,
    inst_len: u32,
    const_off: u32,
    const_len: u32,
    /// World-XY bounds for plan-view draw-range culling. Unbounded whenever a
    /// source wire does not carry a trustworthy entity AABB.
    aabb: [f32; 4],
    /// Entity-level draw depth used when this slab was emitted. Individual
    /// consts may carry block-local offsets around it; structural edits shift
    /// the whole slab by the base-depth delta instead of flattening those
    /// offsets.
    base_depth: f32,
}

pub struct WireArena {
    inst_buf: wgpu::Buffer,
    inst_cap: u32,
    inst_tail: u32,
    const_buf: wgpu::Buffer,
    const_bind_group: std::sync::Arc<wgpu::BindGroup>,
    const_cap: u32,
    const_tail: u32,
    /// CPU mirror of the const buffer so a structural edit can refresh every
    /// slab's draw_depth (denominator change) without re-emitting geometry.
    consts_cpu: Vec<WireConst>,
    slabs: FxHashMap<Handle, Slab>,
    /// Temporarily hidden Modified slabs. Grip drag blanks these but keeps their
    /// offsets so commit/cancel can restore the original submission order.
    vacant: FxHashMap<Handle, Slab>,
    /// Tombstoned instances (blanked, not reclaimed) — past half the tail a patch
    /// bails so the caller compacts with a full rebuild.
    tombstoned: u32,
    /// Whether this arena's wires are 3D mesh/solid edges (`is_3d_mesh_edge` on
    /// the draw batch): the draw loop hides them in clean-shaded modes and draws
    /// them black in filled-with-edges modes. The regular and mesh-edge subsets
    /// of the resident set each get their own arena so both patch incrementally.
    mesh_edge: bool,
    /// Conservative submission-order sensitivity of current arena content.
    order_sensitive: bool,
}

fn handle_of(w: &WireModel) -> Option<Handle> {
    crate::scene::Scene::handle_from_wire_name(&w.name)
}

/// True when `w`'s edge segments belong to a mesh/solid whose fill is drawn in a
/// separate pass — such wires need the draw loop's mesh-edge treatment, so they
/// go in their own arena. `mesh_names` are the entities that emit a fill.
pub fn is_mesh_edge(w: &WireModel, mesh_names: &rustc_hash::FxHashSet<u64>) -> bool {
    !w.points.is_empty() && handle_of(w).map_or(false, |h| mesh_names.contains(&h.value()))
}

/// True when appending a new entity at the tail could change the image, so the
/// arena must fall back to a full rebuild instead of relocating a slab. Two
/// cases where draw order (not the z-bias) decides the winning pixel:
///   * transparency — alpha blends in submission order;
///   * a wire with NO draw-order depth — 3D solids (Solid3D / Region / Body /
///     Surface) are excluded from `draw_depth_map`, so their fallback edge wires
///     get draw_depth 0.0. Two coincident opaque such wires share a z-bias and
///     resolve by submission order, which a tail relocation would flip.
fn order_sensitive(wires: &[&WireModel], depth_map: &FxHashMap<u64, [f32; 2]>) -> bool {
    wires.iter().any(|w| {
        w.color[3] < 0.999
            || handle_of(w).map_or(true, |h| !depth_map.contains_key(&h.value()))
    })
}

/// handle → wire-slot index for the selection / text-highlight overlays, built
/// from the resident Vec (independent of the arena's slab layout).
pub fn build_handle_index(wires: &[WireModel]) -> std::sync::Arc<FxHashMap<u64, Vec<u32>>> {
    let mut index: FxHashMap<u64, Vec<u32>> = FxHashMap::default();
    index.reserve(wires.len());
    for (idx, w) in wires.iter().enumerate() {
        if let Ok(h) = w.name.parse::<u64>() {
            index.entry(h).or_default().push(idx as u32);
        }
    }
    std::sync::Arc::new(index)
}

/// Apply the resident Vec's exact splice operations to the selection/text
/// handle index. Avoids reparsing every wire name after a one-entity patch.
pub(crate) fn patch_handle_index(
    index: &mut std::sync::Arc<FxHashMap<u64, Vec<u32>>>,
    edits: &[crate::scene::WireIndexEdit],
) {
    let index = std::sync::Arc::make_mut(index);
    for edit in edits {
        index.remove(&edit.handle.value());
        let old_end = edit.start + edit.old_len;
        let delta = edit.new_len as isize - edit.old_len as isize;
        if delta != 0 {
            for slots in index.values_mut() {
                for slot in slots {
                    if *slot as usize >= old_end {
                        *slot = (*slot as isize + delta) as u32;
                    }
                }
            }
        }
        if edit.new_len != 0 {
            index.insert(
                edit.handle.value(),
                (edit.start..edit.start + edit.new_len)
                    .map(|slot| slot as u32)
                    .collect(),
            );
        }
    }
}

/// Group `wires` (draw-order sorted, entity-contiguous) into per-handle ranges.
fn handle_ranges(wires: &[&WireModel]) -> Option<Vec<(Handle, usize, usize)>> {
    let mut out: Vec<(Handle, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < wires.len() {
        let h = handle_of(wires[i])?;
        let mut j = i + 1;
        while j < wires.len() && handle_of(wires[j]) == Some(h) {
            j += 1;
        }
        out.push((h, i, j));
        i = j;
    }
    Some(out)
}

fn run_aabb(wires: &[&WireModel]) -> [f32; 4] {
    let mut out = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for wire in wires {
        let [x0, y0, x1, y1] = wire.aabb;
        if !x0.is_finite()
            || !y0.is_finite()
            || !x1.is_finite()
            || !y1.is_finite()
            || x0 > x1
            || y0 > y1
        {
            return WireModel::UNBOUNDED_AABB;
        }
        let pad = (wire.world_width * 0.5).max(0.0);
        out[0] = out[0].min(x0 - pad);
        out[1] = out[1].min(y0 - pad);
        out[2] = out[2].max(x1 + pad);
        out[3] = out[3].max(y1 + pad);
    }
    if out[0].is_finite() {
        out
    } else {
        WireModel::UNBOUNDED_AABB
    }
}

fn make_const_bg(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    buf: &wgpu::Buffer,
) -> std::sync::Arc<wgpu::BindGroup> {
    std::sync::Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("wire_arena.const.bg"),
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buf.as_entire_binding(),
        }],
    }))
}

fn alloc_inst_initialized(
    device: &wgpu::Device,
    cap: u64,
    data: &[WireInstance],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wire_arena.ibuf"),
        size: cap * std::mem::size_of::<WireInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    if !data.is_empty() {
        let bytes = bytemuck::cast_slice(data);
        let mut mapped = buffer
            .slice(..bytes.len() as u64)
            .get_mapped_range_mut();
        mapped.copy_from_slice(bytes);
        drop(mapped);
    }
    buffer.unmap();
    buffer
}

fn alloc_const_initialized(
    device: &wgpu::Device,
    cap: u64,
    data: &[WireConst],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wire_arena.cbuf"),
        size: cap * std::mem::size_of::<WireConst>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    if !data.is_empty() {
        let bytes = bytemuck::cast_slice(data);
        let mut mapped = buffer
            .slice(..bytes.len() as u64)
            .get_mapped_range_mut();
        mapped.copy_from_slice(bytes);
        drop(mapped);
    }
    buffer.unmap();
    buffer
}

fn blank_const() -> WireConst {
    <WireConst as bytemuck::Zeroable>::zeroed()
}

/// A blank instance: zero-length segment at const slot 0 (half_width 0) — the
/// shader expands it to a zero-area quad, so it rasterises nothing.
fn blank_instance() -> WireInstance {
    WireInstance {
        pos_a: [0.0; 3],
        pos_a_low: [0.0; 3],
        pos_b: [0.0; 3],
        pos_b_low: [0.0; 3],
        distance_a: 0.0,
        distance_b: 0.0,
        wire_id: 0,
        taper_ratio: [0; 2],
    }
}

impl WireArena {
    /// Build a fresh arena from the full resident set, or `None` if it isn't a
    /// single scissor-free batch or a wire is unnamed (caller keeps the batched
    /// path).
    pub fn build(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        wires: &[&WireModel],
        depth_map: &FxHashMap<u64, [f32; 2]>,
        const_bgl: &wgpu::BindGroupLayout,
        mesh_edge: bool,
    ) -> Option<Self> {
        let ranges = handle_ranges(wires)?;
        let perf = std::env::var_os("OCS_PERF").is_some();
        let total_started = std::time::Instant::now();

        // Reject an oversized batch before parallel emission allocates hundreds
        // of megabytes. `points.len() - 1` is an upper bound because NaN-break
        // segments are skipped by emit_wire_native.
        let max_instances: usize = wires
            .iter()
            .map(|w| w.points.len().saturating_sub(1))
            .sum();
        if max_instances as u64 > MAX_INSTANCES || wires.len() as u64 + 1 > MAX_CONSTS {
            return None;
        }

        struct BuildPlan {
            handle: Handle,
            start: usize,
            end: usize,
            const_off: u32,
            base_depth: f32,
        }
        struct PackedSlab {
            handle: Handle,
            const_off: u32,
            base_depth: f32,
            aabb: [f32; 4],
            instances: Vec<WireInstance>,
            consts: Vec<WireConst>,
        }

        // Assign global const slots serially so parallel workers can emit final
        // wire_id values directly. Indexed parallel collect preserves handle and
        // wire submission order.
        let mut next_const = 1u32;
        let plans: Vec<BuildPlan> = ranges
            .into_iter()
            .map(|(handle, start, end)| {
                let const_off = next_const;
                next_const += (end - start) as u32;
                let base_depth = if mesh_edge {
                    0.0
                } else {
                    depth_map.get(&handle.value()).map_or(0.0, |d| d[0])
                };
                BuildPlan {
                    handle,
                    start,
                    end,
                    const_off,
                    base_depth,
                }
            })
            .collect();

        let pack_started = std::time::Instant::now();
        use crate::par::prelude::*;
        let packed: Vec<PackedSlab> = plans
            .par_iter()
            .map(|plan| {
                let run = &wires[plan.start..plan.end];
                let capacity: usize = run
                    .iter()
                    .map(|w| w.points.len().saturating_sub(1))
                    .sum();
                let mut instances: Vec<WireInstance> = Vec::with_capacity(capacity);
                let mut consts: Vec<WireConst> = Vec::with_capacity(run.len());
                for (local, &w) in run.iter().enumerate() {
                    let wire_id = plan.const_off + local as u32;
                    // 3D mesh outline edges are occluded by true depth and must NOT
                    // take the draw-order z-bias (or hidden back edges peek through
                    // the shaded fill) — matching WireGpu::from_run.
                    let dd = if mesh_edge { 0.0 } else { wire_draw_depth(w, depth_map) };
                    let (mut emitted, cst) = emit_wire_native(w, wire_id, w.color, dd);
                    instances.append(&mut emitted);
                    consts.push(cst);
                }
                PackedSlab {
                    handle: plan.handle,
                    const_off: plan.const_off,
                    base_depth: plan.base_depth,
                    aabb: run_aabb(run),
                    instances,
                    consts,
                }
            })
            .collect();
        let pack_ms = pack_started.elapsed().as_secs_f64() * 1000.0;

        let inst_count: usize = packed.iter().map(|slab| slab.instances.len()).sum();
        let const_count: usize = 1 + packed.iter().map(|slab| slab.consts.len()).sum::<usize>();
        if inst_count as u64 > MAX_INSTANCES || const_count as u64 > MAX_CONSTS {
            return None;
        }

        // const slot 0 = blank tombstone target.
        let mut instances: Vec<WireInstance> = Vec::with_capacity(inst_count);
        let mut consts_cpu: Vec<WireConst> = Vec::with_capacity(const_count);
        consts_cpu.push(blank_const());
        let mut slabs: FxHashMap<Handle, Slab> =
            FxHashMap::with_capacity_and_hasher(packed.len(), Default::default());
        for mut packed_slab in packed {
            let inst_off = instances.len() as u32;
            let inst_len = packed_slab.instances.len() as u32;
            let const_len = packed_slab.consts.len() as u32;
            instances.append(&mut packed_slab.instances);
            consts_cpu.append(&mut packed_slab.consts);
            slabs.insert(
                packed_slab.handle,
                Slab {
                    inst_off,
                    inst_len,
                    const_off: packed_slab.const_off,
                    const_len,
                    aabb: packed_slab.aabb,
                    base_depth: packed_slab.base_depth,
                },
            );
        }

        let inst_tail = instances.len() as u32;
        let const_tail = consts_cpu.len() as u32;
        // A batch bigger than one buffer can't be an arena — let the caller chunk
        // it via the batched path.
        if inst_tail as u64 > MAX_INSTANCES || const_tail as u64 > MAX_CONSTS {
            return None;
        }
        let inst_cap = ((inst_tail as u64 * HEADROOM_NUM / HEADROOM_DEN)
            .max(MIN_INST_CAP)
            .min(MAX_INSTANCES)) as u32;
        let const_cap = ((const_tail as u64 * HEADROOM_NUM / HEADROOM_DEN)
            .max(MIN_CONST_CAP)
            .min(MAX_CONSTS)) as u32;
        let upload_started = std::time::Instant::now();
        let inst_buf = alloc_inst_initialized(device, inst_cap as u64, &instances);
        let const_buf = alloc_const_initialized(device, const_cap as u64, &consts_cpu);
        let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
        let const_bind_group = make_const_bg(device, const_bgl, &const_buf);
        if perf {
            eprintln!(
                "[perf] arena-build-detail total={:.1}ms pack={:.1} mapped-upload={:.1} handles={} wires={} instances={} instance-bytes={} consts={}",
                total_started.elapsed().as_secs_f64() * 1000.0,
                pack_ms,
                upload_ms,
                slabs.len(),
                wires.len(),
                inst_tail,
                inst_tail as usize * std::mem::size_of::<WireInstance>(),
                const_tail,
            );
        }

        Some(Self {
            inst_buf,
            inst_cap,
            inst_tail,
            const_buf,
            const_bind_group,
            const_cap,
            const_tail,
            consts_cpu,
            slabs,
            vacant: FxHashMap::default(),
            tombstoned: 0,
            mesh_edge,
            order_sensitive: order_sensitive(wires, depth_map),
        })
    }

    fn write_insts(&self, queue: &wgpu::Queue, off: u32, data: &[WireInstance]) {
        if data.is_empty() {
            return;
        }
        let sz = std::mem::size_of::<WireInstance>() as u64;
        queue.write_buffer(&self.inst_buf, off as u64 * sz, bytemuck::cast_slice(data));
    }

    /// Apply the changed handles in place; returns false (⇒ full rebuild) when the
    /// arena can't absorb the change: not eligible, a transparent append, a
    /// capacity overflow, or too much tombstone waste.
    pub fn patch(
        &mut self,
        queue: &wgpu::Queue,
        changes: &[(Handle, ChangeKind)],
        runs: &FxHashMap<Handle, Vec<&WireModel>>,
        new_handles_are_suffix: bool,
        depth_map: &FxHashMap<u64, [f32; 2]>,
    ) -> bool {
        let depth_structural = changes
            .iter()
            .any(|(_, kind)| !matches!(kind, ChangeKind::Modified));
        for &(h, kind) in changes {
            let run = runs.get(&h).map(Vec::as_slice).unwrap_or(&[]);

            // Removed / now-hidden ⇒ tombstone the slab. A handle not in THIS
            // arena's subset (it belongs to the other batch) simply isn't in its
            // slabs, so this is a no-op for it.
            if matches!(kind, ChangeKind::Removed) || run.is_empty() {
                if let Some(slab) = self.slabs.remove(&h) {
                    let blanks = vec![blank_instance(); slab.inst_len as usize];
                    self.write_insts(queue, slab.inst_off, &blanks);
                    self.tombstoned += slab.inst_len;
                    if matches!(kind, ChangeKind::Modified) {
                        self.vacant.insert(h, slab);
                    }
                }
                if matches!(kind, ChangeKind::Removed) {
                    self.vacant.remove(&h);
                }
                continue;
            }

            // Emit into fresh, run-local const slots (patched to absolute below).
            let mut insts: Vec<WireInstance> = Vec::new();
            let mut csts: Vec<WireConst> = Vec::new();
            for &w in run {
                let wire_id = csts.len() as u32;
                let dd = if self.mesh_edge { 0.0 } else { wire_draw_depth(w, depth_map) };
                let (mut wi, c) = emit_wire_native(w, wire_id, w.color, dd);
                insts.append(&mut wi);
                csts.push(c);
            }
            let inst_len = insts.len() as u32;
            let const_len = csts.len() as u32;
            let base_depth = if self.mesh_edge {
                0.0
            } else {
                depth_map.get(&h.value()).map_or(0.0, |d| d[0])
            };
            let aabb = run_aabb(run);

            if !self.slabs.contains_key(&h)
                && self
                    .vacant
                    .get(&h)
                    .is_some_and(|s| s.inst_len == inst_len && s.const_len == const_len)
            {
                let slab = self.vacant.remove(&h).unwrap();
                self.tombstoned = self.tombstoned.saturating_sub(slab.inst_len);
                self.slabs.insert(h, slab);
            }
            let in_place = self
                .slabs
                .get(&h)
                .map(|s| s.inst_len == inst_len && s.const_len == const_len)
                .unwrap_or(false);

            if in_place {
                let (inst_off, const_off) = {
                    let s = self.slabs.get(&h).unwrap();
                    (s.inst_off, s.const_off)
                };
                for w in insts.iter_mut() {
                    w.wire_id += const_off;
                }
                self.write_insts(queue, inst_off, &insts);
                for (k, c) in csts.iter().enumerate() {
                    self.consts_cpu[const_off as usize + k] = *c;
                }
                // Push the entity's consts to the GPU too — an in-place edit is
                // NOT structural, so the whole-buffer refresh below won't run and
                // a colour change would otherwise never reach the shader.
                let csz = std::mem::size_of::<WireConst>() as u64;
                queue.write_buffer(
                    &self.const_buf,
                    const_off as u64 * csz,
                    bytemuck::cast_slice(&csts),
                );
                let slab = self.slabs.get_mut(&h).unwrap();
                slab.base_depth = base_depth;
                slab.aabb = aabb;
                continue;
            }

            // Layout changed ⇒ append at the tail. Unsafe to relocate when the set
            // resolves overlap by submission order: transparency, a wire with no
            // draw-order depth, or the mesh-edge arena (all its wires are forced
            // to depth 0, so coincident edges resolve by submission order). Fall
            // back to a full rebuild instead.
            let is_new = !self.slabs.contains_key(&h);
            let preserves_submission_order = is_new && new_handles_are_suffix;
            let run_order_sensitive = order_sensitive(run, depth_map);
            if (self.order_sensitive || run_order_sensitive || self.mesh_edge)
                && !preserves_submission_order
            {
                return false;
            }
            if self.inst_tail + inst_len > self.inst_cap
                || self.const_tail + const_len > self.const_cap
            {
                return false;
            }
            self.vacant.remove(&h);
            if let Some(s) = self.slabs.remove(&h) {
                let blanks = vec![blank_instance(); s.inst_len as usize];
                self.write_insts(queue, s.inst_off, &blanks);
                self.tombstoned += s.inst_len;
            }
            let inst_off = self.inst_tail;
            let const_off = self.const_tail;
            for w in insts.iter_mut() {
                w.wire_id += const_off;
            }
            self.write_insts(queue, inst_off, &insts);
            for c in &csts {
                self.consts_cpu.push(*c);
            }
            self.inst_tail += inst_len;
            self.const_tail += const_len;
            self.slabs.insert(
                h,
                Slab {
                    inst_off,
                    inst_len,
                    const_off,
                    const_len,
                    aabb,
                    base_depth,
                },
            );
            self.order_sensitive |= run_order_sensitive;
        }

        if depth_structural {
            // The entity count changed ⇒ draw_depth_map re-normalised every
            // entity's z-bias. Refresh each live slab's draw_depth from the new
            // depth map and re-upload the whole (small) const buffer; the instance
            // buffer is untouched.
            // Preserve block-local depth composition. A slab may contain many
            // different child offsets around its entity base; shifting every
            // const by the base delta keeps those offsets intact.
            for (h, slab) in &mut self.slabs {
                // Mesh-edge wires keep depth 0 (no draw-order bias); regular wires
                // take the re-normalised map value.
                let dd = if self.mesh_edge {
                    0.0
                } else {
                    depth_map.get(&h.value()).map_or(0.0, |d| d[0])
                };
                let delta = dd - slab.base_depth;
                for k in 0..slab.const_len {
                    self.consts_cpu[(slab.const_off + k) as usize].draw_depth += delta;
                }
                slab.base_depth = dd;
            }
            queue.write_buffer(&self.const_buf, 0, bytemuck::cast_slice(&self.consts_cpu));
        }
        if self.tombstoned > self.inst_tail / 2 {
            return false;
        }
        true
    }

    /// One draw batch wrapping the persistent instance buffer. `instance_count`
    /// is the whole tail (tombstones included — they draw nothing).
    pub fn wire_gpus(&self) -> Vec<WireGpu> {
        if self.inst_tail == 0 {
            return vec![];
        }
        vec![WireGpu {
            instance_buffer: self.inst_buf.clone(),
            first_instance: 0,
            instance_count: self.inst_tail,
            is_3d_mesh_edge: self.mesh_edge,
            const_bind_group: Some(self.const_bind_group.clone()),
        }]
    }

    /// Draw only plan-view entity ranges that can reach the viewport. The
    /// instance buffer stays fully resident, so pan/zoom changes only this tiny
    /// list of offsets — no repack and no GPU upload. Non-plan views use the
    /// conservative full draw because a 2-D entity AABB does not contain Z.
    pub fn wire_gpus_visible(
        &self,
        view_rot: glam::Mat4,
        eye: glam::DVec3,
        clip_w: u32,
        clip_h: u32,
    ) -> Vec<WireGpu> {
        if self.inst_tail == 0 {
            return vec![];
        }
        let projected_x = view_rot.transform_vector3(glam::Vec3::X);
        let projected_y = view_rot.transform_vector3(glam::Vec3::Y);
        let projected_z = view_rot.transform_vector3(glam::Vec3::Z);
        let xy_scale = projected_x
            .truncate()
            .length()
            .max(projected_y.truncate().length())
            .max(f32::MIN_POSITIVE);
        if projected_z.truncate().length() > xy_scale * 1e-5 {
            return self.wire_gpus();
        }

        let mut ranges: Vec<(u32, u32)> = self
            .slabs
            .values()
            .filter(|slab| {
                slab.inst_len > 0
                    && !super::aabb_offscreen(slab.aabb, view_rot, eye, clip_w, clip_h)
            })
            .map(|slab| (slab.inst_off, slab.inst_off + slab.inst_len))
            .collect();
        ranges.sort_unstable_by_key(|range| range.0);
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            if let Some((_, previous_end)) = merged.last_mut() {
                if *previous_end == start {
                    *previous_end = end;
                    continue;
                }
            }
            merged.push((start, end));
        }
        let mut ranges = merged;
        if ranges.is_empty() {
            return vec![];
        }

        // Cap CPU draw-call overhead on pathologically interleaved draw order.
        // Merging a few separated visible spans draws their offscreen gap too,
        // but retains order and still avoids the rest of a multi-million
        // instance drawing.
        const MAX_RANGES: usize = 64;
        if ranges.len() > MAX_RANGES {
            let group = (ranges.len() + MAX_RANGES - 1) / MAX_RANGES;
            ranges = ranges
                .chunks(group)
                .map(|chunk| (chunk[0].0, chunk[chunk.len() - 1].1))
                .collect();
        }

        if std::env::var_os("OCS_PERF").is_some() {
            let submitted: u64 = ranges
                .iter()
                .map(|(start, end)| (end - start) as u64)
                .sum();
            if submitted < self.inst_tail as u64 {
                eprintln!(
                    "[perf] wire-cull submitted={} resident={} ranges={}",
                    submitted,
                    self.inst_tail,
                    ranges.len(),
                );
            }
        }

        ranges
            .into_iter()
            .map(|(start, end)| WireGpu {
                instance_buffer: self.inst_buf.clone(),
                first_instance: start,
                instance_count: end - start,
                is_3d_mesh_edge: self.mesh_edge,
                const_bind_group: Some(self.const_bind_group.clone()),
            })
            .collect()
    }
}
