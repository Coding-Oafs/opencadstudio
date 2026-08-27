//! Engine-agnostic macro scripting host for Open CAD Studio.
//!
//! A script runs on its own worker thread and talks to the application
//! through a channel: every `ocs::*` call in a script becomes one
//! [`ScriptRequest`] the app executes on its main thread through
//! [`OcsScriptApi`], returning a JSON value. Scripts therefore read live
//! state — source lists, class statistics, export progress — without ever
//! freezing the UI, and every operation goes through the same audited paths
//! as the ribbon and command line.
//!
//! The first engine is Rhai (pure Rust, native and web); the out-of-process
//! CPython worker speaks the same request protocol over JSON lines.

use rhai::plugin::*;
use std::path::Path;
use std::sync::mpsc;

#[cfg(not(target_arch = "wasm32"))]
pub mod python;
#[cfg(not(target_arch = "wasm32"))]
pub use python::{python_package_path, run_python};

/// Everything a script can ask the application to do. Implemented by the app
/// on its main thread; mocked in tests.
pub trait OcsScriptApi {
    /// Runs one command-line command (`"LAYER Walls"`, `"POINTCLOUDINDEX"`).
    fn command(&mut self, command: &str) -> ScriptValue;
    /// Attaches one LAS/LAZ file; returns its dataset source id.
    fn cloud_attach(&mut self, path: &str) -> ScriptValue;
    /// Attaches every LAS/LAZ under a folder (queued); returns queued count.
    fn cloud_attach_folder(&mut self, path: &str) -> ScriptValue;
    /// Attached sources: `[{id, path, points, displayed, edits}]`.
    fn cloud_sources(&mut self) -> ScriptValue;
    /// Per-class point counts over the current display working set.
    fn cloud_stats(&mut self) -> ScriptValue;
    /// Sets the persistent attribute filter used by spatial selections.
    fn cloud_filter(&mut self, filter_json: &str) -> ScriptValue;
    /// Selects points between two survey elevations.
    fn cloud_select_slice(&mut self, low: f64, high: f64) -> ScriptValue;
    /// Clears the active selection in every source.
    fn cloud_select_clear(&mut self) -> ScriptValue;
    /// Reclassifies the active selection as one ASPRS class.
    fn cloud_classify_selection(&mut self, classification: i64) -> ScriptValue;
    /// Classifies explicit source indices (`"10,25-40"`) of one source.
    fn cloud_classify(
        &mut self,
        source_id: &str,
        classification: i64,
        indices: &str,
    ) -> ScriptValue;
    /// Undoes the most recent point edit action.
    fn cloud_undo(&mut self) -> ScriptValue;
    /// Starts a merged export of every source; returns immediately.
    fn cloud_export_all(&mut self, path: &str) -> ScriptValue;
    /// Export/reprojection progress: `{running, completed, total}`.
    fn cloud_export_status(&mut self) -> ScriptValue;
    /// Detaches every attached source (session only; sources unchanged).
    fn cloud_detach(&mut self) -> ScriptValue;
    /// Lists the LAS/LAZ files directly under a folder (not recursive).
    fn cloud_list_folder(&mut self, path: &str) -> ScriptValue;
    /// Starts a native urban classification from a settings JSON preset;
    /// returns `{started, reason}` immediately.
    fn cloud_urban_classify(&mut self, settings_json: &str) -> ScriptValue;
    /// Urban job status: `{running, stage, tile, tiles, points_done,
    /// points_total, building_features, road_features, tree_features,
    /// elapsed_ms, status}`.
    fn cloud_urban_status(&mut self) -> ScriptValue;
    /// Requests cancellation of the running urban job.
    fn cloud_urban_cancel(&mut self) -> ScriptValue;
    /// Prints a line to the script console.
    fn print(&mut self, message: &str);
}

/// A script-visible value: JSON-shaped, engine-independent.
pub type ScriptValue = serde_json::Value;

/// Outcome of a finished script: everything it printed, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptOutcome {
    pub log: Vec<String>,
}

/// A request from a running script to the application.
pub enum ScriptRequest {
    Call {
        /// Name of the `ocs` module function, e.g. `"cloud_stats"`.
        function: String,
        /// JSON-encoded arguments.
        args: Vec<ScriptValue>,
        /// Replies with the call's JSON result or an error message.
        reply: mpsc::Sender<Result<ScriptValue, String>>,
    },
    /// A `print` line for the script console.
    Print(String),
}

