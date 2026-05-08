use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use broski_store::{ArtifactKind, ArtifactStore, CachedArtifact, ExecutionRecord, PruneReport};
use rusqlite::{params, Connection};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct LocalArtifactStore {
    root: PathBuf,
    objects_dir: PathBuf,
    db_path: PathBuf,
}

impl LocalArtifactStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let objects_dir = root.join("objects");
        let db_path = root.join("metadata.sqlite3");

        fs::create_dir_all(&objects_dir)
            .with_context(|| format!("creating cache objects dir at {}", objects_dir.display()))?;

        let store = Self { root, objects_dir, db_path };
        store.init_db()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn init_db(&self) -> Result<()> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("opening sqlite db {}", self.db_path.display()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS executions (
                task_name TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                manifest_json TEXT NOT NULL DEFAULT '{}',
                artifacts_json TEXT NOT NULL,
                stdout TEXT NOT NULL,
                stderr TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(task_name, fingerprint)
            );
            CREATE INDEX IF NOT EXISTS idx_executions_task_created_at
            ON executions(task_name, created_at DESC);
            ",
        )
        .context("initializing sqlite schema")?;
        ensure_manifest_column(&conn)?;
        ensure_duration_column(&conn)?;
        Ok(())
    }

    fn connection(&self) -> Result<Connection> {
        Connection::open(&self.db_path)
            .with_context(|| format!("opening sqlite db {}", self.db_path.display()))
    }
}

