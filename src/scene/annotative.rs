//! Shared annotative-object detection + annotation-scale resolution.
//!
//! Both the Properties panel (Annotative row / applied scale name) and the
//! tessellation bake (which scales annotative content by the current annotation
//! scale) must agree on *which* entities are annotative — so that logic lives
//! here, once. An entity is annotative if it carries a per-object annotation
//! context, legacy annotative XDATA, or an entity-level annotative flag. Text
//! style state is consulted only while creating a new text object: changing a
//! style later must not retroactively scale existing text.

use acadrust::entities::{EntityCommon, EntityType};
use acadrust::objects::{
    Dictionary, HatchScaleContext, MTextContext, ObjectContextData, ObjectContextKind, ObjectType,
};
use acadrust::types::{Vector2, Vector3};
use acadrust::{CadDocument, Handle};
use std::borrow::Cow;

/// Resolve a handle to a `Dictionary` object, if it is one.
pub fn as_dict(doc: &CadDocument, handle: Handle) -> Option<&Dictionary> {
    match doc.objects.get(&handle) {
        Some(ObjectType::Dictionary(d)) => Some(d),
        _ => None,
    }
}

/// Resolve the drawing's root named-objects dictionary, creating one if the
/// file has none reachable.
///
/// The canonical pointer is `header.named_objects_dict_handle`, but DWGs
/// written by some programs leave it dangling — pointing at a handle that never
/// loaded, or at a non-dictionary — while the real named-object sub-dictionaries
/// (`ACAD_LAYOUT`, `ACAD_SCALELIST`, …) are instead owned by an unrelated handle
/// that is not a dictionary. When the pointer can't be resolved we adopt any
/// top-level (`owner == NULL`) dictionary as the root; failing that we synthesise
/// a fresh, empty root so that *registering* a new named-object entry (a page
/// setup, an annotation scale, the `CTAB` variable) actually persists instead of
/// silently no-opping against a missing dictionary.
///
/// Idempotent: the resolved or created handle is written back to the header, so
/// later calls return the same dictionary rather than minting another root. On a
/// well-formed drawing this returns the existing root untouched.
pub fn root_named_dict_handle(doc: &mut CadDocument) -> Handle {
    let h = doc.header.named_objects_dict_handle;
    if matches!(doc.objects.get(&h), Some(ObjectType::Dictionary(_))) {
        return h;
    }
    // A top-level dictionary is already present (the standard root shape) — adopt
    // the richest one (matching the DWG writer's own root heuristic) and repair
    // the stale header pointer.
    if let Some(root) = doc
        .objects
        .iter()
        .filter_map(|(k, o)| match o {
            ObjectType::Dictionary(d) if d.owner.is_null() => Some((*k, d.entries.len())),
            _ => None,
        })
        .max_by_key(|&(_, n)| n)
        .map(|(k, _)| k)
    {
        doc.header.named_objects_dict_handle = root;
        return root;
    }
    // Nothing reachable — build a fresh, empty root named-objects dictionary.
    let nh = doc.allocate_handle();
    let mut d = Dictionary::new();
    d.handle = nh;
    d.owner = Handle::NULL;
    doc.objects.insert(nh, ObjectType::Dictionary(d));
    doc.header.named_objects_dict_handle = nh;
    nh
}

/// Set the per-object annotative flag on the entity types that carry one
/// (MTEXT, MULTILEADER). Turning it off also strips the per-object annotation
/// context and legacy markers via [`clear_annotation_context`] so the object
/// stops resolving annotative; turning it on leaves the base geometry as the
/// single (implicit, current-scale) representation. TEXT uses a context rather
/// than a native flag; other entity types are not toggled here.
pub fn set_entity_annotative(doc: &mut CadDocument, handle: Handle, want: bool) {
    if let Some(e) = doc.get_entity_mut(handle) {
        match e {
            EntityType::MText(t) => t.is_annotative = want,
            EntityType::MultiLeader(m) => m.enable_annotation_scale = want,
            _ => {}
        }
    }
    if !want {
        clear_annotation_context(doc, handle);
    }
}

/// Derive the per-scale context payload for an entity from its current
/// placement. Returns the concrete class name and the context kind, or `None`
/// for entity types that do not carry a per-object annotation context (their
/// annotative state comes from a style, e.g. DIMENSION/TABLE).
fn context_kind_for(entity: &EntityType) -> Option<(&'static str, ObjectContextKind)> {
    match entity {
        EntityType::Insert(ins) => Some((
            "ACDB_BLKREFOBJECTCONTEXTDATA_CLASS",
            ObjectContextKind::BlkRef {
                rotation: ins.rotation,
                insertion: ins.insert_point,
                scale_factor: Vector3::new(ins.x_scale(), ins.y_scale(), ins.z_scale()),
            },
        )),
        EntityType::Text(t) => Some((
            "ACDB_TEXTOBJECTCONTEXTDATA_CLASS",
            ObjectContextKind::Text {
                horizontal_mode: t.horizontal_alignment as i16,
                rotation: t.rotation,
                insertion: Vector2::new(t.insertion_point.x, t.insertion_point.y),
                alignment: t
                    .alignment_point
                    .map(|p| Vector2::new(p.x, p.y))
                    .unwrap_or(Vector2::new(0.0, 0.0)),
            },
        )),
        EntityType::MText(m) => Some((
            "ACDB_MTEXTOBJECTCONTEXTDATA_CLASS",
            ObjectContextKind::MText(MTextContext {
                attachment: m.attachment_point as i32,
                // MTEXT stores a text X-axis direction; derive it from rotation.
                x_axis_dir: Vector3::new(m.rotation.cos(), m.rotation.sin(), 0.0),
                insertion: m.insertion_point,
                rect_width: m.rectangle_width,
                rect_height: 0.0,
                extents_width: 0.0,
                extents_height: 0.0,
                column_type: 0,
                columns: None,
            }),
        )),
        _ => None,
    }
}

