use acadrust::entities::MLine;

use crate::command::EntityTransform;
use crate::entities::common::{edit_prop as edit, ro_prop, square_grip};
use crate::entities::traits::{Grippable, PropertyEditable, RenderConvertible, Transformable};
use crate::scene::convert::acad_to_render::{RenderEntity, RenderObject};
use crate::scene::model::object::{GripApply, GripDef, PropSection, PropValue, Property};
use crate::scene::model::wire_model::SnapHint;
use crate::t;

/// One drawn line of a multiline: the polyline for a single style element (or
/// an end cap), tagged with the element's colour and linetype so the
/// tessellator can colour-bin and dash each one independently.
pub struct MLineLine {
    pub points: Vec<[f64; 3]>,
    pub color: acadrust::types::Color,
    pub linetype: String,
}

/// Resolve a multiline into its per-element parallel lines in WCS.
///
/// Geometry comes from the referenced MLINESTYLE (element offsets, the
/// justification shift and the entity scale) rather than a fixed ±scale/2 guess,
/// so a custom style's offsets, colours and linetypes render the way the drawing
/// intends. Falls back to a ±0.5 two-line layout only when no MLINESTYLE can be
/// resolved (e.g. the style object is missing).
pub fn resolved_mline_style<'a>(
    m: &MLine,
    document: &'a acadrust::CadDocument,
) -> Option<&'a acadrust::objects::MLineStyle> {
    use acadrust::objects::ObjectType;

    m.style_handle
        .and_then(|handle| match document.objects.get(&handle) {
            Some(ObjectType::MLineStyle(style)) => Some(style),
            _ => None,
        })
        .or_else(|| {
            document.objects.values().find_map(|object| match object {
                ObjectType::MLineStyle(style)
                    if style.name.eq_ignore_ascii_case(&m.style_name) =>
                {
                    Some(style)
                }
                _ => None,
            })
        })
}

pub fn mline_lines(m: &MLine, document: &acadrust::CadDocument) -> Vec<MLineLine> {
    mline_lines_resolved(m, resolved_mline_style(m, document))
}

pub fn mline_lines_with_style(
    m: &MLine,
    style: &acadrust::objects::MLineStyle,
) -> Vec<MLineLine> {
    mline_lines_resolved(m, Some(style))
}

