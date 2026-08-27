//! Native GIS layer commands and project integration.

use super::OpenCADStudio;
use ocs_gis::{
    create_geopackage, feature_tables, open_geopackage, read_layer, validate_topology, write_layer,
    FeatureLayer, TopologyRules,
};
use std::fs;
use std::path::{Path, PathBuf};

impl OpenCADStudio {
    pub(super) fn import_gis_source(&mut self, tab: usize, path: PathBuf) {
        match load_layers(&path, self.default_gis_epsg(tab)) {
            Ok(layers) => {
                let count = layers.len();
                let features: usize = layers.iter().map(|layer| layer.features.len()).sum();
                let names: Vec<String> = layers.iter().map(|layer| layer.name.clone()).collect();
                for layer in layers {
                    if let Some(existing) = self.tabs[tab]
                        .gis_layers
                        .iter_mut()
                        .find(|existing| existing.name.eq_ignore_ascii_case(&layer.name))
                    {
                        *existing = layer;
                    } else {
                        self.tabs[tab].gis_layers.push(layer);
                    }
                }
                self.catalog_gis_source(tab, &path, &names);
                self.command_line.push_output(
                    format!(
                        "GISIMPORT: loaded {count} layer(s), {features} feature(s) from {}.",
                        path.display()
                    )
                    .as_str(),
                );
            }
            Err(error) => self
                .command_line
                .push_error(format!("GISIMPORT: {error}").as_str()),
        }
    }

    pub(super) fn list_gis_layers(&mut self, tab: usize) {
        if self.tabs[tab].gis_layers.is_empty() {
            self.command_line
                .push_info("GISLAYERS: no feature layers are loaded.");
            return;
        }
        let rows: Vec<String> = self.tabs[tab]
            .gis_layers
            .iter()
            .map(|layer| {
                format!(
                    "{} — {} feature(s), {} field(s), EPSG:{}",
                    layer.name,
                    layer.features.len(),
                    layer.fields.len(),
                    layer.epsg
                )
            })
            .collect();
        for row in rows {
            self.command_line.push_output(&row);
        }
    }

