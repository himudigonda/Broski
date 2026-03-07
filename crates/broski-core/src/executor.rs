use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use aho_corasick::AhoCorasick;
use anyhow::{anyhow, Context, Result};
use broski_cache::unix_timestamp_secs;
use broski_store::{ArtifactStore, ExecutionRecord};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use rand::RngCore;
use rayon::prelude::*;
use tempfile::{NamedTempFile, TempDir};
use walkdir::{DirEntry, WalkDir};

use crate::fingerprint::{compute_fingerprint, FingerprintOptions};
use crate::graph::TaskGraph;
use crate::model::{BroskiFile, IsolationMode, TaskMode, TaskSpec};
use crate::resolver::{normalize_relative_path, resolve_inputs};
use crate::runtime::{acquire_runtime_lock, sweep_runtime_state, RuntimeLockGuard};

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub dry_run: bool,
    pub force: bool,
    pub no_cache: bool,
    pub explain: bool,
    pub watch: bool,
    pub force_isolation: bool,
    pub jobs: usize,
    pub passthrough_args: Vec<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            force: false,
            no_cache: false,
            explain: false,
            watch: false,
            force_isolation: false,
            jobs: num_cpus::get().max(1),
            passthrough_args: Vec::new(),
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

#[derive(Debug, Default, Clone)]
struct TaskPhaseTimings {
    resolve_inputs: Option<Duration>,
    fingerprint: Option<Duration>,
    cache_restore: Option<Duration>,
    stage_prep: Option<Duration>,
    command_exec: Option<Duration>,
    output_promote: Option<Duration>,
    cache_store: Option<Duration>,
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

#[derive(Debug, Clone)]
struct WatchContext {
    watch_roots: Vec<PathBuf>,
    tracked_inputs: Vec<PathBuf>,
    ignored_prefixes: Vec<PathBuf>,
}

pub struct Executor {
    workspace_root: PathBuf,
    config: BroskiFile,
    graph: TaskGraph,
    store: Arc<dyn ArtifactStore>,
    loaded_env: BTreeMap<String, String>,
    fingerprint_key: [u8; 32],
    _lock_guard: RuntimeLockGuard,
}

impl Executor {
    pub fn new(
        workspace_root: impl AsRef<Path>,
        config: BroskiFile,
        store: Arc<dyn ArtifactStore>,
    ) -> Result<Self> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let loaded_env = load_env_files(&workspace_root, &config.load_env)?;
        let sweep = sweep_runtime_state(&workspace_root, true)?;
        if sweep.active_lock_detected {
            return Err(anyhow!("another Broski execution is active; aborting startup sweep"));
        }
        let lock_guard = acquire_runtime_lock(&workspace_root)?;
        let graph = TaskGraph::build(&config.task)?;
        let fingerprint_key = load_or_create_fingerprint_key(&workspace_root)?;

        Ok(Self {
            workspace_root,
            config,
            graph,
            store,
            loaded_env,
            fingerprint_key,
            _lock_guard: lock_guard,
        })
    }

    pub fn graph(&self) -> &TaskGraph {
        &self.graph
    }

    pub fn run_target(&self, target: &str, options: &RunOptions) -> Result<RunSummary> {
        if options.watch {
            self.run_target_watch(target, options)
        } else {
            self.run_target_once(target, options)
        }
    }

    fn run_target_once(&self, target: &str, options: &RunOptions) -> Result<RunSummary> {
        if options.force_isolation {
            if !cfg!(target_os = "linux") {
                return Err(anyhow!(
                    "--force-isolation requires Linux; strict sandbox execution is unsupported on this platform"
                ));
            }
            let _ = which::which("bwrap")
                .context("--force-isolation requires bubblewrap (`bwrap`) on PATH")?;
        }

        let resolved_target = self.config.resolve_task_name(target)?;
        let layers = self.graph.layers_for_target(&resolved_target)?;
        self.preflight_requires(&layers)?;
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
            let mut graph_tasks = Vec::new();
            let mut interactive_tasks = Vec::new();

            for task_name in layer {
                let task = self
                    .config
                    .task
                    .get(&task_name)
                    .ok_or_else(|| anyhow!("task '{}' not found", task_name))?;
                if task.inferred_mode() == TaskMode::Interactive {
                    interactive_tasks.push(task_name);
                } else {
                    graph_tasks.push(task_name);
                }
            }

            let graph_outcomes: Vec<Result<TaskOutcome>> = pool.install(|| {
                graph_tasks
                    .par_iter()
                    .map(|task_name| {
                        self.execute_task(task_name, options, progress_sender.as_ref().cloned())
                    })
                    .collect()
            });

            for outcome in graph_outcomes {
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
                apply_outcome(&mut summary, outcome);
            }

            for task_name in interactive_tasks {
                let outcome =
                    self.execute_task(&task_name, options, progress_sender.as_ref().cloned())?;
                apply_outcome(&mut summary, outcome);
            }
        }

        drop(progress_sender.take());
        if let Some(handle) = renderer.take() {
            let _ = handle.join();
        }