impl ArtifactStore for LocalArtifactStore {
    fn fetch_execution(
        &self,
        task_name: &str,
        fingerprint: &str,
    ) -> Result<Option<ExecutionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT manifest_json, artifacts_json, stdout, stderr, created_at, duration_ms
                FROM executions WHERE task_name = ?1 AND fingerprint = ?2",
            )
            .context("preparing select execution statement")?;

        let mut rows =
            stmt.query(params![task_name, fingerprint]).context("querying execution record")?;

        if let Some(row) = rows.next().context("reading execution row")? {
            let manifest_json: String = row.get(0).context("reading manifest_json")?;
            let manifest: BTreeMap<String, String> =
                serde_json::from_str(&manifest_json).context("deserializing manifest_json")?;
            let artifacts_json: String = row.get(1).context("reading artifacts_json")?;
            let artifacts: Vec<CachedArtifact> =
                serde_json::from_str(&artifacts_json).context("deserializing artifacts_json")?;
            let stdout: String = row.get(2).context("reading stdout")?;
            let stderr: String = row.get(3).context("reading stderr")?;
            let created_at: i64 = row.get(4).context("reading created_at")?;
            let duration_ms: i64 = row.get(5).context("reading duration_ms")?;
            Ok(Some(ExecutionRecord {
                task_name: task_name.to_owned(),
                fingerprint: fingerprint.to_owned(),
                manifest,
                artifacts,
                stdout,
                stderr,
                created_at,
                duration_ms: duration_ms.max(0) as u64,
            }))
        } else {
            Ok(None)
        }
    }

    fn fetch_latest_execution(&self, task_name: &str) -> Result<Option<ExecutionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT fingerprint, manifest_json, artifacts_json, stdout, stderr, created_at, duration_ms
                 FROM executions WHERE task_name = ?1
                 ORDER BY created_at DESC LIMIT 1",
            )
            .context("preparing select latest execution statement")?;

        let mut rows =
            stmt.query(params![task_name]).context("querying latest execution record")?;

        if let Some(row) = rows.next().context("reading latest execution row")? {
            let fingerprint: String = row.get(0).context("reading latest fingerprint")?;
            let manifest_json: String = row.get(1).context("reading latest manifest_json")?;
            let manifest: BTreeMap<String, String> = serde_json::from_str(&manifest_json)
                .context("deserializing latest manifest_json")?;
            let artifacts_json: String = row.get(2).context("reading latest artifacts_json")?;
            let artifacts: Vec<CachedArtifact> = serde_json::from_str(&artifacts_json)
                .context("deserializing latest artifacts_json")?;
            let stdout: String = row.get(3).context("reading latest stdout")?;
            let stderr: String = row.get(4).context("reading latest stderr")?;
            let created_at: i64 = row.get(5).context("reading latest created_at")?;
            let duration_ms: i64 = row.get(6).context("reading latest duration_ms")?;

            Ok(Some(ExecutionRecord {
                task_name: task_name.to_owned(),
                fingerprint,
                manifest,
                artifacts,
                stdout,
                stderr,
                created_at,
                duration_ms: duration_ms.max(0) as u64,
            }))
        } else {
            Ok(None)
        }
    }

    fn save_execution(&self, record: &ExecutionRecord) -> Result<()> {
        let conn = self.connection()?;
        let manifest_json = serde_json::to_string(&record.manifest)
            .context("serializing manifest json for sqlite")?;
        let artifacts_json = serde_json::to_string(&record.artifacts)
            .context("serializing artifacts json for sqlite")?;
        let duration_signed: i64 = record.duration_ms.try_into().unwrap_or(i64::MAX);
        conn.execute(
            "INSERT OR REPLACE INTO executions
            (task_name, fingerprint, manifest_json, artifacts_json, stdout, stderr, created_at, duration_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.task_name,
                record.fingerprint,
                manifest_json,
                artifacts_json,
                record.stdout,
                record.stderr,
                record.created_at,
                duration_signed,
            ],
        )
        .context("writing execution record")?;
        Ok(())
    }

    fn fetch_history(
        &self,
        task: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ExecutionRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.connection()?;
        let cap_i64: i64 = limit.try_into().unwrap_or(i64::MAX);

        let (sql, has_task_filter) = match task {
            Some(_) => (
                "SELECT task_name, fingerprint, manifest_json, artifacts_json, stdout, stderr, created_at, duration_ms
                 FROM executions
                 WHERE task_name = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2"
                    .to_string(),
                true,
            ),
            None => (
                // One row per task: the most recent execution per task_name.
                "SELECT e.task_name, e.fingerprint, e.manifest_json, e.artifacts_json,
                        e.stdout, e.stderr, e.created_at, e.duration_ms
                 FROM executions e
                 INNER JOIN (
                     SELECT task_name, MAX(created_at) AS max_created
                     FROM executions
                     GROUP BY task_name
                 ) latest
                 ON latest.task_name = e.task_name AND latest.max_created = e.created_at
                 ORDER BY e.created_at DESC
                 LIMIT ?1"
                    .to_string(),
                false,
            ),
        };

        let mut stmt = conn.prepare(&sql).context("preparing history query")?;
        let mut rows = if has_task_filter {
            stmt.query(params![task.unwrap_or(""), cap_i64])
                .context("querying scoped history")?
        } else {
            stmt.query(params![cap_i64]).context("querying global history")?
        };

        let mut out = Vec::new();
        while let Some(row) = rows.next().context("reading history row")? {
            let task_name: String = row.get(0).context("reading history task_name")?;
            let fingerprint: String = row.get(1).context("reading history fingerprint")?;
            let manifest_json: String = row.get(2).context("reading history manifest_json")?;
            let manifest: BTreeMap<String, String> = serde_json::from_str(&manifest_json)
                .context("deserializing history manifest_json")?;
            let artifacts_json: String = row.get(3).context("reading history artifacts_json")?;
            let artifacts: Vec<CachedArtifact> = serde_json::from_str(&artifacts_json)
                .context("deserializing history artifacts_json")?;
            let stdout: String = row.get(4).context("reading history stdout")?;
            let stderr: String = row.get(5).context("reading history stderr")?;
            let created_at: i64 = row.get(6).context("reading history created_at")?;
            let duration_ms: i64 = row.get(7).context("reading history duration_ms")?;

            out.push(ExecutionRecord {
                task_name,
                fingerprint,
                manifest,
                artifacts,
                stdout,
                stderr,
                created_at,
                duration_ms: duration_ms.max(0) as u64,
            });
        }
        Ok(out)
    }

    fn store_artifacts(
        &self,
        workspace: &Path,
        outputs: &[PathBuf],
    ) -> Result<Vec<CachedArtifact>> {
        let mut cached = Vec::with_capacity(outputs.len());

        for rel_output in outputs {
            let absolute = workspace.join(rel_output);
            if !absolute.exists() {
                return Err(anyhow!(
                    "declared output '{}' is missing after execution",
                    rel_output.display()
                ));
            }

            let (object_hash, kind) = hash_and_kind(&absolute)?;
            let object_dir = self.objects_dir.join(&object_hash);
            if !object_dir.exists() {
                copy_tree(&absolute, &object_dir).with_context(|| {
                    format!("copying artifact '{}' into CAS", absolute.display())
                })?;
            }

            cached.push(CachedArtifact {
                relative_path: rel_output.to_string_lossy().into_owned(),
                object_hash,
                kind,
            });
        }

        Ok(cached)
    }

    fn restore_artifacts(&self, workspace: &Path, artifacts: &[CachedArtifact]) -> Result<()> {
        for artifact in artifacts {
            let rel_path =
                normalize_artifact_relative_path(&artifact.relative_path).with_context(|| {
                    format!("validating cached artifact relative path '{}'", artifact.relative_path)
                })?;
            let dest = workspace.join(&rel_path);
            if !dest.starts_with(workspace) {
                return Err(anyhow!(
                    "cached artifact path '{}' escapes workspace root '{}'",
                    rel_path.display(),
                    workspace.display()
                ));
            }
            validate_object_hash(&artifact.object_hash)?;
            let src = self.objects_dir.join(&artifact.object_hash);
            if !src.exists() {
                return Err(anyhow!(
                    "cache object '{}' is missing for output '{}'",
                    artifact.object_hash,
                    artifact.relative_path
                ));
            }

            remove_path_if_exists(&dest)?;

            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent directory '{}'", parent.display()))?;
            }

            copy_tree(&src, &dest).with_context(|| {
                format!("restoring cache artifact '{}' into '{}'", src.display(), dest.display())
            })?;
        }

        Ok(())
    }

    fn prune(&self, max_size_mb: u64) -> Result<PruneReport> {
        let max_bytes = max_size_mb.saturating_mul(1024 * 1024);
        let mut object_dirs = Vec::new();

        for entry in fs::read_dir(&self.objects_dir)
            .with_context(|| format!("reading objects dir '{}'", self.objects_dir.display()))?
        {
            let entry = entry.context("reading objects dir entry")?;
            let path = entry.path();
            let metadata = entry.metadata().context("reading object metadata")?;
            if !metadata.is_dir() {
                continue;
            }

            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size = dir_size(&path)?;
            object_dirs.push((path, modified, size));
        }

        object_dirs.sort_by_key(|(_, modified, _)| *modified);

        let mut total: u64 = object_dirs.iter().map(|(_, _, size)| *size).sum();
        let mut removed_objects = 0usize;
        let mut removed_bytes = 0u64;

        for (path, _, size) in object_dirs {
            if total <= max_bytes {
                break;
            }

            fs::remove_dir_all(&path)
                .with_context(|| format!("removing cache object '{}'", path.display()))?;
            removed_objects += 1;
            removed_bytes = removed_bytes.saturating_add(size);
            total = total.saturating_sub(size);
        }

        Ok(PruneReport { removed_objects, removed_bytes, remaining_bytes: total })
    }
}

