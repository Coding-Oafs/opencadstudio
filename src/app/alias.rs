//! Command aliases — short abbreviations typed at the command line that expand
//! to a full command (e.g. `L` → `LINE`). The alias table is the single source
//! of truth: it is loaded from a user-editable `aliases.pgp` file at startup and
//! consulted before every command dispatch (see `resolve_alias`), so users can
//! add, remap, or delete aliases without touching the app.
//!
//! File format mirrors the long-standing plain-text `.pgp` convention so it is
//! familiar and hand-editable, stored as `ocad.pgp`:
//!
//! ```text
//! ; lines beginning with a semicolon are comments
//! L,        *LINE
//! CC,       *COPYCLIP
//! ```
//!
//! An alias is the token left of the comma; the command is the token right of
//! it, with an optional leading `*`. Both are matched case-insensitively and
//! stored uppercased. Native builds use `ocad.pgp`; web builds store the same
//! PGP text in `localStorage`.

use super::OpenCADStudio;
use rustc_hash::FxHashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// The shipped default alias file, embedded at compile time. Its aliases live in
/// `assets/ocad.pgp` — not in Rust — so the defaults are just data in the same
/// format the user edits. On first launch this text is written verbatim to the
/// user's config folder (comments and all) as the starting `ocad.pgp`; only that
/// user copy is read thereafter. This embedded copy is the first-run seed /
/// factory default.
const DEFAULT_ALIASES_PGP: &str = include_str!("../../assets/ocad.pgp");

/// Path to the user's alias file, `<config>/ocad.pgp`. `None` when the platform
/// config base can't be resolved (headless, no `HOME`).
#[cfg(not(target_arch = "wasm32"))]
fn alias_file_path() -> Option<PathBuf> {
    Some(crate::config::config_dir()?.join("ocad.pgp"))
}

#[cfg(target_arch = "wasm32")]
const WEB_ALIAS_KEY: &str = "opencadstudio.aliases";

/// Parse `.pgp` text into an `alias → command` map, both uppercased. Skips blank
/// lines and `;` comments; tolerates a leading `*` on the command and arbitrary
/// whitespace padding.
fn parse_pgp(body: &str) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((alias, cmd)) = line.split_once(',') else {
            continue;
        };
        let alias = alias.trim().to_uppercase();
        let cmd = cmd.trim().trim_start_matches('*').trim().to_uppercase();
        if alias.is_empty() || cmd.is_empty() {
            continue;
        }
        map.insert(alias, cmd);
    }
    map
}

/// Serialize an `alias → command` map to `.pgp` text with a header and aligned
/// columns, sorted by alias for a stable, diffable file.
pub(super) fn to_pgp(map: &FxHashMap<String, String>) -> String {
    let mut rows: Vec<(&String, &String)> = map.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::from(
        "; OpenCADStudio command aliases.\n\
         ; Format:  ALIAS,*COMMAND   (lines starting with ';' are comments)\n\
         ; Edit here or via the ALIASEDIT command. One alias per line.\n\n",
    );
    for (alias, cmd) in rows {
        out.push_str(&format!("{:<10}*{}\n", format!("{alias},"), cmd));
    }
    out
}

/// The built-in defaults, parsed from the embedded `.pgp`. Used when no user
/// file exists yet (or can't be reached, e.g. wasm).
fn default_map() -> FxHashMap<String, String> {
    parse_pgp(DEFAULT_ALIASES_PGP)
}

/// Load the alias table at boot. Native reads `ocad.pgp`; web reads the same
/// text from `localStorage`. Missing or unavailable storage falls back to the
/// embedded defaults.
pub(super) fn load_aliases() -> FxHashMap<String, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match alias_file_path() {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(body) => parse_pgp(&body),
                Err(_) => {
                    // No file yet — copy the shipped default file, best-effort.
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&path, DEFAULT_ALIASES_PGP);
                    default_map()
                }
            },
            None => default_map(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item(WEB_ALIAS_KEY).ok().flatten())
            .map(|body| parse_pgp(&body))
            .unwrap_or_else(default_map)
    }
}

/// Persist the alias table to native `ocad.pgp` or web `localStorage`.
pub(super) fn save_map(map: &FxHashMap<String, String>) -> std::io::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = alias_file_path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        return std::fs::write(path, to_pgp(map));
    }

    #[cfg(target_arch = "wasm32")]
    {
        let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        else {
            return Ok(());
        };
        storage
            .set_item(WEB_ALIAS_KEY, &to_pgp(map))
            .map_err(|_| std::io::Error::other("browser alias storage unavailable"))?;
        return Ok(());
    }
}

impl OpenCADStudio {
    /// Rewrite the leading command token through the alias table, leaving any
    /// arguments after the first whitespace untouched. Returns `None` when the
    /// verb is not an alias, so the caller passes the original string through
    /// unchanged. Keyed uppercase so lowercase aliases from scripts/plugins
    /// (which bypass the command line's verb-uppercasing) still resolve.
    pub(super) fn resolve_alias(&self, cmd: &str) -> Option<String> {
        if self.command_aliases.is_empty() {
            return None;
        }
        let (verb, rest) = match cmd.split_once(char::is_whitespace) {
            Some((v, r)) => (v, Some(r)),
            None => (cmd, None),
        };
        let canon = self.command_aliases.get(&verb.to_uppercase())?;
        Some(match rest {
            Some(r) => format!("{canon} {r}"),
            None => canon.clone(),
        })
    }

    /// Replace the alias table (from the editor), persist it, and mirror it into
    /// the command line so autocomplete reflects the edits immediately.
    pub(super) fn set_command_aliases(&mut self, map: FxHashMap<String, String>) {
        let _ = save_map(&map);
        self.command_line.command_aliases = map.clone();
        self.command_aliases = map;
    }

    /// Commit the alias-editor working rows to the table (Apply button): build a
    /// map from the rows, dropping incomplete ones, and persist + sync. Leaves
    /// the editor open so the user can keep editing.
    pub(super) fn apply_alias_editor_rows(&mut self) {
        let map: FxHashMap<String, String> = self
            .alias_editor_rows
            .iter()
            .filter(|(a, c)| !a.trim().is_empty() && !c.trim().is_empty())
            .map(|(a, c)| (a.trim().to_string(), c.trim().to_string()))
            .collect();
        self.set_command_aliases(map);
    }
}
