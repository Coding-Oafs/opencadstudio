// print_to_printer — send the current layout to the system printer.
//
// Strategy:
//   1. Render the drawing to a temporary PDF (reusing the PDF export pipeline).
//   2. Send that PDF to the system printer with `lp` (Linux/macOS) or
//      `ShellExecute PRINT` (Windows).
//
// The function is async so the UI remains responsive while the job is queued.

#[cfg(not(target_arch = "wasm32"))]
use crate::io::pdf_export;
use crate::io::plot_style::PlotStyleTable;
use crate::scene::model::hatch_model::HatchModel;
use crate::scene::WireModel;

/// Extra options for a print job. On CUPS (Linux/macOS) these map to `lp`
/// flags / `-o` options. On Windows only `printer` is honoured (via the
/// "printto" shell verb); copies / quality / colour need the driver DEVMODE
/// and are ignored there.
#[derive(Debug, Clone, Default)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct PrintOptions {
    /// Target printer name, or `None` for the system default.
    pub printer: Option<String>,
    /// Number of copies (treated as at least 1).
    pub copies: u32,
    /// Force grayscale output.
    pub mono: bool,
    /// Print quality: "Draft" | "Normal" | "High" | "Maximum".
    pub quality: Option<String>,
    /// Rasterisation resolution in DPI.
    pub dpi: Option<u32>,
}

// Printing routes through the native PDF pipeline + the OS print command, so it
// is native-only; the web build gets a stub so the call site still compiles.
#[cfg(target_arch = "wasm32")]
pub async fn print_wires(
    _wires: std::sync::Arc<Vec<WireModel>>,
    _hatches: Vec<HatchModel>,
    _wipeouts: Vec<HatchModel>,
    _paper_w: f64,
    _paper_h: f64,
    _offset_x: f64,
    _offset_y: f64,
    _rotation_deg: i32,
    _scale: f32,
    _clip: Option<(f32, f32, f32, f32)>,
    _plot_style: Option<PlotStyleTable>,
) -> Result<String, String> {
    Err("Printing is not available in the web version.".into())
}

#[cfg(target_arch = "wasm32")]
pub fn list_printers() -> Vec<String> {
    Vec::new()
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
pub async fn print_wires_with(
    _wires: std::sync::Arc<Vec<WireModel>>,
    _hatches: Vec<HatchModel>,
    _wipeouts: Vec<HatchModel>,
    _paper_w: f64,
    _paper_h: f64,
    _offset_x: f64,
    _offset_y: f64,
    _rotation_deg: i32,
    _scale: f32,
    _clip: Option<(f32, f32, f32, f32)>,
    _plot_style: Option<PlotStyleTable>,
    _opts: PrintOptions,
) -> Result<String, String> {
    Err("Printing is not available in the web version.".into())
}

#[cfg(target_arch = "wasm32")]
pub fn open_in_viewer(_path: &std::path::Path) -> Result<(), String> {
    Err("Preview is not available in the web version.".into())
}

#[cfg(target_arch = "wasm32")]
pub fn print_existing_pdf(_path: &std::path::Path, _opts: &PrintOptions) -> Result<String, String> {
    Err("Printing is not available in the web version.".into())
}

/// Render `wires` (plus hatch / wipeout fills) to a temp PDF and dispatch it
/// to the default system printer.
///
/// Returns `Ok(printer_name)` on success or `Err(message)` on failure.
#[cfg(not(target_arch = "wasm32"))]
pub async fn print_wires(
    wires: std::sync::Arc<Vec<WireModel>>,
    hatches: Vec<HatchModel>,
    wipeouts: Vec<HatchModel>,
    paper_w: f64,
    paper_h: f64,
    offset_x: f64,
    offset_y: f64,
    rotation_deg: i32,
    scale: f32,
    clip: Option<(f32, f32, f32, f32)>,
    plot_style: Option<PlotStyleTable>,
) -> Result<String, String> {
    // ── 1. Write to a named temp file ─────────────────────────────────────
    let tmp_path = std::env::temp_dir().join("open_cad_studio_print.pdf");
    pdf_export::export_pdf(
        &wires,
        &hatches,
        &wipeouts,
        paper_w,
        paper_h,
        offset_x,
        offset_y,
        rotation_deg,
        scale,
        clip,
        &tmp_path,
        plot_style.as_ref(),
    )?;

    // ── 2. Dispatch to system printer ─────────────────────────────────────
    dispatch_to_printer(&tmp_path)
}

/// Enumerate installed printers. Linux/macOS query CUPS via `lpstat -e`;
/// Windows returns an empty list (the "printto" dispatch targets a named
/// printer directly and the system default is always available).
#[cfg(not(target_arch = "wasm32"))]
pub fn list_printers() -> Vec<String> {
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("lpstat")
            .arg("-e")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(target_os = "windows")]
    {
        Vec::new()
    }
}

