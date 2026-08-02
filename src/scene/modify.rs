// Auto-split from scene/mod.rs. Pure text-move; behaviour unchanged.
use super::*;

/// Orientation fields of a text-like entity, captured so a mirror can keep the
/// glyphs right-reading when MIRRTEXT (`header.mirror_text`) is off.
struct TextOrient {
    rotation: f64,
    oblique: f64,
    x_scale: f64,
}

fn inverse_affine(
    transform: &acadrust::types::Transform,
) -> Option<acadrust::types::Transform> {
    use acadrust::types::{Matrix3, Matrix4, Transform, Vector3};
    let matrix = &transform.matrix.m;
    let linear = Matrix3::from_rows(
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    );
    let inverse = linear.inverse()?;
    let translation = Vector3::new(matrix[0][3], matrix[1][3], matrix[2][3]);
    let inverse_translation = inverse * (translation * -1.0);
    Some(Transform::from_matrix(Matrix4 {
        m: [
            [
                inverse.m[0][0],
                inverse.m[0][1],
                inverse.m[0][2],
                inverse_translation.x,
            ],
            [
                inverse.m[1][0],
                inverse.m[1][1],
                inverse.m[1][2],
                inverse_translation.y,
            ],
            [
                inverse.m[2][0],
                inverse.m[2][1],
                inverse.m[2][2],
                inverse_translation.z,
            ],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }))
}

/// Snapshot the orientation of a text / mtext / shape entity, or `None` for any
/// other kind.
fn capture_text_orient(e: &EntityType) -> Option<TextOrient> {
    match e {
        EntityType::Text(t) => Some(TextOrient {
            rotation: t.rotation,
            oblique: t.oblique_angle,
            x_scale: 0.0,
        }),
        EntityType::MText(m) => Some(TextOrient {
            rotation: m.rotation,
            oblique: 0.0,
            x_scale: 0.0,
        }),
        EntityType::Shape(s) => Some(TextOrient {
            rotation: s.rotation,
            oblique: s.oblique_angle,
            x_scale: s.relative_x_scale,
        }),
        _ => None,
    }
}

/// MIRRTEXT off: after the mirror reflects a text's points and rotation, put
/// the glyphs back to right-reading. Restores the captured rotation / oblique /
/// x-scale, and for single-line TEXT also flips the horizontal justification
/// (left ↔ right) so the box lands mirror-symmetric to the source instead of
/// hugging the axis — AutoCAD's behaviour. Center/Middle/Aligned/Fit are already
/// symmetric about their (reflected) anchor, so they stay.
fn restore_text_orient(e: &mut EntityType, o: &TextOrient) {
    match e {
        EntityType::Text(t) => {
            t.rotation = o.rotation;
            t.oblique_angle = o.oblique;
            use acadrust::entities::TextHorizontalAlignment as HA;
            t.horizontal_alignment = match t.horizontal_alignment {
                HA::Left => HA::Right,
                HA::Right => HA::Left,
                other => other,
            };
            crate::entities::text::sync_text_alignment_point(t);
        }
        EntityType::MText(m) => {
            m.rotation = o.rotation;
        }
        EntityType::Shape(s) => {
            s.rotation = o.rotation;
            s.oblique_angle = o.oblique;
            s.relative_x_scale = o.x_scale;
        }
        _ => {}
    }
}

/// MIRRTEXT on: a real MIRROR makes text a true glyph mirror. `apply_transform`
/// already reflects the baseline rotation; toggling *both* group-71 bits
/// (backward `0x2` + upside-down `0x4`) then renders the exact geometric
/// reflection (the renderer XORs these flags into the effective width-factor
/// sign / 180° flip). Toggling — not setting — keeps a double mirror an
/// involution. Only single-line TEXT carries group 71; MTEXT / Shape keep the
/// reflected rotation (readable-but-rotated), which is standard-conforming.
fn mirror_true_text_flags(e: &mut EntityType) {
    if let EntityType::Text(t) = e {
        t.generation_flags ^= 0x2 | 0x4;
    }
}

impl Scene {
    pub(crate) fn sync_displayed_annotation_context(&mut self, handle: Handle) -> bool {
        let scale = self.displayed_annotation_scale_handle();
        crate::scene::annotative::sync_annotation_context_from_entity(
            &mut self.document,
            handle,
            scale,
        )
    }

    /// Invalidate a dimension's baked block while capturing every removed
    /// sub-entity for an active history transaction.
    pub fn invalidate_dim_block_recorded(&mut self, handle: Handle) {
        if self.is_recording_undo() {
            let owned: Vec<Handle> = self
                .document
                .get_entity(handle)
                .and_then(|entity| match entity {
                    EntityType::Dimension(d) => {
                        let name = d.base().block_name.clone();
                        self.document.block_records.get(&name).map(|record| {
                            let mut handles = record.entity_handles.clone();
                            handles.push(record.block_entity_handle);
                            handles.push(record.block_end_handle);
                            handles
                        })
                    }
                    _ => None,
                })
                .unwrap_or_default();
            if let Some(before) = self.document.get_entity_arc(handle) {
                self.record_undo_before(handle, Some(before));
            }
            for owned_handle in owned {
                if let Some(before) = self.document.get_entity_arc(owned_handle) {
                    self.record_undo_before(owned_handle, Some(before));
                }
            }
            self.poison_undo_recording();
        }
        crate::modules::draw::modify::explode::invalidate_dim_block(&mut self.document, handle);
    }

    // ── Modify (transform / copy) ─────────────────────────────────────────

    pub fn transform_entities(&mut self, handles: &[Handle], t: &EntityTransform) {
        // Never transform objects on a locked layer (defense-in-depth: the pick
        // path already excludes them, but programmatic callers may not).
        let handles: Vec<Handle> = handles
            .iter()
            .copied()
            .filter(|&h| !self.is_layer_locked(h))
            .collect();
        let handles = &handles[..];
        // MIRRTEXT (header.mirror_text): when false AutoCAD positions text /
        // mtext / shape by the mirror but keeps the original rotation +
        // oblique so the text stays right-reading. Capture before the
        // transform and re-apply afterwards.
        let preserve_text_orientation =
            matches!(t, EntityTransform::Mirror { .. }) && !self.document.header.mirror_text;
        // MIRRTEXT on: toggle the group-71 flags after the reflect so text
        // becomes a true glyph mirror (see `mirror_true_text_flags`).
        let mirror_true =
            matches!(t, EntityTransform::Mirror { .. }) && self.document.header.mirror_text;
        let mut text_orient_backup: Vec<(Handle, TextOrient)> = Vec::new();
        if preserve_text_orientation {
            for &h in handles {
                if let Some(entity) = self.document.get_entity(h) {
                    if let Some(o) = capture_text_orient(entity) {
                        text_orient_backup.push((h, o));
                    }
                }
            }
        }
        // A dimension's final geometry is baked into a per-instance `*D`
        // block, and the render draws those sub-entities directly (not the
        // definition points). Transform them with the dimension, or it would
        // stay drawn in place while only its def points move.
        let dim_block_subs: Vec<Handle> = handles
            .iter()
            .filter_map(|&h| match self.document.get_entity(h) {
                Some(EntityType::Dimension(d)) => {
                    let bn = d.base().block_name.clone();
                    if bn.trim().is_empty() {
                        None
                    } else {
                        Some(bn)
                    }
                }
                _ => None,
            })
            .filter_map(|bn| {
                self.document
                    .block_records
                    .iter()
                    .find(|br| br.name.eq_ignore_ascii_case(&bn))
                    .map(|br| br.entity_handles.clone())
            })
            .flatten()
            .collect();
        for &h in handles {
            // Delta-undo: capture the pre-transform image before mutating.
            if self.is_recording_undo() {
                let before = self.document.get_entity_arc(h);
                self.record_undo_before(h, before);
            }
            if let Some(entity) = self.document.get_entity_mut(h) {
                view::dispatch::apply_transform(entity, t);
                if mirror_true {
                    mirror_true_text_flags(entity);
                }
            }
            if self.hatches.contains_key(&h) {
                let existing_color = self.hatches[&h].color;
                let new_model = match self.document.get_entity(h) {
                    Some(EntityType::Hatch(dxf)) => Self::hatch_model_from_dxf(dxf, existing_color),
                    // A DXF SOLID renders as a solid-fill hatch; rebuild it from
                    // the moved corners so the fill follows the transform.
                    Some(EntityType::Solid(s)) => Some(Self::solid_hatch_model(s, existing_color)),
                    _ => None,
                };
                if let Some(model) = new_model {
                    self.hatches.insert(h, model);
                }
            }
        }
        if preserve_text_orientation {
            for (h, o) in text_orient_backup {
                if let Some(entity) = self.document.get_entity_mut(h) {
                    restore_text_orient(entity, &o);
                }
            }
        }
        for &h in handles {
            if self.sync_displayed_annotation_context(h) {
                self.poison_undo_recording();
            }
        }
        // Move the baked dimension-block sub-entities too (collected above).
        // These are entities not named in the reported change set, so a delta
        // must capture their before-images here or a dimension move won't undo.
        for h in &dim_block_subs {
            if self.is_recording_undo() {
                let before = self.document.get_entity_arc(*h);
                self.record_undo_before(*h, before);
            }
            if let Some(entity) = self.document.get_entity_mut(*h) {
                view::dispatch::apply_transform(entity, t);
            }
        }
        // Only the transformed entities changed (a top-level move/rotate/scale/
        // mirror never edits a block definition) — report just those so the
        // resident set re-tessellates only them and every derived cache patches
        // per-handle, keeping the block cache + all other memoized wires.
        let changes: Vec<(Handle, ChangeKind)> =
            handles.iter().map(|&h| (h, ChangeKind::Modified)).collect();
        self.bump_entities(&changes);
    }

    /// Entities in anonymous dimension blocks that visually belong to
    /// dimensions inside `block_record`. They must follow a block-coordinate
    /// reframe and be included in BEDIT's discard snapshot.
    pub(crate) fn block_definition_dependent_handles(
        &self,
        block_record: Handle,
    ) -> Vec<Handle> {
        let Some(record) = self
            .document
            .block_records
            .iter()
            .find(|record| record.handle == block_record)
        else {
            return Vec::new();
        };
        let root_handles: HashSet<Handle> = record.entity_handles.iter().copied().collect();
        let mut result = Vec::new();
        let mut seen = HashSet::default();
        for handle in &record.entity_handles {
            let Some(EntityType::Dimension(dimension)) = self.document.get_entity(*handle) else {
                continue;
            };
            let name = dimension.base().block_name.trim();
            if name.is_empty() {
                continue;
            }
            let Some(dependent) = self
                .document
                .block_records
                .iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            for dependent_handle in &dependent.entity_handles {
                if !root_handles.contains(dependent_handle) && seen.insert(*dependent_handle) {
                    result.push(*dependent_handle);
                }
            }
        }
        result
    }

    /// Bake a transient BEDIT UCS into one block definition.
    ///
    /// `local_from_old` maps the currently stored block coordinates into the
    /// newly selected UCS frame. No UCS record is attached to the block: the
    /// content is transformed and remains canonical around identity/zero.
    pub fn reframe_block_definition(
        &mut self,
        block_record: Handle,
        local_from_old: &acadrust::types::Transform,
    ) -> usize {
        let Some(record) = self
            .document
            .block_records
            .iter()
            .find(|record| record.handle == block_record)
        else {
            return 0;
        };
        let block_name = record.name.clone();
        let root_handles: HashSet<Handle> = record.entity_handles.iter().copied().collect();
        let dependent_handles = self.block_definition_dependent_handles(block_record);
        let mut handles = record.entity_handles.clone();
        handles.extend(dependent_handles.iter().copied());
        let owned: HashSet<Handle> = handles.iter().copied().collect();
        let transform = EntityTransform::Affine(*local_from_old);
        let matrix = &local_from_old.matrix.m;
        let dependent_transform =
            EntityTransform::Affine(acadrust::types::Transform::from_matrix(
                acadrust::types::Matrix4 {
                    m: [
                        [matrix[0][0], matrix[0][1], matrix[0][2], 0.0],
                        [matrix[1][0], matrix[1][1], matrix[1][2], 0.0],
                        [matrix[2][0], matrix[2][1], matrix[2][2], 0.0],
                        [0.0, 0.0, 0.0, 1.0],
                    ],
                },
            ));
        let mut changed = Vec::new();

        for handle in handles {
            let is_structural = self.document.get_entity(handle).is_some_and(|entity| {
                matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_))
            });
            if is_structural {
                continue;
            }
            if self.is_recording_undo() {
                let before = self.document.get_entity_arc(handle);
                self.record_undo_before(handle, before);
            }
            if let Some(entity) = self.document.get_entity_mut(handle) {
                // A nested dimension's anonymous `*D` picture is placed by
                // adding the dimension insertion point. That point already
                // receives the affine translation with the root dimension, so
                // its picture receives only the new basis here; applying the
                // translation twice would shift dimensions on origin changes.
                let entity_transform = if root_handles.contains(&handle) {
                    &transform
                } else {
                    &dependent_transform
                };
                view::dispatch::apply_transform(entity, entity_transform);
                changed.push(handle);
            }
        }

        // ATTRIBs live inline on each INSERT in that reference's owner space.
        // Conjugate the block-local reframe through the INSERT transform:
        // owner' = M · local_from_old · M⁻¹ · owner.
        let references: Vec<(Handle, acadrust::types::Transform)> = self
            .document
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Insert(reference)
                    if reference.block_name.eq_ignore_ascii_case(&block_name)
                        && !reference.attributes.is_empty()
                        && !owned.contains(&reference.common.handle) =>
                {
                    let insertion = reference.get_transform();
                    let inverse = inverse_affine(&insertion)?;
                    Some((
                        reference.common.handle,
                        acadrust::types::Transform::from_matrix(
                            insertion.matrix * local_from_old.matrix * inverse.matrix,
                        ),
                    ))
                }
                _ => None,
            })
            .collect();
        for (handle, attribute_transform) in references {
            if self.is_recording_undo() {
                let before = self.document.get_entity_arc(handle);
                self.record_undo_before(handle, before);
            }
            if let Some(EntityType::Insert(reference)) = self.document.get_entity_mut(handle) {
                for attribute in &mut reference.attributes {
                    acadrust::Entity::apply_transform(attribute, &attribute_transform);
                }
                changed.push(handle);
            }
        }

        for handle in changed.iter().copied() {
            let _ = self.sync_displayed_annotation_context(handle);
        }
        if !changed.is_empty() {
            self.rebuild_derived_caches();
        }
        changed.len()
    }

    /// Give a freshly-cloned entity brand-new handles for every *inline*
    /// sub-entity that stores one (INSERT attributes, 3D-polyline vertices).
    /// `document.add_entity` only assigns the top-level handle, so without this
    /// a copy keeps its source's sub-handles — duplicate handles that corrupt
    /// the saved DWG (file won't reopen in other CAD apps). Vertices that don't
    /// store a handle (LwPolyline / heavy 2D polyline) get one from the writer,
    /// so they need no fix-up here. (#129)
    pub(super) fn reset_clone_subhandles(doc: &mut acadrust::CadDocument, entity: &mut EntityType) {
        match entity {
            EntityType::Insert(ins) => {
                for att in ins.attributes.iter_mut() {
                    att.common.handle = doc.allocate_handle();
                }
            }
            EntityType::Polyline3D(p) => {
                for v in p.vertices.iter_mut() {
                    v.handle = doc.allocate_handle();
                }
            }
            _ => {}
        }
    }

    /// Add a freshly-cloned entity, allocating a new handle for it *and* every
    /// inline sub-entity so the copy never shares a handle with its source.
    /// Use this (not `add_entity`) whenever inserting a duplicate. (#129)
    pub fn add_entity_clone(&mut self, mut entity: EntityType) -> Handle {
        Self::reset_clone_subhandles(&mut self.document, &mut entity);
        entity.common_mut().handle = Handle::NULL;
        self.add_entity(entity)
    }

    /// Duplicate the anonymous block `src_name`, transforming every sub-entity
    /// by `t`, and return the new block's name. A dimension's drawn geometry
    /// lives in such a baked `*D` block, so a copied dimension needs its own
    /// transformed block — otherwise it still references the source block and
    /// renders on top of the original instead of at the copy. Returns None when
    /// the source block is missing or empty. (#161)
    fn clone_transformed_block(&mut self, src_name: &str, t: &EntityTransform) -> Option<String> {
        let sub_handles = self
            .document
            .block_records
            .iter()
            .find(|br| br.name.eq_ignore_ascii_case(src_name))
            .map(|br| br.entity_handles.clone())?;
        let subs: Vec<EntityType> = sub_handles
            .iter()
            .filter_map(|&sh| self.document.get_entity(sh).cloned())
            .collect();
        self.define_transformed_block(&subs, t)
    }

    /// Build a fresh anonymous `*D<n>` block from `subs` — a source block's
    /// sub-entities in its own (WCS-baked) coordinates — transforming each by
    /// `t`, and return the new block's name. Shared by the in-drawing copy
    /// (source block still lives in this document, via `clone_transformed_block`)
    /// and clipboard paste (source block snapshotted into the clipboard, so a
    /// pasted dimension gets its own transformed block cross-drawing too — see
    /// `finalize_paste`, #290). Returns None when `subs` is empty.
    pub(crate) fn define_transformed_block(
        &mut self,
        subs: &[EntityType],
        t: &EntityTransform,
    ) -> Option<String> {
        if subs.is_empty() {
            return None;
        }
        // Smallest free `*D<n>` anonymous name.
        let mut n = 0u64;
        let new_name = loop {
            let cand = format!("*D{n}");
            if self.document.block_records.get(&cand).is_none() {
                break cand;
            }
            n += 1;
        };
        let next = self.document.next_handle();
        let br_handle = Handle::new(next);
        let block_handle = Handle::new(next + 1);
        let end_handle = Handle::new(next + 2);
        let mut br = acadrust::tables::BlockRecord::new(&new_name);
        br.handle = br_handle;
        br.block_entity_handle = block_handle;
        br.block_end_handle = end_handle;
        self.document.block_records.add(br).ok()?;
        let mut block = Block::new(&new_name, acadrust::types::Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = br_handle;
        self.document.add_entity(EntityType::Block(block)).ok()?;
        let mut block_end = BlockEnd::new();
        block_end.common.handle = end_handle;
        block_end.common.owner_handle = br_handle;
        self.document
            .add_entity(EntityType::BlockEnd(block_end))
            .ok()?;
        for sub in subs {
            let mut sub = sub.clone();
            view::dispatch::apply_transform(&mut sub, t);
            Self::reset_clone_subhandles(&mut self.document, &mut sub);
            sub.common_mut().handle = Handle::NULL;
            sub.common_mut().owner_handle = br_handle;
            let _ = self.document.add_entity(sub);
        }
        Some(new_name)
    }

    pub fn copy_entities(&mut self, handles: &[Handle], t: &EntityTransform) -> Vec<Handle> {
        // Objects on a locked layer can't be copied (they can't be selected).
        let clones: Vec<(Handle, EntityType)> = handles
            .iter()
            .filter(|&&h| !self.is_layer_locked(h))
            .filter_map(|&h| self.document.get_entity(h).cloned().map(|e| (h, e)))
            .collect();
        // MIRRTEXT also governs the copy path (default MIRROR keeps the source
        // and adds a mirrored copy): keep the copied text right-reading when the
        // header flag is off.
        let preserve_text_orientation =
            matches!(t, EntityTransform::Mirror { .. }) && !self.document.header.mirror_text;
        let mirror_true =
            matches!(t, EntityTransform::Mirror { .. }) && self.document.header.mirror_text;
        let mut new_handles = Vec::with_capacity(clones.len());
        let mut handle_map = rustc_hash::FxHashMap::default();
        for (src_handle, mut entity) in clones {
            let text_orient = if preserve_text_orientation {
                capture_text_orient(&entity)
            } else {
                None
            };
            view::dispatch::apply_transform(&mut entity, t);
            if let Some(o) = &text_orient {
                restore_text_orient(&mut entity, o);
            }
            if mirror_true {
                mirror_true_text_flags(&mut entity);
            }
            // A dimension draws from its baked `*D` block; give the copy its own
            // transformed block so it lands at the copy position rather than
            // rendering on top of the source. (#161)
            if let EntityType::Dimension(d) = &entity {
                let bn = d.base().block_name.clone();
                if !bn.trim().is_empty() {
                    if let Some(new_bn) = self.clone_transformed_block(&bn, t) {
                        // Delta-undo: a copied dimension adds a fresh anonymous
                        // *D block record — non-entity state a pure-entity delta
                        // can't restore. Poison so the app keeps a full snapshot.
                        self.poison_undo_recording();
                        if let EntityType::Dimension(d) = &mut entity {
                            d.base_mut().block_name = new_bn;
                        }
                    }
                }
            }
            Self::reset_clone_subhandles(&mut self.document, &mut entity);
            entity.common_mut().handle = Handle::NULL;
            let h = self.document.add_entity(entity).unwrap_or(Handle::NULL);
            if !h.is_null() {
                // Delta-undo: a copy's before-image is "nothing" (undo erases it).
                if self.is_recording_undo() {
                    self.record_undo_before(h, None);
                }
                let new_model = match self.document.get_entity(h) {
                    Some(EntityType::Hatch(dxf)) => {
                        let color = convert::tess_util::aci_to_rgba(&dxf.common.color);
                        Self::hatch_model_from_dxf(dxf, color)
                    }
                    Some(EntityType::Solid(s)) => {
                        let color = convert::tess_util::aci_to_rgba(&s.common.color);
                        Some(Self::solid_hatch_model(s, color))
                    }
                    _ => None,
                };
                if let Some(model) = new_model {
                    self.hatches.insert(h, model);
                }
            }
            new_handles.push(h);
            if !h.is_null() {
                handle_map.insert(src_handle, h);
            }
        }

        // Complete group copies record their new Group objects and dictionary
        // entry as targeted object deltas inside copy_complete_groups.
        self.copy_complete_groups(&handle_map);
        // The copies are new handles (natural memo misses, tessellated fresh)
        // and reference only already-cached blocks — no block defn changes.
        // Report them as additions so derived caches patch in exactly the copies.
        let changes: Vec<(Handle, ChangeKind)> = new_handles
            .iter()
            .map(|&h| (h, ChangeKind::Added))
            .collect();
        self.bump_entities(&changes);
        new_handles
    }

    // ── Grip editing ──────────────────────────────────────────────────────

    pub fn apply_grip(&mut self, handle: Handle, grip_id: usize, apply: GripApply) {
        // Objects on a locked layer can't be grip-edited.
        if self.is_layer_locked(handle) {
            return;
        }
        // For Solid3D / Region / Body, record the old point_of_reference so we
        // can translate the pre-tessellated MeshModel by the same delta after
        // the grip is applied (the ACIS data itself is not modified).
        let old_por: Option<[f64; 3]> = self
            .document
            .get_entity(handle)
            .and_then(crate::entities::solid3d::point_of_reference)
            .map(|p| [p.x, p.y, p.z]);

        if let Some(entity) = self.document.get_entity_mut(handle) {
            view::dispatch::apply_grip(entity, grip_id, apply);
        }
        if self.sync_displayed_annotation_context(handle) {
            self.poison_undo_recording();
        }
        // A dimension loaded from a file renders through its baked *D block;
        // the grip only moved a definition point, so the baked graphics are
        // stale — drop them so tessellation falls back to the live geometry
        // and the next save re-bakes (no-op for non-dimensions). (#398)
        self.invalidate_dim_block_recorded(handle);

        // Translate MeshModel vertices by the same delta the grip applied.
        if let Some(old) = old_por {
            let new_por: Option<[f64; 3]> = self
                .document
                .get_entity(handle)
                .and_then(crate::entities::solid3d::point_of_reference)
                .map(|p| [p.x, p.y, p.z]);
            if let Some(new) = new_por {
                let delta = [new[0] - old[0], new[1] - old[1], new[2] - old[2]];
                if let Some(set) = self.meshes.get_mut(&handle) {
                    let translate_split =
                        |high: &mut [f32; 3], low: &mut [f32; 3]| {
                            let absolute = [
                                high[0] as f64 + low[0] as f64 + delta[0],
                                high[1] as f64 + low[1] as f64 + delta[1],
                                high[2] as f64 + low[2] as f64 + delta[2],
                            ];
                            *high = [
                                absolute[0] as f32,
                                absolute[1] as f32,
                                absolute[2] as f32,
                            ];
                            *low = [
                                (absolute[0] - high[0] as f64) as f32,
                                (absolute[1] - high[1] as f64) as f32,
                                (absolute[2] - high[2] as f64) as f32,
                            ];
                        };
                    for lod in &mut set.lods {
                        if lod.verts_low.len() != lod.verts.len() {
                            lod.verts_low = vec![[0.0; 3]; lod.verts.len()];
                        }
                        for (high, low) in lod.verts.iter_mut().zip(lod.verts_low.iter_mut()) {
                            translate_split(high, low);
                        }
                    }
                    if set.edge_verts_low.len() != set.edge_verts.len() {
                        set.edge_verts_low = vec![[0.0; 3]; set.edge_verts.len()];
                    }
                    for (high, low) in set
                        .edge_verts
                        .iter_mut()
                        .zip(set.edge_verts_low.iter_mut())
                    {
                        translate_split(high, low);
                    }
                    for generator in &mut set.curved_gens {
                        match generator {
                            crate::scene::model::mesh_model::CurvedGen::Cone {
                                base,
                                base_low,
                                ..
                            } => translate_split(base, base_low),
                            crate::scene::model::mesh_model::CurvedGen::Sphere {
                                center,
                                center_low,
                                ..
                            }
                            | crate::scene::model::mesh_model::CurvedGen::Torus {
                                center,
                                center_low,
                                ..
                            } => translate_split(center, center_low),
                        }
                    }
                    for silhouette in &mut set.stored_silhouettes {
                        silhouette.target[0] += delta[0] as f32;
                        silhouette.target[1] += delta[1] as f32;
                        silhouette.target[2] += delta[2] as f32;
                        if silhouette.edge_verts_low.len() != silhouette.edge_verts.len() {
                            silhouette.edge_verts_low =
                                vec![[0.0; 3]; silhouette.edge_verts.len()];
                        }
                        for (high, low) in silhouette
                            .edge_verts
                            .iter_mut()
                            .zip(silhouette.edge_verts_low.iter_mut())
                        {
                            translate_split(high, low);
                        }
                    }
                    set.metrics.centroid[0] += delta[0];
                    set.metrics.centroid[1] += delta[1];
                    set.metrics.centroid[2] += delta[2];
                    set.recompute_aabb();
                }
            }
        }

        // Rebuild GPU hatch/solid model when a boundary vertex or corner moves.
        match self.document.get_entity(handle) {
            Some(EntityType::Hatch(dxf)) => {
                let color = convert::tess_util::aci_to_rgba(&dxf.common.color);
                if let Some(model) = Self::hatch_model_from_dxf(dxf, color) {
                    self.hatches.insert(handle, model);
                } else {
                    self.hatches.remove(&handle);
                }
            }
            Some(EntityType::Solid(solid)) => {
                let color = convert::tess_util::aci_to_rgba(&solid.common.color);
                self.hatches
                    .insert(handle, Self::solid_hatch_model(solid, color));
            }
            _ => {}
        }
        // NOTE: no `bump_geometry()` here. The grip-drag caller hides the
        // edited entity and previews it as an overlay during the drag (so a
        // move doesn't re-tessellate the whole model), then bumps once on
        // commit. Any other caller must bump geometry itself.
    }
}
