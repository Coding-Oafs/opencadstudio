//! Plot Style Table — CTB (color-based) and STB (named) file support.
//!
//! CTB files map indexed drawing colors (ACI, 1-255) to pen properties:
//! RGB color override, lineweight, and screeing percentage.
//!
//! File format: deflate-compressed text (key = value pairs) with
//! 255 `begin_plot_style … end_plot_style` blocks.
//!
//! STB files follow the same format but use named styles instead of
//! ACI indices; they are read into a `Vec<NamedPlotStyle>`.

use rustc_hash::FxHashMap as HashMap;
use std::io::Read;
use std::path::Path;

// ── Standard lineweight table (index → mm) ───────────────────────────────────

/// Lineweight table: index value → mm, matching the stored LWEIGHT codes.
/// Index 0 = 0.00 mm (hairline), others follow the DXF lineweight enum.
pub const LW_TABLE: &[f32] = &[
    0.00, 0.05, 0.09, 0.10, 0.13, 0.15, 0.18, 0.20, 0.25, 0.30, 0.35, 0.40, 0.50, 0.53, 0.60, 0.70,
    0.80, 0.90, 1.00, 1.06, 1.20, 1.40, 1.58, 2.00, 2.11,
];

// ── Per-color entry ───────────────────────────────────────────────────────────

/// A single entry in a CTB or STB plot style table.
#[derive(Debug, Clone)]
pub struct PlotStyleEntry {
    /// Optional display name (empty for CTB; the style name for STB).
    pub description: String,
    /// If `Some([r,g,b])`, override the entity color with this RGB value (0..255).
    /// If `None`, use the object color.
    pub color: Option<[u8; 3]>,
    /// Lineweight index into `LW_TABLE`.  255 = use object lineweight.
    pub lineweight: u8,
    /// Screen percentage 0–100 (100 = opaque).
    pub screening: u8,
}

impl Default for PlotStyleEntry {
    fn default() -> Self {
        PlotStyleEntry {
            description: String::new(),
            color: None,
            lineweight: 255, // use object lineweight
            screening: 100,
        }
    }
}

// ── Plot Style Table ──────────────────────────────────────────────────────────

/// A loaded CTB or STB plot style table.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PlotStyleTable {
    /// File name (without path), e.g. "monochrome.ctb".
    pub name: String,
    /// Whether this is a named-style (STB) table rather than color-based (CTB).
    pub is_stb: bool,
    /// For CTB: entries indexed by ACI (index 0 unused; 1..=255 are valid).
    pub aci_entries: Vec<PlotStyleEntry>, // 256 entries, index = ACI
    /// For STB: named style entries.
    pub named_entries: HashMap<String, PlotStyleEntry>,
}

impl PlotStyleTable {
    /// Create an identity CTB table (no overrides for any color).
    pub fn identity(name: impl Into<String>) -> Self {
        PlotStyleTable {
            name: name.into(),
            is_stb: false,
            aci_entries: (0..=255).map(|_| PlotStyleEntry::default()).collect(),
            named_entries: HashMap::default(),
        }
    }

    /// Load a CTB or STB file from disk.
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read(path).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let is_stb = name.to_lowercase().ends_with(".stb");
        let text = decompress_ctb(&raw)?;
        parse_plot_style_text(&text, name, is_stb)
    }

    /// Write this table to disk as a CTB/STB file.
    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = self.to_text();
        let compressed = compress_ctb(text.as_bytes())?;
        std::fs::write(path, compressed).map_err(|e| e.to_string())
    }

    /// Resolve the effective print RGB color for the given ACI index.
    /// Returns None if no override (use object color).
    pub fn resolve_color(&self, aci: u8) -> Option<[f32; 3]> {
        let entry = self.aci_entries.get(aci as usize)?;
        entry
            .color
            .map(|[r, g, b]| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0])
    }

    /// Resolve the effective lineweight in mm for the given ACI index.
    /// Returns None if no override (use object lineweight).
    pub fn resolve_lineweight(&self, aci: u8) -> Option<f32> {
        let entry = self.aci_entries.get(aci as usize)?;
        if entry.lineweight == 255 {
            None
        } else {
            LW_TABLE.get(entry.lineweight as usize).copied()
        }
    }

    /// Resolve the screening factor for the given ACI index.
    pub fn resolve_screening(&self, aci: u8) -> f32 {
        self.aci_entries
            .get(aci as usize)
            .map(|entry| entry.screening.min(100) as f32 / 100.0)
            .unwrap_or(1.0)
    }

    // ── Internal serialisation ────────────────────────────────────────────

    fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("description=\n");
        s.push_str("apply_factor=0\n");
        s.push_str("unit_type=1\n");
        s.push_str("custom_lineweight_display_units=0\n");
        for (_idx, entry) in self.aci_entries.iter().enumerate().skip(1) {
            s.push_str("begin_plot_style\n");
            s.push_str(&format!(" description={}\n", entry.description));
            s.push_str(" physical_pen_number=0\n");
            s.push_str(" virtual_pen_number=0\n");
            s.push_str(&format!(" screen={}\n", entry.screening));
            s.push_str(" linepattern_size=0.5\n");
            s.push_str(" linetype=31\n");
            s.push_str(" adaptive_linetype=TRUE\n");
            s.push_str(&format!(" lineweight={}\n", entry.lineweight));
            s.push_str(" fill_style=64\n");
            s.push_str(" end_style=0\n");
            s.push_str(" join_style=0\n");
            if let Some([r, g, b]) = entry.color {
                s.push_str(&format!(" color1=#{:02X}{:02X}{:02X}\n", r, g, b));
            } else {
                // 0xC2000000 = "use object color"
                s.push_str(" color1=-1056964608\n");
            }
            s.push_str("end_plot_style\n");
        }
        s
    }
}

