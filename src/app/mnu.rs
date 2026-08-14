//! Bentley-compatible function-key `.mnu` interchange.

use std::collections::BTreeMap;

pub(crate) struct MnuImport {
    pub bindings: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

pub(crate) fn parse(text: &str) -> Result<MnuImport, String> {
    let mut bindings = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut saw_header = false;
    for (line_index, original) in text.lines().enumerate() {
        let line = original.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.eq_ignore_ascii_case("$FK5.0$") {
            saw_header = true;
            continue;
        }
        let (code, action) = line
            .split_once(',')
            .ok_or_else(|| format!("MNU line {} has no key/action comma", line_index + 1))?;
        let key = decode_key(code.trim()).ok_or_else(|| {
            format!(
                "MNU line {} has unsupported key code {code}",
                line_index + 1
            )
        })?;
        let action = action.trim();
        if action.is_empty() {
            continue;
        }
        let (translated, warning) = translate_action(action);
        if let Some(warning) = warning {
            warnings.push(format!("{key}: {warning}"));
        }
        bindings.insert(key, translated);
    }
    if !saw_header {
        return Err("MNU header $FK5.0$ was not found".into());
    }
    Ok(MnuImport { bindings, warnings })
}

pub(crate) fn write(bindings: impl IntoIterator<Item = (String, String)>) -> String {
    let mut rows: Vec<_> = bindings
        .into_iter()
        .filter_map(|(key, action)| encode_key(&key).map(|code| (code, action)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut output = String::from("$FK5.0$\r\n; Open CAD Studio function-key menu\r\n");
    for (code, action) in rows {
        output.push_str(&format!("{code},{action}\r\n"));
    }
    output
}

fn translate_action(action: &str) -> (String, Option<String>) {
    let normalized = action.trim().to_ascii_uppercase();
    let translated = if let Some(class) = normalized.strip_prefix("CLASSIFY USING BRUSH ") {
        format!("POINTCLOUDBRUSHCLASSIFY {class}")
    } else if let Some(class) = normalized.strip_prefix("CLASSIFY ABOVE LINE ") {
        format!("POINTCLOUDABOVELINE {class}")
    } else if let Some(class) = normalized.strip_prefix("CLASSIFY BELOW LINE ") {
        format!("POINTCLOUDBELOWLINE {class}")
    } else if normalized == "ASSIGN POINT CLASS" {
        "POINTCLOUDCLASSIFYSELECTION".into()
    } else if normalized == "CREATE EDITABLE MODEL" {
        "POINTCLOUDINDEX".into()
    } else {
        normalized.clone()
    };
    let external = normalized.starts_with("VBA RUN ")
        || normalized.starts_with("MDL LOAD ")
        || normalized.starts_with("SCAN DISPLAY ");
    let profile_line = normalized.starts_with("CLASSIFY ABOVE LINE ")
        || normalized.starts_with("CLASSIFY BELOW LINE ");
    let warning = external
        .then(|| {
            "preserved external TerraScan/VBA/MDL key-in; Open CAD Studio cannot execute it yet"
                .to_string()
        })
        .or_else(|| {
            profile_line.then(|| {
                "translated profile-view line classifier; this build reports it but does not execute screen-line classification yet"
                    .to_string()
            })
        });
    (translated, warning)
}

fn decode_key(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches("0x").to_ascii_lowercase();
    for function in (1_u8..=12).rev() {
        let suffix = format!("{function:x}");
        let Some(modifier) = value.strip_suffix(&suffix) else {
            continue;
        };
        let modifier = u8::from_str_radix(modifier, 16).ok()?;
        let prefix = modifier_prefix(modifier)?;
        return Some(format!("{prefix}F{function}"));
    }
    None
}

fn encode_key(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_uppercase();
    let function = normalized
        .rsplit('+')
        .next()?
        .strip_prefix('F')?
        .parse::<u8>()
        .ok()?;
    if !(1..=12).contains(&function) {
        return None;
    }
    let ctrl = normalized.split('+').any(|part| part == "CTRL");
    let alt = normalized.split('+').any(|part| part == "ALT");
    let shift = normalized.split('+').any(|part| part == "SHIFT");
    let modifier = match (ctrl, alt, shift) {
        (false, false, false) => 0x03,
        (true, false, false) => 0x0b,
        (false, true, false) => 0x07,
        (false, false, true) => 0x13,
        (true, true, false) => 0x0f,
        (true, false, true) => 0x1b,
        (false, true, true) => 0x17,
        (true, true, true) => 0x1f,
    };
    Some(format!("{modifier:x}{function:x}"))
}

fn modifier_prefix(value: u8) -> Option<&'static str> {
    match value {
        0x03 => Some(""),
        0x0b => Some("CTRL+"),
        0x07 => Some("ALT+"),
        0x13 => Some("SHIFT+"),
        0x0f => Some("CTRL+ALT+"),
        0x1b => Some("CTRL+SHIFT+"),
        0x17 => Some("ALT+SHIFT+"),
        0x1f => Some("CTRL+ALT+SHIFT+"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_and_exports_function_key_combinations() {
        let imported = parse(
            "$FK5.0$\n; survey keys\n31,draw vertical section\nb5,classify using brush 15\n1fc,VBA run StepThru.ChooseFolder\n",
        )
        .unwrap();
        assert_eq!("DRAW VERTICAL SECTION", imported.bindings["F1"]);
        assert_eq!("POINTCLOUDBRUSHCLASSIFY 15", imported.bindings["CTRL+F5"]);
        assert_eq!(1, imported.warnings.len());
        let output = write(imported.bindings);
        assert!(output.contains("31,DRAW VERTICAL SECTION"));
        assert!(output.contains("b5,POINTCLOUDBRUSHCLASSIFY 15"));
    }
}
