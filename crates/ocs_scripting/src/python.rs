//! Out-of-process CPython worker host.
//!
//! The Python side (repository `python/ocs`) runs in its own interpreter and
//! speaks JSON lines on stdout: each line is either a request
//! `{"id", "function", "args"}` or a `{"print"}` console line. This host
//! pumps that stream, dispatches requests through the same
//! [`ScriptBridge`] the Rhai engine uses, and writes replies to the child's
//! stdin. Stderr passes through prefixed for diagnostics.
//!
//! Running scripts in a separate process is the roadmap's deliberate
//! choice: nothing ever executes Python on the render/UI thread, and a
//! crashing or runaway script cannot take the application with it.

use crate::{ScriptBridge, ScriptOutcome};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where the `ocs` Python package lives, for embedding in the spawned
/// interpreter's `PYTHONPATH`.
pub fn python_package_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("OCS_PYTHON_PATH") {
        let path = PathBuf::from(path);
        if path.join("ocs").join("__init__.py").is_file() {
            return Ok(path);
        }
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let Some(exe_dir) = exe.parent() else {
        return Err("cannot locate the application directory".to_string());
    };
    // Installed layout: python/ beside the executable. Development layout:
    // <repo>/target/debug/<exe> with python/ at the repository root.
    for candidate in [
        exe_dir.join("python"),
        exe_dir.join("..").join("python"),
        exe_dir.join("..").join("..").join("python"),
    ] {
        if candidate.join("ocs").join("__init__.py").is_file() {
            return Ok(candidate);
        }
    }
    Err(
        "the ocs Python package was not found; set OCS_PYTHON_PATH to the folder containing it"
            .to_string(),
    )
}

/// Locate a Python interpreter: `OCS_PYTHON` override, then `python` on PATH.
fn python_interpreter() -> PathBuf {
    std::env::var_os("OCS_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"))
}

/// Spawn the worker for `script`, pump its protocol until it exits, and
/// return the outcome. Blocks the calling thread; the application runs it
/// on the same script worker thread pattern as Rhai.
pub fn run_python(bridge: &ScriptBridge, script: &Path) -> Result<ScriptOutcome, String> {
    let package_path = python_package_path()?;
    let interpreter = python_interpreter();
    let mut python_path = package_path.display().to_string();
    if let Some(existing) = std::env::var_os("PYTHONPATH") {
        python_path = format!("{};{}", python_path, existing.to_string_lossy());
    }
    let mut command = Command::new(&interpreter);
    command
        .arg("-u")
        .arg("-m")
        .arg("ocs.worker")
        .arg(script)
        .env("PYTHONPATH", python_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start {interpreter:?}: {error}"))?;
    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "the worker has no stdin".to_string())?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "the worker has no stdout".to_string())?;
    let child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| "the worker has no stderr".to_string())?;

    // Stderr passthrough: diagnostics arrive prefixed on the console.
    let stderr_bridge = bridge.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(child_stderr);
        for line in reader.lines().map_while(Result::ok) {
            stderr_bridge.print(&format!("[python] {line}"));
        }
    });

    let mut stdin = child_stdin;
    let reader = BufReader::new(child_stdout);
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            bridge.print(&format!("[python] {line}"));
            continue;
        };
        if let Some(message) = value.get("print").and_then(|v| v.as_str()) {
            bridge.print(message);
            continue;
        }
        let (Some(id), Some(function), args) = (
            value.get("id").and_then(|v| v.as_u64()),
            value
                .get("function")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            value
                .get("args")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        ) else {
            bridge.print(&format!("[python] malformed request: {line}"));
            continue;
        };
        let reply = match bridge.call(&function, args) {
            Ok(result) => serde_json::json!({"id": id, "ok": true, "value": result}),
            Err(error) => serde_json::json!({"id": id, "ok": false, "error": error}),
        };
        let mut payload = serde_json::to_string(&reply)
            .map_err(|error| format!("cannot encode reply: {error}"))?;
        payload.push('\n');
        if stdin.write_all(payload.as_bytes()).is_err() || stdin.flush().is_err() {
            break; // the interpreter went away
        }
    }
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| format!("cannot wait for the worker: {error}"))?;
    if status.success() {
        Ok(ScriptOutcome::default())
    } else {
        Err(format!(
            "python worker exited with {status}; see the console for the traceback"
        ))
    }
}
