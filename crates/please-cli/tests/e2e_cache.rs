use std::fs;

use predicates::prelude::*;

mod support;

#[test]
fn test_golden_path_idempotency_and_invalidation() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: process"));

    let out_content = fs::read_to_string(workspace.join("dist/out.txt")).expect("read output");
    assert_eq!(out_content.trim(), "v1");

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache hits: process"));

    fs::write(workspace.join("src/input.txt"), "v2").expect("mutate input");
    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: process"));

    let out_content = fs::read_to_string(workspace.join("dist/out.txt")).expect("read new output");
    assert_eq!(out_content.trim(), "v2");
}

#[test]
fn test_run_missing_task_fails_with_message() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("run")
        .arg("missing_task")
        .assert()
        .failure()
        .stderr(predicate::str::contains("task 'missing_task' not found"));
}

#[test]
fn test_run_alias_executes_target_task() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("run")
        .arg("p")
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: process"));
}

#[test]
fn test_implicit_run_executes_task() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: process"));
}
