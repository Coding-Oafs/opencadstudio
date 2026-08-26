//! Typed tool descriptors shared by UI, command, scripting, and plugins.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensityRequirement {
    #[default]
    Display,
    FullSource,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoBehavior {
    #[default]
    None,
    Transaction,
    DerivedOutput,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolRequirements {
    pub density: DensityRequirement,
    pub requires_crs: bool,
    pub requires_vertical_crs: bool,
    pub background: bool,
    pub cancellable: bool,
    pub checkpointable: bool,
    pub undo: UndoBehavior,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub requirements: ToolRequirements,
    pub api_version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub tool_id: String,
    pub parameters: Value,
    pub source: InvocationSource,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationSource {
    #[default]
    Ui,
    CommandLine,
    Rhai,
    Python,
    Plugin,
    Workflow,
}

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDescriptor>,
}

impl ToolRegistry {
    pub fn register(&mut self, descriptor: ToolDescriptor) -> Result<(), String> {
        validate_identifier(&descriptor.id)?;
        if descriptor.name.trim().is_empty() || descriptor.category.trim().is_empty() {
            return Err("tool name and category must not be empty".to_string());
        }
        if descriptor.api_version == 0 {
            return Err("tool api_version must be at least 1".to_string());
        }
        if self.tools.contains_key(&descriptor.id) {
            return Err(format!("tool '{}' is already registered", descriptor.id));
        }
        self.tools.insert(descriptor.id.clone(), descriptor);
        Ok(())
    }

    pub fn descriptor(&self, id: &str) -> Option<&ToolDescriptor> {
        self.tools.get(id)
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &ToolDescriptor> {
        self.tools.values()
    }

    pub fn search(&self, query: &str) -> Vec<&ToolDescriptor> {
        let query = query.trim().to_ascii_lowercase();
        self.tools
            .values()
            .filter(|tool| {
                query.is_empty()
                    || tool.id.to_ascii_lowercase().contains(&query)
                    || tool.name.to_ascii_lowercase().contains(&query)
                    || tool.category.to_ascii_lowercase().contains(&query)
                    || tool.description.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }
}

pub fn production_lidar_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    for descriptor in [
        tool(
            "lidar.select.full_density",
            "Full-density spatial selection",
            "LiDAR / Selection",
            true,
            UndoBehavior::Transaction,
        ),
        tool(
            "lidar.classify.pipeline",
            "Classification pipeline",
            "LiDAR / Classification",
            true,
            UndoBehavior::Transaction,
        ),
        tool(
            "lidar.surface.dtm",
            "Create DTM",
            "LiDAR / Surfaces",
            true,
            UndoBehavior::DerivedOutput,
        ),
        tool(
            "lidar.surface.dsm",
            "Create DSM",
            "LiDAR / Surfaces",
            true,
            UndoBehavior::DerivedOutput,
        ),
        tool(
            "lidar.surface.hillshade",
            "Create hillshade",
            "LiDAR / Surfaces",
            true,
            UndoBehavior::DerivedOutput,
        ),
        tool(
            "lidar.breakline.validate",
            "Validate breaklines",
            "LiDAR / Breaklines",
            false,
            UndoBehavior::None,
        ),
        tool(
            "lidar.import.e57",
            "Import E57",
            "LiDAR / Import",
            true,
            UndoBehavior::DerivedOutput,
        ),
        tool(
            "lidar.attach.copc",
            "Attach COPC",
            "LiDAR / Sources",
            true,
            UndoBehavior::None,
        ),
    ] {
        registry
            .register(descriptor)
            .expect("built-in tool ids are valid");
    }
    registry
}

fn tool(
    id: &str,
    name: &str,
    category: &str,
    full_density: bool,
    undo: UndoBehavior,
) -> ToolDescriptor {
    ToolDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        category: category.to_string(),
        description: format!("OpenCADStudio {name}"),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        requirements: ToolRequirements {
            density: if full_density {
                DensityRequirement::FullSource
            } else {
                DensityRequirement::Display
            },
            requires_crs: full_density,
            background: full_density,
            cancellable: full_density,
            checkpointable: full_density,
            undo,
            ..Default::default()
        },
        api_version: 1,
    }
}

fn validate_identifier(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.starts_with('.')
        || id.ends_with('.')
        || !id.bytes().all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'.' || value == b'_'
        })
    {
        return Err(format!("invalid stable tool identifier '{id}'"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_registry_exposes_full_density_contract() {
        let registry = production_lidar_tools();
        let tool = registry.descriptor("lidar.classify.pipeline").unwrap();
        assert_eq!(DensityRequirement::FullSource, tool.requirements.density);
        assert!(tool.requirements.cancellable);
        assert!(tool.requirements.checkpointable);
        assert!(!registry.search("surface").is_empty());
    }
}
