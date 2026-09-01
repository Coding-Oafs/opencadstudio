//! Streaming 3MF Core importer.
//!
//! 3MF is an OPC/ZIP package. The primary model part is discovered through
//! the package StartPart relationship and parsed with a forward-only XML
//! reader; the (potentially hundreds of megabytes) model XML is never copied
//! into a `String`. Core meshes, base materials, components, build items,
//! units and affine transforms are converted to persistent DWG MESH entities.

use acadrust::entities::mesh::{Mesh, MeshFace};
use acadrust::tables::Layer;
use acadrust::types::{Color, Vector3};
use acadrust::{CadDocument, EntityType};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::Path;

const START_PART_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
const MAX_ARCHIVE_ENTRIES: usize = 4096;
const MAX_TOTAL_UNCOMPRESSED: u128 = 4 * 1024 * 1024 * 1024;
const MAX_MODEL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_VERTICES: usize = 100_000_000;
const MAX_TRIANGLES: usize = 200_000_000;
const MAX_COMPONENT_DEPTH: usize = 64;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportStats {
    pub source_objects: usize,
    pub mesh_entities: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub components: usize,
    pub build_items: usize,
    /// Base materials across every `basematerials` group.
    pub materials: usize,
    /// Declared model unit (3MF defaults to millimeter).
    pub unit: String,
    /// Bounds of the emitted build in transformed model space: (min, max).
    pub bounds: Option<([f64; 3], [f64; 3])>,
    /// Package parts the Core importer does not consume (thumbnails, texture
    /// atlases, print settings…), reported so silent data loss is visible.
    pub skipped_parts: Vec<String>,
    /// Total number of skipped parts; `skipped_parts` holds a capped sample.
    pub skipped_parts_total: usize,
}

/// OPC infrastructure present in every conforming package — not a "feature"
/// that was skipped.
fn is_infrastructure_part(name: &str) -> bool {
    name.eq_ignore_ascii_case("[Content_Types].xml") || name.starts_with("_rels/")
}

impl ImportStats {
    /// Command-line diagnostics surfaced when an imported model finishes
    /// opening: counts, unit, bounds, materials, and skipped package parts.
    pub fn report_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(4);
        lines.push(format!(
            "3MF import: {} source object(s) -> {} CAD mesh entities, {} vertices, {} triangles",
            self.source_objects, self.mesh_entities, self.vertices, self.triangles
        ));
        lines.push(format!(
            "3MF model: unit {}, {} component(s), {} build item(s), {} base material(s)",
            self.unit, self.components, self.build_items, self.materials
        ));
        if let Some((min, max)) = self.bounds {
            lines.push(format!(
                "3MF bounds: min ({:.3}, {:.3}, {:.3}) max ({:.3}, {:.3}, {:.3})",
                min[0], min[1], min[2], max[0], max[1], max[2]
            ));
        }
        if self.skipped_parts_total > 0 {
            let extra = self
                .skipped_parts_total
                .saturating_sub(self.skipped_parts.len());
            let mut listed = self.skipped_parts.join(", ");
            if extra > 0 {
                listed.push_str(&format!(" (+{extra} more)"));
            }
            lines.push(format!(
                "3MF skipped package parts (not imported): {listed}"
            ));
        }
        lines
    }
}

pub struct ImportResult {
    pub document: CadDocument,
    pub stats: ImportStats,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    values: [f64; 12],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            values: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        }
    }
}

