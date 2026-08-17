//! Rhai macro scripting: the SCRIPT command, the request pump, and the app's
//! implementation of the script API.
//!
//! A script runs on its own thread and never touches app state directly:
//! each `ocs.*` call becomes a request drained by `Message::ScriptPump` on
//! the main thread, so the UI stays responsive while scripts see live
//! results. Output goes to the command line prefixed with `[script]` until a
//! dedicated script console lands.

use super::{Message, OpenCADStudio};
use iced::Task;
use ocs_scripting::{OcsScriptApi, ScriptRequest, ScriptValue};
use serde_json::json;
use std::sync::mpsc;

/// A running script: its request inbox plus the outcome the worker thread
/// will deliver when it finishes.
pub(super) struct ScriptRunner {
    pub(super) requests: mpsc::Receiver<ScriptRequest>,
    pub(super) outcome: mpsc::Receiver<Result<ocs_scripting::ScriptOutcome, String>>,
    pub(super) name: String,
}

impl OpenCADStudio {
    /// `SCRIPT <path>`: launches the script worker thread and starts the
    /// request pump.
    pub(super) fn start_script(&mut self, path: std::path::PathBuf) -> Task<Message> {

        if self.script_runner.is_some() {
            self.command_line
                .push_error("SCRIPT: a script is already running.");
            return Task::none();
        }
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                self.command_line
                    .push_error(format!("SCRIPT: cannot read \"{}\": {error}", path.display()).as_str());
                return Task::none();
            }
        };
        let (request_tx, request_rx) = mpsc::channel();
        let (outcome_tx, outcome_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let bridge = ocs_scripting::ScriptBridge::new(request_tx);
            let outcome = ocs_scripting::run_rhai(&bridge, &source);
            let _ = outcome_tx.send(outcome);
        });
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "script".to_string());
        self.script_runner = Some(ScriptRunner {
            requests: request_rx,
            outcome: outcome_rx,
            name,
        });
        self.command_line
            .push_info(format!("SCRIPT: \"{}\" started.", path.display()).as_str());
        Task::done(Message::ScriptPump)
    }

    /// Drains pending script requests, dispatches them against the app, and
    /// reschedules itself while the script runs. The short sleep keeps the
    /// pump from busy-spinning between script calls.
    pub(super) fn pump_script(&mut self) -> Task<Message> {
        // Drain first, then dispatch: the dispatcher needs `&mut self`, so
        // the runner must not stay borrowed across it.
        let pending: Vec<ScriptRequest> = {
            let Some(runner) = self.script_runner.as_ref() else {
                return Task::none();
            };
            let mut pending = Vec::new();
            while let Ok(request) = runner.requests.try_recv() {
                pending.push(request);
            }
            pending
        };
        for request in pending {
            ocs_scripting::dispatch_script_request(self, request);
        }
        let Some(runner) = self.script_runner.as_ref() else {
            return Task::none();
        };
        match runner.outcome.try_recv() {
            Ok(Ok(outcome)) => {
                let name = runner.name.clone();
                self.script_runner = None;
                for line in &outcome.log {
                    self.command_line
                        .push_output(format!("[script] {line}").as_str());
                }
                self.command_line
                    .push_output(format!("SCRIPT: \"{name}\" finished.").as_str());
                Task::none()
            }
            Ok(Err(error)) => {
                let name = runner.name.clone();
                self.script_runner = None;
                self.command_line
                    .push_error(format!("SCRIPT: \"{name}\" failed: {error}").as_str());
                Task::none()
            }
            Err(mpsc::TryRecvError::Empty) => {
                Task::perform(script_pump_delay(), move |_| Message::ScriptPump)
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // The worker finished but its outcome raced the drain; one
                // more pump picks it up.
                Task::done(Message::ScriptPump)
            }
        }
    }
}

async fn script_pump_delay() {
    // The timer runs on its own thread so the async wrapper stays executor
    // friendly without adding a timer dependency.
    let (trigger, done) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = trigger.send(());
    });
    let _ = done.await;
}

/// Maps the engine-agnostic script API onto the live application state of
/// the active tab. Long-running operations (attach, export) start their
/// usual background jobs and return immediately; scripts observe progress by
/// polling `cloud_sources` / `cloud_export_status`.
impl OcsScriptApi for OpenCADStudio {
    fn command(&mut self, command: &str) -> ScriptValue {
        let _ = self.run_command_line(command);
        json!(true)
    }

    fn cloud_attach(&mut self, path: &str) -> ScriptValue {
        let _ = self.start_point_cloud_load(path.into());
        json!("queued")
    }

