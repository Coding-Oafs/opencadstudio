//! Editable keyboard shortcut table shared by the CUI dialog and key events.

use super::{Message, OpenCADStudio};
use iced::Task;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;

#[cfg(target_os = "macos")]
const ACCEL: &str = "CMD";
#[cfg(not(target_os = "macos"))]
const ACCEL: &str = "CTRL";

/// Every active binding shipped with a fresh configuration. Values are command
/// names where possible; the small set of input actions is resolved below.
pub(super) fn default_bindings() -> BTreeMap<String, String> {
    [
        ("F1".to_string(), "HELP"),
        ("F2".to_string(), "COMMANDHISTORY"),
        ("F3".to_string(), "TOGGLEOSNAP"),
        ("F7".to_string(), "GRID"),
        ("F8".to_string(), "ORTHO"),
        ("F9".to_string(), "SNAP"),
        ("F10".to_string(), "POLAR"),
        ("F11".to_string(), "OTRACK"),
        ("F12".to_string(), "DYNINPUT"),
        (format!("{ACCEL}+0"), "CLEANSCREEN"),
        (format!("{ACCEL}+1"), "PROPERTIES"),
        (format!("{ACCEL}+N"), "NEW"),
        (format!("{ACCEL}+O"), "OPEN"),
        (format!("{ACCEL}+P"), "PLOT"),
        (format!("{ACCEL}+Q"), "QUIT"),
        (format!("{ACCEL}+S"), "SAVE"),
        (format!("{ACCEL}+SHIFT+S"), "SAVEAS"),
        (format!("{ACCEL}+Z"), "UNDO"),
        (format!("{ACCEL}+SHIFT+Z"), "REDO"),
        (format!("{ACCEL}+Y"), "REDO"),
        (format!("{ACCEL}+F"), "FIND"),
        (format!("{ACCEL}+H"), "FIND"),
        (format!("{ACCEL}+A"), "SELECTALL"),
        (format!("{ACCEL}+C"), "COPYCLIP"),
        (format!("{ACCEL}+SHIFT+C"), "COPYBASE"),
        (format!("{ACCEL}+X"), "CUTCLIP"),
        (format!("{ACCEL}+V"), "PASTECLIP"),
        (format!("{ACCEL}+SHIFT+V"), "PASTEBLOCK"),
        ("ENTER".to_string(), "FINALIZE"),
        ("SPACE".to_string(), "COMMANDSPACE"),
        ("ESCAPE".to_string(), "CANCEL"),
        ("DELETE".to_string(), "DELETESELECTED"),
        ("BACKSPACE".to_string(), "BACKSPACE"),
        ("TAB".to_string(), "DYNTAB"),
        ("UP".to_string(), "HISTORYPREV"),
        ("DOWN".to_string(), "HISTORYNEXT"),
        ("LEFT".to_string(), "CARETLEFT"),
        ("RIGHT".to_string(), "CARETRIGHT"),
    ]
    .into_iter()
    .map(|(key, action)| (key, action.to_string()))
    .collect()
}

/// Canonical form used by the editor, command interface, config and key event
/// resolver. Modifier order is stable so equivalent user input shares one row.
pub(super) fn normalize_key(value: &str) -> String {
    let mut ctrl = false;
    let mut cmd = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = String::new();

    for part in value.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_uppercase().as_str() {
            "CTRL" | "CONTROL" => ctrl = true,
            "CMD" | "COMMAND" | "META" | "SUPER" => cmd = true,
            "ALT" | "OPTION" => alt = true,
            "SHIFT" => shift = true,
            "ESC" => key = "ESCAPE".to_string(),
            "RETURN" => key = "ENTER".to_string(),
            "ARROWUP" => key = "UP".to_string(),
            "ARROWDOWN" => key = "DOWN".to_string(),
            "ARROWLEFT" => key = "LEFT".to_string(),
            "ARROWRIGHT" => key = "RIGHT".to_string(),
            other => key = other.to_string(),
        }
    }

    let mut parts = Vec::with_capacity(5);
    if ctrl {
        parts.push("CTRL");
    }
    if cmd {
        parts.push("CMD");
    }
    if alt {
        parts.push("ALT");
    }
    if shift {
        parts.push("SHIFT");
    }
    if !key.is_empty() {
        parts.push(&key);
    }
    parts.join("+")
}

/// Keys that used to bypass focused widgets. Clipboard and selection commands
/// intentionally stay out so text fields retain their native shortcuts.
pub(super) fn is_global_key(key: &str) -> bool {
    let base = key.rsplit('+').next().unwrap_or(key);
    base.strip_prefix('F').is_some_and(|digits| {
        !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
    })
        || matches!(base, "ESCAPE")
        || (key.starts_with(ACCEL)
            && !matches!(
                key.rsplit('+').next(),
                Some("A" | "C" | "V" | "X")
            ))
}

fn is_named_key(key: &str) -> bool {
    matches!(
        key,
        "ENTER"
            | "SPACE"
            | "ESCAPE"
            | "DELETE"
            | "BACKSPACE"
            | "TAB"
            | "UP"
            | "DOWN"
            | "LEFT"
            | "RIGHT"
            | "HOME"
            | "END"
            | "PAGEUP"
            | "PAGEDOWN"
            | "INSERT"
    ) || key.strip_prefix('F').is_some_and(|digits| {
        !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
    })
}

impl OpenCADStudio {
    pub(super) fn apply_shortcut_editor_rows(&mut self) {
        let bindings: FxHashMap<String, String> = self
            .shortcut_editor_rows
            .iter()
            .filter_map(|(key, action)| {
                let key = normalize_key(key);
                let action = action.trim().to_uppercase();
                (!key.is_empty() && !action.is_empty()).then_some((key, action))
            })
            .collect();
        self.shortcut_bindings = bindings;
        self.persist_settings_if_changed();
    }

    pub(super) fn run_shortcut(&mut self, key: &str) -> Task<Message> {
        let action = self.shortcut_bindings.get(key).or_else(|| {
            let base = key.rsplit('+').next()?;
            is_named_key(base).then(|| self.shortcut_bindings.get(base)).flatten()
        });
        let Some(action) = action.cloned() else {
            return Task::none();
        };
        let message = match action.as_str() {
            "FINALIZE" => Message::CommandFinalize,
            "COMMANDSPACE" => Message::CommandSpace,
            "CANCEL" => Message::CommandEscape,
            "DELETESELECTED" => Message::DeleteSelected,
            "BACKSPACE" => Message::CommandBackspace,
            "DYNTAB" => Message::DynTabNext,
            "HISTORYPREV" => Message::CommandHistoryPrev,
            "HISTORYNEXT" => Message::CommandHistoryNext,
            "CARETLEFT" => Message::MTextCaretMove(-1),
            "CARETRIGHT" => Message::MTextCaretMove(1),
            "COMMANDHISTORY" => Message::CommandHistoryToggle,
            "TOGGLEOSNAP" => Message::ToggleSnapEnabled,
            "OTRACK" => Message::ToggleOTrack,
            "DYNINPUT" => Message::ToggleDynInput,
            "SELECTALL" => Message::SelectAllShortcut,
            "PASTECLIP" => Message::PasteShortcut,
            other => Message::Command(other.to_string()),
        };
        self.update(message)
    }
}
