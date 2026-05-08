//! End-to-end test of the data path the TUI relies on:
//! Executor with `event_sink + capture_output` -> mpsc -> `TuiState::apply`.
//!
//! Doesn't open a terminal — that's the only piece app::run adds on top of
//! this pipeline.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use broski_cache::LocalArtifactStore;
use broski_core::model::BroskiSection;
use broski_core::{
    BroskiFile, Executor, IsolationMode, ProgressEvent, RunOptions, RunSpec, TaskMode, TaskSpec,
};
use broski_store::{ArtifactStore, ExecutionRecord};
use broski_tui::state::{TaskState, TuiState};

fn graph_task(cmd: &str, output: &str) -> TaskSpec {
    TaskSpec {
        deps: vec![],
        description: None,
        resolved_variables: BTreeMap::new(),
        inputs: vec!["src/input.txt".to_string()],
        stage_ro: vec![],
        outputs: vec![output.to_string()],
        env: BTreeMap::new(),
        env_inherit: Vec::new(),
        secret_env: Vec::new(),
        run: RunSpec::Shell(cmd.to_string()),
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
fn executor_event_stream_folds_into_tui_state_with_logs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(workspace.join("src")).expect("src");
    let mut input = std::fs::File::create(workspace.join("src/input.txt")).expect("input");
    input.write_all(b"hi").expect("write input");

    let mut tasks = BTreeMap::new();
    tasks.insert(
        "build".to_string(),
        graph_task(
            "mkdir -p dist && echo TUI_MARKER_OUT && echo TUI_MARKER_ERR 1>&2 && echo ok > dist/out.txt",
            "dist/out.txt",
        ),
    );

    let config = BroskiFile {
        broski: BroskiSection { version: "0.5".to_string() },
        task: tasks,
        alias: BTreeMap::new(),
        load_env: Vec::new(),
    };
    let cache = LocalArtifactStore::new(workspace.join(".broski/cache")).expect("cache");
    let executor = Executor::new(&workspace, config, Arc::new(cache)).expect("executor");

    let (tx, rx) = mpsc::channel::<ProgressEvent>();
    let opts = RunOptions { event_sink: Some(tx), capture_output: true, ..RunOptions::default() };
    executor.run_target("build", &opts).expect("run ok");
    drop(opts);

    let mut state = TuiState::new();
    while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_millis(500)) {
        state.apply(ev);
    }

    assert_eq!(state.target.as_deref(), Some("build"));
    assert!(state.run_finished, "run_finished should be true once RunFinished arrives");
    assert_eq!(state.task_order, vec!["build"]);
    let info = state.tasks.get("build").expect("task info present");
    assert_eq!(info.state, TaskState::Done);
    assert_eq!(state.done_count, 1);
    assert_eq!(state.failed_count, 0);
    assert!(info.logs.iter().any(|r| r.line.contains("TUI_MARKER_OUT")));
    assert!(info.logs.iter().any(|r| r.line.contains("TUI_MARKER_ERR")));
}

#[test]
fn second_run_records_duration_for_eta_consumption() {
    // Run a task twice; the second run's ExecutionRecord must carry a
    // non-zero duration_ms — that's what the TUI's prefetch_etas reads.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(workspace.join("src")).expect("src");
    let mut input = std::fs::File::create(workspace.join("src/input.txt")).expect("input");
    input.write_all(b"hi").expect("write input");

    let mut tasks = BTreeMap::new();
    tasks.insert(
        "build".to_string(),
        graph_task("mkdir -p dist && echo ok > dist/out.txt", "dist/out.txt"),
    );

    let config = BroskiFile {
        broski: BroskiSection { version: "0.5".to_string() },
        task: tasks,
        alias: BTreeMap::new(),
        load_env: Vec::new(),
    };
    let cache = Arc::new(LocalArtifactStore::new(workspace.join(".broski/cache")).expect("cache"));
    let executor = Executor::new(&workspace, config, cache.clone() as Arc<dyn ArtifactStore>)
        .expect("executor");
    executor.run_target("build", &RunOptions::default()).expect("run ok");

    let record =
        cache.fetch_latest_execution("build").expect("fetch latest").expect("record exists");
    // Even a trivial echo should land in the millisecond range; 0 means the
    // executor didn't populate the field.
    assert!(record.duration_ms > 0, "expected non-zero duration_ms, got {}", record.duration_ms);

    // And feeding that into a fresh TuiState's etas seeds remaining_eta
    // before any RunStarted event arrives.
    let mut etas = BTreeMap::new();
    etas.insert("build".to_string(), Duration::from_millis(record.duration_ms));
    let state = TuiState::with_etas(etas);
    assert_eq!(state.etas.get("build").copied(), Some(Duration::from_millis(record.duration_ms)));
}

#[test]
fn prefetch_skips_tasks_without_history() {
    // A task with no prior run yields no ETA — the TUI shows just elapsed.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let cache = LocalArtifactStore::new(workspace.join(".broski/cache")).expect("cache");
    let result = cache.fetch_latest_execution("never_ran").expect("fetch");
    assert!(result.is_none());

    // Sanity: zero-duration records don't register either (TUI's filter:
    // `if record.duration_ms > 0`).
    let zero = ExecutionRecord {
        task_name: "stale".to_string(),
        fingerprint: "fp".to_string(),
        manifest: BTreeMap::new(),
        artifacts: vec![],
        stdout: "".to_string(),
        stderr: "".to_string(),
        created_at: 0,
        duration_ms: 0,
    };
    cache.save_execution(&zero).expect("save");
    let fetched = cache.fetch_latest_execution("stale").expect("fetch").expect("present");
    assert_eq!(fetched.duration_ms, 0);
}