impl Transform {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let parsed: Vec<f64> = value
            .split_whitespace()
            .map(|part| {
                part.parse::<f64>()
                    .map_err(|_| format!("invalid 3MF transform value: {part}"))
            })
            .collect::<Result<_, _>>()?;
        let values: [f64; 12] = parsed
            .try_into()
            .map_err(|_| "3MF transform must contain exactly 12 numbers".to_string())?;
        if !values.iter().all(|value| value.is_finite()) {
            return Err("3MF transform contains a non-finite number".to_string());
        }
        Ok(Self { values })
    }

    /// Apply the row-vector affine matrix ordering defined by 3MF.
    fn apply(self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let m = self.values;
        [
            x * m[0] + y * m[3] + z * m[6] + m[9],
            x * m[1] + y * m[4] + z * m[7] + m[10],
            x * m[2] + y * m[5] + z * m[8] + m[11],
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PropertyRef {
    pid: u32,
    index: u32,
}

#[derive(Debug, Clone)]
struct Triangle {
    vertices: [u32; 3],
    property: Option<PropertyRef>,
}

#[derive(Debug, Clone)]
struct Component {
    object_id: u32,
    transform: Transform,
}

#[derive(Debug, Clone, Default)]
struct ObjectResource {
    id: u32,
    name: String,
    property: Option<PropertyRef>,
    vertices: Vec<[f64; 3]>,
    triangles: Vec<Triangle>,
    components: Vec<Component>,
}

#[derive(Debug, Clone, Copy)]
struct BuildItem {
    object_id: u32,
    transform: Transform,
}

#[derive(Debug, Default)]
struct Model {
    unit: String,
    materials: HashMap<u32, Vec<(String, [u8; 4])>>,
    objects: HashMap<u32, ObjectResource>,
    build: Vec<BuildItem>,
    stats: ImportStats,
}

/// Open a 3MF package and convert its primary build to persistent CAD meshes.
pub fn import_path(
    path: &Path,
    progress: Option<&(dyn Fn(u16) + Sync)>,
) -> Result<ImportResult, String> {
    let file = File::open(path).map_err(|error| format!("failed to open 3MF: {error}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| format!("invalid 3MF ZIP package: {error}"))?;
    let mut feature_parts = validate_archive(&mut archive)?;
    let model_part = primary_model_part(&mut archive)?;
    feature_parts.retain(|part| !part.eq_ignore_ascii_case(&model_part));
    let model_size = {
        let entry = archive
            .by_name(&model_part)
            .map_err(|_| format!("3MF model part is missing: /{model_part}"))?;
        entry.size()
    };
    if model_size > MAX_MODEL_BYTES {
        return Err(format!(
            "3MF model XML is too large ({model_size} bytes; limit {MAX_MODEL_BYTES})"
        ));
    }
    let entry = archive
        .by_name(&model_part)
        .map_err(|_| format!("3MF model part is missing: /{model_part}"))?;
    let buffered = BufReader::with_capacity(1024 * 1024, entry);
    let mut model = parse_model(buffered, model_size, progress)?;
    model.stats.unit = model.unit.clone();
    model.stats.materials = model.materials.values().map(Vec::len).sum();
    model.stats.skipped_parts_total = feature_parts.len();
    model.stats.skipped_parts = feature_parts;
    model.stats.skipped_parts.truncate(8);
    let mut result = model.into_document()?;
    result.document.source_path = Some(path.to_string_lossy().into_owned());
    if let Some(progress) = progress {
        progress(1000);
    }
    Ok(result)
}

/// Validates entry-count, expansion, path-safety and compression-ratio limits,
/// and returns every non-infrastructure part name for skip diagnostics.
fn validate_archive<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<Vec<String>, String> {
    if archive.len() == 0 || archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "3MF package contains an unsafe number of entries ({})",
            archive.len()
        ));
    }
    let mut feature_parts = Vec::new();
    let mut total = 0u128;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("invalid 3MF ZIP entry: {error}"))?;
        total = total.saturating_add(entry.size() as u128);
        if total > MAX_TOTAL_UNCOMPRESSED {
            return Err(format!(
                "3MF package expands to more than {MAX_TOTAL_UNCOMPRESSED} bytes"
            ));
        }
        // Explicit ZIP directory entries are legal in OPC packages. Validate
        // their path without treating the conventional trailing slash as an
        // empty (and therefore unsafe) path component.
        let entry_name = entry.name().trim_end_matches('/');
        if !entry_name.is_empty() {
            let normalized = normalize_part_name(entry_name)?;
            if !is_infrastructure_part(&normalized) {
                feature_parts.push(normalized);
            }
        }
        if entry.compressed_size() > 0 && entry.size() / entry.compressed_size().max(1) > 1000 {
            return Err(format!(
                "3MF ZIP entry has an unsafe compression ratio: {}",
                entry.name()
            ));
        }
    }
    Ok(feature_parts)
}

fn normalize_part_name(name: &str) -> Result<String, String> {
    let name = name.trim_start_matches('/').replace('\\', "/");
    if name.is_empty()
        || name.starts_with('.')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe 3MF package part name: {name}"));
    }
    Ok(name)
}

