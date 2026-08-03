use acadrust::{EntityType, Handle};
use glam::DVec3;

use crate::command::{CadCommand, CmdOption, CmdResult};
use crate::entities::traits::EntityTypeOps;
use crate::scene::model::wire_model::WireModel;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AreaMode {
    Single,
    Add,
    Subtract,
}

#[derive(Clone, Copy)]
struct AreaMeasurement {
    area: f64,
    perimeter: Option<f64>,
}

pub struct AreaCommand {
    mode: AreaMode,
    points: Vec<DVec3>,
    object_pick: bool,
    picked_entity: Option<EntityType>,
    picked_surface_area: Option<f64>,
    total_area: f64,
    total_perimeter: f64,
}

impl AreaCommand {
    pub fn new() -> Self {
        Self {
            mode: AreaMode::Single,
            points: Vec::new(),
            object_pick: false,
            picked_entity: None,
            picked_surface_area: None,
            total_area: 0.0,
            total_perimeter: 0.0,
        }
    }

    fn option(label: String, keyword: &str) -> CmdOption {
        CmdOption { label, keyword: keyword.to_string() }
    }

    fn point_measurement(points: &[DVec3], close_perimeter: bool) -> AreaMeasurement {
        if points.len() < 2 {
            return AreaMeasurement { area: 0.0, perimeter: Some(0.0) };
        }

        let origin = points[0];
        let mut area_vector = DVec3::ZERO;
        for index in 0..points.len() {
            let a = points[index] - origin;
            let b = points[(index + 1) % points.len()] - origin;
            area_vector += a.cross(b);
        }

        let mut perimeter = points
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).length())
            .sum::<f64>();
        if close_perimeter {
            perimeter += (points[0] - points[points.len() - 1]).length();
        }

        AreaMeasurement { area: area_vector.length() * 0.5, perimeter: Some(perimeter) }
    }

    fn bulged_polyline_measurement(
        points: &[(f64, f64)],
        bulges: &[f64],
        closed: bool,
    ) -> AreaMeasurement {
        let count = points.len();
        if count < 2 {
            return AreaMeasurement { area: 0.0, perimeter: Some(0.0) };
        }

        let segment_count = if closed { count } else { count - 1 };
        let origin = points[0];
        let mut signed_area = 0.0;
        let mut perimeter = 0.0;
        for index in 0..segment_count {
            let a = points[index];
            let b = points[(index + 1) % count];
            let local_a = (a.0 - origin.0, a.1 - origin.1);
            let local_b = (b.0 - origin.0, b.1 - origin.1);
            signed_area += 0.5 * (local_a.0 * local_b.1 - local_b.0 * local_a.1);

            let chord = (b.0 - a.0).hypot(b.1 - a.1);
            let bulge = bulges.get(index).copied().unwrap_or(0.0);
            if bulge.abs() < 1e-12 || chord <= 1e-12 {
                perimeter += chord;
                continue;
            }

            let angle = 4.0 * bulge.atan();
            let sine = (angle * 0.5).sin().abs();
            if sine <= 1e-12 {
                perimeter += chord;
                continue;
            }
            let radius = chord / (2.0 * sine);
            signed_area += 0.5 * radius * radius * (angle - angle.sin());
            perimeter += radius * angle.abs();
        }

        AreaMeasurement { area: signed_area.abs(), perimeter: Some(perimeter) }
    }

    fn entity_measurement(entity: &EntityType) -> Option<AreaMeasurement> {
        match entity {
            EntityType::LwPolyline(polyline) => {
                let points = polyline
                    .vertices
                    .iter()
                    .map(|vertex| (vertex.location.x, vertex.location.y))
                    .collect::<Vec<_>>();
                let bulges = polyline.vertices.iter().map(|vertex| vertex.bulge).collect::<Vec<_>>();
                Some(Self::bulged_polyline_measurement(
                    &points,
                    &bulges,
                    polyline.is_closed,
                ))
            }
            EntityType::Polyline2D(polyline) => {
                let points = polyline
                    .vertices
                    .iter()
                    .map(|vertex| (vertex.location.x, vertex.location.y))
                    .collect::<Vec<_>>();
                let bulges = polyline.vertices.iter().map(|vertex| vertex.bulge).collect::<Vec<_>>();
                Some(Self::bulged_polyline_measurement(
                    &points,
                    &bulges,
                    polyline.is_closed(),
                ))
            }
            EntityType::Polyline(polyline) => {
                let points = polyline
                    .vertices
                    .iter()
                    .map(|vertex| {
                        DVec3::new(vertex.location.x, vertex.location.y, vertex.location.z)
                    })
                    .collect::<Vec<_>>();
                Some(Self::point_measurement(&points, polyline.is_closed()))
            }
            EntityType::Polyline3D(polyline) => {
                let points = polyline
                    .vertices
                    .iter()
                    .map(|vertex| {
                        DVec3::new(vertex.position.x, vertex.position.y, vertex.position.z)
                    })
                    .collect::<Vec<_>>();
                Some(Self::point_measurement(&points, polyline.is_closed()))
            }
            EntityType::Spline(spline) => {
                let points = crate::entities::spline::measurement_polyline(spline)
                    .into_iter()
                    .map(|point| DVec3::new(point[0], point[1], point[2]))
                    .collect::<Vec<_>>();
                Some(Self::point_measurement(
                    &points,
                    spline.flags.closed || spline.flags.periodic,
                ))
            }
            _ => entity.mass_props().map(|props| AreaMeasurement {
                area: props.area,
                perimeter: Some(props.perimeter),
            }),
        }
    }

    fn result_message(measurement: AreaMeasurement) -> String {
        match measurement.perimeter {
            Some(perimeter) => crate::tr!(
                "area-result",
                area = format!("{:.4}", measurement.area),
                perimeter = format!("{perimeter:.4}"),
            ),
            None => crate::tr!("area-result-area-only", area = format!("{:.4}", measurement.area)),
        }
    }

    fn running_result_message(&self, measurement: AreaMeasurement) -> String {
        match measurement.perimeter {
            Some(perimeter) => crate::tr!(
                "area-running-result",
                area = format!("{:.4}", measurement.area),
                perimeter = format!("{perimeter:.4}"),
                total_area = format!("{:.4}", self.total_area),
                total_perimeter = format!("{:.4}", self.total_perimeter),
            ),
            None => crate::tr!(
                "area-running-result-area-only",
                area = format!("{:.4}", measurement.area),
                total_area = format!("{:.4}", self.total_area),
            ),
        }
    }

    fn finish_measurement(&mut self, measurement: AreaMeasurement) -> CmdResult {
        if self.mode == AreaMode::Single {
            return CmdResult::Measurement(Self::result_message(measurement));
        }

        let sign = if self.mode == AreaMode::Add { 1.0 } else { -1.0 };
        self.total_area += sign * measurement.area;
        if let Some(perimeter) = measurement.perimeter {
            self.total_perimeter += sign * perimeter;
        }
        self.points.clear();
        self.object_pick = false;
        CmdResult::ReportMeasurement(self.running_result_message(measurement))
    }
}

