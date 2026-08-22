use acadrust::entities::MLine;
use acadrust::objects::MLineStyle;
use acadrust::{EntityType, Handle};
use glam::{DVec2, DVec3};
use rustc_hash::FxHashMap as HashMap;

use crate::command::{CadCommand, CmdOption, CmdResult};

#[derive(Clone)]
pub struct MlineEditTarget {
    pub entity: MLine,
    pub style: MLineStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    ClosedCross,
    OpenCross,
    MergedCross,
    ClosedTee,
    OpenTee,
    MergedTee,
    CornerJoint,
    AddVertex,
    DeleteVertex,
    CutSingle,
    CutAll,
    WeldAll,
}

enum Mode {
    Choose,
    PickFirst(Tool),
    PickSecond {
        tool: Tool,
        first: Handle,
        first_pick: DVec3,
    },
    PickRangeEnd {
        tool: Tool,
        target: Handle,
        start: DVec3,
    },
}

pub struct MlineEditCommand {
    targets: HashMap<u64, MlineEditTarget>,
    mode: Mode,
}

impl MlineEditCommand {
    pub fn new(targets: HashMap<u64, MlineEditTarget>) -> Self {
        Self {
            targets,
            mode: Mode::Choose,
        }
    }

    fn target(&self, handle: Handle) -> Option<&MlineEditTarget> {
        self.targets.get(&handle.value())
    }

    fn replace(handle: Handle, mline: MLine) -> CmdResult {
        CmdResult::ReplaceMany(
            vec![(handle, vec![EntityType::MLine(mline)])],
            Vec::new(),
        )
    }

    fn edit_vertex(&self, tool: Tool, handle: Handle, point: DVec3) -> Option<CmdResult> {
        let target = self.target(handle)?;
        let mut mline = target.entity.clone();
        match tool {
            Tool::AddVertex => {
                let (segment, _, projected, _) = closest_segment(&mline, point)?;
                let insert = segment + 1;
                let mut vertex = mline.vertices[segment].clone();
                vertex.position = acadrust::types::Vector3::new(
                    projected.x,
                    projected.y,
                    projected.z,
                );
                mline.vertices.insert(insert, vertex);
                mline.rebuild_geometry();
            }
            Tool::DeleteVertex => {
                let vertex = closest_vertex(&mline, point)?;
                if !mline.remove_vertex(vertex) {
                    return None;
                }
            }
            _ => return None,
        }
        crate::modules::draw::draw::mline::sync_mline_element_parameters(
            &mut mline,
            &target.style,
        );
        Some(Self::replace(handle, mline))
    }

    fn edit_range(
        &self,
        tool: Tool,
        handle: Handle,
        start: DVec3,
        end: DVec3,
    ) -> Option<CmdResult> {
        let target = self.target(handle)?;
        let mut mline = target.entity.clone();
        let (segment, _, _, _) = closest_segment(&mline, start)?;
        let elements: Vec<usize> = match tool {
            Tool::CutSingle => vec![closest_element(&mline, segment, start)?],
            _ => (0..mline.style_element_count).collect(),
        };
        for element in elements {
            let (a, b) = element_segment(&mline, segment, element)?;
            let direction = b - a;
            let length = direction.length();
            if length <= 1.0e-10 {
                continue;
            }
            let unit = direction / length;
            let first = (start - a).dot(unit).clamp(0.0, length);
            let second = (end - a).dot(unit).clamp(0.0, length);
            let low = first.min(second);
            let high = first.max(second);
            let parameters = &mut mline.vertices[segment].segments[element].parameters;
            if tool == Tool::WeldAll {
                add_drawn_range(parameters, length, low, high);
            } else {
                remove_drawn_range(parameters, length, low, high);
            }
        }
        Some(Self::replace(handle, mline))
    }

