// Auto-split from scene/mod.rs. Pure text-move; behaviour unchanged.
use super::*;

impl Scene {
    // ── Preview wire ──────────────────────────────────────────────────────

    pub fn set_preview_wires(&mut self, wires: Vec<WireModel>) {
        // Preview wires are an overlay appended to the cached base wire set in
        // `build_primitive`; they are NOT part of the tessellation cache. So a
        // preview update must NOT bump `geometry_epoch` — that would re-
        // tessellate the whole model on every rubber-band frame. The overlay
        // forces a GPU wire re-upload on its own (the `has_overlay` content-id
        // path), and iced redraws after the message that set the preview.
        self.preview_wires = wires;
    }

    /// Publish all edited hatches as one live fill overlay.
    pub fn set_preview_hatches(&mut self, handles: &[Handle]) {
        let mut models = Vec::new();
        for &handle in handles {
            // Keep the existing direct Hatch/Solid preview path. INSERT has no
            // entry in `self.hatches`, so this is a no-op for it and the full
            // block expansion below supplies its fills.
            self.append_preview_hatch(handle, &mut models);
        }
        models.extend(self.preview_insert_hatch_models(handles));
        self.preview_hatches = std::sync::Arc::new(models);
    }

    fn append_preview_hatch(&self, handle: Handle, models: &mut Vec<HatchModel>) {
        let Some(mut model) = self.hatches.get(&handle).cloned() else {
            return;
        };
        let Some(entity) = self.document.get_entity(handle) else {
            return;
        };

        let style = self.render_style(entity);
        model.aci = style.4;
        model.line_weight_px = style.3;
        if !matches!(
            model.pattern,
            crate::scene::model::hatch_model::HatchPattern::Gradient { .. }
        ) {
            model.color = style.0;
        }
        if self.selected.contains(&handle) {
            model.color = [0.15, 0.55, 1.00, model.color[3]];
        }
        model.draw_depth = self
            .draw_depth_map()
            .get(&handle.value())
            .map_or(0.0, |depth| depth[0]);

        if let EntityType::Hatch(hatch) = entity {
            if let Some(background) = crate::entities::hatch::background_color(hatch) {
                let mut backdrop = model.clone();
                backdrop.pattern =
                    crate::scene::model::hatch_model::HatchPattern::Solid;
                let (background_color, background_aci) = match background {
                    acadrust::types::Color::ByLayer => {
                        let layer = self.document.layers.get(&hatch.common.layer);
                        let aci = layer
                            .and_then(|layer| match &layer.color {
                                acadrust::types::Color::Index(index) => Some(*index),
                                _ => None,
                            })
                            .unwrap_or(0);
                        (
                            crate::scene::view::render::layer_render_style(
                                &self.document,
                                &hatch.common.layer,
                            )
                            .color,
                            aci,
                        )
                    }
                    acadrust::types::Color::ByBlock => (style.0, style.4),
                    acadrust::types::Color::Index(index) => (
                        crate::scene::convert::tess_util::aci_to_rgba(
                            &acadrust::types::Color::Index(index),
                        ),
                        index,
                    ),
                    other => (
                        crate::scene::convert::tess_util::aci_to_rgba(&other),
                        0,
                    ),
                };
                backdrop.color = background_color;
                backdrop.aci = background_aci;
                backdrop.name = "SOLID".into();
                models.push(backdrop);
            }
        }
        models.push(model);
    }

    pub fn set_preview_text(&mut self, verts: Vec<crate::scene::pipeline::text_gpu::TextVertex>) {
        // Overlay glyphs — same reasoning as `set_preview_wires`: no geometry
        // bump. Uploaded to a dedicated per-frame text buffer in `prepare`.
        self.preview_text = verts;
    }

    pub fn clear_preview_wire(&mut self) {
        // No geometry bump — see `set_preview_wires`. Dropping the overlay
        // flips the wire content id back to the base tessellation id, which
        // re-uploads the base wires (without the preview) on the next frame.
        self.preview_wires = vec![];
        if !self.preview_hatches.is_empty() {
            self.preview_hatches = std::sync::Arc::new(Vec::new());
        }
        self.preview_text = vec![];
        self.interim_wire = None;
        // Drop any point-picked window marquee (STRETCH) so it doesn't linger
        // after the command ends before the next mouse move. (#291)
        self.selection.borrow_mut().preview_box = None;
    }

    pub fn wire_models_for(&self, handles: &[acadrust::Handle]) -> Vec<WireModel> {
        handles
            .iter()
            .flat_map(|h| {
                match self.document.get_entity(*h) {
                    // Hatches carry no outline in the normal wire set, but an
                    // edit preview (move / copy / array / grip-drag) needs to
                    // show the shape following the cursor. Build a live boundary
                    // from the current HatchModel — `apply_grip` keeps it in
                    // step, so the preview tracks a dragged grip in real time.
                    Some(EntityType::Hatch(_)) => {
                        self.hatch_outline_wire(*h).into_iter().collect()
                    }
                    Some(e) => self.tessellate_one(e),
                    None => Vec::new(),
                }
            })
            .collect()
    }