fn mline_lines_resolved(
    m: &MLine,
    style: Option<&acadrust::objects::MLineStyle>,
) -> Vec<MLineLine> {
    use acadrust::entities::{MLineFlags, MLineJustification};
    use acadrust::types::Color;

    if m.vertices.is_empty() {
        return Vec::new();
    }

    // (offset, colour, linetype) per element.
    let elems: Vec<(f64, Color, String)> = match style {
        Some(s) if !s.elements.is_empty() => s
            .elements
            .iter()
            .map(|e| (e.offset, e.color, e.linetype.clone()))
            .collect(),
        _ => vec![
            (0.5, Color::ByLayer, "ByLayer".to_string()),
            (-0.5, Color::ByLayer, "ByLayer".to_string()),
        ],
    };

    // Justification shifts every element so the picked path runs along the top /
    // centre / bottom element of the style. (Only the fallback path needs it —
    // stored vertex parameters already bake justification in.)
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for (o, _, _) in &elems {
        lo = lo.min(*o);
        hi = hi.max(*o);
    }
    let shift = match m.justification {
        MLineJustification::Top => -hi,
        MLineJustification::Bottom => -lo,
        MLineJustification::Zero => 0.0,
    };

    let scale = m.scale_factor;
    let closed = m.flags.contains(MLineFlags::CLOSED);
    let n = m.vertices.len();

    // The element's miter-space offset at vertex `vi`: the stored per-vertex
    // parameter[0] when present — it is measured ALONG THE MITER, so corner
    // vertices already carry the 1/cos(θ/2) miter lengthening and the
    // justification shift (using the flat style offset instead pinched the
    // channel at every corner and made the ends look flared). The style
    // offset × scale is only the fallback for files without parameters.
    let elem_off = |vi: usize, ei: usize| -> f64 {
        m.vertices[vi]
            .segments
            .get(ei)
            .and_then(|sg| sg.parameters.first().copied())
            .unwrap_or_else(|| (elems[ei].0 + shift) * scale)
    };
    let off_pt = |vi: usize, d: f64| -> [f64; 3] {
        let v = &m.vertices[vi];
        [
            v.position.x + v.miter.x * d,
            v.position.y + v.miter.y * d,
            v.position.z + v.miter.z * d,
        ]
    };
    let endpoint_pt = |vi: usize, ei: usize| -> [f64; 3] {
        let point = off_pt(vi, elem_off(vi, ei));
        if closed || (vi != 0 && vi + 1 != n) {
            return point;
        }
        let Some(style) = style else {
            return point;
        };
        let angle = if vi == 0 {
            style.start_angle
        } else {
            style.end_angle
        };
        let tangent = glam::DVec3::new(
            m.vertices[vi].direction.x,
            m.vertices[vi].direction.y,
            m.vertices[vi].direction.z,
        )
        .normalize_or(glam::DVec3::X);
        let normal = glam::DVec3::new(m.normal.x, m.normal.y, m.normal.z)
            .normalize_or(glam::DVec3::Z);
        let transverse = normal.cross(tangent).normalize_or(glam::DVec3::Y);
        let base = glam::DVec3::new(
            m.vertices[vi].position.x,
            m.vertices[vi].position.y,
            m.vertices[vi].position.z,
        );
        let current = glam::DVec3::new(point[0], point[1], point[2]);
        let tangent_shift = if angle.tan().abs() > 1.0e-9 {
            (current - base).dot(transverse) / angle.tan()
        } else {
            0.0
        };
        let adjusted = current + tangent * tangent_shift;
        [adjusted.x, adjusted.y, adjusted.z]
    };

    let mut out: Vec<MLineLine> = Vec::with_capacity(elems.len() + 2);
    for (ei, (_, color, linetype)) in elems.iter().enumerate() {
        let mut pts: Vec<[f64; 3]> = Vec::new();
        // Walk each segment (vi → vi+1, wrapping when closed); the vertex's
        // parameters[1..] are draw-toggle distances along the element line —
        // this is how crossing/merged multilines store their gaps. An odd
        // toggle count leaves the final run open to the segment end.
        let seg_count = if closed { n } else { n.saturating_sub(1) };
        let mut pen_at_end = false;
        for k in 0..seg_count {
            let vi = k;
            let wi = (k + 1) % n;
            let a = if !closed && vi == 0 {
                endpoint_pt(vi, ei)
            } else {
                off_pt(vi, elem_off(vi, ei))
            };
            let b = if !closed && wi + 1 == n {
                endpoint_pt(wi, ei)
            } else {
                off_pt(wi, elem_off(wi, ei))
            };
            let seg = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let len = (seg[0] * seg[0] + seg[1] * seg[1] + seg[2] * seg[2]).sqrt();
            if len < 1e-12 {
                continue;
            }
            let dirn = [seg[0] / len, seg[1] / len, seg[2] / len];
            let at = |t: f64| -> [f64; 3] {
                [a[0] + dirn[0] * t, a[1] + dirn[1] * t, a[2] + dirn[2] * t]
            };
            let toggles: &[f64] = m
                .vertices[vi]
                .segments
                .get(ei)
                .map(|sg| sg.parameters.get(1..).unwrap_or(&[]))
                .unwrap_or(&[]);
            // Build (start, end) runs from the toggle list.
            let mut runs: Vec<(f64, f64)> = Vec::new();
            if toggles.is_empty() {
                runs.push((0.0, len));
            } else {
                let mut i = 0;
                while i < toggles.len() {
                    let start = toggles[i].clamp(0.0, len);
                    let end = toggles
                        .get(i + 1)
                        .copied()
                        .unwrap_or(len)
                        .clamp(0.0, len);
                    if end - start > 1e-9 {
                        runs.push((start, end));
                    }
                    i += 2;
                }
            }
            for (ri, (t0, t1)) in runs.iter().enumerate() {
                let continuous = pen_at_end && ri == 0 && *t0 <= 1e-9 && !pts.is_empty();
                if !continuous {
                    if !pts.is_empty() {
                        pts.push([f64::NAN; 3]);
                    }
                    pts.push(at(*t0));
                }
                pts.push(at(*t1));
            }
            pen_at_end = runs.last().is_some_and(|(_, t1)| (len - t1).abs() <= 1e-9);
        }
        if pts.len() < 2 {
            continue;
        }
        out.push(MLineLine {
            points: pts,
            color: *color,
            linetype: linetype.clone(),
        });
    }

    // Style-defined joints and end caps.
    if let Some(s) = style {
        let outer_points = |vi: usize, endpoint: bool| -> Option<([f64; 3], [f64; 3])> {
            let mut order: Vec<usize> = (0..elems.len()).collect();
            order.sort_by(|a, b| elem_off(vi, *a).total_cmp(&elem_off(vi, *b)));
            let first = *order.first()?;
            let last = *order.last()?;
            let point = |ei| {
                if endpoint {
                    endpoint_pt(vi, ei)
                } else {
                    off_pt(vi, elem_off(vi, ei))
                }
            };
            Some((point(first), point(last)))
        };

        if s.flags.display_joints {
            let vertices: Box<dyn Iterator<Item = usize>> = if closed {
                Box::new(0..n)
            } else {
                Box::new(1..n.saturating_sub(1))
            };
            for vi in vertices {
                if let Some((a, b)) = outer_points(vi, false) {
                    out.push(MLineLine {
                        points: vec![a, b],
                        color: Color::ByLayer,
                        linetype: "ByLayer".to_string(),
                    });
                }
            }
        }

        if !closed && n >= 2 {
            let start_suppressed = m.flags.contains(MLineFlags::NO_START_CAPS);
            let end_suppressed = m.flags.contains(MLineFlags::NO_END_CAPS);
            for (vi, start, suppressed, square, inner, round) in [
                (
                    0,
                    true,
                    start_suppressed,
                    s.flags.start_square_cap,
                    s.flags.start_inner_arcs_cap,
                    s.flags.start_round_cap,
                ),
                (
                    n - 1,
                    false,
                    end_suppressed,
                    s.flags.end_square_cap,
                    s.flags.end_inner_arcs_cap,
                    s.flags.end_round_cap,
                ),
            ] {
                if suppressed {
                    continue;
                }
                let Some((a, b)) = outer_points(vi, true) else {
                    continue;
                };
                if square {
                    out.push(MLineLine {
                        points: vec![a, b],
                        color: Color::ByLayer,
                        linetype: "ByLayer".to_string(),
                    });
                }
                let direction = glam::DVec3::new(
                    m.vertices[vi].direction.x,
                    m.vertices[vi].direction.y,
                    m.vertices[vi].direction.z,
                )
                .normalize_or(glam::DVec3::X);
                if round {
                    out.push(MLineLine {
                        points: semicircle_cap(a, b, direction, start),
                        color: Color::ByLayer,
                        linetype: "ByLayer".to_string(),
                    });
                }
                if inner && elems.len() > 2 {
                    let mut order: Vec<usize> = (0..elems.len()).collect();
                    order.sort_by(|left, right| {
                        elem_off(vi, *left).total_cmp(&elem_off(vi, *right))
                    });
                    for pair in order.windows(2) {
                        out.push(MLineLine {
                            points: semicircle_cap(
                                endpoint_pt(vi, pair[0]),
                                endpoint_pt(vi, pair[1]),
                                direction,
                                start,
                            ),
                            color: Color::ByLayer,
                            linetype: "ByLayer".to_string(),
                        });
                    }
                }
            }
        }
    }

    out
}