/// Executes one request against `api`. The app's message loop and the test
/// mock share this, so script-visible behavior can never drift between them.
pub fn dispatch_script_request(api: &mut dyn OcsScriptApi, request: ScriptRequest) -> ScriptValue {
    match request {
        ScriptRequest::Print(message) => {
            api.print(&message);
            ScriptValue::Null
        }
        ScriptRequest::Call {
            function,
            args,
            reply,
        } => {
            let result = dispatch_call(api, &function, &args);
            let _ = reply.send(result.clone());
            result.unwrap_or(ScriptValue::Null)
        }
    }
}

fn arg<T: serde::de::DeserializeOwned>(
    args: &[ScriptValue],
    index: usize,
    function: &str,
) -> Result<T, String> {
    serde_json::from_value(args.get(index).cloned().unwrap_or(ScriptValue::Null))
        .map_err(|_| format!("{function}: argument {index} has the wrong type"))
}

fn dispatch_call(
    api: &mut dyn OcsScriptApi,
    function: &str,
    args: &[ScriptValue],
) -> Result<ScriptValue, String> {
    match function {
        "command" => {
            let command: String = arg(args, 0, function)?;
            Ok(api.command(&command))
        }
        "cloud_attach" => {
            let path: String = arg(args, 0, function)?;
            Ok(api.cloud_attach(&path))
        }
        "cloud_attach_folder" => {
            let path: String = arg(args, 0, function)?;
            Ok(api.cloud_attach_folder(&path))
        }
        "cloud_sources" => Ok(api.cloud_sources()),
        "cloud_stats" => Ok(api.cloud_stats()),
        "cloud_filter" => {
            let filter: String = arg(args, 0, function)?;
            Ok(api.cloud_filter(&filter))
        }
        "cloud_select_slice" => {
            let (low, high): (f64, f64) = (arg(args, 0, function)?, arg(args, 1, function)?);
            Ok(api.cloud_select_slice(low, high))
        }
        "cloud_select_clear" => Ok(api.cloud_select_clear()),
        "cloud_classify_selection" => {
            let classification: i64 = arg(args, 0, function)?;
            Ok(api.cloud_classify_selection(classification))
        }
        "cloud_classify" => {
            let (source, classification, indices): (String, i64, String) = (
                arg(args, 0, function)?,
                arg(args, 1, function)?,
                arg(args, 2, function)?,
            );
            Ok(api.cloud_classify(&source, classification, &indices))
        }
        "cloud_undo" => Ok(api.cloud_undo()),
        "cloud_export_all" => {
            let path: String = arg(args, 0, function)?;
            Ok(api.cloud_export_all(&path))
        }
        "cloud_export_status" => Ok(api.cloud_export_status()),
        "cloud_detach" => Ok(api.cloud_detach()),
        "cloud_list_folder" => {
            let path: String = arg(args, 0, function)?;
            Ok(api.cloud_list_folder(&path))
        }
        "cloud_urban_classify" => {
            let settings_json: String = arg(args, 0, function)?;
            Ok(api.cloud_urban_classify(&settings_json))
        }
        "cloud_urban_status" => Ok(api.cloud_urban_status()),
        "cloud_urban_cancel" => Ok(api.cloud_urban_cancel()),
        other => Err(format!("unknown script function: {other}")),
    }
}

/// Script-side handle: sends each `ocs` call to the application and blocks
/// for the reply. Cheap to clone; one per script run.
#[derive(Clone)]
pub struct ScriptBridge {
    requests: mpsc::Sender<ScriptRequest>,
}

impl ScriptBridge {
    pub fn new(requests: mpsc::Sender<ScriptRequest>) -> Self {
        Self { requests }
    }

    pub fn call(&self, function: &str, args: Vec<ScriptValue>) -> Result<ScriptValue, String> {
        let (reply, reply_rx) = mpsc::channel();
        self.requests
            .send(ScriptRequest::Call {
                function: function.to_string(),
                args,
                reply,
            })
            .map_err(|_| "the application stopped accepting script calls".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "the application dropped a script call".to_string())?
    }

    pub fn print(&self, message: &str) {
        let _ = self
            .requests
            .send(ScriptRequest::Print(message.to_string()));
    }
}