pub fn unix_timestamp_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn hash_and_kind(path: &Path) -> Result<(String, ArtifactKind)> {
    if path.is_file() {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"file");
        let mut file = fs::File::open(path)
            .with_context(|| format!("opening file '{}' for hashing", path.display()))?;
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .with_context(|| format!("reading file '{}' for hashing", path.display()))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        return Ok((hasher.finalize().to_hex().to_string(), ArtifactKind::File));
    }

    if path.is_dir() {
        let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
        for entry in WalkDir::new(path) {
            let entry = entry.context("walking output directory for hashing")?;
            let child = entry.path();
            if child.is_dir() {
                continue;
            }
            let rel = child
                .strip_prefix(path)
                .with_context(|| format!("stripping prefix '{}'", path.display()))?
                .to_string_lossy()
                .into_owned();
            files.insert(rel, child.to_path_buf());
        }

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"dir");

        for (rel, child) in files {
            hasher.update(rel.as_bytes());
            let mut file = fs::File::open(&child)
                .with_context(|| format!("opening file '{}' for hashing", child.display()))?;
            let mut buffer = [0u8; 16 * 1024];
            loop {
                let count = file
                    .read(&mut buffer)
                    .with_context(|| format!("reading file '{}' for hashing", child.display()))?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
        }

        return Ok((hasher.finalize().to_hex().to_string(), ArtifactKind::Directory));
    }

    Err(anyhow!("artifact path '{}' must be a file or directory", path.display()))
}

fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    if src.is_file() {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory '{}'", parent.display()))?;
        }
        copy_file_with_reflink_fallback(src, dest)?;
        return Ok(());
    }

    if src.is_dir() {
        fs::create_dir_all(dest)
            .with_context(|| format!("creating directory '{}'", dest.display()))?;

        for entry in WalkDir::new(src) {
            let entry = entry.context("walking directory while copying tree")?;
            let child = entry.path();
            let rel = child
                .strip_prefix(src)
                .with_context(|| format!("stripping prefix '{}'", src.display()))?;

            if rel.as_os_str().is_empty() {
                continue;
            }

            let target = dest.join(rel);
            if child.is_dir() {
                fs::create_dir_all(&target)
                    .with_context(|| format!("creating directory '{}'", target.display()))?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating directory '{}'", parent.display()))?;
                }
                copy_file_with_reflink_fallback(child, &target)?;
            }
        }

        return Ok(());
    }

    Err(anyhow!(
        "cannot copy path '{}' because it is neither a file nor a directory",
        src.display()
    ))
}

fn copy_file_with_reflink_fallback(src: &Path, dest: &Path) -> Result<()> {
    match reflink_copy::reflink(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(src, dest).with_context(|| {
                format!("copying file '{}' -> '{}'", src.display(), dest.display())
            })?;
            Ok(())
        }
    }
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        fs::remove_file(path).with_context(|| format!("removing file '{}'", path.display()))?;
    } else if path.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("removing directory '{}'", path.display()))?;
    }
    Ok(())
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in WalkDir::new(path) {
        let entry = entry.context("walking cache object directory")?;
        if entry.path().is_file() {
            total = total.saturating_add(entry.metadata().context("reading file metadata")?.len());
        }
    }
    Ok(total)
}

fn normalize_artifact_relative_path(raw: &str) -> Result<PathBuf> {
    if raw.trim().is_empty() {
        return Err(anyhow!("cached artifact path cannot be empty"));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(anyhow!("cached artifact path '{}' must be relative", raw));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                return Err(anyhow!("cached artifact path '{}' cannot contain '..' segments", raw));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("cached artifact path '{}' must be relative", raw));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(anyhow!("cached artifact path '{}' cannot resolve to current directory", raw));
    }

    Ok(normalized)
}

fn validate_object_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "cached artifact object hash '{}' is invalid; expected 64 hex characters",
            hash
        ));
    }
    Ok(())
}

fn ensure_manifest_column(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "manifest_json")? {
        conn.execute(
            "ALTER TABLE executions ADD COLUMN manifest_json TEXT NOT NULL DEFAULT '{}'",
            [],
        )
        .context("adding manifest_json column")?;
    }
    Ok(())
}

fn ensure_duration_column(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "duration_ms")? {
        conn.execute(
            "ALTER TABLE executions ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .context("adding duration_ms column")?;
    }
    Ok(())
}

