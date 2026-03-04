use predicates::prelude::*;

mod support;

#[test]
fn test_doctor_no_repair_reports_workspace() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("doctor")
        .arg("--no-repair")
        .assert()
        .success()
        .stdout(predicate::str::contains("workspace:"))
        .stdout(predicate::str::contains(workspace.to_string_lossy().to_string()));
}
