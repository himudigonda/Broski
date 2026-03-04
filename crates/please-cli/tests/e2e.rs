use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::assert::OutputAssertExt;
use predicates::prelude::*;

fn setup_workspace(dir: &Path) {
    fs::create_dir_all(dir.join("src")).expect("create src dir");
    fs::write(dir.join("src/input.txt"), "v1").expect("write input");

    let pleasefile = r#"
        [please]
        version = "0.1"

        [task.process]
        inputs = ["src/input.txt"]
        outputs = ["dist/out.txt"]
        run = "mkdir -p dist && cat src/input.txt > dist/out.txt"

        [task.fail_midway]
        inputs = ["src/input.txt"]
        outputs = ["dist/bad.txt"]
        run = "mkdir -p dist && echo 'poison' > dist/bad.txt && exit 1"
    "#;
    fs::write(dir.join("pleasefile"), pleasefile).expect("write pleasefile");
}

#[test]
fn test_golden_path_idempotency_and_invalidation() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workspace = temp.path();
    setup_workspace(workspace);

    // Run 1: Cold start. Must execute.
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("please"));
    cmd.arg("--workspace")
        .arg(workspace)
        .arg("run")
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: process"));

    let out_content = fs::read_to_string(workspace.join("dist/out.txt")).expect("read output");
    assert_eq!(out_content.trim(), "v1");

    // Run 2: Idempotency check. Must hit cache in 0ms.
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("please"));
    cmd.arg("--workspace")
        .arg(workspace)
        .arg("run")
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache hits: process"));

    // Run 3: Cache invalidation. Mutate input, must re-execute.
    fs::write(workspace.join("src/input.txt"), "v2").expect("mutate input");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("please"));
    cmd.arg("--workspace")
        .arg(workspace)
        .arg("run")
        .arg("process")
        .assert()
        .success()
        .stdout(predicate::str::contains("executed: process"));

    let out_content = fs::read_to_string(workspace.join("dist/out.txt")).expect("read new output");
    assert_eq!(out_content.trim(), "v2");
}

#[test]
fn test_acid_rollback_on_failure() {
    let temp = tempfile::tempdir().expect("create tempdir");
    let workspace = temp.path();
    setup_workspace(workspace);

    // Provide a pre-existing valid state
    fs::create_dir_all(workspace.join("dist")).expect("create dist");
    fs::write(workspace.join("dist/bad.txt"), "stable").expect("write stable state");

    // Execute task that fails midway
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("please"));
    cmd.arg("--workspace")
        .arg(workspace)
        .arg("run")
        .arg("fail_midway")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed with status"));

    // Ensure the staging 'poison' data did not corrupt the workspace
    let out_content =
        fs::read_to_string(workspace.join("dist/bad.txt")).expect("read rollback state");
    assert_eq!(out_content.trim(), "stable", "ACID rollback failed: workspace was polluted");
}
