use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedArtifact {
    pub relative_path: String,
    pub object_hash: String,
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub task_name: String,
    pub fingerprint: String,
    pub manifest: BTreeMap<String, String>,
    pub artifacts: Vec<CachedArtifact>,
    pub stdout: String,
    pub stderr: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct PruneReport {
    pub removed_objects: usize,
    pub removed_bytes: u64,
    pub remaining_bytes: u64,
}

pub trait ArtifactStore: Send + Sync {
    fn fetch_execution(
        &self,
        task_name: &str,
        fingerprint: &str,
    ) -> Result<Option<ExecutionRecord>>;
    fn fetch_latest_execution(&self, task_name: &str) -> Result<Option<ExecutionRecord>>;
    fn save_execution(&self, record: &ExecutionRecord) -> Result<()>;
    fn store_artifacts(&self, workspace: &Path, outputs: &[PathBuf])
        -> Result<Vec<CachedArtifact>>;
    fn restore_artifacts(&self, workspace: &Path, artifacts: &[CachedArtifact]) -> Result<()>;
    fn prune(&self, max_size_mb: u64) -> Result<PruneReport>;
}
