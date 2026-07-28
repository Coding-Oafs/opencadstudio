// Auto-split from scene/mod.rs. Pure text-move; behaviour unchanged.
use super::*;

impl Scene {
    // ── Selection ─────────────────────────────────────────────────────────

    pub fn select_entity(&mut self, handle: Handle, exclusive: bool) {
        if exclusive {
            self.selected.clear();
        }
        self.selected.insert(handle);
        self.bump_selection();
    }

    pub fn deselect_all(&mut self) {
        self.selected.clear();
        self.bump_selection();
    }

    /// Remove a single entity from the selection (Shift+click subtractive pick).
    pub fn deselect_entity(&mut self, handle: Handle) {
        if self.selected.remove(&handle) {
            self.bump_selection();
        }
    }

    pub fn selected_entities(&self) -> Vec<(Handle, &EntityType)> {
        self.selected
            .iter()
            .filter_map(|&h| self.document.get_entity(h).map(|e| (h, e)))
            .collect()
    }

    /// Iterates every entity owned by the current layout's block-record.
    /// Returns an empty vec when the block-record is missing or holds no
    /// entity handles (legacy DXF without group-code 330 — we err on the
    /// side of "no candidates" instead of scanning the whole document, so
    /// model-block entities don't leak into a paper-layout selection).
    fn current_layout_entity_handles(&self) -> Vec<Handle> {
        let block = self.current_layout_block_handle();
        self.document
            .block_records
            .iter()
            .find(|br| br.handle == block)
            .map(|br| br.entity_handles.clone())
            .unwrap_or_default()
    }

    /// Extends the current selection with every entity in the active
    /// layout that matches one of the selected entities by `(variant,
    /// layer)`. The seed selection stays selected. No-op when nothing is
    /// selected. Returns the number of newly-added entities.
    pub fn select_similar(&mut self) -> usize {
        use crate::entities::traits::entity_type_name;
        if self.selected.is_empty() {
            return 0;
        }
        let pairs: rustc_hash::FxHashSet<(&str, String)> = self
            .selected
            .iter()
            .filter_map(|h| self.document.get_entity(*h))
            .map(|e| (entity_type_name(e), e.as_entity().layer().to_string()))
            .collect();
        let handles = self.current_layout_entity_handles();
        let mut added = 0;
        for h in handles {
            if self.selected.contains(&h) {
                continue;
            }
            if let Some(e) = self.document.get_entity(h) {
                let key = (entity_type_name(e), e.as_entity().layer().to_string());
                if pairs.contains(&key) {
                    self.selected.insert(h);
                    added += 1;
                }
            }
        }
        if added > 0 {
            self.bump_selection();
        }
        added
    }

    /// Replace the selection with its complement: every selectable object
    /// in the active layout that isn't currently selected. The candidate
    /// set is the visible wire set, so objects on off/frozen layers (which
    /// can't be picked anyway) are excluded. Returns the new count.
    pub fn invert_selection(&mut self) -> usize {
        let prev: rustc_hash::FxHashSet<Handle> = self.selected.iter().copied().collect();
        let all: Vec<Handle> = self
            .entity_wires()
            .iter()
            .filter_map(|w| Self::handle_from_wire_name(&w.name))
            .collect();
        self.selected.clear();
        for h in all {
            if !prev.contains(&h) {
                self.selected.insert(h);
            }
        }
        self.bump_selection();
        self.selected.len()
    }

    /// Replaces (or extends, when `append` is true) the current
    /// selection with every entity in the active layout that matches
    /// the filter. Returns the number of newly-matching entities.
    ///
    /// `type_name` of `None` means "any type". `property_field` of
    /// `None` skips the property test (only the type filter applies).
    /// The operator's `Any` variant also skips the property test.
    /// Numeric operators (`Gt` / `Lt`) parse both sides as `f64` and
    /// reject anything non-numeric.
    pub fn qselect(
        &mut self,
        type_name: Option<&str>,
        property_field: Option<&str>,
        op: crate::app::QSelectOp,
        value: &str,
        append: bool,
    ) -> usize {
        use crate::app::QSelectOp;
        use crate::entities::traits::entity_type_name;
        if !append {
            self.selected.clear();
        }
        let handles = self.current_layout_entity_handles();
        let mut matched = 0;
        for h in handles {
            let Some(e) = self.document.get_entity(h) else {
                continue;
            };
            // Never quick-select objects on a locked layer.
            if self
                .document
                .layers
                .get(&e.common().layer)
                .map(|l| l.is_locked())
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(t) = type_name {
                if entity_type_name(e) != t {
                    continue;
                }
            }
            let prop_ok = match (property_field, op) {
                (None, _) | (_, QSelectOp::Any) => true,
                (Some(field), op) => {
                    let Some(actual) = self.entity_property_value(e, field) else {
                        continue;
                    };
                    match op {
                        QSelectOp::Eq => actual.eq_ignore_ascii_case(value),
                        QSelectOp::Neq => !actual.eq_ignore_ascii_case(value),
                        QSelectOp::Gt | QSelectOp::Lt => {
                            let (Ok(a), Ok(b)) =
                                (actual.parse::<f64>(), value.parse::<f64>())
                            else {
                                continue;
                            };
                            if matches!(op, QSelectOp::Gt) {
                                a > b
                            } else {
                                a < b
                            }
                        }
                        QSelectOp::Any => true,
                    }
                }
            };
            if prop_ok {
                self.selected.insert(h);
                matched += 1;
            }
        }
        self.bump_selection();
        matched
    }