/// Give an entity a per-object annotation context for `scale_handle`,
/// synthesizing the extension-dictionary chain it hangs from when absent:
///
/// ```text
/// entity xdict → "AcDbContextDataManager" → "ACDB_ANNOTATIONSCALES" → "*An" → leaf
/// ```
///
/// The leaf is an [`ObjectContextData`] whose placement is copied from the
/// entity's current geometry and whose `340` handle references `scale_handle`
/// (an `AcDbScale` in `ACAD_SCALELIST`). Idempotent: a leaf for that scale is
/// not duplicated. Exactly one leaf per object is marked `is_default` (the
/// first one created — the native representation). Returns `false` for entity
/// kinds that carry no per-object context (their annotative-ness is style-driven).
pub fn create_annotation_context(
    doc: &mut CadDocument,
    entity_handle: Handle,
    scale_handle: Handle,
) -> bool {
    let Some((class_name, kind)) = doc.get_entity(entity_handle).and_then(context_kind_for) else {
        return false;
    };
    // The writer emits a 500+ class number only for registered classes.
    doc.register_object_context_class(class_name);

    // Extension dictionary (hard-owns its entries; 280 = 1). Create it if the
    // entity has none, and point the entity at it.
    let xdict_h = match doc
        .get_entity(entity_handle)
        .and_then(|e| e.common().xdictionary_handle)
    {
        Some(h) if as_dict(doc, h).is_some() => h,
        _ => {
            let h = doc.allocate_handle();
            let mut d = Dictionary::new();
            d.handle = h;
            d.owner = entity_handle;
            d.hard_owner = true;
            doc.objects.insert(h, ObjectType::Dictionary(d));
            if let Some(e) = doc.get_entity_mut(entity_handle) {
                e.common_mut().xdictionary_handle = Some(h);
            }
            h
        }
    };

    let mgr_h = get_or_create_child_dict(doc, xdict_h, "AcDbContextDataManager");
    let coll_h = get_or_create_child_dict(doc, mgr_h, "ACDB_ANNOTATIONSCALES");

    // Idempotent: if a leaf already applies to this scale, keep it.
    let existing = as_dict(doc, coll_h)
        .map(|d| {
            d.entries.iter().any(|(_, lh)| {
                matches!(
                    doc.objects.get(lh),
                    Some(ObjectType::ObjectContextData(c)) if c.scale == scale_handle
                )
            })
        })
        .unwrap_or(false);
    if existing {
        return true;
    }

    // The first representation created is the default (native) one.
    let is_default = as_dict(doc, coll_h).map(|d| d.entries.is_empty()).unwrap_or(true);
    let n = as_dict(doc, coll_h).map(|d| d.entries.len()).unwrap_or(0) + 1;
    let key = format!("*A{n}");

    let leaf_h = doc.allocate_handle();
    let leaf = ObjectContextData {
        handle: leaf_h,
        owner_handle: coll_h,
        reactors: vec![coll_h],
        xdictionary_handle: None,
        class_version: 3,
        is_default,
        scale: scale_handle,
        kind,
    };
    doc.objects
        .insert(leaf_h, ObjectType::ObjectContextData(leaf));
    if let Some(ObjectType::Dictionary(coll)) = doc.objects.get_mut(&coll_h) {
        coll.add_entry(key, leaf_h);
    }
    true
}

/// True when `common` belongs to an annotative object whose per-object scale
/// contexts are *all* bound to scales other than the current one — an
/// off-scale representation that must stay hidden. A drawing may materialise
/// each annotation scale as its own object (e.g. a block insert per scale on a
/// "0 @ <scale>" layer); only the current scale's representation should draw.
///
/// False for non-annotative objects (no per-object context — the vast
/// majority) and for objects whose contexts include the current scale. Gated
/// on an extension dictionary so non-annotative entities skip the lookup.
pub fn annotative_offscale(doc: &CadDocument, common: &EntityCommon) -> bool {
    if !common
        .xdictionary_handle
        .map(|h| !h.is_null())
        .unwrap_or(false)
    {
        return false;
    }
    let scales = object_scale_memberships(doc, common.handle);
    if scales.is_empty() {
        return false;
    }
    let cur = &doc.header.current_annotation_scale;
    if scales.iter().any(|(name, _)| name.eq_ignore_ascii_case(cur)) {
        return false;
    }
    // Off-scale (no context for the current scale). If some representation in
    // the drawing DOES provide the current scale, hide this one — the matching
    // representation is the one to show.
    if current_scale_provided(doc) {
        return true;
    }
    // The current scale is unsupported by any representation. Fall back to the
    // base "1:1" representation: keep it, hide the enlarged copies — otherwise
    // every scale representation stacks (or, if all were hidden, the object
    // vanishes). Without this, opening at e.g. CANNOSCALE 10:1 shows both a 1×
    // and a 10× copy of the same block.
    !scales.iter().any(|(name, _)| name.eq_ignore_ascii_case("1:1"))
}