/// Like [`print_wires`] but honours a [`PrintOptions`] bundle (printer, copies,
/// grayscale, quality, DPI).
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub async fn print_wires_with(
    wires: std::sync::Arc<Vec<WireModel>>,
    hatches: Vec<HatchModel>,
    wipeouts: Vec<HatchModel>,
    paper_w: f64,
    paper_h: f64,
    offset_x: f64,
    offset_y: f64,
    rotation_deg: i32,
    scale: f32,
    clip: Option<(f32, f32, f32, f32)>,
    plot_style: Option<PlotStyleTable>,
    opts: PrintOptions,
) -> Result<String, String> {
    let tmp_path = std::env::temp_dir().join("open_cad_studio_print.pdf");
    pdf_export::export_pdf(
        &wires,
        &hatches,
        &wipeouts,
        paper_w,
        paper_h,
        offset_x,
        offset_y,
        rotation_deg,
        scale,
        clip,
        &tmp_path,
        plot_style.as_ref(),
    )?;
    dispatch_to_printer_opts(&tmp_path, &opts)
}

/// Send an already-rendered PDF to a printer with [`PrintOptions`]. Used for
/// clipped window plots, whose PDF is built with a scale + clip the plain
/// `print_wires_with` path doesn't expose.
#[cfg(not(target_arch = "wasm32"))]
pub fn print_existing_pdf(path: &std::path::Path, opts: &PrintOptions) -> Result<String, String> {
    dispatch_to_printer_opts(path, opts)
}

/// Open a file with the OS default application (used for print preview).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_in_viewer(path: &std::path::Path) -> Result<(), String> {
    let p = path.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", &p]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(&p);
        c
    };
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&p);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open preview: {e}"))
}

/// Dispatch a PDF to a specific printer with [`PrintOptions`].
#[cfg(not(target_arch = "wasm32"))]
fn dispatch_to_printer_opts(
    path: &std::path::Path,
    opts: &PrintOptions,
) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // Target a named printer via the "printto" verb; fall back to the
        // default-printer "print" verb. Copies / quality / colour aren't
        // expressible through a shell verb, so they are ignored here.
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let wide = |s: &str| -> Vec<u16> { OsStr::new(s).encode_wide().chain(Some(0)).collect() };
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let (verb, params, label) = match opts.printer.as_deref() {
            Some(p) if !p.is_empty() => (wide("printto"), Some(wide(p)), p.to_string()),
            _ => (wide("print"), None, "default printer".to_string()),
        };
        let params_ptr = params.as_ref().map(|v| v.as_ptr()).unwrap_or(std::ptr::null());
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                path_wide.as_ptr(),
                params_ptr,
                std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
            ) as usize
        };
        if result > 32 {
            Ok(label)
        } else {
            Err(format!("ShellExecute failed (code {result})"))
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let path_str = path.to_string_lossy();
        let mut cmd = std::process::Command::new("lp");
        if let Some(p) = opts.printer.as_deref() {
            if !p.is_empty() {
                cmd.arg("-d").arg(p);
            }
        }
        let copies = opts.copies.max(1);
        if copies > 1 {
            cmd.arg("-n").arg(copies.to_string());
        }
        if opts.mono {
            cmd.arg("-o").arg("ColorModel=Gray");
        }
        if let Some(dpi) = opts.dpi {
            cmd.arg("-o").arg(format!("Resolution={dpi}dpi"));
        }
        if let Some(q) = opts.quality.as_deref() {
            // CUPS print-quality: 3 = draft, 4 = normal, 5 = high / best.
            let pq = match q {
                "Draft" => "3",
                "High" | "Maximum" => "5",
                _ => "4",
            };
            cmd.arg("-o").arg(format!("print-quality={pq}"));
        }
        let out = cmd
            .arg("--")
            .arg(path_str.as_ref())
            .output()
            .map_err(|e| format!("Could not launch lp: {e}"))?;
        if out.status.success() {
            let msg = String::from_utf8_lossy(&out.stdout);
            let printer = msg
                .split_whitespace()
                .find(|w| w.contains('-'))
                .unwrap_or("printer")
                .to_string();
            Ok(printer)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }
}

/// Platform-specific dispatch of a PDF path to the system printer.
#[cfg(not(target_arch = "wasm32"))]
fn dispatch_to_printer(path: &std::path::Path) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // Windows: ShellExecute with "print" verb.
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let verb: Vec<u16> = OsStr::new("print\0").encode_wide().collect();
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                path_wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
            ) as usize
        };
        if result > 32 {
            Ok("default printer".to_string())
        } else {
            Err(format!("ShellExecute PRINT failed (code {result})"))
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Linux / macOS: prefer `lp`, fall back to `lpr`.
        let path_str = path.to_string_lossy();

        // Try `lp` first (CUPS).
        let lp = std::process::Command::new("lp")
            .arg("--")
            .arg(path_str.as_ref())
            .output();

        match lp {
            Ok(out) if out.status.success() => {
                // `lp` prints the job ID on stdout, e.g. "request id is lp-42 (1 file(s))"
                let msg = String::from_utf8_lossy(&out.stdout);
                let printer = msg
                    .split_whitespace()
                    .find(|w| w.contains('-'))
                    .unwrap_or("default")
                    .to_string();
                return Ok(printer);
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).into_owned();
                // Fall through to lpr.
                if !err.is_empty() {
                    // Try lpr as alternative.
                }
            }
            Err(_) => {
                // lp not found — try lpr.
            }
        }

        // Fall back to `lpr`.
        let lpr = std::process::Command::new("lpr")
            .arg(path_str.as_ref())
            .output()
            .map_err(|e| format!("Could not launch lp or lpr: {e}"))?;

        if lpr.status.success() {
            Ok("default printer".to_string())
        } else {
            let err = String::from_utf8_lossy(&lpr.stderr).into_owned();
            Err(format!("lpr error: {err}"))
        }
    }
}