fn primary_model_part<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<String, String> {
    let mut relationships = Vec::new();
    archive
        .by_name("_rels/.rels")
        .map_err(|_| "3MF package is missing /_rels/.rels".to_string())?
        .take(1024 * 1024)
        .read_to_end(&mut relationships)
        .map_err(|error| format!("failed reading 3MF relationships: {error}"))?;
    let mut reader = Reader::from_reader(Cursor::new(relationships));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"Relationship" =>
            {
                let attrs = attributes(&element, reader.decoder())?;
                if attrs
                    .get("Type")
                    .is_some_and(|value| value == START_PART_REL)
                {
                    let target = attrs
                        .get("Target")
                        .ok_or_else(|| "3MF StartPart relationship has no target".to_string())?;
                    return normalize_part_name(target);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("invalid 3MF relationships XML: {error}")),
        }
        buffer.clear();
    }
    Err("3MF package has no primary 3D model relationship".to_string())
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn attributes(
    element: &BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
) -> Result<HashMap<String, String>, String> {
    let mut result = HashMap::new();
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| format!("invalid 3MF XML attribute: {error}"))?;
        let key = String::from_utf8_lossy(local_name(attribute.key.as_ref())).into_owned();
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
            .map_err(|error| format!("invalid 3MF XML attribute value: {error}"))?
            .into_owned();
        result.insert(key, value);
    }
    Ok(result)
}

fn required_u32(attrs: &HashMap<String, String>, name: &str) -> Result<u32, String> {
    attrs
        .get(name)
        .ok_or_else(|| format!("3MF element is missing required {name} attribute"))?
        .parse::<u32>()
        .map_err(|_| format!("invalid 3MF {name} index"))
}

fn optional_property(
    attrs: &HashMap<String, String>,
    fallback: Option<PropertyRef>,
) -> Result<Option<PropertyRef>, String> {
    let Some(pid) = attrs.get("pid") else {
        return Ok(fallback);
    };
    let pid = pid
        .parse::<u32>()
        .map_err(|_| "invalid 3MF pid".to_string())?;
    let index = attrs
        .get("p1")
        .or_else(|| attrs.get("pindex"))
        .map(String::as_str)
        .unwrap_or("0")
        .parse::<u32>()
        .map_err(|_| "invalid 3MF property index".to_string())?;
    Ok(Some(PropertyRef { pid, index }))
}

