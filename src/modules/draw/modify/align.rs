// ALIGN command — align selected objects using 1 or 2 point pairs.
//
// Workflow:
//   1. Select objects (Enter to finish selection)
//   2. First source point → first destination point
//   3. Second source point → second destination point (Enter to skip = translate only)
//   4. Enter = apply (scale = optional: Y/N prompt after 2nd pair)
//
// With 1 pair:  pure translation (src1 → dst1)
// With 2 pairs: translate + rotate (+ optional uniform scale to fit)

use acadrust::Handle;
use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult, EntityTransform};

pub struct AlignCommand {
    state: AlignState,
    handles: Vec<Handle>,
    src1: Option<DVec3>,
    dst1: Option<DVec3>,
    src2: Option<DVec3>,
    dst2: Option<DVec3>,
}

#[derive(PartialEq)]
enum AlignState {
    Gathering,
    Src1,
    Dst1,
    Src2,
    Dst2,
    AskScale,
}

impl AlignCommand {
    pub fn new() -> Self {
        Self {
            state: AlignState::Gathering,
            handles: vec![],
            src1: None,
            dst1: None,
            src2: None,
            dst2: None,
        }
    }
}

impl CadCommand for AlignCommand {
    fn name(&self) -> &'static str {
        "ALIGN"
    }

    fn prompt(&self) -> String {
        match self.state {
            AlignState::Gathering => t!(
                "ALIGN  Select objects (%{count} selected, Enter when done):",
                count = self.handles.len()
            )
            .into_owned(),
            AlignState::Src1 => t!("ALIGN  Specify 1st source point:").into_owned(),
            AlignState::Dst1 => t!("ALIGN  Specify 1st destination point:").into_owned(),
            AlignState::Src2 => {
                t!("ALIGN  Specify 2nd source point (Enter = translate only):").into_owned()
            }
            AlignState::Dst2 => t!("ALIGN  Specify 2nd destination point:").into_owned(),
            AlignState::AskScale => {
                t!("ALIGN  Scale objects based on alignment points? [Y/N]:").into_owned()
            }
        }
    }

    fn is_selection_gathering(&self) -> bool {
        self.state == AlignState::Gathering
    }

    fn on_selection_complete(&mut self, handles: Vec<Handle>) -> CmdResult {
        self.handles = handles;
        CmdResult::NeedPoint
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        match self.state {
            AlignState::Gathering => CmdResult::NeedPoint,
            AlignState::Src1 => {
                self.src1 = Some(pt);
                self.state = AlignState::Dst1;
                CmdResult::NeedPoint
            }
            AlignState::Dst1 => {
                self.dst1 = Some(pt);
                self.state = AlignState::Src2;
                CmdResult::NeedPoint
            }
            AlignState::Src2 => {
                self.src2 = Some(pt);
                self.state = AlignState::Dst2;
                CmdResult::NeedPoint
            }
            AlignState::Dst2 => {
                self.dst2 = Some(pt);
                self.state = AlignState::AskScale;
                CmdResult::NeedPoint
            }
            AlignState::AskScale => CmdResult::NeedPoint,
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        match self.state {
            AlignState::Gathering => {
                if self.handles.is_empty() {
                    return CmdResult::Cancel;
                }
                self.state = AlignState::Src1;
                CmdResult::NeedPoint
            }
            AlignState::Src2 => {
                // Only 1 pair — pure translation
                match (self.src1, self.dst1) {
                    (Some(s), Some(d)) => {
                        let delta = d - s;
                        CmdResult::TransformSelected(
                            self.handles.clone(),
                            EntityTransform::Translate(delta),
                        )
                    }
                    _ => CmdResult::Cancel,
                }
            }
            AlignState::AskScale => {
                // No scale (default N)
                self.compute_align(false)
            }
            _ => CmdResult::Cancel,
        }
    }

    fn wants_text_input(&self) -> bool {
        self.state == AlignState::AskScale
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        if self.state != AlignState::AskScale {
            return None;
        }
        let scale = text.trim().to_uppercase().starts_with('Y');
        Some(self.compute_align(scale))
    }
}

impl AlignCommand {
    fn compute_align(&self, with_scale: bool) -> CmdResult {
        let (s1, d1, s2, d2) = match (self.src1, self.dst1, self.src2, self.dst2) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => return CmdResult::Cancel,
        };

        // Build transform: move s1→d1, rotate so s2-s1 aligns with d2-d1 (in the world XY plane)
        let src_vec = s2 - s1;
        let dst_vec = d2 - d1;

        let src_len = src_vec.length();
        let dst_len = dst_vec.length();

        if src_len < 1e-6 || dst_len < 1e-6 {
            // Degenerate: just translate
            let delta = d1 - s1;
            return CmdResult::TransformSelected(
                self.handles.clone(),
                EntityTransform::Translate(delta),
            );
        }

        // Angle from src_vec to dst_vec in the world XY plane
        let src_angle = src_vec.y.atan2(src_vec.x);
        let dst_angle = dst_vec.y.atan2(dst_vec.x);
        let angle = dst_angle - src_angle;

        let scale_factor = if with_scale { dst_len / src_len } else { 1.0 };

        // Apply: translate to origin (s1), scale, rotate, translate to d1
        // We use the EntityTransform enum — it doesn't support composed transforms directly.
        // Return a special align result that carries the full matrix.
        let _ = (angle, scale_factor, with_scale);

        // Compose via AlignTransform CmdResult
        CmdResult::AlignSelected {
            handles: self.handles.clone(),
            src1: s1,
            dst1: d1,
            angle_rad: angle,
            scale: scale_factor,
        }
    }
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["ALIGN"] });  // AlignCommand
