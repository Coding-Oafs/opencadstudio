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
                self.command_line.push_error(
                    format!("SCRIPT: cannot read \"{}\": {error}", path.display()).as_str(),
                );
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

    /// `PYSCRIPT <path.py>`: runs the script in an out-of-process CPython
    /// worker. Requests arrive on the same pump as Rhai scripts, so both
    /// engines dispatch through one audited API surface.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn start_python_script(&mut self, path: std::path::PathBuf) -> Task<Message> {
        if self.script_runner.is_some() {
            self.command_line
                .push_error("PYSCRIPT: a script is already running.");
            return Task::none();
        }
        if !path.is_file() {
            self.command_line
                .push_error(format!("PYSCRIPT: \"{}\" was not found.", path.display()).as_str());
            return Task::none();
        }
        if let Err(error) = ocs_scripting::python_package_path() {
            self.command_line
                .push_error(format!("PYSCRIPT: {error}").as_str());
            return Task::none();
        }
        let (request_tx, request_rx) = mpsc::channel();
        let (outcome_tx, outcome_rx) =
            mpsc::channel::<Result<ocs_scripting::ScriptOutcome, String>>();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let bridge = ocs_scripting::ScriptBridge::new(request_tx);
            let outcome = ocs_scripting::run_python(&bridge, &worker_path);
            let _ = outcome_tx.send(outcome);
        });
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "python script".to_string());
        self.script_runner = Some(ScriptRunner {
            requests: request_rx,
            outcome: outcome_rx,
            name,
        });
        self.command_line
            .push_info(format!("PYSCRIPT: \"{}\" started.", path.display()).as_str());
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
                self.tabs[tab]
                    .point_cloud
                    .note_edit_sources(vec![source_id.to_string()]);
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

    fn cloud_urban_classify(&mut self, settings_json: &str) -> ScriptValue {
        let tab = self.active_tab;
        match serde_json::from_str::<ocs_pointcloud::UrbanClassificationSettings>(settings_json) {
            Ok(settings) => {
                match self.start_point_cloud_urban_classification_from_settings(tab, settings) {
                    Ok(_) => json!({ "started": true }),
                    Err(error) => json!({ "started": false, "reason": error }),
                }
            }
            Err(error) => json!({
                "started": false,
                "reason": format!("invalid urban classification settings: {error}"),
            }),
        }
    }

    fn cloud_urban_status(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        let dataset = &self.tabs[tab].point_cloud;
        match dataset.urban_job.as_ref() {
            None => json!({
                "running": false,
                "status": if dataset.urban_status.is_empty() {
                    "idle".to_string()
                } else {
                    dataset.urban_status.clone()
                },
            }),
            Some(job) => {
                let snapshot = job.snapshot();
                json!({
                    "running": true,
                    "stage": match snapshot.stage {
                        0 => "loading_references",
                        1 => "classifying",
                        2 => "validating",
                        _ => "completed",
                    },
                    "tile": snapshot.tile_index,
                    "tiles": snapshot.tile_total,
                    "points_done": snapshot.points_done,
                    "points_total": snapshot.points_total,
                    "building_features": snapshot.building_features,
                    "road_features": snapshot.road_features,
                    "tree_features": snapshot.tree_features,
                    "elapsed_ms": snapshot.elapsed_ms as u64,
                })
            }
        }
    }

    fn cloud_urban_cancel(&mut self) -> ScriptValue {
        self.cancel_point_cloud_urban_classification();
        json!(true)
    }

    fn project_info(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        match self.tabs[tab].spatial_project.as_ref() {
            Some((path, project)) => json!({
                "open": true,
                "api_version": 1,
                "id": project.id,
                "name": project.name,
                "path": path.display().to_string(),
                "schema_version": project.schema_version,
                "crs": project.spatial_reference.horizontal,
                "sources": project.sources.len(),
                "jobs": project.jobs.len(),
                "history": project.history.len(),
                "transactions": project.platform.transactions.len(),
                "workflows": project.platform.workflows.len(),
                "provenance": project.platform.provenance.len(),
                "sections": project.sections,
            }),
            None => json!({"open": false}),
        }
    }

    fn gis_layers(&mut self) -> ScriptValue {
        let tab = self.active_tab;
        json!(self.tabs[tab].gis_layers.iter().map(|layer| json!({
            "name": layer.name,
            "epsg": layer.epsg,
            "features": layer.features.len(),
            "fields": layer.fields,
            "envelope": layer.envelope(),
        })).collect::<Vec<_>>())
    }

    fn gis_import(&mut self, path: &str) -> ScriptValue {
        let before = self.tabs[self.active_tab].gis_layers.len();
        self.import_gis_source(self.active_tab, path.into());
        let after = self.tabs[self.active_tab].gis_layers.len();
        json!({"ok": after > before, "layers_added": after - before})
    }

    fn gis_export(&mut self, layer: &str, path: &str) -> ScriptValue {
        self.export_gis_layer(self.active_tab, layer, path.into());
        json!({"ok": std::path::Path::new(path).is_file(), "path": path})
    }

    fn gis_transform(&mut self, layer: &str, target_epsg: i64) -> ScriptValue {
        let Ok(target_epsg) = u16::try_from(target_epsg) else {
            return json!({"ok": false, "error": "EPSG code is out of range"});
        };
        self.transform_gis_layer(self.active_tab, layer, target_epsg);
        let epsg = self.tabs[self.active_tab]
            .gis_layers
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(layer))
            .map(|candidate| candidate.epsg);
        json!({"ok": epsg == Some(target_epsg), "epsg": epsg})
    }

    fn section_create(&mut self, definition: ScriptValue) -> ScriptValue {
        let tab = self.active_tab;
        let Some((project_path, project)) = self.tabs[tab].spatial_project.as_mut() else {
            return json!({"ok": false, "error": "create or open a spatial project first"});
        };
        let vector3 = |name: &str| -> Option<[f64; 3]> {
            let values = definition.get(name)?.as_array()?;
            if values.len() != 3 {
                return None;
            }
            Some([
                values[0].as_f64()?,
                values[1].as_f64()?,
                values[2].as_f64()?,
            ])
        };
        let Some(name) = definition.get("name").and_then(|value| value.as_str()) else {
            return json!({"ok": false, "error": "section name is required"});
        };
        let (Some(origin), Some(normal), Some(width)) = (
            vector3("origin"),
            vector3("normal"),
            definition.get("width").and_then(|value| value.as_f64()),
        ) else {
            return json!({"ok": false, "error": "origin, normal, and width are required"});
        };
        let kind = match definition.get("kind").and_then(|value| value.as_str()).unwrap_or("cross_section") {
            "plan" => ocs_pointcloud::SectionKind::Plan,
            "profile" => ocs_pointcloud::SectionKind::Profile,
            "arbitrary_plane" => ocs_pointcloud::SectionKind::ArbitraryPlane,
            _ => ocs_pointcloud::SectionKind::CrossSection,
        };
        let vertical_limits = definition
            .get("vertical_limits")
            .and_then(|value| value.as_array())
            .filter(|values| values.len() == 2)
            .and_then(|values| Some([values[0].as_f64()?, values[1].as_f64()?]));
        let id = format!("section-{}", project.sections.len() + 1);
        let section = ocs_pointcloud::NamedSection {
            id: id.clone(),
            name: name.into(),
            kind,
            origin,
            normal,
            axis_length: definition.get("axis_length").and_then(|value| value.as_f64()).unwrap_or(100.0),
            total_width: width,
            vertical_limits,
            crs: project.spatial_reference.horizontal.clone(),
            locked: false,
        };
        match project
            .upsert_section(section.clone())
            .and_then(|_| project.save_atomic(project_path.clone()))
        {
            Ok(()) => json!({"ok": true, "id": id, "name": name, "volume": section}),
            Err(error) => json!({"ok": false, "error": error.to_string()}),
        }
    }

    fn tools_list(&mut self) -> ScriptValue {
        let mut descriptors: Vec<_> = ocs_pointcloud::production_lidar_tools()
            .descriptors()
            .cloned()
            .collect();
        descriptors.extend(ocs_reality::reality_tools());
        descriptors.extend([
            ocs_pointcloud::ToolDescriptor {
                id: "gis.import".into(), name: "Import feature layer".into(), category: "GIS / Data".into(),
                description: "Import GeoPackage or GeoJSON".into(), input_schema: json!({"type":"object","required":["path"]}), output_schema: json!({"type":"object"}),
                requirements: Default::default(), api_version: 1,
            },
            ocs_pointcloud::ToolDescriptor {
                id: "gis.transform".into(), name: "Transform feature layer".into(), category: "GIS / Geodesy".into(),
                description: "Explicit CRS transformation with provenance".into(), input_schema: json!({"type":"object","required":["layer","target_epsg"]}), output_schema: json!({"type":"object"}),
                requirements: ocs_pointcloud::ToolRequirements { requires_crs: true, undo: ocs_pointcloud::UndoBehavior::Transaction, ..Default::default() }, api_version: 1,
            },
        ]);
        json!(descriptors)
    }

    fn tool_run(&mut self, tool_id: &str, parameters: ScriptValue) -> ScriptValue {
        match tool_id {
            "gis.import" => parameters.get("path").and_then(|value| value.as_str()).map_or_else(
                || json!({"ok": false, "error": "path is required"}),
                |path| self.gis_import(path),
            ),
            "gis.transform" => match (
                parameters.get("layer").and_then(|value| value.as_str()),
                parameters.get("target_epsg").and_then(|value| value.as_i64()),
            ) {
                (Some(layer), Some(epsg)) => self.gis_transform(layer, epsg),
                _ => json!({"ok": false, "error": "layer and target_epsg are required"}),
            },
            _ => json!({"ok": false, "error": format!("tool '{tool_id}' has no application executor yet")}),
        }
    }

    fn print(&mut self, message: &str) {
        self.command_line
            .push_output(format!("[script] {message}").as_str());
    }
}
