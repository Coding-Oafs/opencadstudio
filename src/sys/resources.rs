//! System resource detection and point-cloud budget derivation.
//!
//! LiDAR workflows are bounded by the machine's real memory, not by a fixed
//! default: a 4-million-point display budget that is comfortable on a 32 GB
//! workstation will thrash an 8 GB laptop. This module reports installed and
//! available RAM, records the wgpu device limits once the renderer is up, and
//! derives the point-cloud CPU/GPU/point budgets from them. Explicit user
//! overrides are persisted to `performance.json` in the config directory and
//! win over detection.

use std::sync::{Mutex, OnceLock};

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * 1024 * 1024;

// ── System memory ────────────────────────────────────────────────────────────

/// Installed and currently available system memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemMemory {
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

static MEMORY: OnceLock<SystemMemory> = OnceLock::new();

/// Installed memory, detected once on first use.
pub fn system_memory() -> SystemMemory {
    *MEMORY.get_or_init(detect_memory)
}

/// Installed RAM in bytes, if the platform reports it.
pub fn total_memory() -> Option<u64> {
    system_memory().total_bytes
}

/// RAM that could be allocated right now. Queried live (not from the cached
/// snapshot) so attach-time guards see what other applications left free.
pub fn available_memory() -> Option<u64> {
    let fresh = detect_memory();
    fresh.available_bytes.or_else(|| {
        // Platforms without an available-memory counter: assume half of the
        // installed total is a safe working share.
        system_memory().total_bytes.map(|t| t / 2)
    })
}

#[cfg(target_os = "windows")]
fn detect_memory() -> SystemMemory {
    use windows_sys::Win32::System::SystemInformation::{
        GlobalMemoryStatusEx, MEMORYSTATUSEX,
    };
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            SystemMemory {
                total_bytes: Some(status.ullTotalPhys),
                available_bytes: Some(status.ullAvailPhys),
            }
        } else {
            SystemMemory::default()
        }
    }
}

#[cfg(target_os = "linux")]
fn detect_memory() -> SystemMemory {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return SystemMemory::default();
    };
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let kB = || {
            value
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| n.saturating_mul(1024))
        };
        match key.trim() {
            "MemTotal" => total = total.or_else(kB),
            "MemAvailable" => available = available.or_else(kB),
            _ => {}
        }
    }
    SystemMemory {
        total_bytes: total,
        available_bytes: available,
    }
}

#[cfg(target_os = "macos")]
fn detect_memory() -> SystemMemory {
    // `sysctl -n hw.memsize` prints installed bytes. macOS publishes no
    // stable "available memory" counter without private APIs, so report the
    // total only; `available_memory` falls back to half of it.
    let total = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    SystemMemory {
        total_bytes: total,
        available_bytes: None,
    }
}

#[cfg(target_arch = "wasm32")]
fn detect_memory() -> SystemMemory {
    // `navigator.deviceMemory` is a coarse whole-GiB value (Chromium only,
    // capped at 8): enough to pick a conservative tier, never enough to be
    // greedy with. Browsers don't expose available memory.
    let total = web_sys::window().and_then(|w| {
        let nav = w.navigator();
        js_sys::Reflect::get(
            nav.as_ref(),
            &wasm_bindgen::JsValue::from_str("deviceMemory"),
        )
        .ok()
        .and_then(|v| v.as_f64())
        .map(|gb| (gb.max(1.0).ceil() as u64).saturating_mul(GB))
    });
    SystemMemory {
        total_bytes: total,
        available_bytes: None,
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_arch = "wasm32"
)))]
fn detect_memory() -> SystemMemory {
    SystemMemory::default()
}

// ── GPU limits ───────────────────────────────────────────────────────────────

/// Device limits captured from the wgpu device at pipeline creation. wgpu
/// cannot report total VRAM; `max_buffer_size` is the strongest available
/// proxy for how much the GPU will accept per buffer, and the point arena
/// shards within it.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuLimits {
    pub max_buffer_size: Option<u64>,
}

static GPU: OnceLock<GpuLimits> = OnceLock::new();

/// Record the renderer device's limits. Called once when the scene pipelines
/// are built; later calls are ignored so pane re-creation doesn't matter.
pub fn record_gpu_limits(max_buffer_size: u64) {
    let _ = GPU.set(GpuLimits {
        max_buffer_size: Some(max_buffer_size),
    });
}

/// The recorded device limits, if the renderer is up yet.
pub fn gpu_limits() -> GpuLimits {
    GPU.get().copied().unwrap_or_default()
}

// ── Budget derivation ────────────────────────────────────────────────────────

/// How the effective budget was chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BudgetPreset {
    /// Derived from detected RAM and GPU limits.
    #[default]
    Auto,
    /// Fixed conservative values for weak machines.
    Low,
    /// Today's historical defaults.
    Medium,
    /// Fixed aggressive values for workstations.
    High,
    /// Per-field user overrides; unset fields fall back to Auto.
    Custom,
}

