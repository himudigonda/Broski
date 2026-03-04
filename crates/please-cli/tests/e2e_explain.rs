use std::fs;

use predicates::prelude::*;

mod support;

#[test]
fn test_explain_shows_first_run_reason() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .arg("--explain")
        .assert()
        .success()
        .stdout(predicate::str::contains("explain process:"))
        .stdout(predicate::str::contains("cache miss: no prior execution record"));
}

#[test]
fn test_explain_shows_input_change_reason() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace).arg("run").arg("process").assert().success();

    fs::write(workspace.join("src/input.txt"), "v2").expect("mutate input");

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .arg("--explain")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache miss: input changed: src/input.txt"));
}

#[test]
fn test_explain_shows_force_and_no_cache_bypass_reasons() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace).arg("run").arg("process").assert().success();

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .arg("--force")
        .arg("--explain")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache bypass: --force supplied"));

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .arg("--no-cache")
        .arg("--explain")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache bypass: --no-cache supplied"));
}

#[test]
fn test_explain_shows_command_change_reason() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace).arg("run").arg("process").assert().success();

    let pleasefile = fs::read_to_string(workspace.join("pleasefile")).expect("read pleasefile");
    let updated =
        pleasefile.replace("cat src/input.txt > dist/out.txt", "cp src/input.txt dist/out.txt");
    fs::write(workspace.join("pleasefile"), updated).expect("write pleasefile");

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .arg("--explain")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache miss: task command changed"));
}

#[test]
fn test_explain_shows_env_change_reason() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    let pleasefile = fs::read_to_string(workspace.join("pleasefile")).expect("read pleasefile");
    let with_env = pleasefile.replace(
        "run = \"mkdir -p dist && cat src/input.txt > dist/out.txt\"",
        "env = { MODE = \"a\" }\nrun = \"mkdir -p dist && cat src/input.txt > dist/out.txt\"",
    );
    fs::write(workspace.join("pleasefile"), with_env).expect("write pleasefile with env");

    support::please_cmd(workspace).arg("run").arg("process").assert().success();

    let pleasefile = fs::read_to_string(workspace.join("pleasefile")).expect("read pleasefile");
    let changed_env = pleasefile.replace("MODE = \"a\"", "MODE = \"b\"");
    fs::write(workspace.join("pleasefile"), changed_env).expect("write changed env");

    support::please_cmd(workspace)
        .arg("run")
        .arg("process")
        .arg("--explain")
        .assert()
        .success()
        .stdout(predicate::str::contains("cache miss: env changed: MODE"));
}