    fn cloud_attach_folder(&mut self, path: &str) -> ScriptValue {
        let _ = self.start_point_cloud_folder_load(path.into());
        json!("queued")
    }

    fn cloud_sources(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        let dataset = &self.tabs[tab].point_cloud;
        json!(dataset
            .sources
            .iter()
            .map(|source| {
                json!({
                    "id": source.id,
                    "path": source.source_path.display().to_string(),
                    "points": source.sample.metadata.point_count,
                    "displayed": source.sample.points.len(),
                    "edits": source.edits.len(),
                })
            })
            .collect::<Vec<_>>())
    }

    fn cloud_stats(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        let dataset = &self.tabs[tab].point_cloud;
        let points = dataset.sources.iter().flat_map(|source| {
            source.sample.points.iter().cloned().map(|point| {
                source
                    .edits
                    .patch_for(point.source_index)
                    .map_or(point.clone(), |patch| point.with_patch(patch))
            })
        });
        let stats = ocs_pointcloud::classification_statistics(points);
        let mut map = serde_json::Map::new();
        for (class, stats) in stats {
            map.insert(class.to_string(), json!(stats.total));
        }
        ScriptValue::Object(map)
    }

    fn cloud_filter(&mut self, filter_json: &str) -> ScriptValue {
        match serde_json::from_str::<ocs_pointcloud::PointFilter>(filter_json) {
            Ok(filter) => {
                let tab = self.active_tab;
                self.set_point_cloud_selection_filter(tab, filter);
                json!(true)
            }
            Err(error) => json!(format!("error: {error}")),
        }
    }

    fn cloud_select_slice(&mut self, low: f64, high: f64) -> ScriptValue {
        let tab = self.active_tab;
        self.point_cloud_select_elevation_slice(tab, low, high);
        json!(true)
    }

    fn cloud_select_clear(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        self.clear_point_cloud_selections(tab);
        json!(true)
    }

    fn cloud_classify_selection(&mut self, classification: i64) -> ScriptValue {
        let class = u8::try_from(classification.max(0)).unwrap_or(0);
        let tab = self.active_tab;
        self.patch_point_cloud_selection(
            tab,
            &format!("Script assign class {class}"),
            ocs_pointcloud::PointPatch::classification(class),
        );
        json!(true)
    }

    fn cloud_classify(
        &mut self,
        source_id: &str,
        classification: i64,
        indices: &str,
    ) -> ScriptValue {
        let class = u8::try_from(classification.max(0)).unwrap_or(0);
        let tab = self.active_tab;
        let dataset = &mut self.tabs[tab].point_cloud;
        let Some(cloud) = dataset.source_mut(source_id) else {
            return json!(format!("error: unknown source {source_id}"));
        };
        let count = cloud.sample.metadata.point_count;
        match super::point_cloud::parse_source_indices(indices, count) {
            Ok(indices) => {
                let changed = cloud.edits.apply(
                    format!("Script assign class {class}"),
                    indices,
                    ocs_pointcloud::PointPatch::classification(class),
                );
                drop(cloud);
                self.tabs[tab].point_cloud.note_edit_sources(vec![source_id.to_string()]);
                self.tabs[tab].point_cloud.mark_display_changed();
                let model = self.tabs[tab].point_cloud.display_model();
                self.tabs[tab].scene.set_point_cloud(model);
                json!(changed)
            }
            Err(error) => json!(format!("error: {error}")),
        }
    }

    fn cloud_undo(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        self.undo_point_cloud_edit(tab);
        json!(true)
    }

    fn cloud_export_all(&mut self, path: &str) -> ScriptValue {
        let _ = self.start_point_cloud_export_all(path.into());
        json!("queued")
    }

    fn cloud_export_status(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        match self.point_cloud_export_progress(tab) {
            Some((completed, total)) => {
                json!({ "running": true, "completed": completed, "total": total })
            }
            None => json!({ "running": false }),
        }
    }

    fn cloud_detach(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        self.detach_point_cloud(tab);
        json!(true)
    }

    fn cloud_list_folder(&mut self, path: &str) -> ScriptValue {
        let mut files: Vec<String> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("las")
                            || extension.eq_ignore_ascii_case("laz")
                    })
            })
            .map(|path| path.display().to_string())
            .collect();
        files.sort();
        json!(files)
    }

    fn print(&mut self, message: &str) {
        self.command_line
            .push_output(format!("[script] {message}").as_str());
    }
}