    pub(super) fn validate_gis_layer(&mut self, tab: usize, name: Option<&str>) {
        let layers: Vec<&FeatureLayer> = match name {
            Some(name) => self.tabs[tab]
                .gis_layers
                .iter()
                .filter(|layer| layer.name.eq_ignore_ascii_case(name))
                .collect(),
            None => self.tabs[tab].gis_layers.iter().collect(),
        };
        if layers.is_empty() {
            self.command_line
                .push_error("GISTOPOLOGY: layer was not found.");
            return;
        }
        let reports: Vec<String> = layers
            .into_iter()
            .map(|layer| {
                let issues = validate_topology(layer, TopologyRules::editing_defaults());
                let sample = issues
                    .iter()
                    .take(3)
                    .map(|issue| format!("{:?} {:?}", issue.kind, issue.feature_ids))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "GISTOPOLOGY: {} — {} issue(s){}",
                    layer.name,
                    issues.len(),
                    if sample.is_empty() {
                        String::new()
                    } else {
                        format!(" ({sample})")
                    }
                )
            })
            .collect();
        for report in reports {
            self.command_line.push_output(&report);
        }
    }

    pub(super) fn export_gis_layer(&mut self, tab: usize, name: &str, path: PathBuf) {
        let Some(layer) = self.tabs[tab]
            .gis_layers
            .iter()
            .find(|layer| layer.name.eq_ignore_ascii_case(name))
            .cloned()
        else {
            self.command_line
                .push_error(format!("GISEXPORT: layer '{name}' was not found.").as_str());
            return;
        };
        let result = match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("geojson" | "json") => {
                if path.exists() {
                    Err(format!("refusing to overwrite {}", path.display()))
                } else {
                    fs::write(&path, layer.to_geojson()).map_err(|error| error.to_string())
                }
            }
            Some("gpkg") => {
                create_geopackage(&path).and_then(|connection| write_layer(&connection, &layer))
            }
            _ => Err("output must use .gpkg or .geojson".into()),
        };
        match result {
            Ok(()) => self.command_line.push_output(
                format!("GISEXPORT: wrote '{}' to {}.", layer.name, path.display()).as_str(),
            ),
            Err(error) => self
                .command_line
                .push_error(format!("GISEXPORT: {error}").as_str()),
        }
    }

    pub(super) fn transform_gis_layer(&mut self, tab: usize, name: &str, target_epsg: u16) {
        let Some(index) = self.tabs[tab]
            .gis_layers
            .iter()
            .position(|layer| layer.name.eq_ignore_ascii_case(name))
        else {
            self.command_line
                .push_error(format!("GISTRANSFORM: layer '{name}' was not found.").as_str());
            return;
        };
        let source_epsg = self.tabs[tab].gis_layers[index].epsg;
        let catalog = ocs_spatial::CrsCatalog::well_known();
        let plan = match ocs_spatial::plan_transformation(&catalog, source_epsg, target_epsg, &[]) {
            Ok(plan) => plan,
            Err(error) => {
                self.command_line
                    .push_error(format!("GISTRANSFORM: {error}").as_str());
                return;
            }
        };
        let point_count: usize = self.tabs[tab].gis_layers[index]
            .features
            .iter()
            .map(|feature| feature.geometry.point_count())
            .sum();
        let mut coordinates = Vec::with_capacity(point_count);
        for feature in &self.tabs[tab].gis_layers[index].features {
            let mut geometry = feature.geometry.clone();
            geometry.for_each_point_mut(|point| coordinates.push(*point));
        }
        let provenance = match ocs_spatial::transform_xy(&plan, &mut coordinates) {
            Ok(provenance) => provenance,
            Err(error) => {
                self.command_line
                    .push_error(format!("GISTRANSFORM: {error}").as_str());
                return;
            }
        };
        let mut transformed = coordinates.into_iter();
        for feature in &mut self.tabs[tab].gis_layers[index].features {
            feature.geometry.for_each_point_mut(|point| {
                *point = transformed.next().expect("point count is stable")
            });
        }
        self.tabs[tab].gis_layers[index].epsg = target_epsg;
        if let Some((project_path, project)) = self.tabs[tab].spatial_project.as_mut() {
            project.history.push(ocs_pointcloud::ProcessingHistoryEntry {
                id: format!("history-gis-transform-{}", project.history.len() + 1),
                created_unix_ms: project.updated_unix_ms,
                tool_id: "gis.transform".into(),
                inputs: vec![name.into()],
                outputs: vec![name.into()],
                parameters: serde_json::json!({"source_epsg": source_epsg, "target_epsg": target_epsg}),
                software_version: env!("CARGO_PKG_VERSION").into(),
                crs_transformations: vec![serde_json::to_string(&provenance).unwrap_or_default()],
                status: "completed".into(),
                detail: format!("transformed {point_count} vertices"),
            });
            if let Err(error) = project.save_atomic(project_path.clone()) {
                self.command_line.push_error(
                    format!("GISTRANSFORM: project provenance save failed: {error}").as_str(),
                );
                return;
            }
        }
        self.command_line.push_output(
            format!("GISTRANSFORM: transformed '{name}' from EPSG:{source_epsg} to EPSG:{target_epsg} ({point_count} vertices).").as_str(),
        );
    }

    fn default_gis_epsg(&self, tab: usize) -> u16 {
        self.tabs[tab]
            .spatial
            .drawing_crs
            .as_ref()
            .and_then(|crs| crs.as_crs_info().horizontal_epsg)
            .filter(|epsg| *epsg > 0)
            .unwrap_or(4326)
    }

    fn catalog_gis_source(&mut self, tab: usize, path: &Path, layer_names: &[String]) {
        let Some((project_path, project)) = self.tabs[tab].spatial_project.as_mut() else {
            return;
        };
        let file_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(stable_id)
            .unwrap_or_else(|| "features".into());
        for layer_name in layer_names {
            let id = format!("feature-{file_id}-{}", stable_id(layer_name));
            let mut source = match ocs_pointcloud::ProjectSource::local(
                &id,
                project_path.clone(),
                path,
                ocs_pointcloud::SourceKind::Feature,
            ) {
                Ok(source) => source,
                Err(error) => {
                    self.command_line
                        .push_error(format!("GISIMPORT: project catalog failed: {error}").as_str());
                    continue;
                }
            };
            source.name = layer_name.clone();
            source.metadata.insert(
                "layer_name".into(),
                serde_json::Value::String(layer_name.clone()),
            );
            if let Some(existing) = project.sources.iter_mut().find(|item| item.id == id) {
                *existing = source;
            } else {
                project.sources.push(source);
            }
        }
        if let Err(error) = project.save_atomic(project_path.clone()) {
            self.command_line
                .push_error(format!("GISIMPORT: project save failed: {error}").as_str());
        }
    }
}

fn load_layers(path: &Path, default_epsg: u16) -> Result<Vec<FeatureLayer>, String> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("geojson" | "json") => {
            let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
            let name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("features");
            FeatureLayer::from_geojson(&text, name, default_epsg).map(|layer| vec![layer])
        }
        Some("gpkg") => {
            let connection = open_geopackage(path)?;
            feature_tables(&connection)?
                .into_iter()
                .map(|table| read_layer(&connection, &table))
                .collect()
        }
        _ => Err("source must use .gpkg or .geojson".into()),
    }
}

fn stable_id(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

inventory::submit!(crate::command::CommandRegistration {
    names: &[
        "GISIMPORT",
        "GISLAYERS",
        "GISTOPOLOGY",
        "GISEXPORT",
        "GISTRANSFORM",
    ]
});