/// Whether any annotative representation in the drawing targets the current
/// annotation scale.
fn current_scale_provided(doc: &CadDocument) -> bool {
    let cur = &doc.header.current_annotation_scale;
    doc.objects.values().any(|o| {
        if let ObjectType::ObjectContextData(cd) = o {
            if let Some(ObjectType::Scale(s)) = doc.objects.get(&cd.scale) {
                return s.name.eq_ignore_ascii_case(cur);
            }
        }
        false
    })
}

/// The annotation scales an object currently carries a per-object context for,
/// as `(scale name, scale handle)` pairs (one per representation). Empty when
/// the object has no per-object context chain.
pub fn object_scale_memberships(doc: &CadDocument, entity: Handle) -> Vec<(String, Handle)> {
    let mut out = Vec::new();
    let Some(coll_h) = annotation_scales_dict(doc, entity) else {
        return out;
    };
    if let Some(coll) = as_dict(doc, coll_h) {
        for (_, lh) in &coll.entries {
            if let Some(ObjectType::ObjectContextData(leaf)) = doc.objects.get(lh) {
                if let Some(ObjectType::Scale(s)) = doc.objects.get(&leaf.scale) {
                    out.push((s.name.clone(), leaf.scale));
                }
            }
        }
    }
    out
}

/// Remove the per-object context representation that applies to `scale_handle`,
/// keeping the object's other representations. When that was the object's last
/// representation the whole context chain (and the annotative markers) are torn
/// down via [`clear_annotation_context`] so the object becomes non-annotative.
/// Returns `true` if a representation was removed.
pub fn remove_annotation_context_for_scale(
    doc: &mut CadDocument,
    entity: Handle,
    scale_handle: Handle,
) -> bool {
    let Some(coll_h) = annotation_scales_dict(doc, entity) else {
        return false;
    };
    // Find the leaf that applies to this scale.
    let leaf = as_dict(doc, coll_h).and_then(|c| {
        c.entries.iter().find_map(|(_, lh)| {
            matches!(
                doc.objects.get(lh),
                Some(ObjectType::ObjectContextData(o)) if o.scale == scale_handle
            )
            .then_some(*lh)
        })
    });
    let Some(leaf_h) = leaf else {
        return false;
    };
    // If this is the object's only representation, fully de-annotate it (drop the
    // whole chain AND the native flag, like the Yes→No toggle) so it stops
    // resolving annotative; otherwise drop just this leaf.
    let last = as_dict(doc, coll_h).map(|c| c.entries.len() <= 1).unwrap_or(true);
    if last {
        set_entity_annotative(doc, entity, false);
        return true;
    }
    doc.objects.remove(&leaf_h);
    if let Some(ObjectType::Dictionary(coll)) = doc.objects.get_mut(&coll_h) {
        coll.entries.retain(|(_, h)| *h != leaf_h);
    }
    true
}

/// Resolve an entity's `ACDB_ANNOTATIONSCALES` collection dictionary handle, if
/// its context chain exists.
fn annotation_scales_dict(doc: &CadDocument, entity: Handle) -> Option<Handle> {
    let xd = doc.get_entity(entity).and_then(|e| e.common().xdictionary_handle)?;
    let mgr = as_dict(doc, xd).and_then(|d| d.get("AcDbContextDataManager"))?;
    as_dict(doc, mgr).and_then(|d| d.get("ACDB_ANNOTATIONSCALES"))
}

/// Resolve the representation leaf used by the drawing's current annotation
/// scale. Broken scale handles are ignored. When the current named scale is
/// absent, the leaf explicitly marked as the native/default representation is
/// preferred, followed by the first valid leaf.
pub fn active_object_context(
    doc: &CadDocument,
    entity: Handle,
) -> Option<&ObjectContextData> {
    let coll_h = annotation_scales_dict(doc, entity)?;
    let coll = as_dict(doc, coll_h)?;
    let mut default = None;
    let mut first = None;
    for (_, leaf_h) in &coll.entries {
        let Some(ObjectType::ObjectContextData(leaf)) = doc.objects.get(leaf_h) else {
            continue;
        };
        first.get_or_insert(leaf);
        if leaf.is_default {
            default = Some(leaf);
        }
        let Some(ObjectType::Scale(scale)) = doc.objects.get(&leaf.scale) else {
            continue;
        };
        if scale
            .name
            .eq_ignore_ascii_case(&doc.header.current_annotation_scale)
        {
            return Some(leaf);
        }
    }
    default.or(first)
}

