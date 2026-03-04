use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use please_cache::unix_timestamp_secs;
use please_store::{ArtifactStore, ExecutionRecord};
use rayon::prelude::*;
use tempfile::TempDir;
use walkdir::{DirEntry, WalkDir};

use crate::fingerprint::compute_fingerprint;
use crate::graph::TaskGraph;
use crate::model::{IsolationMode, PleaseFile, TaskSpec};
use crate::resolver::{normalize_relative_path, resolve_inputs};
use crate::runtime::{acquire_runtime_lock, sweep_runtime_state, RuntimeLockGuard};

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub dry_run: bool,
    pub force: bool,
    pub no_cache: bool,
    pub explain: bool,
    pub force_isolation: bool,
    pub jobs: usize,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            force: false,
            no_cache: false,
            explain: false,
            force_isolation: false,
            jobs: num_cpus::get().max(1),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct RunSummary {
    pub executed: Vec<String>,
    pub cache_hits: Vec<String>,
    pub dry_run: Vec<String>,
    pub cache_miss_reasons: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
struct TaskOutcome {
    task_name: String,
    from_cache: bool,
    dry_run: bool,
    cache_miss_reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum TaskProgressStatus {
    Executed,
    CacheHit,
    DryRun,
    Failed,
}

#[derive(Debug, Clone)]
enum ProgressEvent {
    TaskStarted(String),
    TaskFinished(String, TaskProgressStatus),
}

pub struct Executor {
    workspace_root: PathBuf,
    config: PleaseFile,
    graph: TaskGraph,
    store: Arc<dyn ArtifactStore>,
    _lock_guard: RuntimeLockGuard,
}

impl Executor {
    pub fn new(
        workspace_root: impl AsRef<Path>,
        config: PleaseFile,
        store: Arc<dyn ArtifactStore>,
    ) -> Result<Self> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let sweep = sweep_runtime_state(&workspace_root, true)?;
        if sweep.active_lock_detected {
            return Err(anyhow!("another Please execution is active; aborting startup sweep"));
        }
        let lock_guard = acquire_runtime_lock(&workspace_root)?;
        let graph = TaskGraph::build(&config.task)?;

        Ok(Self { workspace_root, config, graph, store, _lock_guard: lock_guard })
    }

    pub fn graph(&self) -> &TaskGraph {
        &self.graph
    }

    pub fn run_target(&self, target: &str, options: &RunOptions) -> Result<RunSummary> {
        if options.force_isolation {
            if !cfg!(target_os = "linux") {
                return Err(anyhow!(
                    "--force-isolation requires Linux; strict sandbox execution is unsupported on this platform"
                ));
            }
            let _ = which::which("bwrap")
                .context("--force-isolation requires bubblewrap (`bwrap`) on PATH")?;
        }

        let layers = self.graph.layers_for_target(target)?;
        let mut summary = RunSummary::default();
        let progress_enabled = io::stderr().is_terminal();
        let mut renderer: Option<thread::JoinHandle<()>> = None;
        let mut progress_sender: Option<Sender<ProgressEvent>> = None;

        if progress_enabled {
            let (tx, rx) = mpsc::channel::<ProgressEvent>();
            progress_sender = Some(tx);
            renderer = Some(thread::spawn(move || run_progress_renderer(rx)));
        }

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(options.jobs.max(1))
            .build()
            .context("building worker pool")?;

        for mut layer in layers {
            layer.sort();
            let outcomes: Vec<Result<TaskOutcome>> = pool.install(|| {
                layer
                    .par_iter()
                    .map(|task_name| {
                        self.execute_task(task_name, options, progress_sender.as_ref().cloned())
                    })
                    .collect()
            });

            for outcome in outcomes {
                let outcome = match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        drop(progress_sender.take());
                        if let Some(handle) = renderer.take() {
                            let _ = handle.join();
                        }
                        return Err(error);
                    }
                };
                let task_name = outcome.task_name.clone();
                if outcome.dry_run {
                    summary.dry_run.push(task_name.clone());
                } else if outcome.from_cache {
                    summary.cache_hits.push(task_name.clone());
                } else {
                    summary.executed.push(task_name.clone());
                }
                if !outcome.cache_miss_reasons.is_empty() {
                    summary.cache_miss_reasons.insert(task_name, outcome.cache_miss_reasons);
                }
            }
        }

        drop(progress_sender.take());
        if let Some(handle) = renderer.take() {
            let _ = handle.join();
        }

        Ok(summary)
    }

    fn execute_task(
        &self,
        task_name: &str,
        options: &RunOptions,
        progress: Option<Sender<ProgressEvent>>,
    ) -> Result<TaskOutcome> {
        emit_progress(&progress, ProgressEvent::TaskStarted(task_name.to_string()));
        let task = self
            .config
            .task
            .get(task_name)
            .ok_or_else(|| anyhow!("task '{}' not found", task_name))?;

        let outputs = normalize_outputs(task)?;
        let inputs = resolve_inputs(&self.workspace_root, &task.inputs)?;
        let fingerprint_result =
            compute_fingerprint(&self.workspace_root, task_name, task, &inputs)?;
        let mut cache_miss_reasons = Vec::new();

        if !options.force && !options.no_cache {
            if let Some(record) =
                self.store.fetch_execution(task_name, &fingerprint_result.fingerprint.0)?
            {
                if options.dry_run {
                    emit_progress(
                        &progress,
                        ProgressEvent::TaskFinished(
                            task_name.to_string(),
                            TaskProgressStatus::DryRun,
                        ),
                    );
                    return Ok(TaskOutcome {
                        task_name: task_name.to_string(),
                        from_cache: true,
                        dry_run: true,
                        cache_miss_reasons: Vec::new(),
                    });
                }

                self.store
                    .restore_artifacts(&self.workspace_root, &record.artifacts)
                    .with_context(|| format!("restoring cache hit for task '{}'", task_name))?;

                emit_progress(
                    &progress,
                    ProgressEvent::TaskFinished(
                        task_name.to_string(),
                        TaskProgressStatus::CacheHit,
                    ),
                );
                return Ok(TaskOutcome {
                    task_name: task_name.to_string(),
                    from_cache: true,
                    dry_run: false,
                    cache_miss_reasons: Vec::new(),
                });
            }
        }

        if options.explain {
            cache_miss_reasons =
                self.explain_cache_miss(task_name, options, &fingerprint_result.manifest)?;
        }

        if options.dry_run {
            emit_progress(
                &progress,
                ProgressEvent::TaskFinished(task_name.to_string(), TaskProgressStatus::DryRun),
            );
            return Ok(TaskOutcome {
                task_name: task_name.to_string(),
                from_cache: false,
                dry_run: true,
                cache_miss_reasons,
            });
        }

        let stage = self.create_stage_snapshot(task_name)?;
        let output = self
            .run_task_command(task_name, task, stage.path(), options)
            .with_context(|| format!("executing task '{}'", task_name))?;

        if !output.status.success() {
            emit_progress(
                &progress,
                ProgressEvent::TaskFinished(task_name.to_string(), TaskProgressStatus::Failed),
            );
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            return Err(anyhow!(
                "task '{}' failed with status {}\nstdout:\n{}\nstderr:\n{}",
                task_name,
                output.status,
                stdout,
                stderr
            ));
        }

        self.promote_outputs(stage.path(), &outputs)
            .with_context(|| format!("promoting outputs for task '{}'", task_name))?;

        if !options.no_cache {
            let artifacts = self.store.store_artifacts(&self.workspace_root, &outputs)?;
            let record = ExecutionRecord {
                task_name: task_name.to_string(),
                fingerprint: fingerprint_result.fingerprint.0,
                manifest: fingerprint_result.manifest,
                artifacts,
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                created_at: unix_timestamp_secs(),
            };
            self.store.save_execution(&record)?;
        }

        emit_progress(
            &progress,
            ProgressEvent::TaskFinished(task_name.to_string(), TaskProgressStatus::Executed),
        );
        Ok(TaskOutcome {
            task_name: task_name.to_string(),
            from_cache: false,
            dry_run: false,
            cache_miss_reasons,
        })
    }

    fn explain_cache_miss(
        &self,
        task_name: &str,
        options: &RunOptions,
        current_manifest: &BTreeMap<String, String>,
    ) -> Result<Vec<String>> {
        if options.force {
            return Ok(vec!["cache bypass: --force supplied".to_string()]);
        }
        if options.no_cache {
            return Ok(vec!["cache bypass: --no-cache supplied".to_string()]);
        }

        let Some(previous) = self.store.fetch_latest_execution(task_name)? else {
            return Ok(vec!["cache miss: no prior execution record".to_string()]);
        };

        let mut reasons = explain_manifest_delta(&previous.manifest, current_manifest);
        if reasons.is_empty() {
            reasons.push("cache miss: fingerprint changed".to_string());
        }
        Ok(reasons)
    }

    fn create_stage_snapshot(&self, task_name: &str) -> Result<TempDir> {
        let stage_parent = self.workspace_root.join(".please/stage");
        fs::create_dir_all(&stage_parent)
            .with_context(|| format!("creating stage parent '{}'", stage_parent.display()))?;

        let stage = tempfile::Builder::new()
            .prefix(&format!("{}-", task_name))
            .tempdir_in(&stage_parent)
            .with_context(|| format!("creating stage dir for task '{}'", task_name))?;

        copy_workspace_snapshot(&self.workspace_root, stage.path())?;

        Ok(stage)
    }

    fn run_task_command(
        &self,
        _task_name: &str,
        task: &TaskSpec,
        stage_workspace: &Path,
        options: &RunOptions,
    ) -> Result<Output> {
        let isolation_mode = selected_isolation(task, options);

        let mut command = match isolation_mode {
            IsolationMode::Strict if cfg!(target_os = "linux") => {
                let bwrap = which::which("bwrap").context(
                    "strict isolation requires bubblewrap (`bwrap`) to be installed on Linux",
                )?;
                let mut cmd = Command::new(bwrap);
                cmd.arg("--die-with-parent")
                    .arg("--new-session")
                    .arg("--unshare-net")
                    .arg("--ro-bind")
                    .arg("/")
                    .arg("/")
                    .arg("--bind")
                    .arg(stage_workspace)
                    .arg(stage_workspace)
                    .arg("--proc")
                    .arg("/proc")
                    .arg("--dev")
                    .arg("/dev")
                    .arg("--tmpfs")
                    .arg("/tmp")
                    .arg("--chdir")
                    .arg(stage_workspace)
                    .arg("/bin/sh")
                    .arg("-lc")
                    .arg(task.run_as_shell());
                cmd
            }
            IsolationMode::Strict => {
                return Err(anyhow!(
                    "strict isolation is only supported on Linux in v0.1; use best_effort on this platform"
                ));
            }
            IsolationMode::BestEffort | IsolationMode::Off => {
                let mut cmd = Command::new("/bin/sh");
                cmd.arg("-lc").arg(task.run_as_shell());
                cmd
            }
        };

        command.current_dir(stage_workspace);

        match isolation_mode {
            IsolationMode::Strict | IsolationMode::BestEffort => {
                command.env_clear();
                for key in ["PATH", "HOME", "USER", "TMPDIR", "SHELL", "TERM"] {
                    if let Ok(value) = env::var(key) {
                        command.env(key, value);
                    }
                }
            }
            IsolationMode::Off => {}
        }

        for (key, value) in &task.env {
            command.env(key, value);
        }

        command.output().with_context(|| format!("spawning task command '{}'", task.run_as_shell()))
    }

    fn promote_outputs(&self, stage_workspace: &Path, outputs: &[PathBuf]) -> Result<()> {
        let tx_parent = self.workspace_root.join(".please/tx");
        fs::create_dir_all(&tx_parent)
            .with_context(|| format!("creating tx directory '{}'", tx_parent.display()))?;

        let tx = tempfile::Builder::new()
            .prefix("tx-")
            .tempdir_in(&tx_parent)
            .context("creating transactional output directory")?;

        let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();

        for output in outputs {
            let destination = self.workspace_root.join(output);
            let staged = stage_workspace.join(output);

            if !staged.exists() {
                return Err(anyhow!(
                    "declared output '{}' was not produced in staged execution",
                    output.display()
                ));
            }

            if destination.exists() {
                let backup_path = tx.path().join(output);
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("creating backup parent '{}'", parent.display())
                    })?;
                }
                fs::rename(&destination, &backup_path).with_context(|| {
                    format!(
                        "moving existing output '{}' to backup '{}'",
                        destination.display(),
                        backup_path.display()
                    )
                })?;
                backups.push((destination.clone(), backup_path));
            }
        }

        let mut promoted: Vec<PathBuf> = Vec::new();

        let promote_result = (|| {
            for output in outputs {
                let staged = stage_workspace.join(output);
                let destination = self.workspace_root.join(output);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("creating destination parent '{}'", parent.display())
                    })?;
                }

                match fs::rename(&staged, &destination) {
                    Ok(()) => {}
                    Err(_) => {
                        copy_tree(&staged, &destination)?;
                        remove_path_if_exists(&staged)?;
                    }
                }

                promoted.push(destination);
            }
            Ok(())
        })();

        if let Err(error) = promote_result {
            for destination in &promoted {
                let _ = remove_path_if_exists(destination);
            }
            for (destination, backup) in backups.iter().rev() {
                if backup.exists() {
                    if let Some(parent) = destination.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::rename(backup, destination);
                }
            }
            return Err(error);
        }

        Ok(())
    }
}