impl BudgetPreset {
    fn as_str(self) -> &'static str {
        match self {
            BudgetPreset::Auto => "AUTO",
            BudgetPreset::Low => "LOW",
            BudgetPreset::Medium => "MEDIUM",
            BudgetPreset::High => "HIGH",
            BudgetPreset::Custom => "CUSTOM",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "AUTO" => Some(BudgetPreset::Auto),
            "LOW" => Some(BudgetPreset::Low),
            "MEDIUM" => Some(BudgetPreset::Medium),
            "HIGH" => Some(BudgetPreset::High),
            "CUSTOM" => Some(BudgetPreset::Custom),
            _ => None,
        }
    }
}

/// Effective point-cloud working limits for this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    /// Maximum points contributing to the merged display across all sources.
    pub point_budget: usize,
    /// VRAM the point arena may fill.
    pub gpu_budget_bytes: usize,
    /// RAM the resident tile pool and attach samples may fill.
    pub cpu_budget_bytes: usize,
    pub preset: BudgetPreset,
}

/// Budget tier for an installed-RAM class: (points, GPU bytes, CPU bytes).
/// Mid-range machines keep the historical defaults so the tier system never
/// makes an upgrade worse.
fn auto_tier(total_bytes: Option<u64>) -> (usize, usize, usize) {
    let Some(t) = total_bytes else {
        return (4_000_000, 512 * MB as usize, 1024 * MB as usize);
    };
    if t < 8 * GB {
        (1_000_000, 128 * MB as usize, 256 * MB as usize)
    } else if t < 16 * GB {
        (2_000_000, 256 * MB as usize, 384 * MB as usize)
    } else if t < 32 * GB {
        (4_000_000, 512 * MB as usize, 1024 * MB as usize)
    } else if t < 64 * GB {
        (8_000_000, 1024 * MB as usize, 2 * GB as usize)
    } else {
        (16_000_000, 2 * GB as usize, 4 * GB as usize)
    }
}

fn preset_values(preset: BudgetPreset) -> Option<(usize, usize, usize)> {
    match preset {
        BudgetPreset::Low => Some((1_000_000, 128 * MB as usize, 256 * MB as usize)),
        BudgetPreset::Medium => Some((4_000_000, 512 * MB as usize, 1024 * MB as usize)),
        BudgetPreset::High => Some((16_000_000, 2 * GB as usize, 4 * GB as usize)),
        _ => None,
    }
}

/// The detected budget: RAM tier, then clamped so the GPU share stays within
/// half of the device's per-buffer limit on low-limit adapters.
fn detected_budget() -> ResourceBudget {
    let (mut points, mut gpu, cpu) = auto_tier(total_memory());
    if let Some(max_buffer) = gpu_limits().max_buffer_size {
        // Low-limit adapters (WebGL2-era, small iGPUs) would reject the tier's
        // buffer requests; keep at least a minimal arena.
        let per_buffer_cap = ((max_buffer / 2).max(64 * MB)) as usize;
        gpu = gpu.min(per_buffer_cap);
    }
    points = points.max(1_000);
    ResourceBudget {
        point_budget: points,
        gpu_budget_bytes: gpu,
        cpu_budget_bytes: cpu,
        preset: BudgetPreset::Auto,
    }
}

/// The budget in force: detected values, or the user's persisted preset /
/// custom overrides when set.
pub fn effective_budget() -> ResourceBudget {
    let stored = stored_settings();
    let detected = detected_budget();
    let preset = BudgetPreset::parse(&stored.preset).unwrap_or_default();
    match preset_values(preset) {
        Some((points, gpu, cpu)) => ResourceBudget {
            point_budget: points,
            gpu_budget_bytes: gpu,
            cpu_budget_bytes: cpu,
            preset,
        },
        // Auto resolves to the detected values; Custom overlays only the
        // fields the user actually set.
        None => {
            let (points, gpu, cpu) = if preset == BudgetPreset::Custom {
                (
                    stored.point_budget.unwrap_or(detected.point_budget),
                    stored
                        .gpu_budget_bytes
                        .unwrap_or(detected.gpu_budget_bytes),
                    stored
                        .cpu_budget_bytes
                        .unwrap_or(detected.cpu_budget_bytes),
                )
            } else {
                (
                    detected.point_budget,
                    detected.gpu_budget_bytes,
                    detected.cpu_budget_bytes,
                )
            };
            ResourceBudget {
                point_budget: points,
                gpu_budget_bytes: gpu,
                cpu_budget_bytes: cpu,
                preset,
            }
        }
    }
}

