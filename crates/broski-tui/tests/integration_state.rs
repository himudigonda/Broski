//! End-to-end test of the data path the TUI relies on:
//! Executor with `event_sink + capture_output` -> mpsc -> `TuiState::apply`.
//!
//! Doesn't open a terminal — that's the only piece app::run adds on top of
//! this pipeline.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{mpsc, Arc};

use broski_cache::LocalArtifactStore;
use broski_core::model::BroskiSection;
use broski_core::{
    BroskiFile, Executor, IsolationMode, ProgressEvent, RunOptions, RunSpec, TaskMode, TaskSpec,
};
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
    let opts = RunOptions {
        event_sink: Some(tx),
        capture_output: true,
        ..RunOptions::default()
    };
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
