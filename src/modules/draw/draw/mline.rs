// MLINE command — create a multiline (parallel lines).
//
// Workflow: pick vertices, Enter to finish.
// Text input (when >= 1 point picked):
//   C / CLOSE  → close and commit
//   S <value>  → set scale factor then continue picking

use acadrust::entities::MLine;
use acadrust::types::Vector3;
use acadrust::EntityType;
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult, WorkingPlane};
use crate::scene::model::wire_model::WireModel;

pub struct MlineCommand {
    points: Vec<DVec3>,
    scale: f64,
    waiting_scale: bool,
    style_name: String,
    style_handle: Option<acadrust::Handle>,
    style_element_count: usize,
    plane: WorkingPlane,
}

impl MlineCommand {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            points: vec![],
            scale: 1.0,
            waiting_scale: false,
            style_name: "Standard".into(),
            style_handle: None,
            style_element_count: 2,
            plane: WorkingPlane::default(),
        }
    }

    pub fn with_style(
        style_name: impl Into<String>,
        style_handle: Option<acadrust::Handle>,
        style_element_count: usize,
    ) -> Self {
        Self {
            points: vec![],
            scale: 1.0,
            waiting_scale: false,
            style_name: style_name.into(),
            style_handle,
            style_element_count: style_element_count.max(1),
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for MlineCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "MLINE"
    }

    fn prompt(&self) -> String {
        if self.waiting_scale {
            t!("MLINE  Enter scale factor:").into_owned()
        } else if self.points.is_empty() {
            t!(
                "MLINE  Specify start point (scale=%{scale}):",
                scale = format!("{:.2}", self.scale)
            )
            .into_owned()
        } else {
            t!(
                "MLINE  Specify next point (%{count} pts, Enter to finish, C to close, S to set scale):",
                count = self.points.len()
            )
            .into_owned()
        }
    }

    fn wants_text_input(&self) -> bool {
        self.waiting_scale || !self.points.is_empty()
    }

    fn point_step_accepts_keywords(&self) -> bool {
        // The vertex steps accept J / S keywords but are point picks, so keep
        // polar dynamic input. The scale prompt (`waiting_scale`) is genuine
        // text entry and is excluded.
        !self.waiting_scale && !self.points.is_empty()
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        // Waiting for scale value
        if self.waiting_scale {
            let v: f64 = text
                .trim()
                .replace(',', ".")
                .parse()
                .ok()
                .filter(|&v: &f64| v > 0.0)?;
            self.scale = v;
            self.waiting_scale = false;
            return Some(CmdResult::NeedPoint);
        }

        let up = text.trim().to_uppercase();

        // Close command
        if (up == "C" || up == "CLOSE") && self.points.len() >= 3 {
            let entity = build_mline(
                &self
                    .points
                    .iter()
                    .map(|point| self.plane.to_local(*point))
                    .collect::<Vec<_>>(),
                self.scale,
                true,
                &self.style_name,
                self.style_handle,
                self.style_element_count,
            );
            return Some(CmdResult::CommitAndExit(self.plane.place_entity(entity)));
        }

        // Scale: "S" alone → prompt for value
        if up == "S" {
            self.waiting_scale = true;
            return Some(CmdResult::NeedPoint);
        }

        // Scale: "S <value>" inline
        if let Some(rest) = up.strip_prefix("S ") {
            if let Ok(v) = rest.trim().replace(',', ".").parse::<f64>() {
                if v > 0.0 {
                    self.scale = v;
                }
                return Some(CmdResult::NeedPoint);
            }
        }

        None
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        self.points.push(pt);
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        if self.points.len() < 2 {
            return CmdResult::Cancel;
        }
        let entity = build_mline(
            &self
                .points
                .iter()
                .map(|point| self.plane.to_local(*point))
                .collect::<Vec<_>>(),
            self.scale,
            false,
            &self.style_name,
            self.style_handle,
            self.style_element_count,
        );
        CmdResult::CommitAndExit(self.plane.place_entity(entity))
    }

    fn on_mouse_move(&mut self, pt: DVec3) -> Option<WireModel> {
        let pt = pt.as_vec3();
        if self.points.is_empty() {
            return None;
        }
        let mut pts: Vec<[f32; 3]> = self
            .points
            .iter()
            .map(|p| [p.x as f32, p.y as f32, p.z as f32])
            .collect();
        pts.push([pt.x, pt.y, pt.z]);
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
            name: "mline_preview".into(),
            points: pts,
            points_low: Vec::new(),
            color: WireModel::CYAN,
            selected: false,
            pattern_length: 0.0,
            pattern: [0.0; 8],
            line_weight_px: 1.0,
            snap_pts: vec![],
            tangent_geoms: vec![],
            aci: 0,
            key_vertices: vec![],
            aabb: WireModel::UNBOUNDED_AABB,
            plinegen: true,
            fill_tris: vec![],
            fill_tris_low: Vec::new(),
        })
    }
}

fn build_mline(
    pts: &[DVec3],
    scale: f64,
    closed: bool,
    style_name: &str,
    style_handle: Option<acadrust::Handle>,
    style_element_count: usize,
) -> EntityType {
    let mut mline = MLine::new();
    mline.scale_factor = scale;
    mline.style_name = style_name.to_string();
    mline.style_handle = style_handle;
    mline.style_element_count = style_element_count;
    for point in pts {
        mline.add_vertex(Vector3::new(point.x, point.y, point.z));
    }
    if closed {
        mline.close();
    }
    EntityType::MLine(mline)
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["MLINE"] });  // MlineCommand