fn text_horizontal(value: i16) -> acadrust::entities::TextHorizontalAlignment {
    use acadrust::entities::TextHorizontalAlignment;
    match value {
        1 => TextHorizontalAlignment::Center,
        2 => TextHorizontalAlignment::Right,
        3 => TextHorizontalAlignment::Aligned,
        4 => TextHorizontalAlignment::Middle,
        5 => TextHorizontalAlignment::Fit,
        _ => TextHorizontalAlignment::Left,
    }
}

fn mtext_attachment(value: i32) -> acadrust::entities::AttachmentPoint {
    use acadrust::entities::AttachmentPoint;
    match value {
        2 => AttachmentPoint::TopCenter,
        3 => AttachmentPoint::TopRight,
        4 => AttachmentPoint::MiddleLeft,
        5 => AttachmentPoint::MiddleCenter,
        6 => AttachmentPoint::MiddleRight,
        7 => AttachmentPoint::BottomLeft,
        8 => AttachmentPoint::BottomCenter,
        9 => AttachmentPoint::BottomRight,
        _ => AttachmentPoint::TopLeft,
    }
}

fn apply_mtext_context(entity: &mut acadrust::entities::MText, context: &MTextContext) {
    entity.attachment_point = mtext_attachment(context.attachment);
    entity.insertion_point = context.insertion;
    entity.rectangle_width = context.rect_width;
    entity.rectangle_height = (context.rect_height > 0.0).then_some(context.rect_height);
    entity.extents_width = context.extents_width;
    entity.extents_height = context.extents_height;
    entity.rotation = context.x_axis_dir.y.atan2(context.x_axis_dir.x);
    entity.dwg_x_direction = Some(context.x_axis_dir);
    if let Some(columns) = &context.columns {
        entity.column_data.column_type = context.column_type as i16;
        entity.column_data.column_count = columns.num_heights;
        entity.column_data.width = columns.width;
        entity.column_data.gutter = columns.gutter;
        entity.column_data.auto_height = columns.auto_height;
        entity.column_data.flow_reversed = columns.flow_reversed;
        entity.column_data.heights.clone_from(&columns.heights);
    } else {
        entity.column_data.column_type = context.column_type as i16;
        entity.column_data.column_count = 0;
        entity.column_data.heights.clear();
    }
}

fn apply_dimension_context(
    dimension: &mut acadrust::entities::Dimension,
    context: &acadrust::objects::DimContext,
    doc: &CadDocument,
) {
    use acadrust::entities::Dimension;
    use acadrust::objects::DimSubtype;

    {
        let base = dimension.base_mut();
        base.text_middle_point.x = context.def_pt.x;
        base.text_middle_point.y = context.def_pt.y;
        base.text_rotation = context.text_rotation;
        base.text_user_positioned = context.is_def_textloc;
        base.flip_arrow1 = context.flip_arrow1;
        base.flip_arrow2 = context.flip_arrow2;
        if let Some(record) = doc.block_records.iter().find(|r| r.handle == context.block) {
            base.block_name.clone_from(&record.name);
        }
    }

    match (&context.subtype, dimension) {
        (DimSubtype::Aligned { dimline_pt }, Dimension::Aligned(dim)) => {
            dim.definition_point = *dimline_pt;
            dim.base.definition_point = *dimline_pt;
        }
        (DimSubtype::Aligned { dimline_pt }, Dimension::Linear(dim)) => {
            dim.definition_point = *dimline_pt;
            dim.base.definition_point = *dimline_pt;
        }
        (DimSubtype::Angular { arc_pt }, Dimension::Angular2Ln(dim)) => {
            dim.dimension_arc = *arc_pt;
            dim.base.definition_point = *arc_pt;
        }
        (DimSubtype::Angular { arc_pt }, Dimension::Angular3Pt(dim)) => {
            dim.definition_point = *arc_pt;
            dim.base.definition_point = *arc_pt;
        }
        (
            DimSubtype::Diametric {
                first_arc_pt,
                def_pt,
            },
            Dimension::Diameter(dim),
        ) => {
            dim.angle_vertex = *first_arc_pt;
            dim.definition_point = *def_pt;
            dim.base.definition_point = *def_pt;
        }
        (DimSubtype::Radial { first_arc_pt }, Dimension::Radius(dim)) => {
            dim.definition_point = *first_arc_pt;
            dim.base.definition_point = *first_arc_pt;
        }
        (
            DimSubtype::RadialLarge {
                ovr_center,
                jog_point,
            },
            Dimension::LargeRadial(dim),
        ) => {
            dim.override_center = *ovr_center;
            dim.jog_point = *jog_point;
        }
        (
            DimSubtype::Ordinate {
                feature_location_pt,
                leader_endpt,
            },
            Dimension::Ordinate(dim),
        ) => {
            dim.feature_location = *feature_location_pt;
            dim.leader_endpoint = *leader_endpt;
        }
        _ => {}
    }
}