    fn edit_pair(
        &self,
        tool: Tool,
        first_handle: Handle,
        first_pick: DVec3,
        second_handle: Handle,
        second_pick: DVec3,
    ) -> Option<CmdResult> {
        if first_handle == second_handle {
            return None;
        }
        let first_target = self.target(first_handle)?;
        let second_target = self.target(second_handle)?;
        let mut first = first_target.entity.clone();
        let mut second = second_target.entity.clone();
        let (first_segment, _, _, _) = closest_segment(&first, first_pick)?;
        let (second_segment, _, _, _) = closest_segment(&second, second_pick)?;
        let (intersection, first_fraction, second_fraction, sine) = segment_intersection(
            center_segment(&first, first_segment)?,
            center_segment(&second, second_segment)?,
        )?;
        let first_length = center_segment(&first, first_segment)?.1.distance(
            center_segment(&first, first_segment)?.0,
        );
        let second_length = center_segment(&second, second_segment)?.1.distance(
            center_segment(&second, second_segment)?.0,
        );
        let first_at = first_fraction * first_length;
        let second_at = second_fraction * second_length;
        let divisor = sine.abs().max(0.15);
        let first_gap = style_width(&second_target.style, second.scale_factor) * 0.5 / divisor;
        let second_gap = style_width(&first_target.style, first.scale_factor) * 0.5 / divisor;

        match tool {
            Tool::ClosedCross => {
                gap_elements(&mut first, first_segment, first_at, first_gap, None);
            }
            Tool::OpenCross => {
                gap_elements(&mut first, first_segment, first_at, first_gap, None);
                gap_elements(
                    &mut second,
                    second_segment,
                    second_at,
                    second_gap,
                    Some(outer_element_indices(&second_target.style)),
                );
            }
            Tool::MergedCross => {
                gap_elements(
                    &mut first,
                    first_segment,
                    first_at,
                    first_gap,
                    Some(inner_element_indices(&first_target.style)),
                );
                gap_elements(
                    &mut second,
                    second_segment,
                    second_at,
                    second_gap,
                    Some(inner_element_indices(&second_target.style)),
                );
            }
            Tool::ClosedTee | Tool::OpenTee | Tool::MergedTee => {
                move_closest_end(&mut first, first_pick, intersection);
                crate::modules::draw::draw::mline::sync_mline_element_parameters(
                    &mut first,
                    &first_target.style,
                );
                let elements = match tool {
                    Tool::ClosedTee => None,
                    Tool::OpenTee => Some(outer_element_indices(&second_target.style)),
                    Tool::MergedTee => Some(inner_element_indices(&second_target.style)),
                    _ => unreachable!(),
                };
                gap_elements(
                    &mut second,
                    second_segment,
                    second_at,
                    second_gap,
                    elements,
                );
            }
            Tool::CornerJoint => {
                move_closest_end(&mut first, first_pick, intersection);
                move_closest_end(&mut second, second_pick, intersection);
                crate::modules::draw::draw::mline::sync_mline_element_parameters(
                    &mut first,
                    &first_target.style,
                );
                crate::modules::draw::draw::mline::sync_mline_element_parameters(
                    &mut second,
                    &second_target.style,
                );
            }
            _ => return None,
        }

        Some(CmdResult::ReplaceMany(
            vec![
                (first_handle, vec![EntityType::MLine(first)]),
                (second_handle, vec![EntityType::MLine(second)]),
            ],
            Vec::new(),
        ))
    }
}