fn parse_model<R: BufRead>(
    source: R,
    source_size: u64,
    progress: Option<&(dyn Fn(u16) + Sync)>,
) -> Result<Model, String> {
    let mut reader = Reader::from_reader(source);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut model = Model {
        unit: "millimeter".to_string(),
        ..Model::default()
    };
    let mut current_object: Option<ObjectResource> = None;
    let mut current_materials: Option<(u32, Vec<(String, [u8; 4])>)> = None;
    let mut events = 0usize;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("invalid 3MF model XML: {error}"))?;
        match event {
            Event::Start(element) => {
                let qualified_name = element.name();
                let name = local_name(qualified_name.as_ref());
                let attrs = attributes(&element, reader.decoder())?;
                match name {
                    b"model" => {
                        if let Some(unit) = attrs.get("unit") {
                            model.unit = unit.to_ascii_lowercase();
                        }
                        if attrs
                            .get("requiredextensions")
                            .is_some_and(|value| !value.trim().is_empty())
                        {
                            return Err(format!(
                                "3MF requires unsupported extension(s): {}",
                                attrs["requiredextensions"]
                            ));
                        }
                    }
                    b"basematerials" => {
                        current_materials = Some((required_u32(&attrs, "id")?, Vec::new()));
                    }
                    b"object" => {
                        let property = match (attrs.get("pid"), attrs.get("pindex")) {
                            (Some(pid), Some(index)) => Some(PropertyRef {
                                pid: pid
                                    .parse()
                                    .map_err(|_| "invalid 3MF object pid".to_string())?,
                                index: index
                                    .parse()
                                    .map_err(|_| "invalid 3MF object pindex".to_string())?,
                            }),
                            _ => None,
                        };
                        current_object = Some(ObjectResource {
                            id: required_u32(&attrs, "id")?,
                            name: attrs.get("name").cloned().unwrap_or_default(),
                            property,
                            ..ObjectResource::default()
                        });
                    }
                    _ => consume_node(
                        name,
                        &attrs,
                        &mut current_object,
                        &mut current_materials,
                        &mut model,
                    )?,
                }
            }
            Event::Empty(element) => {
                let qualified_name = element.name();
                let name = local_name(qualified_name.as_ref());
                let attrs = attributes(&element, reader.decoder())?;
                consume_node(
                    name,
                    &attrs,
                    &mut current_object,
                    &mut current_materials,
                    &mut model,
                )?;
            }
            Event::End(element) => match local_name(element.name().as_ref()) {
                b"object" => {
                    let object = current_object
                        .take()
                        .ok_or_else(|| "unexpected closing 3MF object element".to_string())?;
                    if model.objects.insert(object.id, object).is_some() {
                        return Err("duplicate 3MF object resource id".to_string());
                    }
                }
                b"basematerials" => {
                    let (id, materials) = current_materials.take().ok_or_else(|| {
                        "unexpected closing 3MF basematerials element".to_string()
                    })?;
                    if model.materials.insert(id, materials).is_some() {
                        return Err("duplicate 3MF material resource id".to_string());
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        events += 1;
        if events & 0x3fff == 0 {
            if let Some(progress) = progress {
                let position = reader.buffer_position().min(source_size);
                progress(((position.saturating_mul(900)) / source_size.max(1)) as u16);
            }
        }
        buffer.clear();
    }
    model.stats.source_objects = model.objects.len();
    model.stats.build_items = model.build.len();
    if model.build.is_empty() {
        return Err("3MF model contains no build items".to_string());
    }
    Ok(model)
}

fn consume_node(
    name: &[u8],
    attrs: &HashMap<String, String>,
    current_object: &mut Option<ObjectResource>,
    current_materials: &mut Option<(u32, Vec<(String, [u8; 4])>)>,
    model: &mut Model,
) -> Result<(), String> {
    match name {
        b"base" => {
            let (_, materials) = current_materials
                .as_mut()
                .ok_or_else(|| "3MF base material is outside a group".to_string())?;
            let color = parse_color(
                attrs
                    .get("displaycolor")
                    .ok_or_else(|| "3MF base material has no displaycolor".to_string())?,
            )?;
            materials.push((attrs.get("name").cloned().unwrap_or_default(), color));
        }
        b"vertex" => {
            let object = current_object
                .as_mut()
                .ok_or_else(|| "3MF vertex is outside an object".to_string())?;
            if object.vertices.len() >= MAX_VERTICES {
                return Err("3MF exceeds the vertex safety limit".to_string());
            }
            let number = |key: &str| -> Result<f64, String> {
                let value = attrs
                    .get(key)
                    .ok_or_else(|| format!("3MF vertex is missing {key}"))?
                    .parse::<f64>()
                    .map_err(|_| format!("invalid 3MF vertex {key}"))?;
                value
                    .is_finite()
                    .then_some(value)
                    .ok_or_else(|| format!("non-finite 3MF vertex {key}"))
            };
            object
                .vertices
                .push([number("x")?, number("y")?, number("z")?]);
        }
        b"triangle" => {
            let object = current_object
                .as_mut()
                .ok_or_else(|| "3MF triangle is outside an object".to_string())?;
            if object.triangles.len() >= MAX_TRIANGLES {
                return Err("3MF exceeds the triangle safety limit".to_string());
            }
            let vertices = [
                required_u32(attrs, "v1")?,
                required_u32(attrs, "v2")?,
                required_u32(attrs, "v3")?,
            ];
            if vertices[0] == vertices[1]
                || vertices[1] == vertices[2]
                || vertices[2] == vertices[0]
            {
                return Err("3MF triangle contains duplicate vertex indices".to_string());
            }
            let property = optional_property(attrs, object.property)?;
            object.triangles.push(Triangle { vertices, property });
        }
        b"component" => {
            let object = current_object
                .as_mut()
                .ok_or_else(|| "3MF component is outside an object".to_string())?;
            object.components.push(Component {
                object_id: required_u32(attrs, "objectid")?,
                transform: Transform::parse(attrs.get("transform").map(String::as_str))?,
            });
            model.stats.components += 1;
        }
        b"item" => model.build.push(BuildItem {
            object_id: required_u32(attrs, "objectid")?,
            transform: Transform::parse(attrs.get("transform").map(String::as_str))?,
        }),
        _ => {}
    }
    Ok(())
}

fn parse_color(value: &str) -> Result<[u8; 4], String> {
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| format!("invalid 3MF color: {value}"))?;
    if hex.len() != 6 && hex.len() != 8 {
        return Err(format!("invalid 3MF color: {value}"));
    }
    let byte = |offset: usize| {
        u8::from_str_radix(&hex[offset..offset + 2], 16)
            .map_err(|_| format!("invalid 3MF color: {value}"))
    };
    Ok([
        byte(0)?,
        byte(2)?,
        byte(4)?,
        if hex.len() == 8 { byte(6)? } else { 255 },
    ])
}

impl Model {
    fn material_color(&self, property: Option<PropertyRef>) -> [u8; 4] {
        property
            .and_then(|property| {
                self.materials
                    .get(&property.pid)?
                    .get(property.index as usize)
            })
            .map(|(_, color)| *color)
            .unwrap_or([178, 178, 217, 255])
    }

    fn into_document(mut self) -> Result<ImportResult, String> {
        let mut document = CadDocument::new();
        document.header.insertion_units = unit_code(&self.unit)?;
        let mut stats = std::mem::take(&mut self.stats);
        let mut active = HashSet::new();
        for item in &self.build {
            self.emit_object(
                item.object_id,
                &[item.transform],
                &mut active,
                &mut document,
                &mut stats,
                0,
            )?;
        }
        if stats.mesh_entities == 0 {
            return Err("3MF build resolves to no usable mesh geometry".to_string());
        }
        Ok(ImportResult { document, stats })
    }

    fn emit_object(
        &self,
        object_id: u32,
        outer_transforms: &[Transform],
        active: &mut HashSet<u32>,
        document: &mut CadDocument,
        stats: &mut ImportStats,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err("3MF component nesting exceeds the safety limit".to_string());
        }
        if !active.insert(object_id) {
            return Err(format!(
                "3MF component cycle detected at object {object_id}"
            ));
        }
        let object = self
            .objects
            .get(&object_id)
            .ok_or_else(|| format!("3MF references missing object {object_id}"))?;
        if !object.vertices.is_empty() || !object.triangles.is_empty() {
            self.emit_mesh(object, outer_transforms, document, stats)?;
        }
        for component in &object.components {
            let mut transforms = Vec::with_capacity(outer_transforms.len() + 1);
            transforms.push(component.transform);
            transforms.extend_from_slice(outer_transforms);
            self.emit_object(
                component.object_id,
                &transforms,
                active,
                document,
                stats,
                depth + 1,
            )?;
        }
        active.remove(&object_id);
        Ok(())
    }

    fn emit_mesh(
        &self,
        object: &ObjectResource,
        transforms: &[Transform],
        document: &mut CadDocument,
        stats: &mut ImportStats,
    ) -> Result<(), String> {
        if object.vertices.is_empty() || object.triangles.is_empty() {
            return Err(format!(
                "3MF object {} has incomplete mesh geometry",
                object.id
            ));
        }
        for triangle in &object.triangles {
            if triangle
                .vertices
                .iter()
                .any(|index| *index as usize >= object.vertices.len())
            {
                return Err(format!(
                    "3MF object {} has an out-of-range triangle index",
                    object.id
                ));
            }
        }

        let mut groups: HashMap<[u8; 4], Vec<&Triangle>> = HashMap::new();
        for triangle in &object.triangles {
            groups
                .entry(self.material_color(triangle.property.or(object.property)))
                .or_default()
                .push(triangle);
        }
        let multiple_materials = groups.len() > 1;
        for (color, triangles) in groups {
            let layer_name = layer_name(object, color, multiple_materials);
            ensure_layer(document, &layer_name, color);
            let mut mesh = Mesh::new();
            mesh.blend_crease = false;
            mesh.common.layer = layer_name;
            mesh.common.color = Color::Rgb {
                r: color[0],
                g: color[1],
                b: color[2],
            };

            if triangles.len() == object.triangles.len() {
                mesh.vertices = object
                    .vertices
                    .iter()
                    .map(|vertex| transformed_vertex(*vertex, transforms))
                    .map(|[x, y, z]| Vector3::new(x, y, z))
                    .collect();
                mesh.faces = triangles
                    .iter()
                    .map(|triangle| {
                        MeshFace::triangle(
                            triangle.vertices[0] as usize,
                            triangle.vertices[1] as usize,
                            triangle.vertices[2] as usize,
                        )
                    })
                    .collect();
            } else {
                let mut remap: HashMap<u32, usize> = HashMap::new();
                for triangle in triangles {
                    let mut mapped = [0usize; 3];
                    for (corner, source) in triangle.vertices.into_iter().enumerate() {
                        mapped[corner] = if let Some(index) = remap.get(&source) {
                            *index
                        } else {
                            let vertex =
                                transformed_vertex(object.vertices[source as usize], transforms);
                            let index = mesh.vertices.len();
                            mesh.vertices
                                .push(Vector3::new(vertex[0], vertex[1], vertex[2]));
                            remap.insert(source, index);
                            index
                        };
                    }
                    mesh.faces
                        .push(MeshFace::triangle(mapped[0], mapped[1], mapped[2]));
                }
            }
            stats.vertices += mesh.vertices.len();
            stats.triangles += mesh.faces.len();
            for vertex in &mesh.vertices {
                expand_bounds(&mut stats.bounds, [vertex.x, vertex.y, vertex.z]);
            }
            document
                .add_entity(EntityType::Mesh(mesh))
                .map_err(|error| format!("failed adding imported 3MF mesh: {error}"))?;
            stats.mesh_entities += 1;
        }
        Ok(())
    }
}

