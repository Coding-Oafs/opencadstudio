// Freehand sketch tool — interactive command.
//
// A sketch is recorded from cursor positions in the active working plane.
// The persisted SKPOLY setting chooses line segments, a lightweight polyline,
// or a fit-point spline. Multiple temporary strokes may be recorded while the
// command remains active; Exit records the remaining strokes and finishes,
// while Quit discards only the unrecorded strokes.

use acadrust::entities::{Line, LwPolyline, LwVertex, Spline};
use acadrust::types::{Vector2, Vector3};
use acadrust::EntityType;
use glam::DVec3;

use crate::command::{CadCommand, CmdOption, CmdResult, WorkingPlane};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::scene::model::wire_model::WireModel;
use crate::t;

const MIN_INCREMENT: f64 = 1.0e-9;

#[allow(dead_code)]
pub fn tool() -> ToolDef {
    ToolDef {
        id: "SKETCH",
        label: "Sketch",
        icon: IconKind::Svg(include_bytes!("../../../../assets/icons/line.svg")),
        event: ModuleEvent::Command("SKETCH".to_string()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SketchType {
    Line,
    Polyline,
    Spline,
}

impl SketchType {
    fn from_header(value: i16) -> Self {
        match value {
            0 => Self::Line,
            2 => Self::Spline,
            _ => Self::Polyline,
        }
    }

    fn header_value(self) -> i16 {
        match self {
            Self::Line => 0,
            Self::Polyline => 1,
            Self::Spline => 2,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Polyline => "Polyline",
            Self::Spline => "Spline",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "L" | "LINE" => Some(Self::Line),
            "P" | "PLINE" | "POLYLINE" => Some(Self::Polyline),
            "S" | "SPLINE" => Some(Self::Spline),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputStage {
    Drawing,
    Type,
    Increment,
    Tolerance,
}

pub struct SketchCommand {
    strokes: Vec<Vec<DVec3>>,
    pen_down: bool,
    erasing: bool,
    erase_engaged: bool,
    stage: InputStage,
    sketch_type: SketchType,
    increment: f64,
    tolerance: f64,
    last_cursor: Option<DVec3>,
    plane: WorkingPlane,
}

impl SketchCommand {
    pub fn new(sketch_type: i16, increment: f64, tolerance: f64) -> Self {
        Self {
            strokes: Vec::new(),
            pen_down: false,
            erasing: false,
            erase_engaged: false,
            stage: InputStage::Drawing,
            sketch_type: SketchType::from_header(sketch_type),
            increment: valid_increment(increment),
            tolerance: valid_tolerance(tolerance),
            last_cursor: None,
            plane: WorkingPlane::default(),
        }
    }

    fn begin_stroke(&mut self, point: DVec3) {
        self.strokes.push(vec![point]);
        self.pen_down = true;
        self.erasing = false;
        self.erase_engaged = false;
    }

    /// Record the actual pointer position after it has moved by at least the
    /// configured increment. The increment is a minimum recording distance,
    /// not a segment length: no synthetic points are inserted between pointer
    /// events, so every chord ends at a real movement position.
    fn sample_to(&mut self, point: DVec3) {
        let Some(stroke) = self.strokes.last_mut() else {
            return;
        };
        let Some(cursor) = stroke.last().copied() else {
            stroke.push(point);
            return;
        };
        let distance = cursor.distance(point);
        if distance.is_finite() && distance + 1.0e-12 >= self.increment {
            stroke.push(point);
        }
    }

    fn finish_stroke(&mut self, point: DVec3) {
        self.sample_to(point);
        if let Some(stroke) = self.strokes.last_mut() {
            if stroke
                .last()
                .is_some_and(|last| last.distance(point) > 1.0e-9)
            {
                stroke.push(point);
            }
        }
        self.pen_down = false;
    }

    fn connect(&mut self) {
        let endpoint = self
            .strokes
            .iter()
            .rev()
            .find_map(|stroke| stroke.last().copied());
        if let Some(endpoint) = endpoint {
            self.begin_stroke(endpoint);
        }
    }

    /// Erasing is armed when the pointer reaches the latest temporary sample.
    /// Moving back over the recorded path then removes samples one by one.
    fn erase_at(&mut self, point: DVec3) {
        let threshold = (self.increment * 0.45).max(1.0e-6);
        loop {
            let Some(stroke) = self.strokes.last_mut() else {
                self.erase_engaged = false;
                break;
            };
            let Some(last) = stroke.last().copied() else {
                self.strokes.pop();
                continue;
            };
            if !self.erase_engaged {
                if last.distance(point) <= threshold {
                    self.erase_engaged = true;
                } else {
                    break;
                }
            }
            if last.distance(point) <= threshold {
                stroke.pop();
                if stroke.is_empty() {
                    self.strokes.pop();
                    self.erase_engaged = false;
                }
            } else {
                break;
            }
        }
    }

    fn build_stroke(&self, points: &[DVec3]) -> Vec<EntityType> {
        if points.len() < 2 {
            return Vec::new();
        }
        let local: Vec<DVec3> = points
            .iter()
            .map(|point| self.plane.to_local(*point))
            .collect();
        match self.sketch_type {
            SketchType::Line => local
                .windows(2)
                .map(|pair| {
                    self.plane.place_entity(EntityType::Line(Line::from_points(
                        to_vector3(pair[0]),
                        to_vector3(pair[1]),
                    )))
                })
                .collect(),
            SketchType::Polyline => {
                let mut polyline = LwPolyline::new();
                polyline.elevation = local[0].z;
                polyline.normal = Vector3::UNIT_Z;
                polyline.vertices = local
                    .iter()
                    .map(|point| LwVertex::new(Vector2::new(point.x, point.y)))
                    .collect();
                vec![self
                    .plane
                    .place_entity(EntityType::LwPolyline(polyline))]
            }
            SketchType::Spline => {
                let fit = simplify_points(&local, self.tolerance);
                if fit.len() < 2 {
                    return Vec::new();
                }
                let mut spline = Spline {
                    degree: (fit.len().saturating_sub(1).min(3)) as i32,
                    fit_points: fit.iter().copied().map(to_vector3).collect(),
                    fit_tolerance: self.tolerance,
                    normal: Vector3::UNIT_Z,
                    ..Default::default()
                };
                spline.flags.planar = true;
                vec![self.plane.place_entity(EntityType::Spline(spline))]
            }
        }
    }

    fn build_all(&self) -> Vec<EntityType> {
        self.strokes
            .iter()
            .flat_map(|stroke| self.build_stroke(stroke))
            .collect()
    }

    fn take_entities(&mut self) -> Vec<EntityType> {
        let entities = self.build_all();
        self.strokes.clear();
        self.pen_down = false;
        self.erasing = false;
        self.erase_engaged = false;
        entities
    }

    fn preview(&self, cursor: Option<DVec3>) -> Option<WireModel> {
        let mut combined = Vec::<[f64; 3]>::new();
        for (index, stroke) in self.strokes.iter().enumerate() {
            let mut points = stroke.clone();
            if self.pen_down && index + 1 == self.strokes.len() {
                if let Some(cursor) = cursor {
                    if points
                        .last()
                        .is_some_and(|last| last.distance(cursor) > 1.0e-9)
                    {
                        points.push(cursor);
                    }
                }
            }
            if points.len() < 2 {
                continue;
            }
            let display = if self.sketch_type == SketchType::Spline {
                spline_preview_points(&simplify_points(&points, self.tolerance))
            } else {
                points
            };
            if display.len() < 2 {
                continue;
            }
            if !combined.is_empty() {
                combined.push([f64::NAN; 3]);
            }
            combined.extend(display.into_iter().map(|point| [point.x, point.y, point.z]));
        }
        (combined.len() >= 2).then(|| {
            WireModel::solid_f64("sketch_preview".to_string(), combined, WireModel::CYAN, false)
        })
    }

    fn toggle_pen(&mut self) {
        if self.pen_down {
            if let Some(cursor) = self.last_cursor {
                self.finish_stroke(cursor);
            } else {
                self.pen_down = false;
            }
        } else if let Some(cursor) = self.last_cursor {
            self.begin_stroke(cursor);
        }
    }

    fn drawing_options(&self) -> Vec<CmdOption> {
        let mut options = vec![
            CmdOption::new("Pen", "PEN"),
            CmdOption::new("Type", "TYPE"),
            CmdOption::new("Increment", "INCREMENT"),
        ];
        if self.sketch_type == SketchType::Spline {
            options.push(CmdOption::new("Tolerance", "TOLERANCE"));
        }
        options.extend([
            CmdOption::new("Record", "RECORD"),
            CmdOption::new(if self.erasing { "Stop erasing" } else { "Erase" }, "ERASE"),
            CmdOption::new("Connect", "CONNECT"),
            CmdOption::new("Exit", "EXIT"),
            CmdOption::new("Quit", "QUIT"),
        ]);
        options
    }
}

impl CadCommand for SketchCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "SKETCH"
    }

    fn prompt(&self) -> String {
        match self.stage {
            InputStage::Type => t!("SKETCH  Object type [Line/Polyline/Spline]:").into_owned(),
            InputStage::Increment => t!(
                "SKETCH  Record increment <%{value}>:",
                value = self.increment
            )
            .into_owned(),
            InputStage::Tolerance => t!(
                "SKETCH  Spline fit tolerance <%{value}>:",
                value = self.tolerance
            )
            .into_owned(),
            InputStage::Drawing if self.erasing => {
                t!("SKETCH  Erase — move backward over the temporary sketch:").into_owned()
            }
            InputStage::Drawing if self.pen_down => t!(
                "SKETCH (%{type}, increment %{increment})  Pen down — move to sketch:",
                type = self.sketch_type.label(),
                increment = self.increment
            )
            .into_owned(),
            InputStage::Drawing => t!(
                "SKETCH (%{type}, increment %{increment})  Pen up — click to lower the pen:",
                type = self.sketch_type.label(),
                increment = self.increment
            )
            .into_owned(),
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        match self.stage {
            InputStage::Drawing => self.drawing_options(),
            InputStage::Type => vec![
                CmdOption::new("Line", "LINE"),
                CmdOption::new("Polyline", "POLYLINE"),
                CmdOption::new("Spline", "SPLINE"),
            ],
            InputStage::Increment | InputStage::Tolerance => Vec::new(),
        }
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        self.last_cursor = Some(point);
        if self.stage != InputStage::Drawing {
            return CmdResult::NeedPoint;
        }
        if self.erasing {
            self.erasing = false;
            self.erase_engaged = false;
        } else if self.pen_down {
            self.finish_stroke(point);
        } else {
            self.begin_stroke(point);
        }
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        let entities = self.take_entities();
        if entities.is_empty() {
            CmdResult::Cancel
        } else {
            CmdResult::CommitEntitiesAndExit(entities)
        }
    }

    fn on_escape(&mut self) -> CmdResult {
        self.strokes.clear();
        CmdResult::Cancel
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn point_step_accepts_keywords(&self) -> bool {
        self.stage == InputStage::Drawing
    }

    fn sketch_settings(&self) -> Option<(i16, f64, f64)> {
        Some((
            self.sketch_type.header_value(),
            self.increment,
            self.tolerance,
        ))
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let input = text.trim();
        if input.is_empty() {
            return None;
        }
        match self.stage {
            InputStage::Type => {
                if let Some(sketch_type) = SketchType::parse(input) {
                    self.sketch_type = sketch_type;
                    self.stage = InputStage::Drawing;
                }
                Some(CmdResult::NeedPoint)
            }
            InputStage::Increment => {
                if let Ok(value) = input.parse::<f64>() {
                    if value.is_finite() && value > 0.0 {
                        self.increment = value.max(MIN_INCREMENT);
                        self.stage = InputStage::Drawing;
                    }
                }
                Some(CmdResult::NeedPoint)
            }
            InputStage::Tolerance => {
                if let Ok(value) = input.parse::<f64>() {
                    if value.is_finite() && value >= 0.0 {
                        self.tolerance = value;
                        self.stage = InputStage::Drawing;
                    }
                }
                Some(CmdResult::NeedPoint)
            }
            InputStage::Drawing => {
                let keyword = input.to_ascii_uppercase();
                let result = match keyword.as_str() {
                    "P" | "PEN" => {
                        self.toggle_pen();
                        CmdResult::NeedPoint
                    }
                    "T" | "TYPE" => {
                        self.pen_down = false;
                        self.erasing = false;
                        self.stage = InputStage::Type;
                        CmdResult::NeedPoint
                    }
                    "I" | "INCREMENT" => {
                        self.pen_down = false;
                        self.erasing = false;
                        self.stage = InputStage::Increment;
                        CmdResult::NeedPoint
                    }
                    "TO" | "TOLERANCE" if self.sketch_type == SketchType::Spline => {
                        self.pen_down = false;
                        self.erasing = false;
                        self.stage = InputStage::Tolerance;
                        CmdResult::NeedPoint
                    }
                    "R" | "RECORD" => {
                        let entities = self.take_entities();
                        if entities.is_empty() {
                            CmdResult::NeedPoint
                        } else {
                            CmdResult::CommitEntities(entities)
                        }
                    }
                    "E" | "ERASE" => {
                        self.pen_down = false;
                        self.erasing = !self.erasing;
                        self.erase_engaged = false;
                        CmdResult::NeedPoint
                    }
                    "C" | "CONNECT" => {
                        self.connect();
                        CmdResult::NeedPoint
                    }
                    "X" | "EXIT" => {
                        let entities = self.take_entities();
                        if entities.is_empty() {
                            CmdResult::Cancel
                        } else {
                            CmdResult::CommitEntitiesAndExit(entities)
                        }
                    }
                    "Q" | "QUIT" => {
                        self.strokes.clear();
                        CmdResult::Cancel
                    }
                    _ => {
                        if let Some(sketch_type) = SketchType::parse(&keyword) {
                            self.sketch_type = sketch_type;
                        }
                        CmdResult::NeedPoint
                    }
                };
                Some(result)
            }
        }
    }

    fn on_mouse_move(&mut self, point: DVec3) -> Option<WireModel> {
        self.last_cursor = Some(point);
        if self.stage == InputStage::Drawing {
            if self.pen_down {
                self.sample_to(point);
            } else if self.erasing {
                self.erase_at(point);
            }
        }
        self.preview(Some(point))
    }
}

fn valid_increment(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value.max(MIN_INCREMENT)
    } else {
        0.1
    }
}

fn valid_tolerance(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

fn to_vector3(point: DVec3) -> Vector3 {
    Vector3::new(point.x, point.y, point.z)
}

fn point_segment_distance_squared(point: DVec3, start: DVec3, end: DVec3) -> f64 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f64::EPSILON {
        return point.distance_squared(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance_squared(start + segment * t)
}

/// Iterative Ramer-Douglas-Peucker reduction used by spline sketches. The
/// tolerance therefore changes both the stored fit-point count and the curve,
/// rather than being a display-only property.
fn simplify_points(points: &[DVec3], tolerance: f64) -> Vec<DVec3> {
    if points.len() <= 2 || tolerance <= 0.0 {
        return points.to_vec();
    }
    let tolerance_squared = tolerance * tolerance;
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut ranges = vec![(0usize, points.len() - 1)];
    while let Some((start, end)) = ranges.pop() {
        if end <= start + 1 {
            continue;
        }
        let mut farthest = None;
        let mut farthest_distance = tolerance_squared;
        for index in start + 1..end {
            let distance = point_segment_distance_squared(points[index], points[start], points[end]);
            if distance > farthest_distance {
                farthest = Some(index);
                farthest_distance = distance;
            }
        }
        if let Some(index) = farthest {
            keep[index] = true;
            ranges.push((start, index));
            ranges.push((index, end));
        }
    }
    points
        .iter()
        .copied()
        .zip(keep)
        .filter_map(|(point, keep)| keep.then_some(point))
        .collect()
}

fn spline_preview_points(points: &[DVec3]) -> Vec<DVec3> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let spline = Spline {
        degree: (points.len().saturating_sub(1).min(3)) as i32,
        fit_points: points.iter().copied().map(to_vector3).collect(),
        ..Default::default()
    };
    crate::entities::curve::spline_curve(&spline)
        .map(|curve| {
            crate::entities::curve::curve_points(&curve)
                .into_iter()
                .map(|point| DVec3::new(point[0], point[1], point[2]))
                .collect()
        })
        .unwrap_or_else(|| points.to_vec())
}

inventory::submit!(crate::command::CommandRegistration { names: &["SKETCH"] });