fn semicircle_cap(
    first: [f64; 3],
    second: [f64; 3],
    direction: glam::DVec3,
    start: bool,
) -> Vec<[f64; 3]> {
    let first = glam::DVec3::from_array(first);
    let second = glam::DVec3::from_array(second);
    let center = (first + second) * 0.5;
    let transverse = (first - second) * 0.5;
    let radius = transverse.length();
    let outward = if start { -direction } else { direction } * radius;
    (0..=24)
        .map(|step| {
            let angle = std::f64::consts::PI * step as f64 / 24.0;
            (center + transverse * angle.cos() + outward * angle.sin()).to_array()
        })
        .collect()
}

pub fn mline_fill_triangles_with_style(
    m: &MLine,
    style: &acadrust::objects::MLineStyle,
) -> Vec<[f64; 3]> {
    use acadrust::entities::MLineFlags;

    if !style.flags.fill_on || m.vertices.len() < 2 || style.elements.len() < 2 {
        return Vec::new();
    }
    let (low_index, high_index) = style
        .elements
        .iter()
        .enumerate()
        .fold((0, 0), |(low, high), (index, element)| {
            let low = if element.offset < style.elements[low].offset {
                index
            } else {
                low
            };
            let high = if element.offset > style.elements[high].offset {
                index
            } else {
                high
            };
            (low, high)
        });
    let offset_point = |vertex: usize, element: usize| -> [f64; 3] {
        let item = &m.vertices[vertex];
        let distance = item
            .segments
            .get(element)
            .and_then(|segment| segment.parameters.first())
            .copied()
            .unwrap_or(style.elements[element].offset * m.scale_factor);
        [
            item.position.x + item.miter.x * distance,
            item.position.y + item.miter.y * distance,
            item.position.z + item.miter.z * distance,
        ]
    };
    let closed = m.flags.contains(MLineFlags::CLOSED);
    let segment_count = if closed {
        m.vertices.len()
    } else {
        m.vertices.len() - 1
    };
    let mut triangles = Vec::with_capacity(segment_count * 6);
    for vertex in 0..segment_count {
        let next = (vertex + 1) % m.vertices.len();
        let a = offset_point(vertex, low_index);
        let b = offset_point(vertex, high_index);
        let c = offset_point(next, high_index);
        let d = offset_point(next, low_index);
        triangles.extend([a, b, c, a, c, d]);
    }
    triangles
}

