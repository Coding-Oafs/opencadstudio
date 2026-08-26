//! Durable point-cloud job records and restart-safe output reservations.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    #[default]
    Queued,
    Running,
    Paused,
    Cancelling,
    Cancelled,
    Failed,
    Completed,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Failed | Self::Completed)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobCheckpoint {
    pub created_unix_ms: u64,
    pub completed: u64,
    pub total: u64,
    pub state: Value,
}

impl Default for JobCheckpoint {
    fn default() -> Self {
        Self {
            created_unix_ms: unix_ms(),
            completed: 0,
            total: 0,
            state: Value::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobRecord {
    pub id: String,
    pub tool_id: String,
    pub display_name: String,
    pub status: JobStatus,
    pub created_unix_ms: u64,
    pub started_unix_ms: Option<u64>,
    pub finished_unix_ms: Option<u64>,
    pub attempts: u32,
    pub completed: u64,
    pub total: u64,
    pub current_stage: String,
    pub parameters: Value,
    pub inputs: Vec<String>,
    pub outputs: Vec<PathBuf>,
    pub checkpoints: Vec<JobCheckpoint>,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

impl Default for JobRecord {
    fn default() -> Self {
        let now = unix_ms();
        Self {
            id: format!("job-{now}"),
            tool_id: String::new(),
            display_name: String::new(),
            status: JobStatus::Queued,
            created_unix_ms: now,
            started_unix_ms: None,
            finished_unix_ms: None,
            attempts: 0,
            completed: 0,
            total: 0,
            current_stage: "Queued".to_string(),
            parameters: Value::Null,
            inputs: Vec::new(),
            outputs: Vec::new(),
            checkpoints: Vec::new(),
            logs: Vec::new(),
            error: None,
        }
    }
}

impl JobRecord {
    pub fn new(tool_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            tool_id: tool_id.into(),
            display_name: display_name.into(),
            ..Default::default()
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if !matches!(self.status, JobStatus::Queued | JobStatus::Paused) {
            return Err(format!(
                "job '{}' cannot start from {:?}",
                self.id, self.status
            ));
        }
        self.status = JobStatus::Running;
        self.started_unix_ms.get_or_insert_with(unix_ms);
        self.attempts = self.attempts.saturating_add(1);
        self.finished_unix_ms = None;
        self.error = None;
        self.current_stage = "Running".to_string();
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if self.status != JobStatus::Running {
            return Err(format!("job '{}' is not running", self.id));
        }
        self.status = JobStatus::Paused;
        self.current_stage = "Paused".to_string();
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<(), String> {
        if self.status.is_terminal() {
            return Err(format!("job '{}' has already finished", self.id));
        }
        self.status = JobStatus::Cancelling;
        self.current_stage = "Cancelling".to_string();
        Ok(())
    }

    pub fn finish_cancelled(&mut self) {
        self.status = JobStatus::Cancelled;
        self.finished_unix_ms = Some(unix_ms());
        self.current_stage = "Cancelled".to_string();
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.logs.push(format!("ERROR: {error}"));
        self.error = Some(error);
        self.status = JobStatus::Failed;
        self.finished_unix_ms = Some(unix_ms());
        self.current_stage = "Failed".to_string();
    }

    pub fn complete(&mut self) {
        self.completed = self.total.max(self.completed);
        self.status = JobStatus::Completed;
        self.finished_unix_ms = Some(unix_ms());
        self.current_stage = "Completed".to_string();
        self.error = None;
    }

    pub fn retry(&mut self) -> Result<(), String> {
        if !matches!(self.status, JobStatus::Failed | JobStatus::Cancelled) {
            return Err(format!("job '{}' is not retryable", self.id));
        }
        self.status = JobStatus::Queued;
        self.finished_unix_ms = None;
        self.error = None;
        self.current_stage = "Queued for retry".to_string();
        Ok(())
    }

    pub fn set_progress(&mut self, completed: u64, total: u64, stage: impl Into<String>) {
        self.completed = completed.min(total);
        self.total = total;
        self.current_stage = stage.into();
    }

    pub fn checkpoint(&mut self, state: Value) {
        self.checkpoints.push(JobCheckpoint {
            created_unix_ms: unix_ms(),
            completed: self.completed,
            total: self.total,
            state,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobQueue {
    pub max_running: usize,
    pub jobs: Vec<JobRecord>,
}

impl Default for JobQueue {
    fn default() -> Self {
        Self {
            max_running: 1,
            jobs: Vec::new(),
        }
    }
}

impl JobQueue {
    pub fn enqueue(&mut self, job: JobRecord) -> Result<(), String> {
        if self.jobs.iter().any(|existing| existing.id == job.id) {
            return Err(format!("duplicate job id '{}'", job.id));
        }
        self.jobs.push(job);
        Ok(())
    }

    pub fn next_runnable(&self) -> Option<&JobRecord> {
        let running = self
            .jobs
            .iter()
            .filter(|job| job.status == JobStatus::Running)
            .count();
        (running < self.max_running.max(1))
            .then(|| self.jobs.iter().find(|job| job.status == JobStatus::Queued))
            .flatten()
    }

    pub fn job_mut(&mut self, id: &str) -> Option<&mut JobRecord> {
        self.jobs.iter_mut().find(|job| job.id == id)
    }

    pub fn recover_interrupted(&mut self) -> usize {
        let mut recovered = 0;
        for job in &mut self.jobs {
            if matches!(job.status, JobStatus::Running | JobStatus::Cancelling) {
                job.status = JobStatus::Paused;
                job.current_stage = "Paused after application restart".to_string();
                job.logs
                    .push("Recovered interrupted job from project state".to_string());
                recovered += 1;
            }
        }
        recovered
    }

    pub fn status_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for job in &self.jobs {
            *counts
                .entry(format!("{:?}", job.status).to_ascii_lowercase())
                .or_default() += 1;
        }
        counts
    }
}

/// Owns an adjacent partial output and an exclusive lock. The final file is
/// published only by `commit`; otherwise both temporary files are cleaned up.
#[derive(Debug)]
pub struct ProtectedOutput {
    final_path: PathBuf,
    partial_path: PathBuf,
    lock_path: PathBuf,
    committed: bool,
}

impl ProtectedOutput {
    pub fn reserve(path: impl AsRef<Path>, overwrite: bool) -> io::Result<Self> {
        let final_path = path.as_ref().to_path_buf();
        if final_path.exists() && !overwrite {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("output already exists: {}", final_path.display()),
            ));
        }
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Keep the output extension on the partial file. Format-selecting writers
        // (notably LAS/LAZ) inspect the final extension when choosing compression.
        let partial_path = partial_output_path(&final_path);
        let lock_path = append_suffix(&final_path, ".ocslock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        if partial_path.exists() {
            fs::remove_file(&partial_path)?;
        }
        Ok(Self {
            final_path,
            partial_path,
            lock_path,
            committed: false,
        })
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }

    pub fn commit(mut self) -> io::Result<PathBuf> {
        if !self.partial_path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("partial output is missing: {}", self.partial_path.display()),
            ));
        }
        if self.final_path.exists() {
            fs::remove_file(&self.final_path)?;
        }
        fs::rename(&self.partial_path, &self.final_path)?;
        fs::remove_file(&self.lock_path)?;
        self.committed = true;
        Ok(self.final_path.clone())
    }
}

impl Drop for ProtectedOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.partial_path);
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn partial_output_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let extension = path.extension().and_then(|value| value.to_str());
    let name = match extension {
        Some(extension) => format!(".{stem}.ocs-partial.{extension}"),
        None => format!(".{stem}.ocs-partial"),
    };
    path.with_file_name(name)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_jobs_recover_paused_and_can_retry() {
        let mut queue = JobQueue::default();
        let mut job = JobRecord::new("lidar.ground", "Ground classification");
        job.start().unwrap();
        queue.enqueue(job).unwrap();
        assert_eq!(1, queue.recover_interrupted());
        assert_eq!(JobStatus::Paused, queue.jobs[0].status);
        queue.jobs[0].start().unwrap();
        queue.jobs[0].fail("fixture");
        queue.jobs[0].retry().unwrap();
        assert_eq!(JobStatus::Queued, queue.jobs[0].status);
    }

    #[test]
    fn protected_output_publishes_only_after_commit() {
        let root = std::env::temp_dir().join(format!("ocs-output-{}", unix_ms()));
        fs::create_dir_all(&root).unwrap();
        let output = root.join("surface.asc");
        {
            let reservation = ProtectedOutput::reserve(&output, false).unwrap();
            fs::write(reservation.partial_path(), b"grid").unwrap();
        }
        assert!(!output.exists());
        let reservation = ProtectedOutput::reserve(&output, false).unwrap();
        assert_eq!(
            Some("asc"),
            reservation
                .partial_path()
                .extension()
                .and_then(|v| v.to_str())
        );
        fs::write(reservation.partial_path(), b"grid").unwrap();
        reservation.commit().unwrap();
        assert_eq!(b"grid", fs::read(&output).unwrap().as_slice());
        fs::remove_dir_all(root).ok();
    }
}
