//! Bundled PROJ worker integration for checksum-pinned grid transformations.
//!
//! The desktop release ships `ocs-proj-worker` (PyInstaller + pyproj/libproj)
//! and a small, explicit `proj-data` directory. Keeping native PROJ in a
//! worker avoids loading a second C/C++ runtime into the CAD process while
//! preserving PROJ's authoritative grid interpolation and pipeline semantics.

use crate::SpatialError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const CHUNK_POINTS: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjBackendHealth {
    pub worker: PathBuf,
    pub data_dir: PathBuf,
    pub worker_available: bool,
    pub data_dir_available: bool,
}

#[derive(Clone, Debug)]
pub struct ProjGridBackend {
    worker: PathBuf,
    data_dir: PathBuf,
}

impl ProjGridBackend {
    pub fn discover() -> Result<Self, SpatialError> {
        let executable = std::env::current_exe().map_err(|error| {
            SpatialError::VerticalNotExecuted(format!("cannot locate application: {error}"))
        })?;
        let install_dir = executable.parent().unwrap_or_else(|| Path::new("."));
        let worker = std::env::var_os("OCS_PROJ_WORKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                install_dir.join(if cfg!(windows) {
                    "ocs-proj-worker.exe"
                } else {
                    "ocs-proj-worker"
                })
            });
        let data_dir = std::env::var_os("OCS_PROJ_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| install_dir.join("proj-data"));
        let backend = Self { worker, data_dir };
        let health = backend.health();
        if !health.worker_available || !health.data_dir_available {
            return Err(SpatialError::VerticalNotExecuted(format!(
                "bundled PROJ backend is incomplete (worker: {}, data: {})",
                health.worker.display(),
                health.data_dir.display()
            )));
        }
        Ok(backend)
    }

    pub fn new(worker: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            worker: worker.into(),
            data_dir: data_dir.into(),
        }
    }

    pub fn health(&self) -> ProjBackendHealth {
        ProjBackendHealth {
            worker: self.worker.clone(),
            data_dir: self.data_dir.clone(),
            worker_available: self.worker.is_file(),
            data_dir_available: self.data_dir.is_dir(),
        }
    }

    pub fn validate_grid(
        &self,
        name: &str,
        expected_sha256: &str,
    ) -> Result<PathBuf, SpatialError> {
        let relative = Path::new(name);
        if relative.is_absolute()
            || relative.components().count() != 1
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(SpatialError::VerticalNotExecuted(
                "PROJ grid name must be a single relative filename".into(),
            ));
        }
        let path = self.data_dir.join(relative);
        let mut file = fs::File::open(&path).map_err(|error| {
            SpatialError::VerticalNotExecuted(format!(
                "required PROJ grid '{}' is unavailable: {error}",
                path.display()
            ))
        })?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| {
                SpatialError::VerticalNotExecuted(format!("cannot checksum PROJ grid: {error}"))
            })?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let actual = format!("{:x}", digest.finalize());
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            return Err(SpatialError::VerticalNotExecuted(format!(
                "PROJ grid '{}' checksum mismatch (expected {}, found {})",
                name, expected_sha256, actual
            )));
        }
        Ok(path)
    }

    pub fn transform_vertical_grid(
        &self,
        grid_name: &str,
        expected_sha256: &str,
        inverse: bool,
        points_lon_lat_z: &mut [[f64; 3]],
    ) -> Result<(), SpatialError> {
        self.validate_grid(grid_name, expected_sha256)?;
        if points_lon_lat_z.is_empty() {
            return Ok(());
        }
        let inverse_step = if inverse { "+inv " } else { "" };
        let pipeline = format!(
            "+proj=pipeline +step +proj=unitconvert +xy_in=deg +xy_out=rad \
             +step {inverse_step}+proj=vgridshift +grids={grid_name} +multiplier=1 \
             +step +proj=unitconvert +xy_in=rad +xy_out=deg"
        );
        let mut child = Command::new(&self.worker)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                SpatialError::VerticalNotExecuted(format!(
                    "cannot start bundled PROJ worker '{}': {error}",
                    self.worker.display()
                ))
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            SpatialError::VerticalNotExecuted("PROJ worker stdin is unavailable".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            SpatialError::VerticalNotExecuted("PROJ worker stdout is unavailable".into())
        })?;
        let mut writer = BufWriter::new(stdin);
        let mut reader = BufReader::new(stdout);
        let mut response_line = String::new();
        for chunk in points_lon_lat_z.chunks_mut(CHUNK_POINTS) {
            let request = WorkerRequest {
                pipeline: &pipeline,
                data_dir: &self.data_dir,
                points: chunk,
            };
            serde_json::to_writer(&mut writer, &request).map_err(worker_data_error)?;
            writer.write_all(b"\n").map_err(worker_io_error)?;
            writer.flush().map_err(worker_io_error)?;
            response_line.clear();
            if reader
                .read_line(&mut response_line)
                .map_err(worker_io_error)?
                == 0
            {
                let _ = child.kill();
                return Err(SpatialError::VerticalNotExecuted(
                    "PROJ worker closed its output".into(),
                ));
            }
            let response: WorkerResponse =
                serde_json::from_str(&response_line).map_err(worker_data_error)?;
            if let Some(error) = response.error {
                let _ = child.kill();
                return Err(SpatialError::VerticalNotExecuted(format!(
                    "PROJ pipeline failed: {error}"
                )));
            }
            if response.points.len() != chunk.len()
                || response
                    .points
                    .iter()
                    .flatten()
                    .any(|coordinate| !coordinate.is_finite())
            {
                let _ = child.kill();
                return Err(SpatialError::VerticalNotExecuted(
                    "PROJ worker returned invalid coordinates".into(),
                ));
            }
            chunk.copy_from_slice(&response.points);
        }
        drop(writer);
        let status = child.wait().map_err(worker_io_error)?;
        if !status.success() {
            return Err(SpatialError::VerticalNotExecuted(format!(
                "PROJ worker exited with {status}"
            )));
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct WorkerRequest<'a> {
    pipeline: &'a str,
    data_dir: &'a Path,
    points: &'a [[f64; 3]],
}

#[derive(Deserialize)]
struct WorkerResponse {
    #[serde(default)]
    points: Vec<[f64; 3]>,
    #[serde(default)]
    error: Option<String>,
}

fn worker_io_error(error: std::io::Error) -> SpatialError {
    SpatialError::VerticalNotExecuted(format!("PROJ worker I/O failed: {error}"))
}

fn worker_data_error(error: serde_json::Error) -> SpatialError {
    SpatialError::VerticalNotExecuted(format!("PROJ worker protocol failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_validation_is_checksum_pinned_and_rejects_traversal() {
        let root = std::env::temp_dir().join(format!("ocs-proj-grid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("grid.tif"), b"grid fixture").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"grid fixture"));
        let backend = ProjGridBackend::new(root.join("worker"), &root);
        assert!(backend.validate_grid("grid.tif", &digest).is_ok());
        assert!(backend.validate_grid("grid.tif", &"0".repeat(64)).is_err());
        assert!(backend.validate_grid("../grid.tif", &digest).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