fn column_exists(conn: &Connection, name: &str) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(executions)").context("preparing table info")?;
    let mut rows = stmt.query([]).context("querying table info")?;
    while let Some(row) = rows.next().context("reading table info row")? {
        let column_name: String = row.get(1).context("reading table info column name")?;
        if column_name == name {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::Write;

    #[test]
    fn stores_and_restores_artifacts() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");

        let output_rel = PathBuf::from("dist/app.txt");
        let output_abs = workspace.join(&output_rel);
        fs::create_dir_all(output_abs.parent().expect("parent")).expect("create dist");
        let mut file = fs::File::create(&output_abs).expect("create output file");
        file.write_all(b"hello").expect("write output");

        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("create store");
        let artifacts = store
            .store_artifacts(&workspace, std::slice::from_ref(&output_rel))
            .expect("store artifacts");

        fs::remove_file(&output_abs).expect("remove output");
        store.restore_artifacts(&workspace, &artifacts).expect("restore output");

        let restored = fs::read_to_string(&output_abs).expect("read restored output");
        assert_eq!(restored, "hello");
    }

    #[test]
    fn fetch_execution_returns_none_for_unknown_task() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("create store");

        let record =
            store.fetch_execution("missing_task", "fingerprint").expect("fetch missing task");
        assert!(record.is_none());
    }

    #[test]
    fn fetch_execution_returns_none_for_fingerprint_mismatch() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("create store");

        let existing = ExecutionRecord {
            task_name: "build".to_string(),
            fingerprint: "fp-1".to_string(),
            manifest: BTreeMap::new(),
            artifacts: vec![],
            stdout: "".to_string(),
            stderr: "".to_string(),
            created_at: 1,
            duration_ms: 0,
        };
        store.save_execution(&existing).expect("save execution");

        let record = store.fetch_execution("build", "fp-2").expect("fetch mismatched fingerprint");
        assert!(record.is_none());
    }

    #[test]
    fn fetch_latest_execution_returns_most_recent_by_timestamp() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("create store");

        let first = ExecutionRecord {
            task_name: "build".to_string(),
            fingerprint: "fp-1".to_string(),
            manifest: BTreeMap::from([("task:run".to_string(), "old".to_string())]),
            artifacts: vec![],
            stdout: "old".to_string(),
            stderr: "".to_string(),
            created_at: 10,
            duration_ms: 1500,
        };
        let second = ExecutionRecord {
            task_name: "build".to_string(),
            fingerprint: "fp-2".to_string(),
            manifest: BTreeMap::from([("task:run".to_string(), "new".to_string())]),
            artifacts: vec![],
            stdout: "new".to_string(),
            stderr: "".to_string(),
            created_at: 20,
            duration_ms: 2200,
        };

        store.save_execution(&first).expect("save first execution");
        store.save_execution(&second).expect("save second execution");

        let latest = store.fetch_latest_execution("build").expect("fetch latest execution");
        let latest = latest.expect("expected latest execution");
        assert_eq!(latest.fingerprint, "fp-2");
        assert_eq!(latest.stdout, "new");
        assert_eq!(latest.manifest.get("task:run"), Some(&"new".to_string()));
        assert_eq!(latest.duration_ms, 2200);
    }

    fn record(task: &str, fingerprint: &str, created_at: i64, duration_ms: u64) -> ExecutionRecord {
        ExecutionRecord {
            task_name: task.to_string(),
            fingerprint: fingerprint.to_string(),
            manifest: BTreeMap::new(),
            artifacts: vec![],
            stdout: "".to_string(),
            stderr: "".to_string(),
            created_at,
            duration_ms,
        }
    }

    #[test]
    fn fetch_history_scoped_to_task_returns_newest_first() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("store");
        for (i, ts) in [(1, 100), (2, 200), (3, 300), (4, 400)].iter().enumerate() {
            store
                .save_execution(&record("build", &format!("fp-{}", i), ts.1, 1000 + i as u64))
                .expect("save");
        }
        store.save_execution(&record("test", "fp-other", 500, 9999)).expect("save other");

        let rows = store.fetch_history(Some("build"), 3).expect("history");
        assert_eq!(rows.len(), 3, "limit honored");
        let timestamps: Vec<i64> = rows.iter().map(|r| r.created_at).collect();
        assert_eq!(timestamps, vec![400, 300, 200], "newest first");
        for r in &rows {
            assert_eq!(r.task_name, "build", "scoped to requested task");
        }
    }

    #[test]
    fn fetch_history_scoped_returns_empty_for_unknown_task() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("store");
        store.save_execution(&record("build", "fp-1", 100, 1000)).expect("save");

        let rows = store.fetch_history(Some("never_ran"), 10).expect("history");
        assert!(rows.is_empty());
    }

    #[test]
    fn fetch_history_global_returns_one_row_per_task_latest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("store");
        // build: two rows, latest at ts=300
        store.save_execution(&record("build", "b-1", 100, 500)).expect("save");
        store.save_execution(&record("build", "b-2", 300, 700)).expect("save");
        // test: one row at ts=200
        store.save_execution(&record("test", "t-1", 200, 1500)).expect("save");
        // ci: one row at ts=400
        store.save_execution(&record("ci", "c-1", 400, 2500)).expect("save");

        let rows = store.fetch_history(None, 100).expect("history");
        assert_eq!(rows.len(), 3, "one row per task");

        let by_name: std::collections::HashMap<String, ExecutionRecord> =
            rows.into_iter().map(|r| (r.task_name.clone(), r)).collect();
        assert_eq!(by_name.get("build").unwrap().created_at, 300);
        assert_eq!(by_name.get("build").unwrap().fingerprint, "b-2");
        assert_eq!(by_name.get("test").unwrap().created_at, 200);
        assert_eq!(by_name.get("ci").unwrap().created_at, 400);
    }

    #[test]
    fn fetch_history_with_zero_limit_returns_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("store");
        store.save_execution(&record("build", "fp", 1, 1)).expect("save");
        assert!(store.fetch_history(Some("build"), 0).expect("history").is_empty());
        assert!(store.fetch_history(None, 0).expect("history").is_empty());
    }

    #[test]
    fn migrates_old_schema_with_missing_manifest_column() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let cache_root = tmp.path().join("cache");
        fs::create_dir_all(&cache_root).expect("create cache root");
        let db_path = cache_root.join("metadata.sqlite3");
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch(
            "
            CREATE TABLE executions (
                task_name TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                artifacts_json TEXT NOT NULL,
                stdout TEXT NOT NULL,
                stderr TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY(task_name, fingerprint)
            );",
        )
        .expect("create old schema");

        let old_artifacts = serde_json::to_string(&Vec::<CachedArtifact>::new()).expect("json");
        conn.execute(
            "INSERT INTO executions (task_name, fingerprint, artifacts_json, stdout, stderr, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["build", "fp-old", old_artifacts, "", "", 5i64],
        )
        .expect("insert old row");
        drop(conn);

        let store = LocalArtifactStore::new(&cache_root).expect("reopen store with migration");
        let fetched = store
            .fetch_execution("build", "fp-old")
            .expect("fetch migrated row")
            .expect("row exists");
        assert!(fetched.manifest.is_empty());
    }

    #[test]
    fn migrates_old_schema_with_missing_duration_column() {
        // Schema as it existed before the duration_ms column landed: manifest
        // is present (post-v0.5 migration) but duration_ms is not.
        let tmp = tempfile::tempdir().expect("create temp dir");
        let cache_root = tmp.path().join("cache");
        fs::create_dir_all(&cache_root).expect("create cache root");
        let db_path = cache_root.join("metadata.sqlite3");
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch(
            "
            CREATE TABLE executions (
                task_name TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                manifest_json TEXT NOT NULL DEFAULT '{}',
                artifacts_json TEXT NOT NULL,
                stdout TEXT NOT NULL,
                stderr TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY(task_name, fingerprint)
            );",
        )
        .expect("create pre-duration schema");

        let old_artifacts = serde_json::to_string(&Vec::<CachedArtifact>::new()).expect("json");
        conn.execute(
            "INSERT INTO executions
             (task_name, fingerprint, manifest_json, artifacts_json, stdout, stderr, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params!["build", "fp-pre-duration", "{}", old_artifacts, "", "", 9i64],
        )
        .expect("insert pre-duration row");
        drop(conn);

        let store = LocalArtifactStore::new(&cache_root).expect("reopen store with migration");
        let fetched = store
            .fetch_execution("build", "fp-pre-duration")
            .expect("fetch migrated row")
            .expect("row exists");
        // Backfilled rows default to 0 — TUI treats this as "no ETA".
        assert_eq!(fetched.duration_ms, 0);

        // New writes against the migrated schema persist non-zero durations.
        let next = ExecutionRecord {
            task_name: "build".to_string(),
            fingerprint: "fp-new".to_string(),
            manifest: BTreeMap::new(),
            artifacts: vec![],
            stdout: "".to_string(),
            stderr: "".to_string(),
            created_at: 99,
            duration_ms: 4321,
        };
        store.save_execution(&next).expect("save new row after migration");
        let latest =
            store.fetch_latest_execution("build").expect("fetch latest").expect("row exists");
        assert_eq!(latest.duration_ms, 4321);
    }

    #[test]
    fn restore_artifacts_rejects_path_escape() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");

        let output_rel = PathBuf::from("dist/app.txt");
        let output_abs = workspace.join(&output_rel);
        fs::create_dir_all(output_abs.parent().expect("parent")).expect("create dist");
        fs::write(&output_abs, "hello").expect("write output");

        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("create store");
        let mut artifacts = store
            .store_artifacts(&workspace, std::slice::from_ref(&output_rel))
            .expect("store artifacts");
        artifacts[0].relative_path = "../escape.txt".to_string();

        let error =
            store.restore_artifacts(&workspace, &artifacts).expect_err("path escape should fail");
        let message = error.to_string();
        assert!(
            message.contains("validating cached artifact relative path"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn restore_artifacts_rejects_invalid_object_hash() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");

        let artifacts = vec![CachedArtifact {
            relative_path: "dist/app.txt".to_string(),
            object_hash: "not-a-hash".to_string(),
            kind: ArtifactKind::File,
        }];

        let store = LocalArtifactStore::new(tmp.path().join("cache")).expect("create store");
        let error =
            store.restore_artifacts(&workspace, &artifacts).expect_err("invalid hash should fail");
        assert!(error.to_string().contains("expected 64 hex characters"));
    }
}