/// Apply a named preset (Auto/Low/Medium/High) and persist it.
pub fn set_preset(preset: BudgetPreset) {
    if preset == BudgetPreset::Custom {
        return;
    }
    let mut settings = stored_settings();
    settings.preset = preset.as_str().to_string();
    settings.point_budget = None;
    settings.gpu_budget_bytes = None;
    settings.cpu_budget_bytes = None;
    store_settings(&settings);
}

/// Set per-field custom overrides (`None` keeps the detected value for that
/// field) and persist them as the Custom preset.
pub fn set_custom(
    point_budget: Option<usize>,
    gpu_budget_bytes: Option<usize>,
    cpu_budget_bytes: Option<usize>,
) {
    let mut settings = stored_settings();
    settings.preset = BudgetPreset::Custom.as_str().to_string();
    if point_budget.is_some() {
        settings.point_budget = point_budget;
    }
    if gpu_budget_bytes.is_some() {
        settings.gpu_budget_bytes = gpu_budget_bytes;
    }
    if cpu_budget_bytes.is_some() {
        settings.cpu_budget_bytes = cpu_budget_bytes;
    }
    store_settings(&settings);
}

/// Human-readable summary for the command line and the point-cloud manager.
pub fn budget_summary() -> String {
    let memory = system_memory();
    let budget = effective_budget();
    let ram = memory
        .total_bytes
        .map(|b| format!("{:.1} GB", b as f64 / GB as f64))
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "preset {} · RAM {} · points {} · GPU {} MiB · CPU {} MiB",
        budget.preset.as_str(),
        ram,
        budget.point_budget,
        budget.gpu_budget_bytes / MB as usize,
        budget.cpu_budget_bytes / MB as usize,
    )
}

// ── Persistence ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct PerformanceSettings {
    #[serde(default)]
    preset: String,
    #[serde(default)]
    point_budget: Option<usize>,
    #[serde(default)]
    gpu_budget_bytes: Option<usize>,
    #[serde(default)]
    cpu_budget_bytes: Option<usize>,
}

#[cfg(not(target_arch = "wasm32"))]
static SETTINGS: OnceLock<Mutex<PerformanceSettings>> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn settings_store() -> &'static Mutex<PerformanceSettings> {
    SETTINGS.get_or_init(|| {
        let loaded = crate::config::config_dir()
            .map(|d| d.join("performance.json"))
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Mutex::new(loaded)
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn stored_settings() -> PerformanceSettings {
    settings_store()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|p| p.into_inner().clone())
}

#[cfg(not(target_arch = "wasm32"))]
fn store_settings(settings: &PerformanceSettings) {
    if let Ok(mut store) = settings_store().lock() {
        *store = settings.clone();
    }
    if let Some(dir) = crate::config::config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string_pretty(settings) {
            let _ = std::fs::write(dir.join("performance.json"), text);
        }
    }
}

// Web: nothing to persist; the detected budget is always in force.
#[cfg(target_arch = "wasm32")]
fn stored_settings() -> PerformanceSettings {
    PerformanceSettings::default()
}

#[cfg(target_arch = "wasm32")]
fn store_settings(_settings: &PerformanceSettings) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_tier_is_monotonic_in_ram() {
        let tiers = [
            auto_tier(Some(4 * GB)),
            auto_tier(Some(8 * GB)),
            auto_tier(Some(16 * GB)),
            auto_tier(Some(32 * GB)),
            auto_tier(Some(64 * GB)),
        ];
        for pair in tiers.windows(2) {
            assert!(pair[0].0 <= pair[1].0, "points must not shrink with RAM");
            assert!(
                pair[0].1 <= pair[1].1,
                "GPU budget must not shrink with RAM"
            );
            assert!(
                pair[0].2 <= pair[1].2,
                "CPU budget must not shrink with RAM"
            );
        }
    }

    #[test]
    fn auto_tier_keeps_historical_defaults_mid_range() {
        // A 16–32 GB machine keeps the previous hardcoded defaults so the
        // tier system only ever raises budgets for bigger machines.
        let (points, gpu, cpu) = auto_tier(Some(16 * GB));
        assert_eq!(points, 4_000_000);
        assert_eq!(gpu, 512 * MB as usize);
        assert_eq!(cpu, 1024 * MB as usize);
    }

    #[test]
    fn unknown_ram_falls_back_to_conservative_defaults() {
        let (points, gpu, cpu) = auto_tier(None);
        assert_eq!(points, 4_000_000);
        assert_eq!(gpu, 512 * MB as usize);
        assert_eq!(cpu, 1024 * MB as usize);
    }

    #[test]
    fn preset_round_trips_through_names() {
        for preset in [
            BudgetPreset::Auto,
            BudgetPreset::Low,
            BudgetPreset::Medium,
            BudgetPreset::High,
            BudgetPreset::Custom,
        ] {
            assert_eq!(BudgetPreset::parse(preset.as_str()), Some(preset));
        }
        assert_eq!(BudgetPreset::parse("nope"), None);
    }
}