fn emit_progress(sender: &Option<Sender<ProgressEvent>>, event: ProgressEvent) {
    if let Some(tx) = sender {
        let _ = tx.send(event);
    }
}

fn run_progress_renderer(receiver: mpsc::Receiver<ProgressEvent>) {
    let multi = MultiProgress::new();
    let style = ProgressStyle::with_template("{spinner:.green} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_strings(&["-", "\\", "|", "/"]);

    let mut bars: std::collections::HashMap<String, ProgressBar> = std::collections::HashMap::new();

    while let Ok(event) = receiver.recv() {
        match event {
            ProgressEvent::TaskStarted(task) => {
                let bar = bars.entry(task.clone()).or_insert_with(|| {
                    let pb = multi.add(ProgressBar::new_spinner());
                    pb.set_style(style.clone());
                    pb.enable_steady_tick(Duration::from_millis(100));
                    pb
                });
                bar.set_message(format!("{task} running"));
            }
            ProgressEvent::TaskFinished(task, status) => {
                if let Some(bar) = bars.remove(&task) {
                    match status {
                        TaskProgressStatus::Executed => {
                            bar.finish_and_clear();
                        }
                        TaskProgressStatus::CacheHit => {
                            bar.finish_and_clear();
                        }
                        TaskProgressStatus::DryRun => {
                            bar.finish_and_clear();
                        }
                        TaskProgressStatus::Failed => {
                            bar.finish_with_message(format!("{task} failed"));
                        }
                    }
                }
            }
        }
    }
}

fn selected_isolation(task: &TaskSpec, options: &RunOptions) -> IsolationMode {
    if options.force_isolation {
        IsolationMode::Strict
    } else {
        task.effective_isolation()
    }
}

fn normalize_outputs(task: &TaskSpec) -> Result<Vec<PathBuf>> {
    let mut outputs = Vec::with_capacity(task.outputs.len());
    for output in &task.outputs {
        outputs.push(normalize_relative_path(output)?);
    }
    Ok(outputs)
}

fn copy_workspace_snapshot(source_root: &Path, stage_root: &Path) -> Result<()> {
    for entry in WalkDir::new(source_root)
        .into_iter()
        .filter_entry(|entry| should_include(entry, source_root))
    {
        let entry = entry.context("walking workspace snapshot")?;
        let path = entry.path();
        let rel = path
            .strip_prefix(source_root)
            .with_context(|| format!("stripping workspace prefix '{}'", source_root.display()))?;

        if rel.as_os_str().is_empty() {
            continue;
        }

        let target = stage_root.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("creating stage directory '{}'", target.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent '{}'", parent.display()))?;
            }
            copy_file_with_reflink_fallback(path, &target)
                .with_context(|| format!("copying workspace file '{}' to stage", path.display()))?;
        } else if entry.file_type().is_symlink() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;

                let target_link = fs::read_link(path)
                    .with_context(|| format!("reading symlink '{}'", path.display()))?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating parent '{}'", parent.display()))?;
                }
                symlink(target_link, &target)
                    .with_context(|| format!("creating symlink '{}'", target.display()))?;
            }
        }
    }

    Ok(())
}