/// Runs a Rhai script. The bridge is pushed into scope as `ocs`, and every
/// API function registers as a method on it, so scripts read
/// `ocs.cloud_stats()`. Errors inside calls become strings a script can
/// print — macros fail loudly, never silently. The engine is sandboxed with
/// operation and call-depth limits so a runaway macro cannot hang the app.
pub fn run_rhai(bridge: &ScriptBridge, source: &str) -> Result<ScriptOutcome, String> {
    let mut engine = rhai::Engine::new();
    engine.set_max_expr_depths(64, 64);
    engine.set_max_operations(2_000_000);
    engine.set_max_call_levels(64);
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));

    // Method-style registration: the first `&mut ScriptBridge` parameter
    // makes each function callable as `ocs.<name>(...)` once the bridge is
    // in scope under that name.
    macro_rules! method {
        ($name:literal, [$($param:ident),*], $function:literal, [$($json:expr),*]) => {
            engine.register_fn($name, |bridge: &mut ScriptBridge $(, $param: rhai::Dynamic)*| {
                let args = vec![$(dynamic_to_serde(&$param)),*];
                match bridge.call($function, args) {
                    Ok(value) => serde_to_dynamic(value),
                    Err(error) => rhai::Dynamic::from(format!("error: {error}")),
                }
            });
        };
    }

    method!("command", [command], "command", [command.clone()]);
    method!("cloud_attach", [path], "cloud_attach", [path.clone()]);
    method!(
        "cloud_attach_folder",
        [path],
        "cloud_attach_folder",
        [path.clone()]
    );
    method!("cloud_filter", [filter], "cloud_filter", [filter.clone()]);
    method!(
        "cloud_select_slice",
        [low, high],
        "cloud_select_slice",
        [low.clone(), high.clone()]
    );
    method!(
        "cloud_classify_selection",
        [classification],
        "cloud_classify_selection",
        [classification.clone()]
    );
    method!(
        "cloud_classify",
        [source, classification, indices],
        "cloud_classify",
        [source.clone(), classification.clone(), indices.clone()]
    );
    method!(
        "cloud_export_all",
        [path],
        "cloud_export_all",
        [path.clone()]
    );
    method!(
        "cloud_list_folder",
        [path],
        "cloud_list_folder",
        [path.clone()]
    );
    method!(
        "cloud_urban_classify",
        [settings],
        "cloud_urban_classify",
        [settings.clone()]
    );
    method!("cloud_sources", [], "cloud_sources", []);
    method!("cloud_stats", [], "cloud_stats", []);
    method!("cloud_select_clear", [], "cloud_select_clear", []);
    method!("cloud_undo", [], "cloud_undo", []);
    method!("cloud_detach", [], "cloud_detach", []);
    method!("cloud_export_status", [], "cloud_export_status", []);
    method!("cloud_urban_status", [], "cloud_urban_status", []);
    method!("cloud_urban_cancel", [], "cloud_urban_cancel", []);
    let log_for_method = log.clone();
    engine.register_fn(
        "log",
        move |bridge: &mut ScriptBridge, message: rhai::Dynamic| {
            let text = message.to_string();
            log_for_method.borrow_mut().push(text.clone());
            bridge.print(&text);
            rhai::Dynamic::UNIT
        },
    );

    let log_for_print = log.clone();
    let bridge_for_print = bridge.clone();
    engine.on_print(move |message: &str| {
        log_for_print.borrow_mut().push(message.to_string());
        bridge_for_print.print(message);
    });

    let mut scope = rhai::Scope::new();
    scope.push("ocs", bridge.clone());
    let result = engine.eval_with_scope::<rhai::Dynamic>(&mut scope, source);
    let log = std::mem::take(&mut *log.borrow_mut());
    match result {
        Ok(_) => Ok(ScriptOutcome { log }),
        Err(error) => Err(format!("script error: {error}")),
    }
}

/// Rhai values script arguments arrive as; only the shapes the API needs are
/// translated, anything exotic becomes its string form.
fn dynamic_to_serde(value: &rhai::Dynamic) -> ScriptValue {
    if value.is_unit() {
        return ScriptValue::Null;
    }
    if let Some(flag) = value.clone().try_cast::<bool>() {
        return ScriptValue::Bool(flag);
    }
    if let Some(int) = value.clone().try_cast::<i64>() {
        return ScriptValue::from(int);
    }
    if let Some(float) = value.clone().try_cast::<f64>() {
        return ScriptValue::from(float);
    }
    ScriptValue::String(value.to_string())
}

