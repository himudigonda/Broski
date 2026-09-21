use predicates::prelude::*;

mod support;

#[test]
fn nonexistent_workspace_fails_with_clean_error() {
    let temp = support::workspace_from_fixture("basic");
    let missing = temp.path().join("does-not-exist");

    support::broski_cmd(&missing)
        .arg("doctor")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found or inaccessible"));
}