    /// Returns the sorted set of entity-type names present in the active
    /// layout. Used to populate the Quick Select "Object type" dropdown
    /// with only the types that actually exist in the drawing.
    pub fn entity_type_names_in_layout(&self) -> Vec<String> {
        use crate::entities::traits::entity_type_name;
        let mut names: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for h in self.current_layout_entity_handles() {
            if let Some(e) = self.document.get_entity(h) {
                names.insert(entity_type_name(e).to_string());
            }
        }
        names.into_iter().collect()
    }

    /// True when `handle`'s entity type is allowed by the selection filter.
    /// The filter stores excluded type names; empty = everything allowed.
    pub fn passes_selection_filter(&self, handle: Handle) -> bool {
        if self.selection_filter.is_empty() {
            return true;
        }
        match self.document.get_entity(handle) {
            Some(e) => !self
                .selection_filter
                .contains(crate::entities::traits::entity_type_name(e)),
            None => true,
        }
    }

    /// True when the selection filter is excluding at least one type.
    pub fn selection_filter_active(&self) -> bool {
        !self.selection_filter.is_empty()
    }

    /// Returns the list of `(field, label)` pairs the Quick Select
    /// "Properties" dropdown should show given the current type filter:
    ///
    /// * Common properties (Layer, Color, Linetype, Lineweight) are
    ///   always included.
    /// * When `type_name` names a specific entity type present in the
    ///   active layout, the first entity of that type contributes its
    ///   `geometry_properties()` rows (Start X, Length, Radius, …) so
    ///   type-specific filtering works.
    pub fn qselect_properties(
        &self,
        type_name: Option<&str>,
    ) -> Vec<(String, String)> {
        use crate::entities::traits::{entity_type_name, EntityTypeOps};
        let mut out: Vec<(String, String)> = vec![
            ("layer".to_string(), "Layer".to_string()),
            ("color".to_string(), "Color".to_string()),
            ("linetype".to_string(), "Linetype".to_string()),
            ("lineweight".to_string(), "Lineweight".to_string()),
        ];
        if let Some(t) = type_name {
            let text_style_names: Vec<String> = self
                .document
                .text_styles
                .iter()
                .map(|s| s.name.clone())
                .collect();
            let sample = self
                .current_layout_entity_handles()
                .into_iter()
                .filter_map(|h| self.document.get_entity(h))
                .find(|e| entity_type_name(e) == t);
            if let Some(sample) = sample {
                for section in sample.geometry_properties(&text_style_names) {
                    for prop in section.props {
                        // Skip rows that don't sensibly compare via
                        // `entity_property_value` (read-only labels are
                        // fine — users can still match against them).
                        out.push((prop.field.to_string(), prop.label.clone()));
                    }
                }
            }
        }
        out
    }

    /// Reads a property value from an entity for QSELECT comparison.
    /// Returns the canonical string used as the left-hand side of the
    /// operator test. Common properties have hand-rolled formatting so
    /// `"ByLayer"` / `"7"` / `"0.30mm"` are stable; everything else
    /// goes through `geometry_properties()` and pulls the matching
    /// row's value out.
    pub fn entity_property_value(
        &self,
        entity: &acadrust::EntityType,
        field: &str,
    ) -> Option<String> {
        use crate::entities::traits::EntityTypeOps;
        use crate::scene::model::object::PropValue;
        match field {
            "layer" => Some(entity.common().layer.clone()),
            "color" => Some(Self::format_color(entity.common().color)),
            "linetype" => Some(entity.common().linetype.clone()),
            "lineweight" => Some(Self::format_lineweight(entity.common().line_weight)),
            _ => {
                let text_style_names: Vec<String> = self
                    .document
                    .text_styles
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                let prop = entity
                    .geometry_properties(&text_style_names)
                    .into_iter()
                    .flat_map(|s| s.props)
                    .find(|p| p.field == field)?;
                Some(match prop.value {
                    PropValue::ReadOnly(s) | PropValue::EditText(s) => s,
                    PropValue::LayerChoice(s) => s,
                    PropValue::Choice { selected, .. } => selected,
                    PropValue::EditChoice { value, .. } => value,
                    PropValue::ColorChoice(c) => Self::format_color(c),
                    PropValue::LwChoice(lw) => Self::format_lineweight(lw),
                    PropValue::LinetypeChoice(s) => s,
                    PropValue::HatchPatternChoice(s) => s,
                    PropValue::BoolToggle { value, .. } => value.to_string(),
                    PropValue::AttrText { value, .. } => value,
                    PropValue::Stepper { display, .. } => display,
                    PropValue::ColorVaries | PropValue::LwVaries => return None,
                })
            }
        }
    }