fn apply_attribute_context(
    insertion_point: &mut Vector3,
    alignment_point: &mut Vector3,
    rotation: &mut f64,
    horizontal_alignment: &mut acadrust::entities::HorizontalAlignment,
    embedded_mtext: &mut Option<Box<acadrust::entities::MText>>,
    context: &acadrust::objects::MTextAttributeContext,
) {
    insertion_point.x = context.insertion.x;
    insertion_point.y = context.insertion.y;
    alignment_point.x = context.alignment.x;
    alignment_point.y = context.alignment.y;
    *rotation = context.rotation;
    *horizontal_alignment =
        acadrust::entities::HorizontalAlignment::from_value(context.horizontal_mode);
    if context.enable_context {
        if let (Some(embedded), Some(mtext)) = (&context.context, embedded_mtext.as_mut()) {
            apply_mtext_context(mtext, &embedded.mtext);
        }
    }
}

fn apply_hatch_context(hatch: &mut acadrust::entities::Hatch, context: &HatchScaleContext) {
    hatch.pattern.lines.clone_from(&context.pattern_lines);
    hatch.pattern_scale = context.pattern_scale;
    for line in &mut hatch.pattern.lines {
        line.base_point.x += context.pattern_base.x;
        line.base_point.y += context.pattern_base.y;
    }
    for (path, bits) in hatch.paths.iter_mut().zip(&context.loop_types) {
        path.flags = acadrust::entities::BoundaryPathFlags::from_bits(*bits as u32);
    }
}

/// Return an ephemeral entity representation with the active scale leaf
/// overlaid on its base geometry. The source document remains unchanged, which
/// keeps save/round-trip data intact while render, picking and block expansion
/// all see the scale-specific placement.
pub fn entity_for_active_context<'a>(
    doc: &'a CadDocument,
    entity: &'a EntityType,
) -> Cow<'a, EntityType> {
    let Some(context) = active_object_context(doc, entity.common().handle) else {
        return Cow::Borrowed(entity);
    };
    let mut placed = entity.clone();
    match (&context.kind, &mut placed) {
        (
            ObjectContextKind::BlkRef {
                rotation,
                insertion,
                scale_factor,
            },
            EntityType::Insert(insert),
        ) => {
            insert.rotation = *rotation;
            insert.insert_point = *insertion;
            insert.set_x_scale(scale_factor.x);
            insert.set_y_scale(scale_factor.y);
            insert.set_z_scale(scale_factor.z);
        }
        (
            ObjectContextKind::Text {
                horizontal_mode,
                rotation,
                insertion,
                alignment,
            },
            EntityType::Text(text),
        ) => {
            text.horizontal_alignment = text_horizontal(*horizontal_mode);
            text.rotation = *rotation;
            text.insertion_point.x = insertion.x;
            text.insertion_point.y = insertion.y;
            let point = text.alignment_point.get_or_insert(Vector3::ZERO);
            point.x = alignment.x;
            point.y = alignment.y;
        }
        (ObjectContextKind::MText(value), EntityType::MText(mtext)) => {
            apply_mtext_context(mtext, value);
        }
        (ObjectContextKind::Dim(value), EntityType::Dimension(dimension)) => {
            apply_dimension_context(dimension, value, doc);
        }
        (ObjectContextKind::MLeader(value), EntityType::MultiLeader(mleader)) => {
            mleader.context.clone_from(value);
        }
        (
            ObjectContextKind::MTextAttribute(value),
            EntityType::AttributeEntity(attribute),
        ) => {
            apply_attribute_context(
                &mut attribute.insertion_point,
                &mut attribute.alignment_point,
                &mut attribute.rotation,
                &mut attribute.horizontal_alignment,
                &mut attribute.embedded_mtext,
                value,
            );
        }
        (
            ObjectContextKind::MTextAttribute(value),
            EntityType::AttributeDefinition(attribute),
        ) => {
            apply_attribute_context(
                &mut attribute.insertion_point,
                &mut attribute.alignment_point,
                &mut attribute.rotation,
                &mut attribute.horizontal_alignment,
                &mut attribute.embedded_mtext,
                value,
            );
        }
        (ObjectContextKind::Leader(value), EntityType::Leader(leader)) => {
            leader.vertices = value
                .points
                .iter()
                .map(|point| *point + value.insertion_offset)
                .collect();
            leader.horizontal_direction = value.x_direction;
            leader.annotation_offset = value.endpoint_projection;
        }
        (
            ObjectContextKind::Fcf {
                location,
                horizontal_direction,
            },
            EntityType::Tolerance(tolerance),
        ) => {
            tolerance.insertion_point = *location;
            tolerance.direction = *horizontal_direction;
        }
        (ObjectContextKind::HatchScale(value), EntityType::Hatch(hatch)) => {
            apply_hatch_context(hatch, value);
        }
        (ObjectContextKind::HatchView(value), EntityType::Hatch(hatch)) => {
            apply_hatch_context(hatch, &value.hatch);
            hatch.normal = value.view_normal;
            hatch.pattern_angle += value.view_rotation;
        }
        _ => {}
    }
    Cow::Owned(placed)
}

