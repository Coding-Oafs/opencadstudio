use acadrust::objects::{Material, MaterialColor, MaterialMap, ObjectType};
use acadrust::{CadDocument, EntityType, Handle};
use std::path::Path;
use std::sync::Arc;

/// Render-ready subset of an AcDbMaterial.
///
/// The complete decoded object stays authoritative in `CadDocument`; this
/// compact copy travels with a tessellated mesh so the GPU path does not need
/// to look back into the document. Map metadata is retained even when the
/// referenced image cannot be resolved, allowing a later texture upload to
/// replace the colour fallback without re-tessellating the solid.
#[derive(Clone, Debug)]
pub struct MeshMaterial {
    pub handle: Option<Handle>,
    pub name: String,
    pub description: String,
    pub diffuse: [f32; 4],
    pub ambient: [f32; 3],
    pub specular: [f32; 3],
    pub gloss: f32,
    pub reflectivity: f32,
    pub self_illumination: f32,
    pub translucence: f32,
    pub refraction_index: f32,
    pub luminance: f32,
    pub two_sided: bool,
    pub illumination_model: i32,
    pub channel_flags: i32,
    pub mode: i32,
    pub indirect_bump_scale: f32,
    pub reflectance_scale: f32,
    pub transmittance_scale: f32,
    pub luminance_mode: i16,
    pub normal_map_method: i16,
    pub normal_map_strength: f32,
    pub is_anonymous: bool,
    pub global_illumination: i16,
    pub final_gather: i16,
    pub color_bleed_scale: f32,
    pub advanced_data_present: bool,
    pub diffuse_map: MeshTextureMap,
    pub specular_map: MeshTextureMap,
    pub reflection_map: MeshTextureMap,
    pub opacity_map: MeshTextureMap,
    pub bump_map: MeshTextureMap,
    pub refraction_map: MeshTextureMap,
    pub normal_map: MeshTextureMap,
}

#[derive(Clone, Debug)]
pub struct MeshTextureMap {
    pub blend_factor: f32,
    pub projection: u8,
    pub tiling: u8,
    pub auto_transform: u8,
    pub transform: [f32; 16],
    pub source: u8,
    pub file_name: String,
    pub procedural: bool,
    pub image: Option<Arc<MaterialImage>>,
}

#[derive(Clone, Debug)]
pub struct MaterialImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    pub path: String,
}

impl Default for MeshTextureMap {
    fn default() -> Self {
        let mut transform = [0.0; 16];
        transform[0] = 1.0;
        transform[5] = 1.0;
        transform[10] = 1.0;
        transform[15] = 1.0;
        Self {
            blend_factor: 1.0,
            projection: 2,
            tiling: 1,
            auto_transform: 1,
            transform,
            source: 0,
            file_name: String::new(),
            procedural: false,
            image: None,
        }
    }
}

impl MeshTextureMap {
    fn from_dwg(map: &MaterialMap, base_dir: Option<&Path>) -> Self {
        let mut transform = [0.0; 16];
        for (dst, src) in transform.iter_mut().zip(map.transform) {
            *dst = src as f32;
        }
        Self {
            blend_factor: map.blend_factor.clamp(0.0, 1.0) as f32,
            projection: map.projection,
            tiling: map.tiling,
            auto_transform: map.auto_transform,
            transform,
            source: map.source,
            file_name: map.file_name.clone(),
            procedural: map.texture.is_some(),
            image: load_map_image(map, base_dir)
                .or_else(|| procedural_map_image(map)),
        }
    }

    pub fn has_content(&self) -> bool {
        (self.source == 1 && !self.file_name.trim().is_empty())
            || (self.source == 2 && self.procedural)
    }
}

impl MeshMaterial {
    pub fn entity_color(color: [f32; 4]) -> Self {
        Self {
            handle: None,
            name: String::new(),
            description: String::new(),
            diffuse: color,
            ambient: [color[0], color[1], color[2]],
            specular: [0.08; 3],
            gloss: 0.5,
            reflectivity: 0.0,
            self_illumination: 0.0,
            translucence: 0.0,
            refraction_index: 1.0,
            luminance: 0.0,
            two_sided: true,
            illumination_model: 0,
            channel_flags: 127,
            mode: 0,
            indirect_bump_scale: 1.0,
            reflectance_scale: 1.0,
            transmittance_scale: 1.0,
            luminance_mode: 0,
            normal_map_method: 0,
            normal_map_strength: 1.0,
            is_anonymous: false,
            global_illumination: 0,
            final_gather: 0,
            color_bleed_scale: 1.0,
            advanced_data_present: false,
            diffuse_map: MeshTextureMap::default(),
            specular_map: MeshTextureMap::default(),
            reflection_map: MeshTextureMap::default(),
            opacity_map: MeshTextureMap::default(),
            bump_map: MeshTextureMap::default(),
            refraction_map: MeshTextureMap::default(),
            normal_map: MeshTextureMap::default(),
        }
    }