        Ok(summary)
    }

    fn run_target_watch(&self, target: &str, options: &RunOptions) -> Result<RunSummary> {
        let resolved_target = self.config.resolve_task_name(target)?;
        let watch_context = self.build_watch_context(&resolved_target)?;
        if self
            .config
            .task
            .get(&resolved_target)
            .is_some_and(|task| task.inferred_mode() == TaskMode::Interactive)
        {
            eprintln!(
                "info: task '{}' is interactive; internal watchers may conflict with --watch",
                resolved_target
            );
        }

        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
        let mut watcher = RecommendedWatcher::new(
            move |event| {
                let _ = tx.send(event);
            },
            NotifyConfig::default(),
        )
        .context("initializing file watcher")?;

        for path in &watch_context.watch_roots {
            watcher
                .watch(path, RecursiveMode::Recursive)
                .with_context(|| format!("watching path '{}'", path.display()))?;
        }

        let mut run_options = options.clone();
        run_options.watch = false;
        let mut last_summary = self.run_target_once(&resolved_target, &run_options)?;
        eprintln!("watch: listening for changes...");

        loop {
            match rx.recv() {
                Ok(Ok(event)) => {
                    if !event_has_relevant_change(&event, &watch_context) {
                        continue;
                    }
                    thread::sleep(Duration::from_millis(200));
                    while let Ok(Ok(event)) = rx.try_recv() {
                        if event_has_relevant_change(&event, &watch_context) {
                            // Drain bursty events before rerun.
                        }
                    }
                    eprintln!("watch: change detected, rerunning '{}'", resolved_target);
                    last_summary = self.run_target_once(&resolved_target, &run_options)?;
                }
                Ok(Err(error)) => {
                    eprintln!("watch: filesystem event error: {}", error);
                }
                Err(_) => break,
            }
        }

        Ok(last_summary)
    }

    fn preflight_requires(&self, layers: &[Vec<String>]) -> Result<()> {
        let mut checked = BTreeSet::new();
        for layer in layers {
            for task_name in layer {
                let task = self
                    .config
                    .task
                    .get(task_name)
                    .ok_or_else(|| anyhow!("task '{}' not found", task_name))?;
                for requirement in &task.requires {
                    if checked.insert(requirement.clone()) && which::which(requirement).is_err() {
                        return Err(anyhow!(
                            "task '{}' requires '{}', but it was not found on PATH",
                            task_name,
                            requirement
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn build_watch_context(&self, target: &str) -> Result<WatchContext> {
        let layers = self.graph.layers_for_target(target)?;
        let mut tracked_inputs = BTreeSet::new();
        let mut ignored_prefixes = BTreeSet::new();
        ignored_prefixes.insert(self.workspace_root.join(".git"));
        ignored_prefixes.insert(self.workspace_root.join(".broski"));

        for layer in &layers {
            for task_name in layer {
                let task = self
                    .config
                    .task
                    .get(task_name)
                    .ok_or_else(|| anyhow!("task '{}' not found", task_name))?;
                for output in &task.outputs {
                    let output_rel = normalize_relative_path(output)?;
                    ignored_prefixes.insert(self.workspace_root.join(output_rel));
                }
                if task.inputs.is_empty() {
                    continue;
                }
                let resolved = resolve_inputs(&self.workspace_root, &task.inputs)?;
                for input in resolved {
                    tracked_inputs.insert(self.workspace_root.join(input));
                }
            }
        }

        if tracked_inputs.is_empty() {
            tracked_inputs.insert(self.workspace_root.clone());
        }

        let mut watch_roots = BTreeSet::new();
        for input in &tracked_inputs {
            if input.is_dir() {
                watch_roots.insert(input.clone());
            } else if let Some(parent) = input.parent() {
                watch_roots.insert(parent.to_path_buf());
            }
        }
        if watch_roots.is_empty() {
            watch_roots.insert(self.workspace_root.clone());
        }

        Ok(WatchContext {
            watch_roots: watch_roots.into_iter().collect(),
            tracked_inputs: tracked_inputs.into_iter().collect(),
            ignored_prefixes: ignored_prefixes.into_iter().collect(),
        })
    }

    fn execute_task(
        &self,
        task_name: &str,
        options: &RunOptions,
        progress: Option<Sender<ProgressEvent>>,
    ) -> Result<TaskOutcome> {
        let task = self
            .config
            .task
            .get(task_name)
            .ok_or_else(|| anyhow!("task '{}' not found", task_name))?;
        let (task, passthrough_args) =
            self.resolve_task_with_params(task_name, task, &options.passthrough_args)?;
        let task_mode = task.inferred_mode();
        if !options.dry_run {
            self.require_task_confirmation(task_name, &task)?;
        }
        let show_progress = task_mode != TaskMode::Interactive;
        if show_progress {
            emit_progress(&progress, ProgressEvent::TaskStarted(task_name.to_string()));
        }
        let (resolved_env, secret_env_keys) = self.resolve_task_env(&task)?;
        let redactor = SecretRedactor::from_env(&resolved_env, &secret_env_keys);

        if task_mode == TaskMode::Interactive {
            if options.force_isolation {
                return Err(anyhow!(
                    "--force-isolation is not supported for interactive task '{}'",
                    task_name
                ));
            }

            if options.dry_run {
                if show_progress {
                    emit_progress(
                        &progress,
                        ProgressEvent::TaskFinished(
                            task_name.to_string(),
                            TaskProgressStatus::DryRun,
                        ),
                    );
                }
                return Ok(TaskOutcome {
                    task_name: task_name.to_string(),
                    from_cache: false,
                    dry_run: true,
                    cache_miss_reasons: if options.explain {
                        vec!["cache bypass: interactive mode".to_string()]
                    } else {
                        Vec::new()
                    },
                });
            }

            self.run_interactive_command(
                task_name,
                &task,
                &resolved_env,
                redactor.as_ref(),
                &passthrough_args,
            )
            .with_context(|| format!("executing interactive task '{}'", task_name))?;
            if show_progress {
                emit_progress(
                    &progress,
                    ProgressEvent::TaskFinished(
                        task_name.to_string(),
                        TaskProgressStatus::Executed,
                    ),
                );
            }
            return Ok(TaskOutcome {
                task_name: task_name.to_string(),
                from_cache: false,
                dry_run: false,
                cache_miss_reasons: if options.explain {
                    vec!["cache bypass: interactive mode".to_string()]
                } else {
                    Vec::new()
                },
            });
        }

        let outputs = normalize_outputs(&task)?;
        let stage_ro = normalize_paths(&task.stage_ro)?;
        let mut timings = TaskPhaseTimings::default();
        let show_timings = options.explain || profile_enabled();

        let started_resolve_inputs = Instant::now();
        let inputs = resolve_inputs(&self.workspace_root, &task.inputs)?;
        timings.resolve_inputs = Some(started_resolve_inputs.elapsed());

        let started_fingerprint = Instant::now();
        let fingerprint_result = compute_fingerprint(
            &self.workspace_root,
            task_name,
            &task,
            &inputs,
            &resolved_env,
            &secret_env_keys,
            FingerprintOptions {
                passthrough_args: &passthrough_args,
                secret_env_key: &self.fingerprint_key,
            },
        )?;
        timings.fingerprint = Some(started_fingerprint.elapsed());
        let mut cache_miss_reasons = Vec::new();

        if !options.force && !options.no_cache {
            if let Some(record) =
                self.store.fetch_execution(task_name, &fingerprint_result.fingerprint.0)?
            {
                if options.dry_run {
                    if show_progress {
                        emit_progress(
                            &progress,
                            ProgressEvent::TaskFinished(
                                task_name.to_string(),
                                TaskProgressStatus::DryRun,
                            ),
                        );
                    }
                    return Ok(TaskOutcome {
                        task_name: task_name.to_string(),
                        from_cache: true,
                        dry_run: true,
                        cache_miss_reasons: Vec::new(),
                    });
                }

                let started_restore = Instant::now();
                self.store
                    .restore_artifacts(&self.workspace_root, &record.artifacts)
                    .with_context(|| format!("restoring cache hit for task '{}'", task_name))?;
                timings.cache_restore = Some(started_restore.elapsed());

                if show_progress {
                    emit_progress(
                        &progress,
                        ProgressEvent::TaskFinished(
                            task_name.to_string(),
                            TaskProgressStatus::CacheHit,
                        ),
                    );
                }
                return Ok(TaskOutcome {
                    task_name: task_name.to_string(),
                    from_cache: true,
                    dry_run: false,
                    cache_miss_reasons: timing_lines(task_name, &timings, show_timings),
                });
            }
        }

        if options.explain {
            cache_miss_reasons =
                self.explain_cache_miss(task_name, options, &fingerprint_result.manifest)?;
        }

        if options.dry_run {
            if show_progress {
                emit_progress(
                    &progress,
                    ProgressEvent::TaskFinished(task_name.to_string(), TaskProgressStatus::DryRun),
                );
            }
            return Ok(TaskOutcome {
                task_name: task_name.to_string(),
                from_cache: false,
                dry_run: true,
                cache_miss_reasons,
            });
        }

        let started_stage = Instant::now();
        let stage = self.create_stage_snapshot(task_name, &inputs, &stage_ro)?;
        timings.stage_prep = Some(started_stage.elapsed());

        let started_command = Instant::now();
        let output = self
            .run_task_command(
                task_name,
                &task,
                stage.path(),
                &resolved_env,
                &passthrough_args,
                options,
            )
            .with_context(|| format!("executing task '{}'", task_name))?;
        timings.command_exec = Some(started_command.elapsed());
        let output = redact_output(output, redactor.as_ref());

        if !output.status.success() {
            if show_progress {
                emit_progress(
                    &progress,
                    ProgressEvent::TaskFinished(task_name.to_string(), TaskProgressStatus::Failed),
                );
            }
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

        let started_promote = Instant::now();
        self.promote_outputs(stage.path(), &outputs)
            .with_context(|| format!("promoting outputs for task '{}'", task_name))?;
        timings.output_promote = Some(started_promote.elapsed());

        if !options.no_cache {
            let started_store = Instant::now();
            let artifacts = self.store.store_artifacts(&self.workspace_root, &outputs)?;
            timings.cache_store = Some(started_store.elapsed());
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

        if show_progress {
            emit_progress(
                &progress,
                ProgressEvent::TaskFinished(task_name.to_string(), TaskProgressStatus::Executed),
            );
        }
        Ok(TaskOutcome {
            task_name: task_name.to_string(),
            from_cache: false,
            dry_run: false,
            cache_miss_reasons: {
                if show_timings {
                    cache_miss_reasons.extend(timing_lines(task_name, &timings, true));
                }
                cache_miss_reasons
            },
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

    fn create_stage_snapshot(
        &self,
        task_name: &str,
        inputs: &[PathBuf],
        stage_ro: &[PathBuf],
    ) -> Result<TempDir> {
        let stage_parent = self.workspace_root.join(".broski/stage");
        fs::create_dir_all(&stage_parent)
            .with_context(|| format!("creating stage parent '{}'", stage_parent.display()))?;

        let stage = tempfile::Builder::new()
            .prefix(&format!("{}-", task_name))
            .tempdir_in(&stage_parent)
            .with_context(|| format!("creating stage dir for task '{}'", task_name))?;

        copy_input_snapshot(&self.workspace_root, stage.path(), inputs)?;
        create_stage_readonly_links(&self.workspace_root, stage.path(), stage_ro)?;

        Ok(stage)
    }

    fn run_task_command(
        &self,
        _task_name: &str,
        task: &TaskSpec,
        stage_workspace: &Path,
        resolved_env: &BTreeMap<String, String>,
        passthrough_args: &[String],
        options: &RunOptions,
    ) -> Result<Output> {
        let isolation_mode = selected_isolation(task, options);
        let invocation = prepare_task_invocation(stage_workspace, task, passthrough_args)?;

        let mut command = match isolation_mode {
            IsolationMode::Strict if cfg!(target_os = "linux") => {
                let bwrap = which::which("bwrap").context(
                    "strict isolation requires bubblewrap (`bwrap`) to be installed on Linux",
                )?;
                let mut cmd = Command::new(bwrap);
                let home_dir = env::var_os("HOME").map(PathBuf::from);
                cmd.args(build_strict_bwrap_args(
                    stage_workspace,
                    &invocation,
                    home_dir.as_deref(),
                ));
                cmd
            }
            IsolationMode::Strict => {
                return Err(anyhow!(
                    "strict isolation is only supported on Linux in v0.1; use best_effort on this platform"
                ));
            }
            IsolationMode::BestEffort | IsolationMode::Off => {
                let mut cmd = Command::new(&invocation.program);
                cmd.args(&invocation.args);
                cmd
            }
        };

        command.current_dir(resolve_execution_dir(stage_workspace, task.working_dir.as_deref())?);

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

        for (key, value) in resolved_env {
            command.env(key, value);
        }

        command
            .output()
            .with_context(|| format!("spawning task command '{}'", invocation.display_command))
    }

    fn run_interactive_command(
        &self,
        task_name: &str,
        task: &TaskSpec,
        resolved_env: &BTreeMap<String, String>,
        redactor: Option<&SecretRedactor>,
        passthrough_args: &[String],
    ) -> Result<()> {
        let invocation = prepare_task_invocation(&self.workspace_root, task, passthrough_args)?;
        println!("[{task_name}] $ {}", invocation.display_command);

        let mut command = Command::new(&invocation.program);
        command.args(&invocation.args);
        command
            .current_dir(resolve_execution_dir(&self.workspace_root, task.working_dir.as_deref())?);
        command.stdin(Stdio::inherit());
        if redactor.is_some() {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }

        for (key, value) in resolved_env {
            command.env(key, value);
        }

        let status = if let Some(redactor) = redactor.cloned() {
            let mut child = command.spawn().with_context(|| {
                format!("spawning interactive task command '{}'", invocation.display_command)
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                anyhow!("interactive command stdout was not captured for redaction")
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                anyhow!("interactive command stderr was not captured for redaction")
            })?;
            let stdout_thread =
                spawn_redaction_thread(stdout, RedactionTarget::Stdout, redactor.clone());
            let stderr_thread = spawn_redaction_thread(stderr, RedactionTarget::Stderr, redactor);
            let status = child.wait().with_context(|| {
                format!("waiting for interactive task command '{}'", invocation.display_command)
            })?;

            stdout_thread
                .join()
                .map_err(|_| anyhow!("interactive stdout redaction thread panicked"))??;
            stderr_thread
                .join()
                .map_err(|_| anyhow!("interactive stderr redaction thread panicked"))??;
            status
        } else {
            command.status().with_context(|| {
                format!("spawning interactive task command '{}'", invocation.display_command)
            })?
        };
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("interactive task '{}' failed with status {}", task_name, status))
        }
    }

    fn resolve_task_with_params(
        &self,
        task_name: &str,
        task: &TaskSpec,
        cli_args: &[String],
    ) -> Result<(TaskSpec, Vec<String>)> {
        let mut resolved = task.clone();
        if task.params.is_empty() {
            return Ok((resolved, cli_args.to_vec()));
        }

        let mut bindings = BTreeMap::new();
        let mut cursor = 0usize;
        for param in &task.params {
            let value =
                cli_args.get(cursor).cloned().or_else(|| param.default.clone()).ok_or_else(
                    || {
                        anyhow!(
                            "task '{}' requires parameter '{}' (usage: {} {})",
                            task_name,
                            param.name,
                            task_name,
                            task.params
                                .iter()
                                .map(|item| {
                                    if item.default.is_some() {
                                        format!("[{}]", item.name)
                                    } else {
                                        format!("<{}>", item.name)
                                    }
                                })
                                .collect::<Vec<String>>()
                                .join(" ")
                        )
                    },
                )?;
            bindings.insert(param.name.clone(), value);
            cursor += 1;
        }

        let passthrough_tail = cli_args[cursor..].to_vec();

        for (key, value) in &bindings {
            resolved.resolved_variables.insert(key.clone(), value.clone());
        }

        resolved.inputs =
            resolved.inputs.iter().map(|value| apply_param_bindings(value, &bindings)).collect();
        resolved.stage_ro =
            resolved.stage_ro.iter().map(|value| apply_param_bindings(value, &bindings)).collect();
        resolved.outputs =
            resolved.outputs.iter().map(|value| apply_param_bindings(value, &bindings)).collect();
        resolved.env = resolved
            .env
            .iter()
            .map(|(key, value)| (key.clone(), apply_param_bindings(value, &bindings)))
            .collect();
        resolved.working_dir =
            resolved.working_dir.as_ref().map(|value| apply_param_bindings(value, &bindings));
        resolved.run = match &resolved.run {
            crate::model::RunSpec::Shell(command) => {
                crate::model::RunSpec::Shell(apply_param_bindings(command, &bindings))
            }
            crate::model::RunSpec::Args(args) => crate::model::RunSpec::Args(
                args.iter().map(|value| apply_param_bindings(value, &bindings)).collect(),
            ),
        };

        Ok((resolved, passthrough_tail))
    }

    fn require_task_confirmation(&self, task_name: &str, task: &TaskSpec) -> Result<()> {
        let Some(prompt) = task.confirm.as_deref() else {
            return Ok(());
        };
        if !io::stdin().is_terminal() {
            return Err(anyhow!(
                "task '{}' requires confirmation but stdin is not interactive",
                task_name
            ));
        }

        eprint!("{prompt} ");
        io::stderr().flush().context("flushing confirmation prompt")?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).context("reading confirmation response")?;
        let answer = answer.trim().to_ascii_lowercase();
        if answer == "y" || answer == "yes" {
            return Ok(());
        }
        Err(anyhow!("task '{}' aborted by user", task_name))
    }

    fn resolve_task_env(
        &self,
        task: &TaskSpec,
    ) -> Result<(BTreeMap<String, String>, BTreeSet<String>)> {
        let mut resolved = BTreeMap::new();
        let mut secret_keys = BTreeSet::new();
        let host_env: BTreeMap<String, String> = env::vars().collect();

        let mut inherit = BTreeSet::new();
        inherit.extend(task.env_inherit.iter().cloned());
        inherit.extend(task.secret_env.iter().cloned());

        for key in inherit {
            let value = self
                .loaded_env
                .get(&key)
                .cloned()
                .or_else(|| host_env.get(&key).cloned())
                .ok_or_else(|| anyhow!("environment variable '{}' is required but missing", key))?;
            resolved.insert(key, value);
        }

        for (key, value) in &task.env {
            resolved.insert(key.clone(), value.clone());
        }

        for key in &task.secret_env {
            secret_keys.insert(key.clone());
        }

        Ok((resolved, secret_keys))
    }

    fn promote_outputs(&self, stage_workspace: &Path, outputs: &[PathBuf]) -> Result<()> {
        let tx_parent = self.workspace_root.join(".broski/tx");
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

fn apply_outcome(summary: &mut RunSummary, outcome: TaskOutcome) {
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

fn event_has_relevant_change(event: &Event, watch_context: &WatchContext) -> bool {
    for path in &event.paths {
        if watch_context.ignored_prefixes.iter().any(|prefix| path.starts_with(prefix)) {
            continue;
        }
        if watch_context.tracked_inputs.iter().any(|input| path.starts_with(input)) {
            return true;
        }
    }
    false
}

#[derive(Clone)]
struct SecretRedactor {
    matcher: AhoCorasick,
    replacements: Vec<String>,
}

impl SecretRedactor {
    fn from_env(
        resolved_env: &BTreeMap<String, String>,
        secret_env_keys: &BTreeSet<String>,
    ) -> Option<Self> {
        let mut patterns = Vec::new();
        for key in secret_env_keys {
            if let Some(value) = resolved_env.get(key) {
                let trimmed = value.trim();
                if trimmed.len() >= 6 {
                    patterns.push(trimmed.to_string());
                }
            }
        }
        patterns.sort();
        patterns.dedup();
        if patterns.is_empty() {
            return None;
        }
        let matcher = AhoCorasick::new(&patterns).ok()?;
        let replacements = vec!["[REDACTED]".to_string(); patterns.len()];
        Some(Self { matcher, replacements })
    }

    fn redact_text(&self, input: &str) -> String {
        let replacements: Vec<&str> = self.replacements.iter().map(String::as_str).collect();
        self.matcher.replace_all(input, &replacements)
    }
}

fn redact_output(mut output: Output, redactor: Option<&SecretRedactor>) -> Output {
    let Some(redactor) = redactor else {
        return output;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    output.stdout = redactor.redact_text(&stdout).into_bytes();
    output.stderr = redactor.redact_text(&stderr).into_bytes();
    output
}

struct TaskInvocation {
    display_command: String,
    program: PathBuf,
    args: Vec<String>,
    _temp_script: Option<NamedTempFile>,
}

fn prepare_task_invocation(
    execution_root: &Path,
    task: &TaskSpec,
    passthrough_args: &[String],
) -> Result<TaskInvocation> {
    let run_command = task.run_as_shell();
    if looks_like_shebang(&run_command) {
        let script = create_temp_shebang_script(execution_root, &run_command)?;
        let display_command = if passthrough_args.is_empty() {
            script.path().display().to_string()
        } else {
            format!(
                "{} {}",
                script.path().display(),
                passthrough_args
                    .iter()
                    .map(|part| shell_escape(part))
                    .collect::<Vec<String>>()
                    .join(" ")
            )
        };
        return Ok(TaskInvocation {
            display_command,
            program: script.path().to_path_buf(),
            args: passthrough_args.to_vec(),
            _temp_script: Some(script),
        });
    }

    let shell_command = build_shell_command(&run_command, passthrough_args);

    #[cfg(target_os = "windows")]
    {
        if task.shell_override.is_none() {
            let script = create_temp_powershell_script(execution_root, &shell_command)?;
            let program = resolve_windows_powershell_program()?;
            let args = vec![
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-NoProfile".to_string(),
                "-File".to_string(),
                script.path().display().to_string(),
            ];
            return Ok(TaskInvocation {
                display_command: shell_command,
                program,
                args,
                _temp_script: Some(script),
            });
        }
    }

    let (program, mut args) = resolve_shell_command(task.shell_override.as_ref())?;
    args.push(shell_command.clone());
    Ok(TaskInvocation { display_command: shell_command, program, args, _temp_script: None })
}

fn create_temp_shebang_script(execution_root: &Path, script_body: &str) -> Result<NamedTempFile> {
    let script_dir = execution_root.join(".broski/tmp");
    fs::create_dir_all(&script_dir)
        .with_context(|| format!("creating shebang temp directory '{}'", script_dir.display()))?;
    let mut script = tempfile::Builder::new()
        .prefix("broski-script-")
        .tempfile_in(&script_dir)
        .with_context(|| format!("creating shebang temp script in '{}'", script_dir.display()))?;
    script
        .as_file_mut()
        .write_all(script_body.as_bytes())
        .context("writing shebang script body")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms =
            script.as_file().metadata().context("reading shebang script metadata")?.permissions();
        perms.set_mode(0o700);
        script
            .as_file()
            .set_permissions(perms)
            .context("setting shebang script executable permissions")?;
    }
    Ok(script)
}

#[cfg(target_os = "windows")]
fn create_temp_powershell_script(
    execution_root: &Path,
    script_body: &str,
) -> Result<NamedTempFile> {
    let script_dir = execution_root.join(".broski/tmp");
    fs::create_dir_all(&script_dir).with_context(|| {
        format!("creating powershell temp directory '{}'", script_dir.display())
    })?;
    let mut script = tempfile::Builder::new()
        .prefix("broski-script-")
        .suffix(".ps1")
        .tempfile_in(&script_dir)
        .with_context(|| {
            format!("creating powershell temp script in '{}'", script_dir.display())
        })?;
    script
        .as_file_mut()
        .write_all(script_body.as_bytes())
        .context("writing powershell script body")?;
    Ok(script)
}

fn resolve_shell_command(
    shell_override: Option<&crate::model::ShellSpec>,
) -> Result<(PathBuf, Vec<String>)> {
    if let Some(shell_spec) = shell_override {
        if shell_spec.program.trim().is_empty() {
            return Err(anyhow!("shell override program cannot be empty"));
        }
        return Ok((PathBuf::from(shell_spec.program.clone()), shell_spec.args.clone()));
    }

    #[cfg(target_os = "windows")]
    {
        Ok((
            resolve_windows_powershell_program()?,
            vec!["-NoProfile".to_string(), "-Command".to_string()],
        ))
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok((PathBuf::from("/bin/sh"), vec!["-lc".to_string()]))
    }
}

fn looks_like_shebang(script: &str) -> bool {
    let first_non_empty = script.lines().find(|line| !line.trim().is_empty());
    first_non_empty.is_some_and(|line| line.trim_start().starts_with("#!"))
}

fn build_shell_command(run_command: &str, passthrough_args: &[String]) -> String {
    let mut command = run_command.to_string();
    if !passthrough_args.is_empty() {
        let joined = passthrough_args
            .iter()
            .map(|part| shell_escape(part))
            .collect::<Vec<String>>()
            .join(" ");
        command.push(' ');
        command.push_str(&joined);
    }
    command
}

#[cfg(target_os = "windows")]
fn resolve_windows_powershell_program() -> Result<PathBuf> {
    if let Ok(pwsh) = which::which("pwsh") {
        return Ok(pwsh);
    }
    if let Ok(powershell) = which::which("powershell") {
        return Ok(powershell);
    }
    Err(anyhow!("failed to locate PowerShell; install 'pwsh' (PowerShell Core) or 'powershell'"))
}

fn apply_param_bindings(input: &str, bindings: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(rel_start) = input[cursor..].find("{{") {
        let start = cursor + rel_start;
        output.push_str(&input[cursor..start]);
        let open_end = start + 2;
        let Some(rel_close) = input[open_end..].find("}}") else {
            output.push_str(&input[start..]);
            return output;
        };
        let close = open_end + rel_close;
        let key = input[open_end..close].trim();
        if let Some(value) = bindings.get(key) {
            output.push_str(value);
        } else {
            output.push_str(&input[start..close + 2]);
        }
        cursor = close + 2;
    }
    output.push_str(&input[cursor..]);
    output
}

fn shell_escape(input: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        if input
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))
        {
            return input.to_string();
        }
        return format!("'{}'", input.replace('\'', "''"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        if input
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))
        {
            return input.to_string();
        }
        format!("'{}'", input.replace('\'', "'\"'\"'"))
    }
}

fn resolve_execution_dir(root: &Path, working_dir: Option<&str>) -> Result<PathBuf> {
    let Some(dir) = working_dir else {
        return Ok(root.to_path_buf());
    };
    let normalized = normalize_relative_path(dir)?;
    Ok(root.join(normalized))
}

fn load_env_files(workspace_root: &Path, files: &[String]) -> Result<BTreeMap<String, String>> {
    let mut env_map = BTreeMap::new();
    for file in files {
        let rel = normalize_relative_path(file)
            .with_context(|| format!("invalid @load path '{}'", file))?;
        let path = workspace_root.join(rel);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("reading env file '{}'", path.display()))?;
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(anyhow!(
                    "invalid env line in '{}':{}; expected KEY=VALUE",
                    path.display(),
                    idx + 1
                ));
            };
            let key = key.trim();
            if key.is_empty() {
                return Err(anyhow!(
                    "invalid env key in '{}':{}; key cannot be empty",
                    path.display(),
                    idx + 1
                ));
            }
            env_map.insert(key.to_string(), value.trim().to_string());
        }
    }
    Ok(env_map)
}

fn load_or_create_fingerprint_key(workspace_root: &Path) -> Result<[u8; 32]> {
    let config_dir = workspace_root.join(".broski/config");
    fs::create_dir_all(&config_dir).with_context(|| {
        format!("creating fingerprint key config directory '{}'", config_dir.display())
    })?;
    let key_path = config_dir.join("salt");
    if key_path.exists() {
        let bytes = fs::read(&key_path)
            .with_context(|| format!("reading fingerprint key file '{}'", key_path.display()))?;
        if bytes.len() != 32 {
            return Err(anyhow!(
                "fingerprint key '{}' must contain exactly 32 bytes; found {}",
                key_path.display(),
                bytes.len()
            ));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(key);
    }

    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);

    let mut temp = tempfile::Builder::new()
        .prefix("fingerprint-salt-")
        .tempfile_in(&config_dir)
        .with_context(|| format!("creating temp fingerprint key in '{}'", config_dir.display()))?;
    temp.as_file_mut()
        .write_all(&key)
        .with_context(|| format!("writing temp fingerprint key in '{}'", config_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = temp
            .as_file()
            .metadata()
            .context("reading fingerprint key temp metadata")?
            .permissions();
        perms.set_mode(0o600);
        temp.as_file().set_permissions(perms).context("setting fingerprint key permissions")?;
    }
    temp.persist(&key_path)
        .map_err(|error| error.error)
        .with_context(|| format!("persisting fingerprint key to '{}'", key_path.display()))?;
    Ok(key)
}

fn normalize_outputs(task: &TaskSpec) -> Result<Vec<PathBuf>> {
    let mut outputs = Vec::with_capacity(task.outputs.len());
    for output in &task.outputs {
        outputs.push(normalize_relative_path(output)?);
    }
    Ok(outputs)
}

fn normalize_paths(raw_paths: &[String]) -> Result<Vec<PathBuf>> {
    let mut normalized = Vec::with_capacity(raw_paths.len());
    for item in raw_paths {
        normalized.push(normalize_relative_path(item)?);
    }
    Ok(normalized)
}

fn copy_input_snapshot(source_root: &Path, stage_root: &Path, inputs: &[PathBuf]) -> Result<()> {
    let mut copied = BTreeSet::new();
    for rel in inputs {
        if !copied.insert(rel.clone()) {
            continue;
        }
        let source = source_root.join(rel);
        if !source.exists() {
            continue;
        }
        if source.is_file() || source.is_symlink() {
            copy_snapshot_file_or_link(source_root, stage_root, &source)?;
            continue;
        }
        if source.is_dir() {
            for entry in WalkDir::new(&source)
                .into_iter()
                .filter_entry(|entry| should_include(entry, source_root))
            {
                let entry = entry.context("walking input snapshot directory")?;
                let path = entry.path();
                if path.is_dir() {
                    continue;
                }
                copy_snapshot_file_or_link(source_root, stage_root, path)?;
            }
        }
    }
    Ok(())
}

fn copy_snapshot_file_or_link(source_root: &Path, stage_root: &Path, source: &Path) -> Result<()> {
    let rel = source
        .strip_prefix(source_root)
        .with_context(|| format!("stripping workspace prefix '{}'", source_root.display()))?;
    let target = stage_root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent '{}'", parent.display()))?;
    }
    if source.is_symlink() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target_link = fs::read_link(source)
                .with_context(|| format!("reading symlink '{}'", source.display()))?;
            symlink(target_link, &target)
                .with_context(|| format!("creating symlink '{}'", target.display()))?;
            return Ok(());
        }
    }
    copy_file_with_reflink_fallback(source, &target)
        .with_context(|| format!("copying workspace file '{}' to stage", source.display()))
}

fn create_stage_readonly_links(
    source_root: &Path,
    stage_root: &Path,
    stage_ro: &[PathBuf],
) -> Result<()> {
    for rel in stage_ro {
        let src = source_root.join(rel);
        if !src.exists() {
            return Err(anyhow!("@stage_ro path '{}' does not exist", rel.display()));
        }
        let dst = stage_root.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent '{}'", parent.display()))?;
        }
        remove_path_if_exists(&dst)?;
        create_readonly_link_or_copy(&src, &dst)?;
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
        Some(Component::Normal(part)) if part == ".broski" || part == ".git"
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

fn create_readonly_link_or_copy(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        if symlink(src, dst).is_ok() {
            return Ok(());
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let result = if src.is_dir() { symlink_dir(src, dst) } else { symlink_file(src, dst) };
        if result.is_ok() {
            return Ok(());
        }
    }

    copy_tree(src, dst)
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

fn profile_enabled() -> bool {
    env::var("BROSKI_PROFILE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn timing_lines(task_name: &str, timings: &TaskPhaseTimings, enabled: bool) -> Vec<String> {
    if !enabled {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if let Some(value) = timings.resolve_inputs {
        lines.push(format!("timing: resolve_inputs={}ms", value.as_millis()));
    }
    if let Some(value) = timings.fingerprint {
        lines.push(format!("timing: fingerprint={}ms", value.as_millis()));
    }
    if let Some(value) = timings.cache_restore {
        lines.push(format!("timing: cache_restore={}ms", value.as_millis()));
    }
    if let Some(value) = timings.stage_prep {
        lines.push(format!("timing: stage_prep={}ms", value.as_millis()));
    }
    if let Some(value) = timings.command_exec {
        lines.push(format!("timing: command_exec={}ms", value.as_millis()));
    }
    if let Some(value) = timings.output_promote {
        lines.push(format!("timing: output_promote={}ms", value.as_millis()));
    }
    if let Some(value) = timings.cache_store {
        lines.push(format!("timing: cache_store={}ms", value.as_millis()));
    }
    if profile_enabled() && !lines.is_empty() {
        eprintln!("profile {}:", task_name);
        for line in &lines {
            eprintln!("- {}", line);
        }
    }
    lines
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
    if key.starts_with("secret_env:") {
        return "cache miss: secret env changed".to_string();
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

enum RedactionTarget {
    Stdout,
    Stderr,
}

fn spawn_redaction_thread<R: io::Read + Send + 'static>(
    stream: R,
    target: RedactionTarget,
    redactor: SecretRedactor,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || stream_redacted_output(stream, target, &redactor))
}

fn stream_redacted_output<R: io::Read>(
    stream: R,
    target: RedactionTarget,
    redactor: &SecretRedactor,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).context("reading interactive output stream")?;
        if bytes == 0 {
            break;
        }
        let redacted = redactor.redact_text(&line);
        match target {
            RedactionTarget::Stdout => {
                let mut stdout = io::stdout().lock();
                stdout
                    .write_all(redacted.as_bytes())
                    .context("writing redacted interactive stdout")?;
                stdout.flush().context("flushing redacted interactive stdout")?;
            }
            RedactionTarget::Stderr => {
                let mut stderr = io::stderr().lock();
                stderr
                    .write_all(redacted.as_bytes())
                    .context("writing redacted interactive stderr")?;
                stderr.flush().context("flushing redacted interactive stderr")?;
            }
        }
    }
    Ok(())
}

fn build_strict_bwrap_args(
    stage_workspace: &Path,
    invocation: &TaskInvocation,
    home_dir: Option<&Path>,
) -> Vec<OsString> {
    let mut args = vec![
        "--die-with-parent".into(),
        "--new-session".into(),
        "--unshare-net".into(),
        "--ro-bind".into(),
        "/usr".into(),
        "/usr".into(),
        "--ro-bind".into(),
        "/bin".into(),
        "/bin".into(),
        "--ro-bind".into(),
        "/lib".into(),
        "/lib".into(),
    ];
    if Path::new("/lib64").exists() {
        args.push("--ro-bind".into());
        args.push("/lib64".into());
        args.push("/lib64".into());
    }
    args.push("--dir".into());
    args.push("/etc".into());
    args.push("--ro-bind-try".into());
    args.push("/etc/resolv.conf".into());
    args.push("/etc/resolv.conf".into());

    if let Some(home_dir) = home_dir.filter(|path| path.is_absolute()) {
        append_bwrap_home_mask_args(&mut args, home_dir);
    }

    args.push("--bind".into());
    args.push(stage_workspace.as_os_str().to_os_string());
    args.push(stage_workspace.as_os_str().to_os_string());
    args.push("--proc".into());
    args.push("/proc".into());
    args.push("--dev".into());
    args.push("/dev".into());
    args.push("--tmpfs".into());
    args.push("/tmp".into());
    args.push("--chdir".into());
    args.push(stage_workspace.as_os_str().to_os_string());
    args.push(invocation.program.as_os_str().to_os_string());
    args.extend(invocation.args.iter().map(OsString::from));
    args
}

fn append_bwrap_home_mask_args(args: &mut Vec<OsString>, home_dir: &Path) {
    if home_dir == Path::new("/") {
        return;
    }
    let mut cursor = PathBuf::from("/");
    for component in home_dir.components() {
        if let Component::Normal(segment) = component {
            cursor.push(segment);
            args.push("--dir".into());
            args.push(cursor.as_os_str().to_os_string());
        }
    }
    args.push("--tmpfs".into());
    args.push(home_dir.as_os_str().to_os_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BroskiSection, RunSpec};
    use blake3::Hasher;
    use broski_cache::LocalArtifactStore;
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::Read;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    #[cfg(target_os = "linux")]
    use std::process::Command as ProcessCommand;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn simple_task(command: &str) -> TaskSpec {
        TaskSpec {
            deps: vec![],
            description: None,
            resolved_variables: BTreeMap::new(),
            inputs: vec!["src/input.txt".to_string()],
            stage_ro: vec![],
            outputs: vec!["dist/output.txt".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell(command.to_string()),
            isolation: Some(IsolationMode::BestEffort),
            mode: Some(TaskMode::Graph),
            working_dir: None,
            params: Vec::new(),
            private: false,
            confirm: None,
            shell_override: None,
            requires: Vec::new(),
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

        let config = BroskiFile {
            broski: BroskiSection { version: "0.2".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };

        let cache = LocalArtifactStore::new(workspace.join(".broski/cache")).expect("create cache");
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
        copy_input_snapshot(&workspace, stage.path(), &[PathBuf::from("data/large.bin")])
            .expect("copy workspace snapshot");
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

    #[test]
    fn stage_snapshot_copies_only_declared_inputs() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).expect("create src dir");
        fs::create_dir_all(workspace.join("tmp")).expect("create tmp dir");
        fs::write(workspace.join("src/input.txt"), "hello").expect("write input");
        fs::write(workspace.join("tmp/other.txt"), "skip").expect("write other");

        let stage = tempfile::tempdir_in(tmp.path()).expect("create stage dir");
        copy_input_snapshot(&workspace, stage.path(), &[PathBuf::from("src/input.txt")])
            .expect("copy input snapshot");

        assert!(stage.path().join("src/input.txt").exists());
        assert!(!stage.path().join("tmp/other.txt").exists());
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

    #[test]
    fn fails_fast_when_required_tool_is_missing() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::write(workspace.join("src/input.txt"), "hello").expect("write input");

        let mut task = simple_task("mkdir -p dist && cp src/input.txt dist/output.txt");
        task.requires = vec!["broski-missing-tool-binary".to_string()];
        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), task);

        let config = BroskiFile {
            broski: BroskiSection { version: "0.4".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };
        let cache = LocalArtifactStore::new(workspace.join(".broski/cache")).expect("create cache");
        let executor = Executor::new(&workspace, config, Arc::new(cache)).expect("create executor");

        let error = executor
            .run_target("build", &RunOptions::default())
            .expect_err("missing requirement should fail");
        assert!(error.to_string().contains("requires 'broski-missing-tool-binary'"));
    }

    #[test]
    fn redacts_secret_values_in_output() {
        let redactor = SecretRedactor::from_env(
            &BTreeMap::from([("TOKEN".to_string(), "supersecret".to_string())]),
            &BTreeSet::from(["TOKEN".to_string()]),
        )
        .expect("redactor");

        let output = Output {
            status: success_exit_status(),
            stdout: b"token=supersecret".to_vec(),
            stderr: b"err supersecret".to_vec(),
        };
        let redacted = redact_output(output, Some(&redactor));
        let stdout = String::from_utf8_lossy(&redacted.stdout);
        let stderr = String::from_utf8_lossy(&redacted.stderr);
        assert!(!stdout.contains("supersecret"));
        assert!(!stderr.contains("supersecret"));
        assert!(stdout.contains("[REDACTED]"));
        assert!(stderr.contains("[REDACTED]"));
    }

    #[test]
    fn redactor_ignores_short_values() {
        let redactor = SecretRedactor::from_env(
            &BTreeMap::from([
                ("TOO_SHORT".to_string(), "1".to_string()),
                ("TOKEN".to_string(), "supersecret".to_string()),
            ]),
            &BTreeSet::from(["TOO_SHORT".to_string(), "TOKEN".to_string()]),
        )
        .expect("redactor");

        let output = Output {
            status: success_exit_status(),
            stdout: b"value=1 token=supersecret".to_vec(),
            stderr: Vec::new(),
        };
        let redacted = redact_output(output, Some(&redactor));
        let stdout = String::from_utf8_lossy(&redacted.stdout);
        assert!(stdout.contains("value=1"));
        assert!(!stdout.contains("supersecret"));
        assert!(stdout.contains("token=[REDACTED]"));
    }

    #[test]
    fn fingerprint_key_is_created_and_reused() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let workspace = tmp.path();
        let first = load_or_create_fingerprint_key(workspace).expect("first key");
        let second = load_or_create_fingerprint_key(workspace).expect("second key");
        assert_eq!(first, second);
        let bytes =
            fs::read(workspace.join(".broski/config/salt")).expect("read fingerprint key file");
        assert_eq!(bytes.len(), 32);
    }

    fn success_exit_status() -> std::process::ExitStatus {
        #[cfg(unix)]
        {
            std::process::ExitStatus::from_raw(0)
        }
        #[cfg(windows)]
        {
            std::process::ExitStatus::from_raw(0)
        }
    }

    #[test]
    fn shebang_invocation_uses_temp_script_and_cleans_up() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let task = TaskSpec {
            deps: vec![],
            description: None,
            resolved_variables: BTreeMap::new(),
            inputs: vec![],
            stage_ro: vec![],
            outputs: vec![],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("#!/usr/bin/env sh\necho hello".to_string()),
            isolation: Some(IsolationMode::Off),
            mode: Some(TaskMode::Interactive),
            working_dir: None,
            params: Vec::new(),
            private: false,
            confirm: None,
            shell_override: None,
            requires: Vec::new(),
        };

        let invocation = prepare_task_invocation(tmp.path(), &task, &[]).expect("invocation");
        let script_path = invocation.program.clone();
        assert!(script_path.exists(), "expected temporary shebang script to exist");
        drop(invocation);
        assert!(
            !script_path.exists(),
            "temporary shebang script should be removed when invocation is dropped"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn default_shell_is_posix_sh() {
        let (program, args) = resolve_shell_command(None).expect("resolve shell");
        assert_eq!(program, PathBuf::from("/bin/sh"));
        assert_eq!(args, vec!["-lc".to_string()]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn default_shell_prefers_pwsh_then_windows_powershell() {
        let (program, args) = resolve_shell_command(None).expect("resolve shell");
        let name = program.file_name().and_then(|part| part.to_str()).unwrap_or_default();
        assert!(
            name.eq_ignore_ascii_case("pwsh.exe")
                || name.eq_ignore_ascii_case("pwsh")
                || name.eq_ignore_ascii_case("powershell.exe")
                || name.eq_ignore_ascii_case("powershell"),
            "unexpected shell program: {}",
            program.display()
        );
        assert_eq!(args, vec!["-NoProfile".to_string(), "-Command".to_string()]);
    }

    #[test]
    fn strict_bwrap_args_limit_mount_surface() {
        let stage = PathBuf::from("/tmp/broski-stage");
        let home = PathBuf::from("/home/tester");
        let invocation = TaskInvocation {
            display_command: "echo hi".to_string(),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-lc".to_string(), "echo hi".to_string()],
            _temp_script: None,
        };
        let args = build_strict_bwrap_args(&stage, &invocation, Some(home.as_path()));
        let rendered: Vec<String> =
            args.iter().map(|arg| arg.to_string_lossy().to_string()).collect();

        assert!(rendered.windows(3).any(|w| w == ["--ro-bind", "/usr", "/usr"]));
        assert!(rendered.windows(3).any(|w| w == ["--ro-bind", "/bin", "/bin"]));
        assert!(rendered.windows(3).any(|w| w == ["--ro-bind", "/lib", "/lib"]));
        assert!(rendered.windows(2).any(|w| w == ["--tmpfs", "/home/tester"]));
        assert!(!rendered.windows(3).any(|w| w == ["--ro-bind", "/", "/"]));
    }

    #[cfg(target_os = "linux")]
    fn strict_bwrap_supported() -> bool {
        let Ok(bwrap) = which::which("bwrap") else {
            return false;
        };
        let stage = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(_) => return false,
        };
        let invocation = TaskInvocation {
            display_command: "echo BROSKI_BWRAP_TEST".to_string(),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-lc".to_string(), "echo BROSKI_BWRAP_TEST".to_string()],
            _temp_script: None,
        };
        let args = build_strict_bwrap_args(
            stage.path(),
            &invocation,
            env::var_os("HOME").as_deref().map(Path::new),
        );

        let Ok(output) = ProcessCommand::new(bwrap).args(args).output() else {
            return false;
        };

        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("BROSKI_BWRAP_TEST")
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
                description: None,
                resolved_variables: BTreeMap::new(),
                inputs: vec!["src/input.txt".to_string()],
                stage_ro: vec![],
                outputs: vec!["dist/output.txt".to_string()],
                env: BTreeMap::new(),
                env_inherit: Vec::new(),
                secret_env: Vec::new(),
                run: RunSpec::Shell(
                    "mkdir -p dist && cp src/input.txt dist/output.txt".to_string(),
                ),
                isolation: Some(IsolationMode::Strict),
                mode: Some(TaskMode::Graph),
                working_dir: None,
                params: Vec::new(),
                private: false,
                confirm: None,
                shell_override: None,
                requires: Vec::new(),
            },
        );

        let config = BroskiFile {
            broski: BroskiSection { version: "0.2".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };
        let cache = LocalArtifactStore::new(workspace.join(".broski/cache")).expect("create cache");
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
                description: None,
                resolved_variables: BTreeMap::new(),
                inputs: vec!["src/input.txt".to_string()],
                stage_ro: vec![],
                outputs: vec!["dist/output.txt".to_string()],
                env: BTreeMap::new(),
                env_inherit: Vec::new(),
                secret_env: Vec::new(),
                run: RunSpec::Shell(
                    "mkdir -p dist && cp src/input.txt dist/output.txt".to_string(),
                ),
                isolation: Some(IsolationMode::Off),
                mode: Some(TaskMode::Graph),
                working_dir: None,
                params: Vec::new(),
                private: false,
                confirm: None,
                shell_override: None,
                requires: Vec::new(),
            },
        );

        let config = BroskiFile {
            broski: BroskiSection { version: "0.2".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };
        let cache = LocalArtifactStore::new(workspace.join(".broski/cache")).expect("create cache");
        let executor = Executor::new(&workspace, config, Arc::new(cache)).expect("create executor");

        let result = executor
            .run_target("build", &RunOptions { force_isolation: true, ..RunOptions::default() });
        assert!(result.is_err());
    }
}
