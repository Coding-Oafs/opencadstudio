//! Bounded faceted IGES import.
//!
//! IGES analytic B-reps require a full surface/trim modeler. This reader
//! deliberately accepts the portable faceted representation (Type 106
//! copious-data polygons) and rejects analytic-only files with a precise
//! diagnostic rather than emitting incomplete placeholder geometry.

use acadrust::entities::mesh::Mesh;
use std::collections::HashMap;

const MAX_DIRECTORY_ENTITIES: usize = 20_000_000;

pub fn parse_iges(text: &str) -> Result<Mesh, String> {
    let mut directory: HashMap<usize, (i32, usize, i32)> = HashMap::new();
    let mut d_lines = Vec::new();
    let mut parameter_fragments: HashMap<usize, String> = HashMap::new();
    for line in text.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 73 {
            continue;
        }
        match bytes[72] {
            b'D' => d_lines.push(line),
            b'P' => {
                let pointer = field(line, 64, 72).and_then(|value| value.parse().ok());
                if let Some(pointer) = pointer {
                    parameter_fragments
                        .entry(pointer)
                        .or_default()
                        .push_str(line.get(..64).unwrap_or(line));
                }
            }
            _ => {}
        }
    }
    if d_lines.len() / 2 > MAX_DIRECTORY_ENTITIES {
        return Err("IGES exceeds the directory safety limit".to_string());
    }
    for pair in d_lines.chunks_exact(2) {
        let sequence = field(pair[0], 73, 80)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let entity_type = field(pair[0], 0, 8)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0);
        let parameter_pointer = field(pair[0], 8, 16)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let form = field(pair[1], 32, 40)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0);
        directory.insert(sequence, (entity_type, parameter_pointer, form));
    }

    let mut triangles = Vec::new();
    let mut analytic = 0usize;
    for (directory_sequence, (entity_type, _parameter_pointer, form)) in directory {
        if entity_type != 106 {
            if matches!(
                entity_type,
                108 | 114 | 118 | 120 | 122 | 128 | 140 | 142 | 143 | 144 | 186 | 510
            ) {
                analytic += 1;
            }
            continue;
        }
        // Columns 65-72 of each P record point back to the owning directory
        // entry.  The parameter-data pointer in the D record is a P-section
        // sequence number and is not guaranteed to have the same value.
        let Some(parameters) = parameter_fragments.get(&directory_sequence) else {
            continue;
        };
        let polygon = copious_polygon(parameters, form)?;
        if polygon.len() < 3 {
            continue;
        }
        let tessellated = crate::entities::mesh::triangulate_planar(&polygon);
        triangles.extend(
            tessellated
                .chunks_exact(3)
                .map(|triangle| [triangle[0], triangle[1], triangle[2]]),
        );
    }
    if triangles.is_empty() {
        return Err(if analytic > 0 {
            format!(
                "IGES contains {analytic} analytic/trimmed surface record(s) but no Type 106 faceted polygons"
            )
        } else {
            "IGES contains no supported Type 106 faceted polygons".to_string()
        });
    }
    crate::io::stl::mesh_from_triangles(triangles)
}

fn field(line: &str, start: usize, end: usize) -> Option<&str> {
    line.get(start..end)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn copious_polygon(parameters: &str, _form: i32) -> Result<Vec<[f64; 3]>, String> {
    let values: Vec<f64> = parameters
        .trim_end_matches(|character: char| character == ';' || character.is_ascii_whitespace())
        .split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.replace('D', "E").replace('d', "e").parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| "IGES Type 106 contains an invalid number".to_string())?;
    if values.len() < 3 || values[0] as i32 != 106 {
        return Err("IGES parameter pointer does not reference Type 106 data".to_string());
    }
    let interpretation = values[1] as i32;
    let count = values[2].max(0.0) as usize;
    let points: Vec<[f64; 3]> = match interpretation {
        1 => {
            let z = *values
                .get(3)
                .ok_or_else(|| "IGES Type 106 is truncated".to_string())?;
            values[4..]
                .chunks_exact(2)
                .take(count)
                .map(|value| [value[0], value[1], z])
                .collect()
        }
        2 => values[3..]
            .chunks_exact(3)
            .take(count)
            .map(|value| [value[0], value[1], value[2]])
            .collect(),
        3 => values[3..]
            .chunks_exact(6)
            .take(count)
            .map(|value| [value[0], value[1], value[2]])
            .collect(),
        _ => {
            return Err(format!(
                "unsupported IGES Type 106 interpretation {interpretation}"
            ));
        }
    };
    if points.len() != count {
        return Err("IGES Type 106 is truncated".to_string());
    }
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_dimensional_copious_polygon() {
        let points = copious_polygon("106,2,3,0,0,0,2,0,0,0,2,0;", 12).unwrap();
        assert_eq!(
            points,
            vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]]
        );
    }

    #[test]
    fn parameter_records_are_matched_by_directory_entry_pointer() {
        let mut d1 = vec![b' '; 80];
        d1[0..8].copy_from_slice(format!("{:>8}", 106).as_bytes());
        // Deliberately not equal to the directory sequence. P records point
        // back to the D entry in columns 65-72.
        d1[8..16].copy_from_slice(format!("{:>8}", 77).as_bytes());
        d1[72] = b'D';
        d1[73..80].copy_from_slice(format!("{:>7}", 1).as_bytes());
        let mut d2 = vec![b' '; 80];
        d2[32..40].copy_from_slice(format!("{:>8}", 12).as_bytes());
        d2[72] = b'D';
        d2[73..80].copy_from_slice(format!("{:>7}", 2).as_bytes());
        let parameters = "106,2,3,0,0,0,2,0,0,0,2,0;";
        let mut p = vec![b' '; 80];
        p[..parameters.len()].copy_from_slice(parameters.as_bytes());
        p[64..72].copy_from_slice(format!("{:>8}", 1).as_bytes());
        p[72] = b'P';
        p[73..80].copy_from_slice(format!("{:>7}", 1).as_bytes());
        let text = [d1, d2, p]
            .into_iter()
            .map(|line| String::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let mesh = parse_iges(&text).expect("faceted IGES");
        assert_eq!(1, mesh.faces.len());
    }
}