impl CadCommand for MlineEditCommand {
    fn name(&self) -> &'static str {
        "MLEDIT"
    }

    fn prompt(&self) -> String {
        match self.mode {
            Mode::Choose => "MLEDIT  Choose an edit tool:".to_string(),
            Mode::PickFirst(_) => "MLEDIT  Select first multiline:".to_string(),
            Mode::PickSecond { .. } => "MLEDIT  Select second multiline:".to_string(),
            Mode::PickRangeEnd { tool: Tool::WeldAll, .. } => {
                "MLEDIT  Specify the end of the weld range:".to_string()
            }
            Mode::PickRangeEnd { .. } => {
                "MLEDIT  Specify the second cut point:".to_string()
            }
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        if !matches!(self.mode, Mode::Choose) {
            return Vec::new();
        }
        vec![
            CmdOption::new("Closed Cross", "CC"),
            CmdOption::new("Open Cross", "OC"),
            CmdOption::new("Merged Cross", "MC"),
            CmdOption::new("Closed Tee", "CT"),
            CmdOption::new("Open Tee", "OT"),
            CmdOption::new("Merged Tee", "MT"),
            CmdOption::new("Corner Joint", "CJ"),
            CmdOption::new("Add Vertex", "AV"),
            CmdOption::new("Delete Vertex", "DV"),
            CmdOption::new("Cut Single", "CS"),
            CmdOption::new("Cut All", "CA"),
            CmdOption::new("Weld All", "WA"),
        ]
    }

    fn wants_text_input(&self) -> bool {
        matches!(self.mode, Mode::Choose)
    }

    fn needs_entity_pick(&self) -> bool {
        matches!(self.mode, Mode::PickFirst(_) | Mode::PickSecond { .. })
    }

    fn entity_pick_highlights_hover(&self) -> bool {
        self.needs_entity_pick()
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if !matches!(self.mode, Mode::Choose) {
            return None;
        }
        let tool = match text.trim().to_uppercase().as_str() {
            "CC" | "CLOSED CROSS" => Tool::ClosedCross,
            "OC" | "OPEN CROSS" => Tool::OpenCross,
            "MC" | "MERGED CROSS" => Tool::MergedCross,
            "CT" | "CLOSED TEE" => Tool::ClosedTee,
            "OT" | "OPEN TEE" => Tool::OpenTee,
            "MT" | "MERGED TEE" => Tool::MergedTee,
            "CJ" | "CORNER JOINT" => Tool::CornerJoint,
            "AV" | "ADD VERTEX" => Tool::AddVertex,
            "DV" | "DELETE VERTEX" => Tool::DeleteVertex,
            "CS" | "CUT SINGLE" => Tool::CutSingle,
            "CA" | "CUT ALL" => Tool::CutAll,
            "WA" | "WELD ALL" => Tool::WeldAll,
            _ => return None,
        };
        self.mode = Mode::PickFirst(tool);
        Some(CmdResult::NeedPoint)
    }

    fn on_entity_pick(&mut self, handle: Handle, point: DVec3) -> CmdResult {
        if self.target(handle).is_none() {
            return CmdResult::NeedPoint;
        }
        match self.mode {
            Mode::PickFirst(tool @ (Tool::AddVertex | Tool::DeleteVertex)) => self
                .edit_vertex(tool, handle, point)
                .unwrap_or(CmdResult::NeedPoint),
            Mode::PickFirst(tool @ (Tool::CutSingle | Tool::CutAll | Tool::WeldAll)) => {
                self.mode = Mode::PickRangeEnd {
                    tool,
                    target: handle,
                    start: point,
                };
                CmdResult::NeedPoint
            }
            Mode::PickFirst(tool) => {
                self.mode = Mode::PickSecond {
                    tool,
                    first: handle,
                    first_pick: point,
                };
                CmdResult::NeedPoint
            }
            Mode::PickSecond {
                tool,
                first,
                first_pick,
            } => self
                .edit_pair(tool, first, first_pick, handle, point)
                .unwrap_or(CmdResult::NeedPoint),
            _ => CmdResult::NeedPoint,
        }
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        match self.mode {
            Mode::PickRangeEnd {
                tool,
                target,
                start,
            } => self
                .edit_range(tool, target, start, point)
                .unwrap_or(CmdResult::NeedPoint),
            _ => CmdResult::NeedPoint,
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

fn center_segment(mline: &MLine, index: usize) -> Option<(DVec3, DVec3)> {
    let first = mline.vertices.get(index)?.position;
    let next = if index + 1 < mline.vertices.len() {
        index + 1
    } else if mline.is_closed() {
        0
    } else {
        return None;
    };
    let second = mline.vertices[next].position;
    Some((
        DVec3::new(first.x, first.y, first.z),
        DVec3::new(second.x, second.y, second.z),
    ))
}

fn closest_segment(mline: &MLine, point: DVec3) -> Option<(usize, f64, DVec3, f64)> {
    let count = if mline.is_closed() {
        mline.vertices.len()
    } else {
        mline.vertices.len().saturating_sub(1)
    };
    (0..count)
        .filter_map(|index| {
            let (a, b) = center_segment(mline, index)?;
            let delta = b - a;
            let length_squared = delta.length_squared();
            if length_squared <= 1.0e-20 {
                return None;
            }
            let fraction = ((point - a).dot(delta) / length_squared).clamp(0.0, 1.0);
            let projected = a + delta * fraction;
            Some((index, fraction, projected, projected.distance_squared(point)))
        })
        .min_by(|left, right| left.3.total_cmp(&right.3))
}

fn closest_vertex(mline: &MLine, point: DVec3) -> Option<usize> {
    mline
        .vertices
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left = DVec3::new(left.position.x, left.position.y, left.position.z)
                .distance_squared(point);
            let right = DVec3::new(right.position.x, right.position.y, right.position.z)
                .distance_squared(point);
            left.total_cmp(&right)
        })
        .map(|(index, _)| index)
}

fn element_segment(mline: &MLine, index: usize, element: usize) -> Option<(DVec3, DVec3)> {
    let next = if index + 1 < mline.vertices.len() {
        index + 1
    } else if mline.is_closed() {
        0
    } else {
        return None;
    };
    let point = |vertex: usize| -> Option<DVec3> {
        let item = mline.vertices.get(vertex)?;
        let offset = item.segments.get(element)?.parameters.first().copied()?;
        Some(DVec3::new(
            item.position.x + item.miter.x * offset,
            item.position.y + item.miter.y * offset,
            item.position.z + item.miter.z * offset,
        ))
    };
    Some((point(index)?, point(next)?))
}

fn closest_element(mline: &MLine, segment: usize, point: DVec3) -> Option<usize> {
    (0..mline.style_element_count)
        .filter_map(|element| {
            let (a, b) = element_segment(mline, segment, element)?;
            let delta = b - a;
            let fraction = ((point - a).dot(delta) / delta.length_squared().max(1.0e-20))
                .clamp(0.0, 1.0);
            Some((element, (a + delta * fraction).distance_squared(point)))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(element, _)| element)
}

fn drawn_ranges(parameters: &[f64], length: f64) -> Vec<(f64, f64)> {
    if parameters.len() <= 1 {
        return vec![(0.0, length)];
    }
    let toggles = &parameters[1..];
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < toggles.len() {
        let start = toggles[index].clamp(0.0, length);
        let end = toggles
            .get(index + 1)
            .copied()
            .unwrap_or(length)
            .clamp(0.0, length);
        if end - start > 1.0e-9 {
            ranges.push((start, end));
        }
        index += 2;
    }
    ranges
}

fn store_drawn_ranges(parameters: &mut Vec<f64>, length: f64, ranges: &[(f64, f64)]) {
    let offset = parameters.first().copied().unwrap_or(0.0);
    parameters.clear();
    parameters.push(offset);
    if ranges.len() == 1 && ranges[0].0 <= 1.0e-9 && ranges[0].1 >= length - 1.0e-9 {
        return;
    }
    for (start, end) in ranges {
        parameters.push(*start);
        if *end < length - 1.0e-9 {
            parameters.push(*end);
        }
    }
}

fn remove_drawn_range(parameters: &mut Vec<f64>, length: f64, low: f64, high: f64) {
    if high - low <= 1.0e-9 {
        return;
    }
    let mut result = Vec::new();
    for (start, end) in drawn_ranges(parameters, length) {
        if low > start + 1.0e-9 {
            result.push((start, low.min(end)));
        }
        if high < end - 1.0e-9 {
            result.push((high.max(start), end));
        }
    }
    store_drawn_ranges(parameters, length, &result);
}

fn add_drawn_range(parameters: &mut Vec<f64>, length: f64, low: f64, high: f64) {
    let mut ranges = drawn_ranges(parameters, length);
    ranges.push((low, high));
    ranges.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut() {
            if range.0 <= last.1 + 1.0e-9 {
                last.1 = last.1.max(range.1);
                continue;
            }
        }
        merged.push(range);
    }
    store_drawn_ranges(parameters, length, &merged);
}

fn style_width(style: &MLineStyle, scale: f64) -> f64 {
    let low = style
        .elements
        .iter()
        .map(|element| element.offset)
        .fold(f64::INFINITY, f64::min);
    let high = style
        .elements
        .iter()
        .map(|element| element.offset)
        .fold(f64::NEG_INFINITY, f64::max);
    if low.is_finite() && high.is_finite() {
        (high - low).abs() * scale.abs()
    } else {
        scale.abs()
    }
}

fn outer_element_indices(style: &MLineStyle) -> Vec<usize> {
    if style.elements.is_empty() {
        return Vec::new();
    }
    let low = style
        .elements
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| left.offset.total_cmp(&right.offset))
        .map(|(index, _)| index)
        .unwrap_or(0);
    let high = style
        .elements
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.offset.total_cmp(&right.offset))
        .map(|(index, _)| index)
        .unwrap_or(low);
    if low == high {
        vec![low]
    } else {
        vec![low, high]
    }
}

