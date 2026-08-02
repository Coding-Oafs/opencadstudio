// ID command — report coordinates of a picked point.

use glam::DVec3;
use crate::t;

use crate::command::{CadCommand, CmdResult};

pub struct IdCommand;

impl IdCommand {
    pub fn new() -> Self {
        Self
    }
}

impl CadCommand for IdCommand {
    fn name(&self) -> &'static str {
        "ID"
    }

    fn prompt(&self) -> String {
        t!("ID  Specify point:").into_owned()
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        // Drawing plane is world XY (z = elevation).
        let x = pt.x;
        let y = pt.y;
        let z = pt.z;
        let x_s = format!("{x:.4}");
        let y_s = format!("{y:.4}");
        let z_s = format!("{z:.4}");
        let msg = t!(
            "X = %{x},  Y = %{y},  Z = %{z}",
            x = x_s,
            y = y_s,
            z = z_s
        )
        .into_owned();
        CmdResult::Measurement(msg)
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}


// ── Autocomplete registry ─────────────────────────────────
inventory::submit!(crate::command::CommandRegistration { names: &["ID"] });  // IdCommand