impl RenderConvertible for MLine {
    fn to_render(&self, document: &acadrust::CadDocument) -> Option<RenderEntity> {
        if self.vertices.is_empty() {
            return None;
        }

        // NaN-separated flat list of every element line (single-colour path used
        // by pick and the edit commands; the coloured render is built in
        // `tessellate`, which special-cases MLINE).
        let lines = mline_lines(self, document);
        let mut pts: Vec<[f64; 3]> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                pts.push([f64::NAN; 3]);
            }
            pts.extend_from_slice(&l.points);
        }

        let key_verts: Vec<[f64; 3]> = self
            .vertices
            .iter()
            .map(|v| [v.position.x, v.position.y, v.position.z])
            .collect();

        let snap_pts = self
            .vertices
            .iter()
            .map(|v| {
                (
                    glam::DVec3::new(v.position.x, v.position.y, v.position.z),
                    SnapHint::Node,
                )
            })
            .collect();

        Some(RenderEntity {
            pick_tris: Vec::new(),
            object: RenderObject::Lines(pts),
            snap_pts,
            tangent_geoms: vec![],
            key_vertices: key_verts,
            fill_tris: vec![],
        })
    }
}

impl Grippable for MLine {
    fn grips(&self) -> Vec<GripDef> {
        self.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                square_grip(
                    i,
                    glam::DVec3::new(v.position.x, v.position.y, v.position.z),
                )
            })
            .collect()
    }

    fn apply_grip(&mut self, grip_id: usize, apply: GripApply) {
        let Some(vertex) = self.vertices.get(grip_id) else {
            return;
        };
        let position = match apply {
            GripApply::Translate(delta) => acadrust::types::Vector3::new(
                vertex.position.x + delta.x as f64,
                vertex.position.y + delta.y as f64,
                vertex.position.z + delta.z as f64,
            ),
            GripApply::Absolute(point) => {
                acadrust::types::Vector3::new(point.x as f64, point.y as f64, point.z as f64)
            }
        };
        let offsets = mline_perpendicular_offsets(self);
        if self.set_vertex_position(grip_id, position) {
            restore_mline_offsets(self, &offsets);
        }
    }

    fn grip_menu(&self, _grip_id: usize) -> Vec<crate::scene::model::object::GripMenuItem> {
        use crate::scene::model::object::{GripMenuAction, GripMenuItem};
        vec![
            GripMenuItem {
                label: "Stretch",
                action: GripMenuAction::Stretch,
            },
            GripMenuItem {
                label: "Add Vertex",
                action: GripMenuAction::AddVertex,
            },
            GripMenuItem {
                label: "Remove Vertex",
                action: GripMenuAction::RemoveVertex,
            },
        ]
    }

    fn apply_grip_menu(&mut self, grip_id: usize, action: crate::scene::model::object::GripMenuAction) {
        use crate::scene::model::object::GripMenuAction as A;
        let n = self.vertices.len();
        match action {
            A::AddVertex if grip_id < n => {
                let i1 = (grip_id + 1).min(n - 1);
                if i1 == grip_id {
                    return;
                }
                let v0 = &self.vertices[grip_id];
                let v1 = &self.vertices[i1];
                let mut new_v = v0.clone();
                new_v.position.x = (v0.position.x + v1.position.x) * 0.5;
                new_v.position.y = (v0.position.y + v1.position.y) * 0.5;
                new_v.position.z = (v0.position.z + v1.position.z) * 0.5;
                self.vertices.insert(i1, new_v);
                let offsets = mline_perpendicular_offsets(self);
                self.rebuild_geometry();
                restore_mline_offsets(self, &offsets);
            }
            A::RemoveVertex if grip_id < n && n > 2 => {
                let mut offsets = mline_perpendicular_offsets(self);
                self.vertices.remove(grip_id);
                offsets.remove(grip_id);
                self.rebuild_geometry();
                restore_mline_offsets(self, &offsets);
            }
            _ => {}
        }
    }
}

