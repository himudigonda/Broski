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
                PRIMARY KEY(task_name, fingerprint)
            );
            CREATE INDEX IF NOT EXISTS idx_executions_task_created_at
            ON executions(task_name, created_at DESC);
            ",
        )
        .context("initializing sqlite schema")?;
        ensure_manifest_column(&conn)?;
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
                "SELECT manifest_json, artifacts_json, stdout, stderr, created_at
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
            Ok(Some(ExecutionRecord {
                task_name: task_name.to_owned(),
                fingerprint: fingerprint.to_owned(),
                manifest,
                artifacts,
                stdout,
                stderr,
                created_at,
            }))
        } else {
            Ok(None)
        }
    }

    fn fetch_latest_execution(&self, task_name: &str) -> Result<Option<ExecutionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT fingerprint, manifest_json, artifacts_json, stdout, stderr, created_at
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

            Ok(Some(ExecutionRecord {
                task_name: task_name.to_owned(),
                fingerprint,
                manifest,
                artifacts,
                stdout,
                stderr,
                created_at,
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
        conn.execute(
            "INSERT OR REPLACE INTO executions
            (task_name, fingerprint, manifest_json, artifacts_json, stdout, stderr, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.task_name,
                record.fingerprint,
                manifest_json,
                artifacts_json,
                record.stdout,
                record.stderr,
                record.created_at,
            ],
        )
        .context("writing execution record")?;
        Ok(())
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
        fs::copy(src, dest)
            .with_context(|| format!("copying file '{}' -> '{}'", src.display(), dest.display()))?;
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
                fs::copy(child, &target).with_context(|| {
                    format!("copying file '{}' -> '{}'", child.display(), target.display())
                })?;
            }
        }

        return Ok(());
    }

    Err(anyhow!(
        "cannot copy path '{}' because it is neither a file nor a directory",
        src.display()
    ))
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
    let mut stmt = conn.prepare("PRAGMA table_info(executions)").context("preparing table info")?;
    let mut rows = stmt.query([]).context("querying table info")?;
    let mut has_manifest = false;
    while let Some(row) = rows.next().context("reading table info row")? {
        let column_name: String = row.get(1).context("reading table info column name")?;
        if column_name == "manifest_json" {
            has_manifest = true;
            break;
        }
    }

    if !has_manifest {
        conn.execute(
            "ALTER TABLE executions ADD COLUMN manifest_json TEXT NOT NULL DEFAULT '{}'",
            [],
        )
        .context("adding manifest_json column")?;
    }
    Ok(())
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
        };
        let second = ExecutionRecord {
            task_name: "build".to_string(),
            fingerprint: "fp-2".to_string(),
            manifest: BTreeMap::from([("task:run".to_string(), "new".to_string())]),
            artifacts: vec![],
            stdout: "new".to_string(),
            stderr: "".to_string(),
            created_at: 20,
        };

        store.save_execution(&first).expect("save first execution");
        store.save_execution(&second).expect("save second execution");

        let latest = store.fetch_latest_execution("build").expect("fetch latest execution");
        let latest = latest.expect("expected latest execution");
        assert_eq!(latest.fingerprint, "fp-2");
        assert_eq!(latest.stdout, "new");
        assert_eq!(latest.manifest.get("task:run"), Some(&"new".to_string()));
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
