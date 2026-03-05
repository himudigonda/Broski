use predicates::prelude::*;

mod support;

#[test]
fn test_cycle_diagnostics_are_actionable() {
    let temp = support::workspace_from_fixture("cycle");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("graph")
        .arg("a")
        .assert()
        .failure()
        .stderr(predicate::str::contains("dependency graph contains a cycle"));
}

#[test]
fn test_graph_supports_alias_target() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("graph")
        .arg("p")
        .assert()
        .success()
        .stdout(predicate::str::contains("process"));
}
