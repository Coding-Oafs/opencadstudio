//! TerraScan-style point class table interchange.
//!
//! TerraScan installations use more than one `.ptc` dialect. This reader
//! accepts the common delimited text forms and preserves code, description and
//! RGB colour. Unknown columns are ignored so project-specific level/draw
//! fields do not prevent import.

use crate::{ClassDefinition, ClassTable};
use std::{error, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtcError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for PtcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PTC line {}: {}", self.line, self.message)
    }
}

impl error::Error for PtcError {}

pub fn parse_ptc(text: &str) -> Result<ClassTable, PtcError> {
    let mut table = ClassTable {
        classes: Default::default(),
    };
    let mut columns = None;
    for (line_index, original) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = original.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with("//")
        {
            continue;
        }
        let fields = split_fields(line);
        if fields.is_empty() {
            continue;
        }
        if fields[0].eq_ignore_ascii_case("code") {
            columns = Some(PtcColumns::from_header(&fields));
            continue;
        }
        let code = fields[0].parse::<u8>().map_err(|_| PtcError {
            line: line_number,
            message: format!("invalid class code {:?}", fields[0]),
        })?;
        if fields.len() < 2 {
            return Err(PtcError {
                line: line_number,
                message: "missing class description".into(),
            });
        }

        let (name, color, visible, locked) = if let Some(columns) = columns {
            columns.parse(&fields).ok_or_else(|| PtcError {
                line: line_number,
                message: "record does not match the PTC column header".into(),
            })?
        } else {
            let (name, color) = parse_name_and_color(&fields).ok_or_else(|| PtcError {
                line: line_number,
                message: "expected RGB values as R,G,B or #RRGGBB".into(),
            })?;
            (name, color, true, false)
        };
        table.upsert(ClassDefinition {
            code,
            name,
            color,
            visible,
            locked,
        });
    }
    if table.classes.is_empty() {
        return Err(PtcError {
            line: 0,
            message: "no class records found".into(),
        });
    }
    Ok(table)
}

#[derive(Clone, Copy)]
struct PtcColumns {
    name: usize,
    red: usize,
    green: usize,
    blue: usize,
    visible: Option<usize>,
    locked: Option<usize>,
}

impl PtcColumns {
    fn from_header(fields: &[String]) -> Self {
        let find = |names: &[&str]| {
            fields
                .iter()
                .position(|field| names.iter().any(|name| field.eq_ignore_ascii_case(name)))
        };
        Self {
            name: find(&["description", "name"]).unwrap_or(1),
            red: find(&["red", "r"]).unwrap_or(2),
            green: find(&["green", "g"]).unwrap_or(3),
            blue: find(&["blue", "b"]).unwrap_or(4),
            visible: find(&["visible", "draw"]),
            locked: find(&["locked", "lock"]),
        }
    }

    fn parse(self, fields: &[String]) -> Option<(String, [u8; 3], bool, bool)> {
        let flag = |index: Option<usize>, fallback| {
            index
                .and_then(|index| fields.get(index))
                .map(|value| {
                    !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "no" | "off"
                    )
                })
                .unwrap_or(fallback)
        };
        Some((
            fields.get(self.name)?.clone(),
            [
                fields.get(self.red)?.parse().ok()?,
                fields.get(self.green)?.parse().ok()?,
                fields.get(self.blue)?.parse().ok()?,
            ],
            flag(self.visible, true),
            flag(self.locked, false),
        ))
    }
}

pub fn write_ptc(table: &ClassTable) -> String {
    let mut output = String::from("Code,Description,Red,Green,Blue,Visible,Locked\n");
    for class in table.classes.values() {
        let escaped = class.name.replace('"', "\"\"");
        output.push_str(&format!(
            "{},\"{}\",{},{},{},{},{}\n",
            class.code,
            escaped,
            class.color[0],
            class.color[1],
            class.color[2],
            u8::from(class.visible),
            u8::from(class.locked),
        ));
    }
    output
}

fn split_fields(line: &str) -> Vec<String> {
    let delimiter = if line.contains(',') {
        ','
    } else if line.contains(';') {
        ';'
    } else if line.contains('\t') {
        '\t'
    } else {
        return line.split_whitespace().map(str::to_owned).collect();
    };
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            value if value == delimiter && !quoted => {
                fields.push(current.trim().to_owned());
                current.clear();
            }
            value => current.push(value),
        }
    }
    fields.push(current.trim().to_owned());
    fields
}

fn parse_name_and_color(fields: &[String]) -> Option<(String, [u8; 3])> {
    if let Some(hex) = fields
        .iter()
        .find(|field| field.len() == 7 && field.starts_with('#'))
    {
        let color = [
            u8::from_str_radix(&hex[1..3], 16).ok()?,
            u8::from_str_radix(&hex[3..5], 16).ok()?,
            u8::from_str_radix(&hex[5..7], 16).ok()?,
        ];
        return Some((fields[1].clone(), color));
    }

    let rgb_start = (2..fields.len().saturating_sub(2)).rev().find(|&index| {
        fields[index].parse::<u8>().is_ok()
            && fields[index + 1].parse::<u8>().is_ok()
            && fields[index + 2].parse::<u8>().is_ok()
    })?;
    let name = fields[1..rgb_start].join(" ");
    Some((
        if name.is_empty() {
            fields[1].clone()
        } else {
            name
        },
        [
            fields[rgb_start].parse().ok()?,
            fields[rgb_start + 1].parse().ok()?,
            fields[rgb_start + 2].parse().ok()?,
        ],
    ))
}
