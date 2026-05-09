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
    /// Total wall-clock duration of the run, in milliseconds. Zero for
    /// records written before the field existed (the `cache.db` migration
    /// backfills the column with `0`).
    #[serde(default)]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PruneReport {
    pub removed_objects: usize,
    pub removed_bytes: u64,
    pub remaining_bytes: u64,
}

/// Snapshot of the cache's on-disk footprint. Surfaced by the TUI launcher
/// as "N objects · X MB" without needing a prune. Walking the objects dir
/// is O(N) on cache size, so callers fetch this once at startup and refresh
/// after `/cache prune` or `/refresh`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub object_count: usize,
    pub total_bytes: u64,
}

pub trait ArtifactStore: Send + Sync {
    fn fetch_execution(
        &self,
        task_name: &str,
        fingerprint: &str,
    ) -> Result<Option<ExecutionRecord>>;
    fn fetch_latest_execution(&self, task_name: &str) -> Result<Option<ExecutionRecord>>;
    /// Return up to `limit` records, newest first. When `task` is `Some`,
    /// scoped to that task; when `None`, returns the most recent record per
    /// known task (one row per task). Used by `broski history`.
    fn fetch_history(&self, task: Option<&str>, limit: usize) -> Result<Vec<ExecutionRecord>>;
    fn save_execution(&self, record: &ExecutionRecord) -> Result<()>;
    fn store_artifacts(&self, workspace: &Path, outputs: &[PathBuf])
        -> Result<Vec<CachedArtifact>>;
    fn restore_artifacts(&self, workspace: &Path, artifacts: &[CachedArtifact]) -> Result<()>;
    fn prune(&self, max_size_mb: u64) -> Result<PruneReport>;
    /// Total cache footprint and object count. Defaults to zeros — backends
    /// that can compute it cheaply should override.
    fn stats(&self) -> Result<StoreStats> {
        Ok(StoreStats::default())
    }
}