impl PropertyEditable for MLine {
    fn geometry_properties(&self, _text_style_names: &[String]) -> Vec<PropSection> {
        let just_str = match self.justification {
            acadrust::entities::MLineJustification::Top => "Top",
            acadrust::entities::MLineJustification::Zero => "Zero",
            acadrust::entities::MLineJustification::Bottom => "Bottom",
        };
        vec![PropSection {
            title: t!("Misc").into_owned(),
            props: vec![
                ro_prop(t!("Style").as_ref(), "ml_style", self.style_name.clone()),
                Property {
                    label: t!("Style justification").into_owned(),
                    field: "ml_justification",
                    value: PropValue::Choice {
                        selected: just_str.to_string(),
                        options: ["Top", "Zero", "Bottom"]
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    },
                },
                edit(t!("Scale").as_ref(), "ml_scale", self.scale_factor),
            ],
        }]
    }

    fn apply_geom_prop(&mut self, field: &str, value: &str) {
        match field {
            "ml_closed" => {
                let closed = if value == "toggle" {
                    !self.flags.contains(acadrust::entities::MLineFlags::CLOSED)
                } else {
                    value == "true"
                };
                self.flags
                    .set(acadrust::entities::MLineFlags::CLOSED, closed);
                return;
            }
            "ml_justification" => {
                self.justification = match value {
                    "Top" => acadrust::entities::MLineJustification::Top,
                    "Bottom" => acadrust::entities::MLineJustification::Bottom,
                    _ => acadrust::entities::MLineJustification::Zero,
                };
                return;
            }
            _ => {}
        }
        let Ok(v) = value.trim().parse::<f64>() else {
            return;
        };
        if field == "ml_scale" {
            set_mline_scale(self, v);
        }
    }
}

fn set_mline_scale(mline: &mut MLine, scale: f64) {
    let old = mline.scale_factor;
    if !scale.is_finite() || !old.is_finite() {
        return;
    }
    if old == 0.0 {
        mline.scale_factor = scale;
        return;
    }
    let ratio = scale / old;
    if !ratio.is_finite() {
        return;
    }
    if mline
        .vertices
        .iter()
        .flat_map(|vertex| &vertex.segments)
        .filter_map(|segment| segment.parameters.first())
        .any(|offset| !offset.is_finite() || !(offset * ratio).is_finite())
    {
        return;
    }
    for segment in mline
        .vertices
        .iter_mut()
        .flat_map(|vertex| vertex.segments.iter_mut())
    {
        if let Some(offset) = segment.parameters.first_mut() {
            *offset *= ratio;
        }
    }
    mline.scale_factor = scale;
}

fn mline_vertex_factor(mline: &MLine, index: usize) -> f64 {
    let vertex = &mline.vertices[index];
    let normal = glam::DVec3::new(mline.normal.x, mline.normal.y, mline.normal.z)
        .normalize_or(glam::DVec3::Z);
    let direction = glam::DVec3::new(vertex.direction.x, vertex.direction.y, vertex.direction.z)
        .normalize_or(glam::DVec3::X);
    let miter = glam::DVec3::new(vertex.miter.x, vertex.miter.y, vertex.miter.z)
        .normalize_or(glam::DVec3::Y);
    miter.dot(normal.cross(direction)).abs().max(1.0e-9)
}

fn mline_perpendicular_offsets(mline: &MLine) -> Vec<Vec<Option<f64>>> {
    mline
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            let factor = mline_vertex_factor(mline, index);
            vertex
                .segments
                .iter()
                .map(|segment| segment.parameters.first().map(|value| value * factor))
                .collect()
        })
        .collect()
}

fn restore_mline_offsets(mline: &mut MLine, offsets: &[Vec<Option<f64>>]) {
    for index in 0..mline.vertices.len().min(offsets.len()) {
        let factor = mline_vertex_factor(mline, index);
        for (segment, offset) in mline.vertices[index]
            .segments
            .iter_mut()
            .zip(&offsets[index])
        {
            if let Some(offset) = offset {
                if let Some(first) = segment.parameters.first_mut() {
                    *first = *offset / factor;
                }
            }
        }
    }
}

impl Transformable for MLine {
    fn apply_transform(&mut self, t: &EntityTransform) {
        crate::scene::view::transform::apply_standard_entity_transform(self, t, |entity, p1, p2| {
            for v in &mut entity.vertices {
                crate::scene::view::transform::reflect_xy_point(
                    &mut v.position.x,
                    &mut v.position.y,
                    p1,
                    p2,
                );
            }
            crate::scene::view::transform::reflect_xy_point(
                &mut entity.start_point.x,
                &mut entity.start_point.y,
                p1,
                p2,
            );
        });
    }
}