    fn from_dwg(
        handle: Handle,
        material: &Material,
        entity_color: [f32; 4],
        base_dir: Option<&Path>,
    ) -> Self {
        let diffuse_map = MeshTextureMap::from_dwg(&material.diffuse_map, base_dir);
        let specular_map = MeshTextureMap::from_dwg(&material.specular_map, base_dir);
        let reflection_map = MeshTextureMap::from_dwg(&material.reflection_map, base_dir);
        let opacity_map = MeshTextureMap::from_dwg(&material.opacity_map, base_dir);
        let bump_map = MeshTextureMap::from_dwg(&material.bump_map, base_dir);
        let refraction_map = MeshTextureMap::from_dwg(&material.refraction_map, base_dir);
        let normal_map = MeshTextureMap::from_dwg(&material.normal_map, base_dir);
        // The stored diffuse component remains authoritative when an external
        // image is unavailable. Replacing it with the entity colour would lose
        // a decoded material value and make the result depend on a guess about
        // the missing asset's intended appearance.
        let diffuse = material_color(&material.diffuse_color, entity_color);
        let ambient = material_color(&material.ambient_color, entity_color);
        let specular = material_color(&material.specular_color, [1.0; 4]);
        let opacity = material.opacity_percent.clamp(0.0, 1.0) as f32;
        Self {
            handle: Some(handle),
            name: material.name.clone(),
            description: material.description.clone(),
            diffuse: [
                diffuse[0],
                diffuse[1],
                diffuse[2],
                entity_color[3] * opacity,
            ],
            ambient: [ambient[0], ambient[1], ambient[2]],
            specular: [specular[0], specular[1], specular[2]],
            gloss: material.specular_gloss_factor.clamp(0.0, 1.0) as f32,
            reflectivity: material.reflectivity.clamp(0.0, 1.0) as f32,
            self_illumination: material.self_illumination.clamp(0.0, 1.0) as f32,
            translucence: material.translucence.clamp(0.0, 1.0) as f32,
            refraction_index: material.refraction_index.max(0.0) as f32,
            luminance: material.luminance.max(0.0) as f32,
            two_sided: material.two_sided_material,
            illumination_model: material.illumination_model,
            channel_flags: material.channel_flags,
            mode: material.mode,
            indirect_bump_scale: material.indirect_bump_scale as f32,
            reflectance_scale: material.reflectance_scale as f32,
            transmittance_scale: material.transmittance_scale as f32,
            luminance_mode: material.luminance_mode,
            normal_map_method: material.normal_map_method,
            normal_map_strength: material.normal_map_strength.max(0.0) as f32,
            is_anonymous: material.is_anonymous,
            global_illumination: material.global_illumination,
            final_gather: material.final_gather,
            color_bleed_scale: material.color_bleed_scale as f32,
            advanced_data_present: material.advanced_data_present,
            diffuse_map,
            specular_map,
            reflection_map,
            opacity_map,
            bump_map,
            refraction_map,
            normal_map,
        }
    }

    pub fn apply_to(&self, set: &mut super::mesh_model::MeshLodSet) {
        for lod in &mut set.lods {
            lod.color = self.diffuse;
        }
        set.material = Some(self.clone());
    }

    pub fn apply_to_with_face_overrides(
        &self,
        set: &mut super::mesh_model::MeshLodSet,
        document: &CadDocument,
        base_dir: Option<&Path>,
    ) {
        self.apply_to(set);
        set.face_materials.clear();
        let handles: rustc_hash::FxHashSet<Handle> = set
            .lods
            .iter()
            .flat_map(|lod| lod.triangle_material_handles.iter().flatten().copied())
            .collect();
        for handle in handles {
            let material =
                resolve_material_handle_with_base(document, handle, self, base_dir);
            set.face_materials.insert(handle, material);
        }
    }
}

fn material_color(value: &MaterialColor, current: [f32; 4]) -> [f32; 4] {
    let base = if value.flag == 1 {
        value.rgb.map_or(current, |packed| {
            let packed = packed as u32;
            [
                ((packed >> 16) & 0xff) as f32 / 255.0,
                ((packed >> 8) & 0xff) as f32 / 255.0,
                (packed & 0xff) as f32 / 255.0,
                current[3],
            ]
        })
    } else {
        current
    };
    let factor = value.factor.clamp(0.0, 1.0) as f32;
    [
        base[0] * factor,
        base[1] * factor,
        base[2] * factor,
        base[3],
    ]
}

fn procedural_map_image(map: &MaterialMap) -> Option<Arc<MaterialImage>> {
    let texture = map.texture.as_ref()?;
    let color1 = material_color(&texture.color1, [0.2, 0.2, 0.2, 1.0]);
    let color2 = material_color(&texture.color2, [0.8, 0.8, 0.8, 1.0]);
    let size = 64u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let color = if ((x / 8) + (y / 8)) & 1 == 0 {
                color1
            } else {
                color2
            };
            rgba.extend(color.map(|channel| {
                (channel.clamp(0.0, 1.0) * 255.0).round() as u8
            }));
        }
    }
    Some(Arc::new(MaterialImage {
        width: size,
        height: size,
        rgba: Arc::new(rgba),
        path: format!("procedural:{}", texture.mode),
    }))
}

