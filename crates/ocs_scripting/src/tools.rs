//! Script tool manifests, interpreter health, and downloaded-script trust.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptToolManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub script: PathBuf,
    pub api_version: u32,
    pub parameters: Value,
    pub background: bool,
    pub cancellable: bool,
}

impl ScriptToolManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let mut manifest: Self = serde_json::from_slice(
            &fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
        )
        .map_err(|error| format!("invalid script tool manifest: {error}"))?;
        if manifest.script.is_relative() {
            manifest.script = path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&manifest.script);
        }
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || !self.id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
            })
            || self.name.trim().is_empty()
            || self.api_version == 0
        {
            return Err("script tool requires a stable lowercase id, name, and api version".into());
        }
        if !self.script.is_file()
            || self.script.extension().and_then(|value| value.to_str()) != Some("py")
        {
            return Err(format!(
                "script tool entry point is not a Python file: {}",
                self.script.display()
            ));
        }
        if !self.parameters.is_object() {
            return Err("script tool parameters must be a JSON Schema object".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentHealth {
    pub interpreter: PathBuf,
    pub available: bool,
    pub version: Option<String>,
    pub isolated: bool,
    pub ocs_package_available: bool,
    pub detail: String,
}

impl EnvironmentHealth {
    pub fn inspect(interpreter: impl Into<PathBuf>, package_root: &Path) -> Self {
        let interpreter = interpreter.into();
        let output = Command::new(&interpreter).arg("-V").output();
        let (available, version, detail) = match output {
            Ok(output) if output.status.success() => {
                let text = if output.stdout.is_empty() {
                    &output.stderr
                } else {
                    &output.stdout
                };
                (
                    true,
                    Some(String::from_utf8_lossy(text).trim().to_string()),
                    "interpreter is healthy".into(),
                )
            }
            Ok(output) => (
                false,
                None,
                format!("interpreter exited with {}", output.status),
            ),
            Err(error) => (false, None, error.to_string()),
        };
        let isolated = interpreter
            .parent()
            .is_some_and(|parent| parent.join("pyvenv.cfg").is_file())
            || interpreter
                .parent()
                .and_then(Path::parent)
                .is_some_and(|parent| parent.join("pyvenv.cfg").is_file());
        let ocs_package_available = package_root.join("ocs").join("__init__.py").is_file();
        Self {
            interpreter,
            available,
            version,
            isolated,
            ocs_package_available,
            detail,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptTrustStore {
    /// Canonical path -> approved SHA-256 content digest.
    pub approved: BTreeMap<PathBuf, String>,
}

impl ScriptTrustStore {
    pub fn digest(path: impl AsRef<Path>) -> Result<String, String> {
        let bytes = fs::read(path.as_ref()).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn approve(&mut self, path: impl AsRef<Path>) -> Result<String, String> {
        let path = fs::canonicalize(path.as_ref()).map_err(|error| error.to_string())?;
        let digest = Self::digest(&path)?;
        self.approved.insert(path, digest.clone());
        Ok(digest)
    }

    pub fn is_approved(&self, path: impl AsRef<Path>) -> Result<bool, String> {
        let path = fs::canonicalize(path.as_ref()).map_err(|error| error.to_string())?;
        let digest = Self::digest(&path)?;
        Ok(self.approved.get(&path) == Some(&digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_resolve_entry_points_and_trust_detects_changes() {
        let directory =
            std::env::temp_dir().join(format!("ocs-script-tool-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("tool.py"), "print('ok')\n").unwrap();
        fs::write(
            directory.join("tool.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": "company.validate", "name": "Validate", "description": "Company validation",
                "script": "tool.py", "api_version": 1, "parameters": {"type": "object"},
                "background": true, "cancellable": true
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = ScriptToolManifest::load(directory.join("tool.json")).unwrap();
        assert!(manifest.script.is_absolute());
        let mut trust = ScriptTrustStore::default();
        trust.approve(&manifest.script).unwrap();
        assert!(trust.is_approved(&manifest.script).unwrap());
        fs::write(&manifest.script, "print('changed')\n").unwrap();
        assert!(!trust.is_approved(&manifest.script).unwrap());
        let _ = fs::remove_dir_all(directory);
    }
}
