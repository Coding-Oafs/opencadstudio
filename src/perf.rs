//! Shared performance trace for terminal output and the in-app `PERF` panel.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_LINES: usize = 500;

static UI_ENABLED: AtomicBool = AtomicBool::new(false);
static ENV_ENABLED: OnceLock<bool> = OnceLock::new();
static LINES: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

fn lines() -> &'static Mutex<VecDeque<String>> {
    LINES.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_LINES)))
}

/// True when tracing was requested with `PERF=1` or the in-app panel is open.
pub fn enabled() -> bool {
    UI_ENABLED.load(Ordering::Relaxed)
        || *ENV_ENABLED.get_or_init(|| std::env::var_os("PERF").is_some())
}

/// Enable or disable collection driven by the in-app `PERF` command.
pub fn set_ui_enabled(enabled: bool) {
    UI_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Write one performance line to stderr and retain it for the in-app panel.
pub fn record(args: fmt::Arguments<'_>) {
    if !enabled() {
        return;
    }
    let line = args.to_string();
    eprintln!("{line}");
    let mut entries = lines().lock().unwrap_or_else(|e| e.into_inner());
    if entries.len() == MAX_LINES {
        entries.pop_front();
    }
    entries.push_back(line);
}

/// Plain-text snapshot used by the panel and its Copy button.
pub fn snapshot_text() -> String {
    let entries = lines().lock().unwrap_or_else(|e| e.into_inner());
    entries
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn clear() {
    lines().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[macro_export]
macro_rules! perf_record {
    ($($arg:tt)*) => {
        $crate::perf::record(format_args!($($arg)*))
    };
}
