use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

pub fn workspace_from_fixture(name: &str) -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("create tempdir");
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    copy_tree(&fixture_root, temp.path()).expect("copy fixture to workspace");
    temp
}

pub fn please_cmd(workspace: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("please"));
    cmd.arg("--workspace").arg(workspace);
    cmd
}

fn copy_tree(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if src.is_file() {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)?;
        return Ok(());
    }

    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)?;
        }
    }

    Ok(())
}
