use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveLock {
    pid: i32,
    started_at: i64,
    host: String,
    #[serde(default)]
    process_start_ticks: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct SweepReport {
    pub stale_lock_detected: bool,
    pub stale_lock_removed: bool,
    pub active_lock_detected: bool,
    pub stage_entries_removed: usize,
    pub tx_entries_removed: usize,
}

#[derive(Debug)]
pub struct RuntimeLockGuard {
    lock_path: PathBuf,
}

impl Drop for RuntimeLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

pub fn sweep_runtime_state(workspace_root: &Path, repair: bool) -> Result<SweepReport> {
    let mut report = SweepReport::default();
    let broski_root = workspace_root.join(".broski");
    let runtime_root = broski_root.join("runtime");
    let lock_path = runtime_root.join("active.lock");

    fs::create_dir_all(&runtime_root)
        .with_context(|| format!("creating runtime root '{}'", runtime_root.display()))?;

    if lock_path.exists() {
        let lock = read_active_lock(&lock_path)?;
        if is_lock_process_active(&lock) {
            report.active_lock_detected = true;
            return Ok(report);
        }

        report.stale_lock_detected = true;
        if repair {
            fs::remove_file(&lock_path)
                .with_context(|| format!("removing stale lock file '{}'", lock_path.display()))?;
            report.stale_lock_removed = true;
        }
    }

    if repair {
        report.stage_entries_removed = purge_children(&broski_root.join("stage"))?;
        report.tx_entries_removed = purge_children(&broski_root.join("tx"))?;
    }

    Ok(report)
}

pub fn acquire_runtime_lock(workspace_root: &Path) -> Result<RuntimeLockGuard> {
    let runtime_root = workspace_root.join(".broski/runtime");
    let lock_path = runtime_root.join("active.lock");
    fs::create_dir_all(&runtime_root)
        .with_context(|| format!("creating runtime root '{}'", runtime_root.display()))?;

    if lock_path.exists() {
        let lock = read_active_lock(&lock_path)?;
        if is_lock_process_active(&lock) {
            return Err(anyhow!("another Broski execution is active (pid={})", lock.pid));
        }
        fs::remove_file(&lock_path)
            .with_context(|| format!("removing stale lock file '{}'", lock_path.display()))?;
    }

    let payload = ActiveLock {
        pid: std::process::id() as i32,
        started_at: unix_timestamp_secs(),
        host: hostname(),
        process_start_ticks: process_start_ticks(std::process::id() as i32),
    };

    let serialized = serde_json::to_string_pretty(&payload).context("serializing active lock")?;

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options
        .open(&lock_path)
        .with_context(|| format!("creating lock file '{}'", lock_path.display()))?;

    use std::io::Write;
    file.write_all(serialized.as_bytes())
        .with_context(|| format!("writing lock file '{}'", lock_path.display()))?;

    Ok(RuntimeLockGuard { lock_path })
}

fn read_active_lock(path: &Path) -> Result<ActiveLock> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading active lock file '{}'", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("parsing active lock file '{}'", path.display()))
}

fn purge_children(root: &Path) -> Result<usize> {
    fs::create_dir_all(root).with_context(|| format!("creating '{}'", root.display()))?;
    let mut removed = 0usize;

    for entry in fs::read_dir(root).with_context(|| format!("reading '{}'", root.display()))? {
        let entry = entry.with_context(|| format!("reading entry in '{}'", root.display()))?;
        let path = entry.path();

        if path.is_dir() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("removing stale dir '{}'", path.display()))?;
        } else {
            fs::remove_file(&path)
                .with_context(|| format!("removing stale file '{}'", path.display()))?;
        }
        removed += 1;
    }

    Ok(removed)
}

fn is_lock_process_active(lock: &ActiveLock) -> bool {
    if !is_pid_alive(lock.pid) {
        return false;
    }
    if let Some(expected_start) = lock.process_start_ticks {
        if let Some(actual_start) = process_start_ticks(lock.pid) {
            return actual_start == expected_start;
        }
    }
    true
}

fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn hostname() -> String {
    if let Ok(host) = std::env::var("HOSTNAME") {
        if !host.trim().is_empty() {
            return host;
        }
    }
    if let Ok(host) = std::env::var("COMPUTERNAME") {
        if !host.trim().is_empty() {
            return host;
        }
    }
    "unknown-host".to_string()
}

#[cfg(target_os = "linux")]
fn process_start_ticks(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }
    let path = format!("/proc/{pid}/stat");
    let content = fs::read_to_string(path).ok()?;
    parse_linux_proc_stat_start_ticks(&content)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_stat_start_ticks(stat: &str) -> Option<u64> {
    // procfs format is: pid (comm) state ppid ... starttime ...
    let close_paren = stat.rfind(')')?;
    let remainder = stat.get(close_paren + 2..)?;
    let fields: Vec<&str> = remainder.split_whitespace().collect();
    fields.get(19)?.parse::<u64>().ok()
}

#[cfg(not(target_os = "linux"))]
fn process_start_ticks(_pid: i32) -> Option<u64> {
    None
}

#[cfg(unix)]
fn is_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }

    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return true;
    }

    let errno = std::io::Error::last_os_error();
    matches!(errno.raw_os_error(), Some(code) if code == libc::EPERM)
}

#[cfg(not(unix))]
fn is_pid_alive(_pid: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_removes_stale_lock_and_orphans() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workspace = tmp.path();
        let runtime = workspace.join(".broski/runtime");
        let stage = workspace.join(".broski/stage");
        let tx = workspace.join(".broski/tx");

        fs::create_dir_all(&runtime).expect("create runtime");
        fs::create_dir_all(stage.join("orphan")).expect("create stage orphan");
        fs::create_dir_all(tx.join("orphan")).expect("create tx orphan");

        let stale = ActiveLock {
            pid: 999_999,
            started_at: 1,
            host: "test".to_string(),
            process_start_ticks: None,
        };
        fs::write(
            runtime.join("active.lock"),
            serde_json::to_string(&stale).expect("serialize lock"),
        )
        .expect("write lock");

        let report = sweep_runtime_state(workspace, true).expect("sweep state");
        assert!(report.stale_lock_detected);
        assert!(report.stale_lock_removed);
        assert_eq!(report.stage_entries_removed, 1);
        assert_eq!(report.tx_entries_removed, 1);
    }

    #[test]
    fn acquire_and_release_runtime_lock() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workspace = tmp.path();
        let lock_path = workspace.join(".broski/runtime/active.lock");

        {
            let _guard = acquire_runtime_lock(workspace).expect("acquire lock");
            assert!(lock_path.exists());
        }

        assert!(!lock_path.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sweep_marks_lock_stale_when_process_start_does_not_match() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workspace = tmp.path();
        let runtime = workspace.join(".broski/runtime");
        fs::create_dir_all(&runtime).expect("create runtime");

        let pid = std::process::id() as i32;
        let start = process_start_ticks(pid).expect("process start");
        let stale = ActiveLock {
            pid,
            started_at: 1,
            host: "test".to_string(),
            process_start_ticks: Some(start.saturating_add(1)),
        };
        fs::write(
            runtime.join("active.lock"),
            serde_json::to_string(&stale).expect("serialize lock"),
        )
        .expect("write lock");

        let report = sweep_runtime_state(workspace, false).expect("sweep state");
        assert!(report.stale_lock_detected);
        assert!(!report.active_lock_detected);
    }
}