fn sync_mtext_context(context: &mut MTextContext, entity: &acadrust::entities::MText) {
    context.attachment = entity.attachment_point as i32;
    context.x_axis_dir = entity.dwg_x_direction.unwrap_or_else(|| {
        Vector3::new(entity.rotation.cos(), entity.rotation.sin(), 0.0)
    });
    context.insertion = entity.insertion_point;
    context.rect_width = entity.rectangle_width;
    context.rect_height = entity.rectangle_height.unwrap_or(0.0);
    context.extents_width = entity.extents_width;
    context.extents_height = entity.extents_height;
    context.column_type = entity.column_data.column_type as i32;
    if context.column_type == 0 {
        context.columns = None;
    } else {
        let columns = context
            .columns
            .get_or_insert_with(|| acadrust::objects::MTextColumns {
                num_heights: 0,
                width: 0.0,
                gutter: 0.0,
                auto_height: false,
                flow_reversed: false,
                heights: Vec::new(),
            });
        columns.num_heights = entity.column_data.column_count;
        columns.width = entity.column_data.width;
        columns.gutter = entity.column_data.gutter;
        columns.auto_height = entity.column_data.auto_height;
        columns.flow_reversed = entity.column_data.flow_reversed;
        columns.heights.clone_from(&entity.column_data.heights);
    }
}

fn sync_dimension_context(
    context: &mut acadrust::objects::DimContext,
    dimension: &acadrust::entities::Dimension,
    block_handle: Option<Handle>,
) {
    use acadrust::entities::Dimension;
    use acadrust::objects::DimSubtype;

    let base = dimension.base();
    context.def_pt = Vector2::new(base.text_middle_point.x, base.text_middle_point.y);
    context.is_def_textloc = base.text_user_positioned;
    context.text_rotation = base.text_rotation;
    context.flip_arrow1 = base.flip_arrow1;
    context.flip_arrow2 = base.flip_arrow2;
    if let Some(handle) = block_handle {
        context.block = handle;
    }
    match (&mut context.subtype, dimension) {
        (DimSubtype::Aligned { dimline_pt }, Dimension::Aligned(dim)) => {
            *dimline_pt = dim.definition_point;
        }
        (DimSubtype::Aligned { dimline_pt }, Dimension::Linear(dim)) => {
            *dimline_pt = dim.definition_point;
        }
        (DimSubtype::Angular { arc_pt }, Dimension::Angular2Ln(dim)) => {
            *arc_pt = dim.dimension_arc;
        }
        (DimSubtype::Angular { arc_pt }, Dimension::Angular3Pt(dim)) => {
            *arc_pt = dim.definition_point;
        }
        (
            DimSubtype::Diametric {
                first_arc_pt,
                def_pt,
            },
            Dimension::Diameter(dim),
        ) => {
            *first_arc_pt = dim.angle_vertex;
            *def_pt = dim.definition_point;
        }
        (DimSubtype::Radial { first_arc_pt }, Dimension::Radius(dim)) => {
            *first_arc_pt = dim.definition_point;
        }
        (
            DimSubtype::RadialLarge {
                ovr_center,
                jog_point,
            },
            Dimension::LargeRadial(dim),
        ) => {
            *ovr_center = dim.override_center;
            *jog_point = dim.jog_point;
        }
        (
            DimSubtype::Ordinate {
                feature_location_pt,
                leader_endpt,
            },
            Dimension::Ordinate(dim),
        ) => {
            *feature_location_pt = dim.feature_location;
            *leader_endpt = dim.leader_endpoint;
        }
        _ => {}
    }
}