    fn format_color(c: acadrust::types::Color) -> String {
        use acadrust::types::Color;
        match c {
            Color::ByLayer => "ByLayer".to_string(),
            Color::None => "None".to_string(),
            Color::ByBlock => "ByBlock".to_string(),
            Color::Index(i) => i.to_string(),
            Color::Rgb { r, g, b } => format!("{},{},{}", r, g, b),
        }
    }

    fn format_lineweight(lw: acadrust::types::LineWeight) -> String {
        use acadrust::types::LineWeight;
        match lw {
            LineWeight::ByLayer => "ByLayer".to_string(),
            LineWeight::ByBlock => "ByBlock".to_string(),
            LineWeight::Default => "Default".to_string(),
            LineWeight::Value(v) => format!("{:.2}mm", v as f64 / 100.0),
        }
    }

    // ── Erase ─────────────────────────────────────────────────────────────

    pub fn erase_entities(&mut self, handles: &[Handle]) {
        let mut handle_set: HashSet<Handle> = HashSet::default();
        let mut erased: Vec<(Handle, ChangeKind)> = Vec::new();
        for &h in handles {
            // Objects on a locked layer can't be erased.
            if self.is_layer_locked(h) {
                continue;
            }
            // Delta-undo: capture the removed entity so an undo can re-insert it.
            if self.is_recording_undo() {
                let before = self.document.get_entity_arc(h);
                self.record_undo_before(h, before);
            }
            self.remember_removed_cache_categories(h);
            self.document.remove_entity_arc(h);
            self.selected.remove(&h);
            self.hatches.remove(&h);
            self.images.remove(&h);
            self.meshes.remove(&h);
            self.block_meshes.remove(&h);
            self.solid_models.remove(&h);
            handle_set.insert(h);
            erased.push((h, ChangeKind::Removed));
        }
        // Capture exactly the group objects that this erase will rewrite, plus
        // the group dictionary when an emptied group will be removed. Do this
        // before taking mutable object-map borrows below.
        if self.is_recording_undo() && !handle_set.is_empty() {
            let affected_groups: Vec<(Handle, ObjectType)> = self
                .document
                .objects
                .iter()
                .filter_map(|(&handle, object)| match object {
                    ObjectType::Group(group)
                        if group.entities.iter().any(|h| handle_set.contains(h)) =>
                    {
                        Some((handle, object.clone()))
                    }
                    _ => None,
                })
                .collect();
            if !affected_groups.is_empty() {
                let removes_group = affected_groups.iter().any(|(_, object)| {
                    matches!(
                        object,
                        ObjectType::Group(group)
                            if group.entities.iter().all(|h| handle_set.contains(h))
                    )
                });
                if removes_group {
                    let dictionary = self.document.header.acad_group_dict_handle;
                    let dictionary_before = self.document.objects.get(&dictionary).cloned();
                    self.record_undo_object_before(dictionary, dictionary_before);
                }
                for (handle, object) in affected_groups {
                    self.record_undo_object_before(handle, Some(object));
                }
            }
        }
        // Remove erased handles from all groups; delete groups that become empty.
        let group_dict_handle = self.document.header.acad_group_dict_handle;
        let to_remove: Vec<Handle> = self
            .document
            .objects
            .values_mut()
            .filter_map(|obj| match obj {
                ObjectType::Group(g) => {
                    g.entities.retain(|h| !handle_set.contains(h));
                    if g.entities.is_empty() {
                        Some(g.handle)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect();
        for gh in &to_remove {
            if let Some(ObjectType::Dictionary(dict)) =
                self.document.objects.get_mut(&group_dict_handle)
            {
                dict.entries.retain(|(_, h)| h != gh);
            }
            self.document.objects.remove(gh);
        }
        // Deleting top-level entities/inserts leaves block definitions intact.
        // Report the exact erased handles so derived caches drop just those and
        // the resident set removes only their wires (bump_entities drops them
        // from the tessellation memos too).
        if !erased.is_empty() {
            self.invalidate_dependency_index();
            self.bump_entities(&erased);
        }
    }

    /// Restore erased Arc-backed entities without re-linking their still-present
    /// block-record handles. Used by OOPS and history replay.
    pub fn restore_erased_entities(&mut self, entities: Vec<Arc<EntityType>>) -> Vec<Handle> {
        let mut restored = Vec::with_capacity(entities.len());
        let mut changes = Vec::with_capacity(entities.len());
        for entity in entities {
            let handle = entity.common().handle;
            if handle.is_null() || self.document.get_entity(handle).is_some() {
                continue;
            }
            if self.is_recording_undo() {
                self.record_undo_before(handle, None);
            }
            if self.document.restore_entity_arc(entity).is_some() {
                self.reseed_derived_caches(handle);
                restored.push(handle);
                changes.push((handle, ChangeKind::Added));
            }
        }
        if !changes.is_empty() {
            self.invalidate_dependency_index();
            self.bump_entities(&changes);
        }
        restored
    }
}