fn should_include(entry: &DirEntry, source_root: &Path) -> bool {
    let path = entry.path();
    let Ok(rel) = path.strip_prefix(source_root) else {
        return true;
    };
    if rel.as_os_str().is_empty() {
        return true;
    }

    let first = rel.components().next();
    !matches!(
        first,
        Some(Component::Normal(part)) if part == ".please" || part == ".git"
    )
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
            let entry = entry.context("walking path while copying tree")?;
            let child = entry.path();
            let rel = child
                .strip_prefix(src)
                .with_context(|| format!("stripping source prefix '{}'", src.display()))?;

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

fn copy_file_with_reflink_fallback(src: &Path, dest: &Path) -> Result<()> {
    match reflink_copy::reflink(src, dest) {
        Ok(()) => Ok(()),
        Err(error) if is_reflink_unsupported(&error) => {
            fs::copy(src, dest).with_context(|| {
                format!("copying file '{}' -> '{}'", src.display(), dest.display())
            })?;
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!("attempting reflink copy for '{}' -> '{}'", src.display(), dest.display())
        }),
    }
}

fn is_reflink_unsupported(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }

    matches!(
        error.raw_os_error(),
        Some(code)
            if code == libc::ENOTSUP
                || code == libc::EOPNOTSUPP
                || code == libc::EXDEV
                || code == libc::EINVAL
    )
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