fn inner_element_indices(style: &MLineStyle) -> Vec<usize> {
    let outer = outer_element_indices(style);
    let inner: Vec<usize> = (0..style.elements.len())
        .filter(|index| !outer.contains(index))
        .collect();
    if inner.is_empty() {
        outer.into_iter().take(1).collect()
    } else {
        inner
    }
}

fn gap_elements(
    mline: &mut MLine,
    segment: usize,
    at: f64,
    half_gap: f64,
    elements: Option<Vec<usize>>,
) {
    let elements = elements.unwrap_or_else(|| (0..mline.style_element_count).collect());
    for element in elements {
        let Some((a, b)) = element_segment(mline, segment, element) else {
            continue;
        };
        let length = a.distance(b);
        if let Some(parameters) = mline.vertices[segment].segments.get_mut(element) {
            remove_drawn_range(
                &mut parameters.parameters,
                length,
                (at - half_gap).max(0.0),
                (at + half_gap).min(length),
            );
        }
    }
}

fn move_closest_end(mline: &mut MLine, pick: DVec3, intersection: DVec3) {
    if mline.vertices.is_empty() || mline.is_closed() {
        return;
    }
    let first = DVec3::new(
        mline.vertices[0].position.x,
        mline.vertices[0].position.y,
        mline.vertices[0].position.z,
    );
    let last_index = mline.vertices.len() - 1;
    let last = DVec3::new(
        mline.vertices[last_index].position.x,
        mline.vertices[last_index].position.y,
        mline.vertices[last_index].position.z,
    );
    let index = if first.distance_squared(pick) <= last.distance_squared(pick) {
        0
    } else {
        last_index
    };
    let _ = mline.set_vertex_position(
        index,
        acadrust::types::Vector3::new(intersection.x, intersection.y, intersection.z),
    );
}

fn segment_intersection(
    first: (DVec3, DVec3),
    second: (DVec3, DVec3),
) -> Option<(DVec3, f64, f64, f64)> {
    let p = first.0.truncate();
    let r = (first.1 - first.0).truncate();
    let q = second.0.truncate();
    let s = (second.1 - second.0).truncate();
    let cross = |left: DVec2, right: DVec2| left.x * right.y - left.y * right.x;
    let denominator = cross(r, s);
    if denominator.abs() <= 1.0e-10 {
        return None;
    }
    let t = cross(q - p, s) / denominator;
    let u = cross(q - p, r) / denominator;
    if !(-1.0e-6..=1.0 + 1.0e-6).contains(&t)
        || !(-1.0e-6..=1.0 + 1.0e-6).contains(&u)
    {
        return None;
    }
    let xy = p + r * t;
    let z = first.0.z + (first.1.z - first.0.z) * t;
    let sine = denominator / (r.length() * s.length()).max(1.0e-20);
    Some((DVec3::new(xy.x, xy.y, z), t, u, sine))
}

inventory::submit!(crate::command::CommandRegistration { names: &["MLEDIT"] });