fn transformed_vertex(mut vertex: [f64; 3], transforms: &[Transform]) -> [f64; 3] {
    for transform in transforms {
        vertex = transform.apply(vertex);
    }
    vertex
}

fn expand_bounds(bounds: &mut Option<([f64; 3], [f64; 3])>, vertex: [f64; 3]) {
    let (mut min, mut max) = bounds.unwrap_or(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]));
    for axis in 0..3 {
        min[axis] = min[axis].min(vertex[axis]);
        max[axis] = max[axis].max(vertex[axis]);
    }
    *bounds = Some((min, max));
}

fn unit_code(unit: &str) -> Result<i16, String> {
    match unit {
        "micron" => Ok(13),
        "millimeter" => Ok(4),
        "centimeter" => Ok(5),
        "meter" => Ok(6),
        "inch" => Ok(1),
        "foot" => Ok(2),
        _ => Err(format!("unsupported 3MF model unit: {unit}")),
    }
}

fn layer_name(object: &ObjectResource, color: [u8; 4], include_color: bool) -> String {
    let fallback = format!("Object {}", object.id);
    let raw = if object.name.trim().is_empty() {
        fallback.as_str()
    } else {
        object.name.trim()
    };
    let mut clean: String = raw
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | '/' | '\\' | '"' | ':' | ';' | '?' | '*' | '|' | ',' | '=' | '`'
                )
            {
                '_'
            } else {
                character
            }
        })
        .take(220)
        .collect();
    if clean.is_empty() {
        clean = fallback;
    }
    if include_color {
        format!(
            "3MF/{clean} #{:02X}{:02X}{:02X}",
            color[0], color[1], color[2]
        )
    } else {
        format!("3MF/{clean}")
    }
}

