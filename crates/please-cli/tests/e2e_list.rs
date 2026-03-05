use predicates::prelude::*;

mod support;

#[test]
fn test_list_shows_task_descriptions_and_aliases() {
    let temp = support::workspace_from_fixture("basic");
    let workspace = temp.path();

    support::please_cmd(workspace)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("process\t- Process input into dist output"))
        .stdout(predicate::str::contains("alias p -> process"));
}