// ── Deflate helpers ───────────────────────────────────────────────────────────

/// Decompress a CTB/STB file's raw bytes into the text content.
///
/// CTB files start with a plain-text header (first line: "PIAFILEVERSION_2.0")
/// followed by raw-deflate compressed content.  Some tools write pure zlib
/// (with the two-byte zlib header 0x78 0x9C) instead — we handle both.
fn decompress_ctb(data: &[u8]) -> Result<String, String> {
    // Find the first newline — everything after it is the compressed payload.
    let split_at = data
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let payload = &data[split_at..];

    // Try zlib (0x78 prefix) first, then raw deflate.
    let mut text = String::new();
    if payload.starts_with(&[0x78]) {
        use flate2::read::ZlibDecoder;
        ZlibDecoder::new(payload)
            .read_to_string(&mut text)
            .map_err(|e| format!("zlib decompress: {e}"))?;
    } else {
        use flate2::read::DeflateDecoder;
        DeflateDecoder::new(payload)
            .read_to_string(&mut text)
            .map_err(|e| format!("deflate decompress: {e}"))?;
    }
    Ok(text)
}

/// Compress plot-style text content as a CTB/STB file.
fn compress_ctb(text: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;
    let header = b"PIAFILEVERSION_2.0\r\n";
    let mut compressed: Vec<u8> = Vec::new();
    {
        let mut enc = ZlibEncoder::new(&mut compressed, Compression::default());
        enc.write_all(text).map_err(|e| e.to_string())?;
    }
    let mut out = header.to_vec();
    out.extend_from_slice(&compressed);
    Ok(out)
}

// ── Text parser ───────────────────────────────────────────────────────────────

fn parse_plot_style_text(text: &str, name: String, is_stb: bool) -> Result<PlotStyleTable, String> {
    let mut aci_entries: Vec<PlotStyleEntry> =
        (0..=255).map(|_| PlotStyleEntry::default()).collect();
    let mut named_entries: HashMap<String, PlotStyleEntry> = HashMap::default();
    let mut style_index: usize = 1; // CTB: 1-based ACI index
    let mut current: Option<PlotStyleEntry> = None;
    let mut current_name: String = String::new();

    for line in text.lines() {
        let line = line.trim();
        if line == "begin_plot_style" {
            current = Some(PlotStyleEntry::default());
            current_name = format!("Color_{}", style_index);
            continue;
        }
        if line == "end_plot_style" {
            if let Some(entry) = current.take() {
                if is_stb {
                    named_entries.insert(current_name.clone(), entry);
                } else if style_index <= 255 {
                    aci_entries[style_index] = entry;
                    style_index += 1;
                }
            }
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "description" => {
                    if !val.is_empty() {
                        entry.description = val.to_string();
                        current_name = val.to_string();
                    }
                }
                "screen" => {
                    if let Ok(v) = val.parse::<u8>() {
                        entry.screening = v;
                    }
                }
                "lineweight" => {
                    if let Ok(v) = val.parse::<u8>() {
                        entry.lineweight = v;
                    }
                }
                "color1" => {
                    if val.starts_with('#') && val.len() == 7 {
                        // #RRGGBB
                        let r = u8::from_str_radix(&val[1..3], 16).unwrap_or(0);
                        let g = u8::from_str_radix(&val[3..5], 16).unwrap_or(0);
                        let b = u8::from_str_radix(&val[5..7], 16).unwrap_or(0);
                        entry.color = Some([r, g, b]);
                    } else if let Ok(packed) = val.parse::<i32>() {
                        // The file packs RGB as a 0xC0RRGGBB negative integer.
                        // Value 0xC2000000 (-1056964608) = use object color.
                        if packed != -1056964608i32 {
                            let u = packed as u32;
                            let r = ((u >> 16) & 0xFF) as u8;
                            let g = ((u >> 8) & 0xFF) as u8;
                            let b = (u & 0xFF) as u8;
                            entry.color = Some([r, g, b]);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(PlotStyleTable {
        name,
        is_stb,
        aci_entries,
        named_entries,
    })
}
