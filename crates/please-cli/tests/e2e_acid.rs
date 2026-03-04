use std::fs;

use predicates::prelude::*;

mod support;

#[test]
fn test_acid_rollback_on_failure() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    fs::create_dir_all(workspace.join("dist")).expect("create dist");
    fs::write(workspace.join("dist/bad.txt"), "stable").expect("write stable state");

    support::please_cmd(workspace)
        .arg("run")
        .arg("fail_midway")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed with status"));

    let out_content =
        fs::read_to_string(workspace.join("dist/bad.txt")).expect("read rollback state");
    assert_eq!(out_content.trim(), "stable", "ACID rollback failed: workspace was polluted");
}