fn explain_manifest_delta(
    previous: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut keys = BTreeSet::new();
    keys.extend(previous.keys().cloned());
    keys.extend(current.keys().cloned());

    let mut reasons = Vec::new();
    for key in keys {
        match (previous.get(&key), current.get(&key)) {
            (Some(old), Some(new)) if old != new => {
                reasons.push(describe_manifest_change("changed", &key))
            }
            (None, Some(_)) => reasons.push(describe_manifest_change("added", &key)),
            (Some(_), None) => reasons.push(describe_manifest_change("removed", &key)),
            _ => {}
        }
    }
    reasons
}

fn describe_manifest_change(action: &str, key: &str) -> String {
    if let Some(path) = key.strip_prefix("input:") {
        return format!("cache miss: input {action}: {path}");
    }
    if let Some(name) = key.strip_prefix("env:") {
        return format!("cache miss: env {action}: {name}");
    }
    if let Some(pattern) = key.strip_prefix("input_pattern:") {
        return format!("cache miss: input pattern {action}: {pattern}");
    }
    if let Some(output) = key.strip_prefix("output:") {
        return format!("cache miss: output contract {action}: {output}");
    }
    if key == "task:run" {
        return format!("cache miss: task command {action}");
    }
    if key == "task:isolation" {
        return format!("cache miss: isolation mode {action}");
    }
    if key.starts_with("task:name:") {
        return format!("cache miss: task identity {action}");
    }
    format!("cache miss: {key} {action}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PleaseSection, RunSpec};
    use blake3::Hasher;
    use please_cache::LocalArtifactStore;
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::Read;
    use std::io::Write;
    #[cfg(target_os = "linux")]
    use std::process::Command as ProcessCommand;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn simple_task(command: &str) -> TaskSpec {
        TaskSpec {
            deps: vec![],
            inputs: vec!["src/input.txt".to_string()],
            outputs: vec!["dist/output.txt".to_string()],
            env: BTreeMap::new(),
            run: RunSpec::Shell(command.to_string()),
            isolation: Some(IsolationMode::BestEffort),
        }
    }

    #[test]
    fn failure_does_not_promote_partial_outputs() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).expect("create src");

        let mut input = fs::File::create(workspace.join("src/input.txt")).expect("create input");
        input.write_all(b"hello").expect("write input");

        fs::create_dir_all(workspace.join("dist")).expect("create dist");
        let mut old_output =
            fs::File::create(workspace.join("dist/output.txt")).expect("create old output");
        old_output.write_all(b"stable").expect("write old output");

        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), simple_task("echo broken > dist/output.txt && exit 42"));

        let config =
            PleaseFile { please: PleaseSection { version: "0.2".to_string() }, task: tasks };

        let cache = LocalArtifactStore::new(workspace.join(".please/cache")).expect("create cache");
        let executor = Executor::new(&workspace, config, Arc::new(cache)).expect("create executor");

        let result = executor.run_target("build", &RunOptions::default());
        assert!(result.is_err());

        let content =
            fs::read_to_string(workspace.join("dist/output.txt")).expect("read old output");
        assert_eq!(content.trim(), "stable");
    }

    #[test]
    fn stage_snapshot_preserves_large_file_content() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("data")).expect("create data dir");

        let source_file = workspace.join("data/large.bin");
        let mut file = File::create(&source_file).expect("create large file");
        let chunk = vec![0x5Au8; 1024 * 1024];
        for _ in 0..128 {
            file.write_all(&chunk).expect("write chunk");
        }
        file.sync_all().expect("sync large file");

        let stage = tempfile::tempdir_in(tmp.path()).expect("create stage dir");
        let start = Instant::now();
        copy_workspace_snapshot(&workspace, stage.path()).expect("copy workspace snapshot");
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(20),
            "snapshot copy exceeded safety budget: {elapsed:?}"
        );

        let stage_file = stage.path().join("data/large.bin");
        assert!(stage_file.exists(), "expected staged large file");

        let source_hash = file_hash(&source_file).expect("hash source");
        let staged_hash = file_hash(&stage_file).expect("hash stage");
        assert_eq!(source_hash, staged_hash, "staged file hash mismatch");
    }

    fn file_hash(path: &Path) -> Result<String> {
        let mut hasher = Hasher::new();
        let mut file = File::open(path)?;
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    #[test]
    fn explain_manifest_delta_reports_changes() {
        let previous = BTreeMap::from([
            ("input:src/input.txt".to_string(), "a".to_string()),
            ("env:MODE".to_string(), "a".to_string()),
        ]);
        let current = BTreeMap::from([
            ("input:src/input.txt".to_string(), "b".to_string()),
            ("env:MODE".to_string(), "a".to_string()),
            ("output:dist/out.txt".to_string(), "x".to_string()),
        ]);

        let reasons = explain_manifest_delta(&previous, &current);
        assert!(reasons.iter().any(|r| r.contains("input changed: src/input.txt")));
        assert!(reasons.iter().any(|r| r.contains("output contract added: dist/out.txt")));
    }

    #[cfg(target_os = "linux")]
    fn strict_bwrap_supported() -> bool {
        let Ok(bwrap) = which::which("bwrap") else {
            return false;
        };

        let Ok(output) = ProcessCommand::new(bwrap)
            .arg("--die-with-parent")
            .arg("--new-session")
            .arg("--unshare-net")
            .arg("--ro-bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--dev")
            .arg("/dev")
            .arg("--tmpfs")
            .arg("/tmp")
            .arg("/bin/sh")
            .arg("-lc")
            .arg("echo PLEASE_BWRAP_TEST")
            .output()
        else {
            return false;
        };

        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("PLEASE_BWRAP_TEST")
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn strict_isolation_executes_when_bwrap_available() {
        // CI/container kernels may expose bwrap but block required namespace operations.
        // Only run this test when strict bwrap execution is actually viable.
        if !strict_bwrap_supported() {
            eprintln!("skipping strict isolation test because bwrap strict mode is unavailable");
            return;
        }

        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::write(workspace.join("src/input.txt"), "hello").expect("write input");

        let mut tasks = BTreeMap::new();
        tasks.insert(
            "build".to_string(),
            TaskSpec {
                deps: vec![],
                inputs: vec!["src/input.txt".to_string()],
                outputs: vec!["dist/output.txt".to_string()],
                env: BTreeMap::new(),
                run: RunSpec::Shell(
                    "mkdir -p dist && cp src/input.txt dist/output.txt".to_string(),
                ),
                isolation: Some(IsolationMode::Strict),
            },
        );

        let config =
            PleaseFile { please: PleaseSection { version: "0.2".to_string() }, task: tasks };
        let cache = LocalArtifactStore::new(workspace.join(".please/cache")).expect("create cache");
        let executor = Executor::new(&workspace, config, Arc::new(cache)).expect("create executor");

        let result = executor.run_target("build", &RunOptions::default());
        assert!(result.is_ok());
        let output =
            fs::read_to_string(workspace.join("dist/output.txt")).expect("read output content");
        assert_eq!(output, "hello");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn force_isolation_fails_on_non_linux() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::write(workspace.join("src/input.txt"), "hello").expect("write input");

        let mut tasks = BTreeMap::new();
        tasks.insert(
            "build".to_string(),
            TaskSpec {
                deps: vec![],
                inputs: vec!["src/input.txt".to_string()],
                outputs: vec!["dist/output.txt".to_string()],
                env: BTreeMap::new(),
                run: RunSpec::Shell(
                    "mkdir -p dist && cp src/input.txt dist/output.txt".to_string(),
                ),
                isolation: Some(IsolationMode::Off),
            },
        );

        let config =
            PleaseFile { please: PleaseSection { version: "0.2".to_string() }, task: tasks };
        let cache = LocalArtifactStore::new(workspace.join(".please/cache")).expect("create cache");
        let executor = Executor::new(&workspace, config, Arc::new(cache)).expect("create executor");

        let result = executor
            .run_target("build", &RunOptions { force_isolation: true, ..RunOptions::default() });
        assert!(result.is_err());
    }
}