    /// Split a MIRROR selection into plain ghost wires (reflected wholesale) and
    /// text ghosts paired with their bounding-box centre. Lets the preview match
    /// the commit for TEXT: MIRRTEXT on → true glyph mirror (full reflection, same
    /// as plain geometry); MIRRTEXT off → right-reading at the mirror-symmetric
    /// position (reflect the centre, translate) instead of hugging the axis.
    pub fn mirror_preview_parts(
        &self,
        handles: &[Handle],
    ) -> (Vec<WireModel>, Vec<(WireModel, glam::DVec3)>) {
        let mut plain: Vec<WireModel> = Vec::new();
        let mut texts: Vec<(WireModel, glam::DVec3)> = Vec::new();
        for h in handles {
            let Some(e) = self.document.get_entity(*h) else {
                continue;
            };
            if matches!(e, EntityType::Text(_)) {
                let wires = self.tessellate_one(e);
                let mut lo = [f64::INFINITY; 2];
                let mut hi = [f64::NEG_INFINITY; 2];
                for w in &wires {
                    for (i, p) in w.points.iter().enumerate() {
                        if !p[0].is_finite() || !p[1].is_finite() {
                            continue;
                        }
                        // Reconstruct f64 world from the double-single pair (the
                        // low residual may be absent on some wires).
                        let l = w.points_low.get(i).copied().unwrap_or([0.0; 3]);
                        let (x, y) = (p[0] as f64 + l[0] as f64, p[1] as f64 + l[1] as f64);
                        lo[0] = lo[0].min(x);
                        lo[1] = lo[1].min(y);
                        hi[0] = hi[0].max(x);
                        hi[1] = hi[1].max(y);
                    }
                }
                let center = if lo[0] <= hi[0] {
                    glam::DVec3::new((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, 0.0)
                } else {
                    glam::DVec3::ZERO
                };
                for w in wires {
                    texts.push((w, center));
                }
            } else {
                plain.extend(self.tessellate_one(e));
            }
        }
        (plain, texts)
    }

    /// Boundary outline wire for a hatch, reconstructed from its cached
    /// `HatchModel` (offsets from `world_origin`). Used only for edit previews —
    /// the normal render shows the fill, not this outline.
    fn hatch_outline_wire(&self, handle: Handle) -> Option<WireModel> {
        let m = self.hatches.get(&handle)?;
        Self::hatch_model_outline_wire(handle, m)
    }

    fn hatch_model_outline_wire(handle: Handle, m: &HatchModel) -> Option<WireModel> {
        let (wx, wy) = (m.world_origin[0], m.world_origin[1]);
        let pts: Vec<[f64; 3]> = m
            .boundary
            .iter()
            .map(|&[x, y]| {
                if x.is_finite() && y.is_finite() {
                    [wx + x as f64, wy + y as f64, 0.0]
                } else {
                    [f64::NAN; 3]
                }
            })
            .collect();
        if pts.len() < 2 {
            return None;
        }
        Some(WireModel::solid_f64(
            handle.value().to_string(),
            pts,
            m.color,
            false,
        ))
    }

    /// Fill-only geometry has no resident wire for the normal rollover xray.
    /// Supply an orange boundary overlay while an entity-pick command previews
    /// a top-level hatch or an Insert whose visible children contain hatches.
    pub fn fill_hover_preview_wires(&self, handle: Handle) -> Vec<WireModel> {
        let mut wires: Vec<WireModel> = match self.document.get_entity(handle) {
            Some(EntityType::Hatch(_)) => self.hatch_outline_wire(handle).into_iter().collect(),
            Some(EntityType::Insert(_)) => self
                .insert_hatches_for_click()
                .get(&handle)
                .into_iter()
                .flatten()
                .filter_map(|model| Self::hatch_model_outline_wire(handle, model))
                .collect(),
            _ => Vec::new(),
        };
        for wire in &mut wires {
            wire.color = WireModel::HOVER;
            wire.line_weight_px = wire.line_weight_px.max(2.0);
        }
        wires
    }

    /// Build wire models for an arbitrary slice of entities (e.g. clipboard contents).
    /// Entities need not be in the document — they are tessellated directly.
    pub fn wires_for_entities(&self, entities: &[acadrust::EntityType]) -> Vec<WireModel> {
        entities
            .iter()
            .flat_map(|e| self.tessellate_one(e))
            .collect()
    }

    pub fn set_interim_wire(&mut self, w: WireModel) {
        // Overlay wire — same reasoning as `set_preview_wires`: no geometry
        // bump, so the model isn't re-tessellated on every interim update.
        self.interim_wire = Some(w);
    }
}