fn ensure_layer(document: &mut CadDocument, name: &str, color: [u8; 4]) {
    if document.layers.contains(name) {
        return;
    }
    let mut layer = Layer::new(name);
    layer.handle = document.allocate_handle();
    layer.color = Color::Rgb {
        r: color[0],
        g: color[1],
        b: color[2],
    };
    let _ = document.layers.add(layer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_components_materials_units_and_transform() {
        let xml = br##"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <resources>
    <basematerials id="5"><base name="green" displaycolor="#00FF00" /></basematerials>
    <object id="1" name="Terrain" pid="5" pindex="0"><mesh>
      <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
      <triangles><triangle v1="0" v2="1" v3="2"/></triangles>
    </mesh></object>
    <object id="2" name="Assembly"><components><component objectid="1" transform="1 0 0 0 1 0 0 0 1 10 20 30"/></components></object>
  </resources>
  <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 1 2 3"/></build>
</model>"##;
        let model = parse_model(Cursor::new(xml), xml.len() as u64, None).unwrap();
        let result = model.into_document().unwrap();
        assert_eq!(result.document.header.insertion_units, 4);
        assert_eq!(result.stats.mesh_entities, 1);
        assert_eq!(result.stats.components, 1);
        let entity = result.document.entities().next().unwrap();
        let EntityType::Mesh(mesh) = entity else {
            panic!("expected mesh")
        };
        assert_eq!(mesh.common.layer, "3MF/Terrain");
        assert_eq!(mesh.common.color, Color::Rgb { r: 0, g: 255, b: 0 });
        assert_eq!(mesh.vertices[0], Vector3::new(11.0, 22.0, 33.0));
    }

    #[test]
    fn rejects_required_unknown_extensions() {
        let xml = br#"<model unit="millimeter" requiredextensions="vendor" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources/><build/></model>"#;
        let error = parse_model(Cursor::new(xml), xml.len() as u64, None).unwrap_err();
        assert!(error.contains("unsupported extension"));
    }

    #[test]
    fn omitted_component_transform_is_identity_and_preserves_z() {
        let xml = br#"<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <resources>
    <object id="1"><mesh><vertices>
      <vertex x="2" y="3" z="7"/><vertex x="3" y="3" z="7"/><vertex x="2" y="4" z="7"/>
    </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>
    <object id="2"><components><component objectid="1"/></components></object>
  </resources>
  <build><item objectid="2"/></build>
</model>"#;
        let model = parse_model(Cursor::new(xml), xml.len() as u64, None).unwrap();
        let result = model.into_document().unwrap();
        let EntityType::Mesh(mesh) = result.document.entities().next().unwrap() else {
            panic!("expected mesh")
        };
        assert_eq!(mesh.vertices[0], Vector3::new(2.0, 3.0, 7.0));
    }

    #[test]
    fn imports_a_minimal_package_and_reports_diagnostics() {
        use std::io::Write;

        let mut package = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut package);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.start_file("_rels/.rels", options).unwrap();
            writer
                .write_all(
                    br##"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Target="/3D/3dmodel.model" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"##,
                )
                .unwrap();
            writer.start_file("3D/3dmodel.model", options).unwrap();
            writer
                .write_all(
                    br##"<?xml version="1.0" encoding="UTF-8"?>
<model unit="centimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <resources>
    <basematerials id="5"><base name="red" displaycolor="#FF0000"/></basematerials>
    <object id="1" pid="5" pindex="0"><mesh>
      <vertices><vertex x="0" y="0" z="1"/><vertex x="2" y="0" z="1"/><vertex x="0" y="2" z="1"/></vertices>
      <triangles><triangle v1="0" v2="1" v3="2"/></triangles>
    </mesh></object>
  </resources>
  <build><item objectid="1"/></build>
</model>"##,
                )
                .unwrap();
            writer
                .start_file("Metadata/Thumbnail/thumbnail.png", options)
                .unwrap();
            writer.write_all(b"not a real png").unwrap();
            writer.finish().unwrap();
        }
        let path =
            std::env::temp_dir().join(format!("ocs-3mf-diagnostic-{}.3mf", std::process::id()));
        std::fs::write(&path, package.get_ref()).unwrap();
        let imported = import_path(&path, None).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(1, imported.stats.mesh_entities);
        assert_eq!(1, imported.stats.materials);
        assert_eq!("centimeter", imported.stats.unit);
        assert_eq!(5, imported.document.header.insertion_units);
        assert_eq!(
            Some(([0.0, 0.0, 1.0], [2.0, 2.0, 1.0])),
            imported.stats.bounds
        );
        assert_eq!(
            vec!["Metadata/Thumbnail/thumbnail.png".to_string()],
            imported.stats.skipped_parts
        );
        assert_eq!(1, imported.stats.skipped_parts_total);
        let report = imported.stats.report_lines().join("\n");
        assert!(report.contains("unit centimeter"), "{report}");
        assert!(report.contains("1 base material"), "{report}");
        assert!(
            report.contains("min (0.000, 0.000, 1.000) max (2.000, 2.000, 1.000)"),
            "{report}"
        );
        assert!(
            report.contains("Metadata/Thumbnail/thumbnail.png"),
            "{report}"
        );
    }

    /// Opt-in stress fixture for developer machines; CI does not ship large
    /// third-party models. Run with `OCS_3MF_STRESS_FILE=<path> cargo test
    /// imports_external_stress_fixture -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn imports_external_stress_fixture() {
        let path = std::env::var_os("OCS_3MF_STRESS_FILE")
            .map(std::path::PathBuf::from)
            .expect("set OCS_3MF_STRESS_FILE to a local 3MF model");
        let imported = import_path(&path, None).unwrap();
        assert!(imported.stats.mesh_entities > 0);
        assert!(imported.stats.vertices > 0);
        assert!(imported.stats.triangles > 0);
        eprintln!("3MF import stats: {:?}", imported.stats);
    }

    /// Full viewport-cache stress pass for a large local 3MF fixture.
    #[test]
    #[ignore]
    fn builds_external_stress_fixture_render_caches() {
        let path = std::env::var_os("OCS_3MF_STRESS_FILE")
            .map(std::path::PathBuf::from)
            .expect("set OCS_3MF_STRESS_FILE to a local 3MF model");
        let imported = import_path(&path, None).unwrap();
        let caches = crate::scene::build_derived_caches_with_progress(
            &imported.document,
            &|_| {},
            path.parent(),
        );
        assert_eq!(caches.meshes.len(), imported.stats.mesh_entities);
        assert!(caches.meshes.values().all(|set| !set.lods.is_empty()));
        assert!(caches.meshes.values().any(|set| set.lods.len() > 1));
        let triangles: usize = caches
            .meshes
            .values()
            .filter_map(|set| set.lods.first())
            .map(|mesh| mesh.indices.len() / 3)
            .sum();
        assert_eq!(triangles, imported.stats.triangles);
    }

    /// Opt-in collection audit for local 3MF libraries. This checks every
    /// package recursively without retaining more than one imported model at
    /// a time, and pins the invariants required by the GPU index path.
    #[test]
    #[ignore]
    fn audits_external_three_mf_directory() {
        let root = std::env::var_os("OCS_3MF_STRESS_DIR")
            .map(std::path::PathBuf::from)
            .expect("set OCS_3MF_STRESS_DIR to a local directory of 3MF models");
        let mut pending = vec![root];
        let mut files = Vec::new();
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("3mf"))
                {
                    files.push(path);
                }
            }
        }
        files.sort();
        assert!(!files.is_empty(), "no 3MF models found");

        for path in files {
            let imported = import_path(&path, None)
                .unwrap_or_else(|error| panic!("failed importing {}: {error}", path.display()));
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            let mut checked_triangles = 0usize;
            for entity in imported.document.entities() {
                let EntityType::Mesh(mesh) = entity else {
                    continue;
                };
                for vertex in &mesh.vertices {
                    let values = [vertex.x, vertex.y, vertex.z];
                    assert!(
                        values.iter().all(|value| value.is_finite()),
                        "non-finite vertex in {}",
                        path.display()
                    );
                    for axis in 0..3 {
                        min[axis] = min[axis].min(values[axis]);
                        max[axis] = max[axis].max(values[axis]);
                    }
                }
                for face in &mesh.faces {
                    assert_eq!(face.vertices.len(), 3, "non-triangle face");
                    assert!(
                        face.vertices
                            .iter()
                            .all(|index| *index < mesh.vertices.len()),
                        "out-of-range triangle index in {}",
                        path.display()
                    );
                    assert!(
                        face.vertices[0] != face.vertices[1]
                            && face.vertices[1] != face.vertices[2]
                            && face.vertices[2] != face.vertices[0],
                        "degenerate triangle index in {}",
                        path.display()
                    );
                    checked_triangles += 1;
                }
            }
            assert_eq!(checked_triangles, imported.stats.triangles);
            eprintln!(
                "3MF audit: {} meshes={} vertices={} triangles={} bounds={min:?}..{max:?}",
                path.display(),
                imported.stats.mesh_entities,
                imported.stats.vertices,
                imported.stats.triangles,
            );
        }
    }
}