impl CadCommand for AreaCommand {
    fn name(&self) -> &'static str {
        "AREA"
    }

    fn prompt(&self) -> String {
        if self.object_pick {
            return crate::tr!("area-prompt-object");
        }
        if !self.points.is_empty() {
            return crate::tr!("area-prompt-next", count = self.points.len());
        }
        match self.mode {
            AreaMode::Single => crate::tr!("area-prompt-first"),
            AreaMode::Add => crate::tr!("area-prompt-add"),
            AreaMode::Subtract => crate::tr!("area-prompt-subtract"),
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        if self.object_pick || !self.points.is_empty() {
            return Vec::new();
        }
        let object = || Self::option(crate::tr!("area-option-object"), "OBJECT");
        let add = || Self::option(crate::tr!("area-option-add"), "ADD");
        let subtract = || Self::option(crate::tr!("area-option-subtract"), "SUBTRACT");
        match self.mode {
            AreaMode::Single => vec![object(), add(), subtract()],
            AreaMode::Add => vec![object(), subtract()],
            AreaMode::Subtract => vec![object(), add()],
        }
    }

    fn wants_text_input(&self) -> bool {
        !self.object_pick
    }

    fn point_step_accepts_keywords(&self) -> bool {
        !self.object_pick
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.object_pick || !self.points.is_empty() {
            return Some(CmdResult::NeedPoint);
        }
        match text.trim().to_ascii_uppercase().as_str() {
            "O" | "OBJECT" => self.object_pick = true,
            "A" | "ADD" => self.mode = AreaMode::Add,
            "S" | "SUBTRACT" => self.mode = AreaMode::Subtract,
            _ => return Some(CmdResult::NeedPoint),
        }
        Some(CmdResult::NeedPoint)
    }

    fn needs_entity_pick(&self) -> bool {
        self.object_pick
    }

    fn entity_pick_includes_fills(&self) -> bool {
        true
    }

    fn entity_pick_highlights_hover(&self) -> bool {
        true
    }

    fn inject_before_entity_pick(&self) -> bool {
        true
    }

    fn inject_picked_entity(&mut self, entity: EntityType) {
        self.picked_entity = Some(entity);
        self.picked_surface_area = None;
    }

    fn inject_picked_surface_area(&mut self, area: f64) {
        self.picked_surface_area = Some(area);
    }

    fn on_entity_pick(&mut self, handle: Handle, _point: DVec3) -> CmdResult {
        if handle.is_null() {
            return CmdResult::NeedPoint;
        }
        let measurement = self
            .picked_entity
            .take()
            .and_then(|entity| Self::entity_measurement(&entity))
            .or_else(|| {
                self.picked_surface_area.take().map(|area| AreaMeasurement {
                    area,
                    perimeter: None,
                })
            });
        match measurement {
            Some(measurement) => self.finish_measurement(measurement),
            None => CmdResult::ReportMeasurement(crate::tr!("area-object-not-measurable")),
        }
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        self.points.push(point);
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.object_pick {
            return CmdResult::Cancel;
        }
        if self.points.is_empty() {
            self.object_pick = true;
            return CmdResult::NeedPoint;
        }
        if self.points.len() < 3 {
            return CmdResult::Cancel;
        }
        let measurement = Self::point_measurement(&self.points, true);
        self.finish_measurement(measurement)
    }

    fn on_mouse_move(&mut self, point: DVec3) -> Option<WireModel> {
        if self.object_pick || self.points.is_empty() {
            return None;
        }
        let to_render = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32];
        let mut points = self.points.iter().map(|point| to_render(*point)).collect::<Vec<_>>();
        points.push(to_render(point));
        points.push(to_render(self.points[0]));
        Some(WireModel {
            taper_widths: Vec::new(),
            world_width: 0.0,
            depth_override: None,
            fill_is_3d: false,
            pick_tris: Vec::new(),
            pick_tris_low: Vec::new(),
            dash_from_start: false,
            dash_align_end: None,
            text_verts: Vec::new(),
            name: "area_preview".into(),
            points,
            points_low: Vec::new(),
            color: WireModel::CYAN,
            selected: false,
            pattern_length: 0.0,
            pattern: [0.0; 8],
            line_weight_px: 1.0,
            snap_pts: Vec::new(),
            tangent_geoms: Vec::new(),
            aci: 0,
            key_vertices: Vec::new(),
            aabb: WireModel::UNBOUNDED_AABB,
            plinegen: true,
            fill_tris: Vec::new(),
            fill_tris_low: Vec::new(),
        })
    }
}

inventory::submit!(crate::command::CommandRegistration { names: &["AREA"] });