fn serde_to_dynamic(value: ScriptValue) -> rhai::Dynamic {
    match value {
        ScriptValue::Null => rhai::Dynamic::UNIT,
        ScriptValue::Bool(value) => rhai::Dynamic::from(value),
        ScriptValue::Number(number) => {
            if let Some(int) = number.as_i64() {
                rhai::Dynamic::from(int)
            } else {
                rhai::Dynamic::from(number.as_f64().unwrap_or_default())
            }
        }
        ScriptValue::String(value) => rhai::Dynamic::from(value),
        ScriptValue::Array(values) => rhai::Dynamic::from_array(
            values
                .into_iter()
                .map(serde_to_dynamic)
                .collect::<Vec<rhai::Dynamic>>(),
        ),
        ScriptValue::Object(map) => rhai::Dynamic::from_map(
            map.into_iter()
                .map(|(key, value)| (key.into(), serde_to_dynamic(value)))
                .collect::<rhai::Map>(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// Loopback "app": serves script requests from a recorded MockApi so the
    /// full script → request → dispatch → reply path runs in-process.
    struct MockApp {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl OcsScriptApi for MockApp {
        fn command(&mut self, command: &str) -> ScriptValue {
            self.calls
                .lock()
                .unwrap()
                .push(format!("command {command}"));
            json!(true)
        }
        fn cloud_attach(&mut self, path: &str) -> ScriptValue {
            self.calls.lock().unwrap().push(format!("attach {path}"));
            json!("source-1")
        }
        fn cloud_attach_folder(&mut self, path: &str) -> ScriptValue {
            self.calls.lock().unwrap().push(format!("folder {path}"));
            json!(6)
        }
        fn cloud_sources(&mut self) -> ScriptValue {
            json!([{ "id": "source-1", "path": "a.laz", "points": 100, "displayed": 50, "edits": 2 }])
        }
        fn cloud_stats(&mut self) -> ScriptValue {
            json!({ "2": 60, "6": 40 })
        }
        fn cloud_filter(&mut self, filter: &str) -> ScriptValue {
            self.calls.lock().unwrap().push(format!("filter {filter}"));
            json!(true)
        }
        fn cloud_select_slice(&mut self, low: f64, high: f64) -> ScriptValue {
            self.calls
                .lock()
                .unwrap()
                .push(format!("slice {low}..{high}"));
            json!(42)
        }
        fn cloud_select_clear(&mut self) -> ScriptValue {
            self.calls.lock().unwrap().push("clear".into());
            json!(true)
        }
        fn cloud_classify_selection(&mut self, classification: i64) -> ScriptValue {
            self.calls
                .lock()
                .unwrap()
                .push(format!("classify {classification}"));
            json!(42)
        }
        fn cloud_classify(&mut self, source: &str, class: i64, indices: &str) -> ScriptValue {
            self.calls
                .lock()
                .unwrap()
                .push(format!("classify {source} {class} {indices}"));
            json!(1)
        }
        fn cloud_undo(&mut self) -> ScriptValue {
            json!(true)
        }
        fn cloud_export_all(&mut self, path: &str) -> ScriptValue {
            self.calls.lock().unwrap().push(format!("export {path}"));
            json!(true)
        }
        fn cloud_export_status(&mut self) -> ScriptValue {
            json!({ "running": false })
        }
        fn cloud_detach(&mut self) -> ScriptValue {
            self.calls.lock().unwrap().push("detach".into());
            json!(true)
        }
        fn cloud_list_folder(&mut self, path: &str) -> ScriptValue {
            json!([format!("{path}\\a.laz"), format!("{path}\\b.laz")])
        }
        fn cloud_urban_classify(&mut self, settings_json: &str) -> ScriptValue {
            self.calls
                .lock()
                .unwrap()
                .push(format!("urban {settings_json}"));
            json!({ "started": true })
        }
        fn cloud_urban_status(&mut self) -> ScriptValue {
            json!({ "running": false })
        }
        fn cloud_urban_cancel(&mut self) -> ScriptValue {
            json!(true)
        }
        fn print(&mut self, message: &str) {
            self.calls.lock().unwrap().push(format!("print {message}"));
        }
    }

    fn serve_loopback() -> (mpsc::Sender<ScriptRequest>, Arc<Mutex<Vec<String>>>) {
        let (tx, rx) = mpsc::channel();
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = calls.clone();
        std::thread::spawn(move || {
            let mut app = MockApp { calls: recorded };
            for request in rx {
                dispatch_script_request(&mut app, request);
            }
        });
        (tx, calls)
    }

    #[test]
    fn rhai_script_runs_the_full_macro_flow() {
        let (tx, calls) = serve_loopback();
        let bridge = ScriptBridge::new(tx);
        let script = r#"
            let count = ocs.cloud_attach_folder("D:\\survey");
            ocs.log("attached " + count + " tiles");
            let stats = ocs.cloud_stats();
            let ground = stats["2"];
            if (ground > 50) {
                ocs.cloud_select_slice(10.0, 20.0);
                ocs.cloud_classify_selection(2);
            }
            ocs.cloud_urban_classify(`{"scope":"folder"}`);
            let urban = ocs.cloud_urban_status();
            if (urban["running"]) {
                ocs.cloud_urban_cancel();
            }
            ocs.cloud_export_all("D:\\out\\merged.laz");
        "#;
        let outcome = run_rhai(&bridge, script).expect("script runs");
        let calls = calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.starts_with("folder")));
        assert!(calls.iter().any(|c| c.contains("slice")));
        assert!(calls.iter().any(|c| c == "classify 2"));
        assert!(calls.iter().any(|c| c.starts_with("urban")));
        assert!(calls.iter().any(|c| c.starts_with("export")));
        assert!(
            outcome.log.iter().any(|line| line.contains("6 tiles")),
            "print output must reach the log: {:?}",
            outcome.log
        );
    }

    #[test]
    fn script_errors_surface_as_errors() {
        let (tx, _calls) = serve_loopback();
        let bridge = ScriptBridge::new(tx);
        let outcome = run_rhai(&bridge, "let x = ;");
        assert!(outcome.is_err());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod python_tests {
    use super::*;
    use serde_json::json;

    fn python_available() -> bool {
        std::process::Command::new("python")
            .arg("-V")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Full loop: a real CPython interpreter runs a script that imports the
    /// `ocs` package, makes calls, and prints; the host dispatches them
    /// against the mock API. Skipped when no interpreter is on PATH.
    #[test]
    fn python_worker_runs_end_to_end() {
        if !python_available() {
            eprintln!("python is not on PATH; skipping");
            return;
        }
        let package_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python");
        assert!(package_path.join("ocs").join("__init__.py").is_file());
        let script_dir = std::env::temp_dir().join(format!("ocs-python-{}", std::process::id()));
        std::fs::create_dir_all(&script_dir).unwrap();
        let script = script_dir.join("probe.py");
        std::fs::write(
            &script,
            "import ocs\n\
             sources = ocs.cloud_sources()\n\
             ocs.log('python sees %d source(s)' % len(sources))\n\
             ocs.cloud_attach('a.las')\n\
             print('printed from python')\n",
        )
        .unwrap();
        std::env::set_var("OCS_PYTHON_PATH", &package_path);
        std::env::set_var("OCS_PYTHON", "python");

        let (tx, rx) = mpsc::channel();
        let (log_tx, log_rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for request in rx {
                match request {
                    ScriptRequest::Print(message) => log_tx.send(message).unwrap(),
                    ScriptRequest::Call {
                        function,
                        args,
                        reply,
                    } => {
                        let result = if function == "cloud_sources" {
                            Ok(json!([{"id": "source-1", "points": 100}]))
                        } else if function == "cloud_attach" {
                            Ok(json!("source-2"))
                        } else {
                            Err(format!("unexpected function {function} {args:?}"))
                        };
                        let _ = reply.send(result);
                    }
                }
            }
        });
        let bridge = ScriptBridge::new(tx);
        let outcome = run_python(&bridge, &script).expect("python worker completes");
        assert!(outcome.log.is_empty() || outcome.log.iter().all(|line| !line.is_empty()));
        // Both the protocol log and print() reached the console channel.
        let mut console: Vec<String> = Vec::new();
        while let Ok(line) = log_rx.try_recv() {
            console.push(line);
        }
        assert!(
            console
                .iter()
                .any(|line| line.contains("python sees 1 source")),
            "console: {console:?}"
        );
        assert!(
            console
                .iter()
                .any(|line| line.contains("printed from python")),
            "console: {console:?}"
        );
    }
}
