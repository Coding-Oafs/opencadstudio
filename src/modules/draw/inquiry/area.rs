use acadrust::{EntityType, Handle};
use glam::DVec3;

use crate::command::{
    AreaPreviewRegion, AreaPreviewSource, CadCommand, CmdOption, CmdResult, SelectionEntity,
};
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
    objects_gathering: bool,
    selected_entities: Vec<SelectionEntity>,
    preview_regions: Vec<AreaPreviewRegion>,
    total_area: f64,
    total_perimeter: f64,
}

impl AreaCommand {
    pub fn new() -> Self {
        Self {
            mode: AreaMode::Single,
            points: Vec::new(),
            objects_gathering: false,
            selected_entities: Vec::new(),
            preview_regions: Vec::new(),
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

    fn selection_entity_measurement(entity: &SelectionEntity) -> Option<AreaMeasurement> {
        Self::entity_measurement(&entity.entity).or_else(|| {
            entity.surface_area.map(|area| AreaMeasurement {
                area,
                perimeter: None,
            })
        })
    }

    fn selection_measurement(&self) -> Option<(AreaMeasurement, Vec<Handle>)> {
        let measured = self
            .selected_entities
            .iter()
            .filter_map(|entity| {
                Self::selection_entity_measurement(entity)
                    .map(|measurement| (entity.handle, measurement))
            })
            .collect::<Vec<_>>();
        if measured.is_empty() {
            return None;
        }

        let area = measured.iter().map(|(_, measurement)| measurement.area).sum();
        let perimeter = measured
            .iter()
            .map(|(_, measurement)| measurement.perimeter)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.into_iter().sum());
        let handles = measured.into_iter().map(|(handle, _)| handle).collect();
        Some((AreaMeasurement { area, perimeter }, handles))
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

    fn finish_measurement(
        &mut self,
        measurement: AreaMeasurement,
        deselect: bool,
    ) -> CmdResult {
        if self.mode == AreaMode::Single {
            return CmdResult::Measurement(Self::result_message(measurement));
        }

        let sign = if self.mode == AreaMode::Add { 1.0 } else { -1.0 };
        self.total_area += sign * measurement.area;
        if let Some(perimeter) = measurement.perimeter {
            self.total_perimeter += sign * perimeter;
        }
        self.points.clear();
        self.objects_gathering = false;
        self.selected_entities.clear();
        let message = self.running_result_message(measurement);
        if deselect {
            CmdResult::ReportMeasurementAndDeselect(message)
        } else {
            CmdResult::ReportMeasurement(message)
        }
    }
}

impl CadCommand for AreaCommand {
    fn name(&self) -> &'static str {
        "AREA"
    }

    fn prompt(&self) -> String {
        if self.objects_gathering {
            return crate::tr!(
                "area-prompt-objects",
                count = self.selected_entities.len()
            );
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
        if self.objects_gathering {
            return vec![Self::option(crate::tr!("area-option-back"), "BACK")];
        }
        if !self.points.is_empty() {
            return Vec::new();
        }
        let objects = || Self::option(crate::tr!("area-option-objects"), "OBJECTS");
        let add = || Self::option(crate::tr!("area-option-add"), "ADD");
        let subtract = || Self::option(crate::tr!("area-option-subtract"), "SUBTRACT");
        match self.mode {
            AreaMode::Single => vec![objects(), add(), subtract()],
            AreaMode::Add => vec![objects(), subtract()],
            AreaMode::Subtract => vec![objects(), add()],
        }
    }

    fn wants_text_input(&self) -> bool {
        !self.objects_gathering
    }

    fn point_step_accepts_keywords(&self) -> bool {
        !self.objects_gathering
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        let input = text.trim().to_ascii_uppercase();
        if self.objects_gathering {
            if matches!(input.as_str(), "B" | "BACK") {
                if let Some((measurement, handles)) = self.selection_measurement() {
                    if self.mode != AreaMode::Single {
                        self.preview_regions.push(AreaPreviewRegion {
                            source: AreaPreviewSource::Handles(handles),
                            subtract: self.mode == AreaMode::Subtract,
                        });
                    }
                    return Some(self.finish_measurement(measurement, true));
                }
                self.objects_gathering = false;
                self.selected_entities.clear();
                return Some(CmdResult::DeselectAndContinue);
            }
            return Some(CmdResult::NeedPoint);
        }
        if !self.points.is_empty() {
            return Some(CmdResult::NeedPoint);
        }
        match input.as_str() {
            "O" | "OBJECT" | "OBJECTS" => self.objects_gathering = true,
            "A" | "ADD" => self.mode = AreaMode::Add,
            "S" | "SUBTRACT" => self.mode = AreaMode::Subtract,
            _ => return Some(CmdResult::NeedPoint),
        }
        Some(CmdResult::NeedPoint)
    }

    fn is_selection_gathering(&self) -> bool {
        self.objects_gathering
    }

    fn selection_forces_add(&self) -> bool {
        self.objects_gathering
    }

    fn inject_selection_entities(&mut self, entities: Vec<SelectionEntity>) {
        self.selected_entities = entities;
    }

    fn on_selection_complete(&mut self, _handles: Vec<Handle>) -> CmdResult {
        CmdResult::NeedPoint
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        if self.objects_gathering {
            return CmdResult::NeedPoint;
        }
        self.points.push(point);
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.objects_gathering {
            let Some((measurement, handles)) = self.selection_measurement() else {
                self.selected_entities.clear();
                return CmdResult::ReportMeasurementAndDeselect(crate::tr!(
                    "area-objects-not-measurable"
                ));
            };
            if self.mode != AreaMode::Single {
                self.preview_regions.push(AreaPreviewRegion {
                    source: AreaPreviewSource::Handles(handles),
                    subtract: self.mode == AreaMode::Subtract,
                });
            }
            return self.finish_measurement(measurement, true);
        }
        if self.points.is_empty() {
            self.objects_gathering = true;
            return CmdResult::NeedPoint;
        }
        if self.points.len() < 3 {
            return CmdResult::Cancel;
        }
        let measurement = Self::point_measurement(&self.points, true);
        if self.mode != AreaMode::Single {
            self.preview_regions.push(AreaPreviewRegion {
                source: AreaPreviewSource::Boundary(
                    self.points.iter().map(|point| [point.x, point.y]).collect(),
                ),
                subtract: self.mode == AreaMode::Subtract,
            });
        }
        self.finish_measurement(measurement, false)
    }

    fn on_mouse_move(&mut self, point: DVec3) -> Option<WireModel> {
        if self.objects_gathering || self.points.is_empty() {
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
            fill_is_2d_solid: false,
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

    fn area_preview_regions(&self) -> Option<Vec<AreaPreviewRegion>> {
        let mut regions = self.preview_regions.clone();
        if self.objects_gathering {
            let handles = self
                .selected_entities
                .iter()
                .map(|entity| entity.handle)
                .collect::<Vec<_>>();
            if !handles.is_empty() {
                regions.push(AreaPreviewRegion {
                    source: AreaPreviewSource::Handles(handles),
                    subtract: self.mode == AreaMode::Subtract,
                });
            }
        } else if self.points.len() >= 3 {
            regions.push(AreaPreviewRegion {
                source: AreaPreviewSource::Boundary(
                    self.points.iter().map(|point| [point.x, point.y]).collect(),
                ),
                subtract: self.mode == AreaMode::Subtract,
            });
        }
        Some(regions)
    }
}

inventory::submit!(crate::command::CommandRegistration { names: &["AREA"] });