/// Resolve an entity's effective material using AcDbEntity's material flags.
///
/// `by_block` supplies the enclosing INSERT's material for a block child.
/// Outside a block a ByBlock material has no owner source and falls back to
/// the entity colour, matching the existing ByBlock colour behaviour.
pub fn resolve_material_with_base(
    document: &CadDocument,
    entity: &EntityType,
    entity_color: [f32; 4],
    by_block: Option<&MeshMaterial>,
    base_dir: Option<&Path>,
) -> MeshMaterial {
    let common = entity.common();
    if common.material_flags == 1 {
        return by_block
            .cloned()
            .unwrap_or_else(|| MeshMaterial::entity_color(entity_color));
    }
    let handle = if common.material_flags == 3 {
        common.material_handle
    } else {
        document
            .layers
            .get(&common.layer)
            .map(|layer| layer.material)
            .filter(|handle| handle.is_valid())
    };
    let Some(handle) = handle else {
        return MeshMaterial::entity_color(entity_color);
    };
    match document.objects.get(&handle) {
        Some(ObjectType::Material(material))
            if material.name.eq_ignore_ascii_case("ByLayer")
                || material.name.eq_ignore_ascii_case("ByBlock") =>
        {
            MeshMaterial::entity_color(entity_color)
        }
        Some(ObjectType::Material(material)) => {
            MeshMaterial::from_dwg(handle, material, entity_color, base_dir)
        }
        _ => MeshMaterial::entity_color(entity_color),
    }
}

pub fn resolve_layer_material_with_base(
    document: &CadDocument,
    layer_name: &str,
    layer_color: [f32; 4],
    base_dir: Option<&Path>,
) -> MeshMaterial {
    let handle = document
        .layers
        .get(layer_name)
        .map(|layer| layer.material)
        .filter(|handle| handle.is_valid());
    match handle.and_then(|handle| document.objects.get(&handle).map(|object| (handle, object))) {
        Some((_, ObjectType::Material(material)))
            if material.name.eq_ignore_ascii_case("ByLayer")
                || material.name.eq_ignore_ascii_case("ByBlock") =>
        {
            MeshMaterial::entity_color(layer_color)
        }
        Some((handle, ObjectType::Material(material))) => {
            MeshMaterial::from_dwg(handle, material, layer_color, base_dir)
        }
        _ => MeshMaterial::entity_color(layer_color),
    }
}

pub fn resolve_material_handle_with_base(
    document: &CadDocument,
    handle: Handle,
    fallback: &MeshMaterial,
    base_dir: Option<&Path>,
) -> MeshMaterial {
    match document.objects.get(&handle) {
        Some(ObjectType::Material(material)) if material.name.eq_ignore_ascii_case("ByLayer") => {
            fallback.clone()
        }
        Some(ObjectType::Material(material)) if material.name.eq_ignore_ascii_case("ByBlock") => {
            fallback.clone()
        }
        Some(ObjectType::Material(material)) => {
            MeshMaterial::from_dwg(handle, material, fallback.diffuse, base_dir)
        }
        _ => fallback.clone(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_map_image(map: &MaterialMap, base_dir: Option<&Path>) -> Option<Arc<MaterialImage>> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<Arc<MaterialImage>>>>> = OnceLock::new();

    if map.source != 1 || map.file_name.trim().is_empty() {
        return None;
    }
    let normalized = map.file_name.trim().replace('\\', "/");
    let source = PathBuf::from(&normalized);
    let mut candidates = Vec::with_capacity(4);
    if source.is_absolute() {
        candidates.push(source.clone());
    }
    if let Some(base) = base_dir {
        candidates.push(base.join(&source));
        if let Some(name) = source.file_name() {
            candidates.push(base.join(name));
            for folder in ["Textures", "textures", "Materials", "materials"] {
                candidates.push(base.join(folder).join(name));
            }
        }
    }
    let path = candidates.into_iter().find(|path| path.is_file())?;
    let path = path.canonicalize().unwrap_or(path);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().ok()?.get(&path).cloned() {
        return hit;
    }
    let loaded = std::fs::read(&path)
        .ok()
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .map(|image| {
            let rgba = image.to_rgba8();
            Arc::new(MaterialImage {
                width: rgba.width(),
                height: rgba.height(),
                rgba: Arc::new(rgba.into_raw()),
                path: path.to_string_lossy().into_owned(),
            })
        });
    cache.lock().ok()?.insert(path, loaded.clone());
    loaded
}

#[cfg(target_arch = "wasm32")]
fn load_map_image(_map: &MaterialMap, _base_dir: Option<&Path>) -> Option<Arc<MaterialImage>> {
    None
}