/// Copy an edited entity's placement back into its active per-scale leaf.
/// Geometry edits therefore remain visible at the current annotation scale and
/// round-trip as genuine `AcDb*ObjectContextData`, while the base entity stays
/// usable as the default representation.
pub fn sync_active_context_from_entity(
    doc: &mut CadDocument,
    entity_handle: Handle,
) -> bool {
    let Some(leaf_handle) = active_object_context(doc, entity_handle).map(|leaf| leaf.handle)
    else {
        return false;
    };
    let Some(entity) = doc.get_entity(entity_handle).cloned() else {
        return false;
    };
    let dim_block_handle = match &entity {
        EntityType::Dimension(dimension) => doc
            .block_records
            .iter()
            .find(|record| record.name.eq_ignore_ascii_case(&dimension.base().block_name))
            .map(|record| record.handle),
        _ => None,
    };
    let Some(ObjectType::ObjectContextData(leaf)) = doc.objects.get_mut(&leaf_handle) else {
        return false;
    };
    match (&entity, &mut leaf.kind) {
        (
            EntityType::Insert(insert),
            ObjectContextKind::BlkRef {
                rotation,
                insertion,
                scale_factor,
            },
        ) => {
            *rotation = insert.rotation;
            *insertion = insert.insert_point;
            *scale_factor =
                Vector3::new(insert.x_scale(), insert.y_scale(), insert.z_scale());
        }
        (
            EntityType::Text(text),
            ObjectContextKind::Text {
                horizontal_mode,
                rotation,
                insertion,
                alignment,
            },
        ) => {
            *horizontal_mode = match text.horizontal_alignment {
                acadrust::entities::TextHorizontalAlignment::Left => 0,
                acadrust::entities::TextHorizontalAlignment::Center => 1,
                acadrust::entities::TextHorizontalAlignment::Right => 2,
                acadrust::entities::TextHorizontalAlignment::Aligned => 3,
                acadrust::entities::TextHorizontalAlignment::Middle => 4,
                acadrust::entities::TextHorizontalAlignment::Fit => 5,
            };
            *rotation = text.rotation;
            *insertion = Vector2::new(text.insertion_point.x, text.insertion_point.y);
            let point = text.alignment_point.unwrap_or(Vector3::ZERO);
            *alignment = Vector2::new(point.x, point.y);
        }
        (EntityType::MText(mtext), ObjectContextKind::MText(context)) => {
            sync_mtext_context(context, mtext);
        }
        (EntityType::Dimension(dimension), ObjectContextKind::Dim(context)) => {
            sync_dimension_context(context, dimension, dim_block_handle);
        }
        (EntityType::MultiLeader(mleader), ObjectContextKind::MLeader(context)) => {
            context.clone_from(&mleader.context);
        }
        (
            EntityType::AttributeEntity(attribute),
            ObjectContextKind::MTextAttribute(context),
        ) => {
            context.horizontal_mode = attribute.horizontal_alignment.to_value();
            context.rotation = attribute.rotation;
            context.insertion =
                Vector2::new(attribute.insertion_point.x, attribute.insertion_point.y);
            context.alignment =
                Vector2::new(attribute.alignment_point.x, attribute.alignment_point.y);
            if let (Some(embedded), Some(mtext)) =
                (&mut context.context, &attribute.embedded_mtext)
            {
                sync_mtext_context(&mut embedded.mtext, mtext);
            }
        }
        (
            EntityType::AttributeDefinition(attribute),
            ObjectContextKind::MTextAttribute(context),
        ) => {
            context.horizontal_mode = attribute.horizontal_alignment.to_value();
            context.rotation = attribute.rotation;
            context.insertion =
                Vector2::new(attribute.insertion_point.x, attribute.insertion_point.y);
            context.alignment =
                Vector2::new(attribute.alignment_point.x, attribute.alignment_point.y);
            if let (Some(embedded), Some(mtext)) =
                (&mut context.context, &attribute.embedded_mtext)
            {
                sync_mtext_context(&mut embedded.mtext, mtext);
            }
        }
        (EntityType::Leader(leader), ObjectContextKind::Leader(context)) => {
            context.points = leader
                .vertices
                .iter()
                .map(|point| *point - context.insertion_offset)
                .collect();
            context.x_direction = leader.horizontal_direction;
            context.endpoint_projection = leader.annotation_offset;
        }
        (
            EntityType::Tolerance(tolerance),
            ObjectContextKind::Fcf {
                location,
                horizontal_direction,
            },
        ) => {
            *location = tolerance.insertion_point;
            *horizontal_direction = tolerance.direction;
        }
        (EntityType::Hatch(hatch), ObjectContextKind::HatchScale(context)) => {
            context.pattern_lines.clone_from(&hatch.pattern.lines);
            for line in &mut context.pattern_lines {
                line.base_point.x -= context.pattern_base.x;
                line.base_point.y -= context.pattern_base.y;
            }
            context.pattern_scale = hatch.pattern_scale;
            context.loop_types = hatch
                .paths
                .iter()
                .map(|path| path.flags.bits() as i32)
                .collect();
        }
        (EntityType::Hatch(hatch), ObjectContextKind::HatchView(context)) => {
            context.hatch.pattern_lines.clone_from(&hatch.pattern.lines);
            for line in &mut context.hatch.pattern_lines {
                line.base_point.x -= context.hatch.pattern_base.x;
                line.base_point.y -= context.hatch.pattern_base.y;
            }
            context.hatch.pattern_scale = hatch.pattern_scale;
            context.hatch.loop_types = hatch
                .paths
                .iter()
                .map(|path| path.flags.bits() as i32)
                .collect();
            context.view_normal = hatch.normal;
        }
        _ => {}
    }
    true
}

/// Get the child dictionary stored under `key` in `parent_h`, creating an empty
/// one (owned by `parent_h`) and registering the entry when absent.
fn get_or_create_child_dict(doc: &mut CadDocument, parent_h: Handle, key: &str) -> Handle {
    if let Some(h) = as_dict(doc, parent_h).and_then(|d| d.get(key)) {
        return h;
    }
    let h = doc.allocate_handle();
    let mut d = Dictionary::new();
    d.handle = h;
    d.owner = parent_h;
    doc.objects.insert(h, ObjectType::Dictionary(d));
    if let Some(ObjectType::Dictionary(p)) = doc.objects.get_mut(&parent_h) {
        p.add_entry(key, h);
    }
    h
}

/// Remove an entity's per-object annotation context — the extension-dictionary
/// `AcDbContextDataManager` → `ACDB_ANNOTATIONSCALES` → per-scale leaf subtree —
/// and the legacy annotative XDATA markers, so [`is_annotative`] no longer fires
/// on it. The shared `SCALE` objects in `ACAD_SCALELIST` are document-level and
/// left intact.
pub fn clear_annotation_context(doc: &mut CadDocument, handle: Handle) {
    if let Some(xdict_h) = doc.get_entity(handle).and_then(|e| e.common().xdictionary_handle) {
        // Collect the manager subtree (manager dict, its scales dict, the leaves)
        // before mutating, then drop them.
        let mut remove = Vec::new();
        if let Some(mgr_h) = as_dict(doc, xdict_h).and_then(|d| d.get("AcDbContextDataManager")) {
            remove.push(mgr_h);
            if let Some(scales_h) =
                as_dict(doc, mgr_h).and_then(|d| d.get("ACDB_ANNOTATIONSCALES"))
            {
                remove.push(scales_h);
                if let Some(scales) = as_dict(doc, scales_h) {
                    for (_, leaf) in &scales.entries {
                        remove.push(*leaf);
                    }
                }
            }
        }
        if let Some(ObjectType::Dictionary(xd)) = doc.objects.get_mut(&xdict_h) {
            xd.entries.retain(|(k, _)| k != "AcDbContextDataManager");
        }
        for h in remove {
            doc.objects.remove(&h);
        }
    }
    // Strip the legacy annotative XDATA markers the detection also honours.
    crate::scene::view::dispatch::set_entity_xdata(doc, handle, "AcAnnoPO", None);
    crate::scene::view::dispatch::set_entity_xdata(doc, handle, "AcAnnotativeData", None);
}

/// Does a style name resolve to `name` (or to "Standard" when `name` is blank)?
fn name_matches(style_name: &str, name: &str) -> bool {
    style_name.eq_ignore_ascii_case(name)
        || (name.trim().is_empty() && style_name.eq_ignore_ascii_case("Standard"))
}

/// Whether `name` currently names an annotative text style.
///
/// Creation paths use this to stamp a new TEXT/MTEXT with its own annotation
/// context. Render-time detection deliberately does not use it: an existing
/// non-annotative object may still reference a style later made annotative.
pub fn text_style_is_annotative(doc: &CadDocument, name: &str) -> bool {
    doc.text_styles
        .iter()
        .find(|s| name_matches(&s.name, name))
        .is_some_and(|s| s.annotative)
}

fn dim_style_annotative(doc: &CadDocument, name: &str) -> bool {
    doc.dim_styles
        .iter()
        .find(|s| name_matches(&s.name, name))
        .is_some_and(|s| s.annotative)
}

fn mleader_style_annotative(doc: &CadDocument, handle: Option<Handle>) -> bool {
    let Some(h) = handle else {
        return false;
    };
    doc.objects.iter().any(|(oh, o)| {
        matches!(o, ObjectType::MultiLeaderStyle(s) if *oh == h && s.is_annotative)
    })
}

fn table_style_annotative(doc: &CadDocument, handle: Option<Handle>) -> bool {
    let Some(h) = handle else {
        return false;
    };
    doc.objects
        .iter()
        .any(|(oh, o)| matches!(o, ObjectType::TableStyle(s) if *oh == h && s.annotative))
}

/// Whether an object carries a per-object annotation context with at least one
/// per-scale representation — its extension dictionary holds an
/// `AcDbContextDataManager` whose `ACDB_ANNOTATIONSCALES` collection is
/// non-empty. This catches objects that are annotative by context even when
/// their style is not.
///
/// The non-empty requirement matters: a context manager with an *empty*
/// `ACDB_ANNOTATIONSCALES` is a single-representation marker with no per-scale
/// reps (common in files where objects were flagged annotative but never given
/// a scale). Such an object has nothing to scale *to*, so it must render at its
/// base geometry — treating it as annotative would (mis)scale it by the
/// annotation factor in annotation-scaled viewports, ballooning the text.
fn has_context_manager(doc: &CadDocument, common: &EntityCommon) -> bool {
    let key = |d: &Dictionary, name: &str| {
        d.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, h)| *h)
    };
    let Some(xd) = common.xdictionary_handle.and_then(|h| as_dict(doc, h)) else {
        return false;
    };
    let Some(mgr) = key(xd, "AcDbContextDataManager").and_then(|h| as_dict(doc, h)) else {
        return false;
    };
    key(mgr, "ACDB_ANNOTATIONSCALES")
        .and_then(|h| as_dict(doc, h))
        .map(|coll| !coll.entries.is_empty())
        .unwrap_or(false)
}

/// Whether an entity participates in annotation scaling.
pub fn is_annotative(doc: &CadDocument, entity: &EntityType) -> bool {
    // Per-object annotation context (works regardless of style).
    if has_context_manager(doc, entity.common()) {
        return true;
    }
    // Legacy annotative XDATA markers.
    let xd = &entity.common().extended_data;
    if xd.get_record("AcAnnoPO").is_some() || xd.get_record("AcAnnotativeData").is_some() {
        return true;
    }
    // Annotative via the entity's own flag or assigned non-text style.
    // Text styles can be made annotative without converting existing text;
    // those objects must keep their stored height until explicitly updated.
    match entity {
        EntityType::Text(_) => false,
        EntityType::MText(t) => t.is_annotative,
        EntityType::Dimension(d) => dim_style_annotative(doc, &d.base().style_name),
        EntityType::Leader(l) => dim_style_annotative(doc, &l.dimension_style),
        EntityType::MultiLeader(ml) => {
            ml.enable_annotation_scale || mleader_style_annotative(doc, ml.style_handle)
        }
        EntityType::Table(t) => table_style_annotative(doc, t.table_style_handle),
        _ => false,
    }
}
