use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::model::TaskSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFingerprint(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintResult {
    pub fingerprint: TaskFingerprint,
    pub manifest: BTreeMap<String, String>,
}

pub fn compute_fingerprint(
    workspace_root: &Path,
    task_name: &str,
    task: &TaskSpec,
    resolved_inputs: &[PathBuf],
    resolved_env: &BTreeMap<String, String>,
    secret_env_keys: &BTreeSet<String>,
    passthrough_args: &[String],
) -> Result<FingerprintResult> {
    let mut manifest = BTreeMap::new();

    manifest.insert("meta:please_version".to_string(), digest_value(env!("CARGO_PKG_VERSION")));
    manifest.insert(format!("task:name:{task_name}"), digest_value(task_name));
    manifest.insert("task:run".to_string(), digest_value(&task.run_as_shell()));
    manifest.insert(
        "task:isolation".to_string(),
        digest_value(&format!("{:?}", task.effective_isolation())),
    );
    manifest.insert("task:mode".to_string(), digest_value(&format!("{:?}", task.inferred_mode())));
    manifest.insert(
        "task:working_dir".to_string(),
        digest_value(task.working_dir.as_deref().unwrap_or(".")),
    );
    manifest.insert(
        "task:passthrough_args".to_string(),
        digest_value(&passthrough_args.join("\u{1f}")),
    );

    for (idx, pattern) in task.inputs.iter().enumerate() {
        manifest.insert(format!("input_pattern:{idx}:{pattern}"), digest_value(pattern));
    }

    for (key, value) in resolved_env {
        if secret_env_keys.contains(key) {
            manifest.insert(format!("secret_env:{key}"), digest_value(value));
        } else {
            manifest.insert(format!("env:{key}"), digest_value(value));
        }
    }

    for input in resolved_inputs {
        let absolute = workspace_root.join(input);
        let entry_key = format!("input:{}", input.to_string_lossy());
        if absolute.exists() {
            let digest = hash_path(&absolute)
                .with_context(|| format!("hashing input '{}'", absolute.display()))?;
            manifest.insert(entry_key, digest);
        } else {
            manifest.insert(entry_key, digest_value("missing"));
        }
    }

    for output in &task.outputs {
        manifest.insert(format!("output:{output}"), digest_value(output));
    }

    let mut aggregate = blake3::Hasher::new();
    aggregate.update(b"please-manifest-v1");
    for (key, value) in &manifest {
        aggregate.update(key.as_bytes());
        aggregate.update(b"=");
        aggregate.update(value.as_bytes());
        aggregate.update(b"\n");
    }

    Ok(FingerprintResult {
        fingerprint: TaskFingerprint(aggregate.finalize().to_hex().to_string()),
        manifest,
    })
}

fn digest_value(value: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(value.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn hash_path(path: &Path) -> Result<String> {
    if path.is_file() {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"file");
        hash_file_into(path, &mut hasher)?;
        return Ok(hasher.finalize().to_hex().to_string());
    }

    if path.is_dir() {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"dir");

        let mut children = Vec::new();
        for entry in WalkDir::new(path) {
            let entry = entry.context("walking input directory while hashing")?;
            if entry.path().is_dir() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(path)
                .with_context(|| format!("stripping input prefix '{}'", path.display()))?
                .to_path_buf();
            children.push((rel, entry.path().to_path_buf()));
        }
        children.sort_by(|a, b| a.0.cmp(&b.0));

        for (rel, child) in children {
            hasher.update(rel.to_string_lossy().as_bytes());
            hasher.update(b"\0");
            hash_file_into(&child, &mut hasher)?;
        }

        return Ok(hasher.finalize().to_hex().to_string());
    }

    Ok(digest_value("missing"))
}

fn hash_file_into(path: &Path, hasher: &mut blake3::Hasher) -> Result<()> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("opening file '{}' for hashing", path.display()))?;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .with_context(|| format!("reading file '{}' for hashing", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RunSpec, TaskSpec};
    use std::collections::BTreeMap;
    use std::io::Write;

    fn base_task() -> TaskSpec {
        TaskSpec {
            deps: vec![],
            description: None,
            inputs: vec!["src/main.rs".to_string()],
            outputs: vec!["dist/out".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("echo hi".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
        }
    }

    #[test]
    fn fingerprint_and_manifest_are_deterministic() {
        let temp = tempfile::tempdir().expect("tempdir");
        let src = temp.path().join("src");
        fs::create_dir_all(&src).expect("create src");
        fs::write(src.join("main.rs"), "fn main() {}\n").expect("write main.rs");

        let task = base_task();
        let resolved = vec![PathBuf::from("src/main.rs")];

        let env = BTreeMap::new();
        let secret = BTreeSet::new();
        let first = compute_fingerprint(temp.path(), "build", &task, &resolved, &env, &secret, &[])
            .expect("first");
        let second =
            compute_fingerprint(temp.path(), "build", &task, &resolved, &env, &secret, &[])
                .expect("second");

        assert_eq!(first, second);
    }

    #[test]
    fn fingerprint_changes_when_env_changes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let src = temp.path().join("src");
        fs::create_dir_all(&src).expect("create src");
        let mut f = fs::File::create(src.join("main.rs")).expect("create main.rs");
        f.write_all(b"fn main() {} ").expect("write main.rs");

        let mut env_a = BTreeMap::new();
        env_a.insert("MODE".to_string(), "a".to_string());

        let mut env_b = BTreeMap::new();
        env_b.insert("MODE".to_string(), "b".to_string());

        let task_a = TaskSpec { env: env_a, ..base_task() };
        let task_b = TaskSpec { env: env_b, ..base_task() };

        let resolved = vec![PathBuf::from("src/main.rs")];

        let env_a = BTreeMap::from([("MODE".to_string(), "a".to_string())]);
        let env_b = BTreeMap::from([("MODE".to_string(), "b".to_string())]);
        let secret = BTreeSet::new();
        let fp_a =
            compute_fingerprint(temp.path(), "build", &task_a, &resolved, &env_a, &secret, &[])
                .expect("fingerprint a");
        let fp_b =
            compute_fingerprint(temp.path(), "build", &task_b, &resolved, &env_b, &secret, &[])
                .expect("fingerprint b");

        assert_ne!(fp_a.fingerprint, fp_b.fingerprint);
        assert_ne!(fp_a.manifest.get("env:MODE"), fp_b.manifest.get("env:MODE"));
    }

    #[test]
    fn missing_input_encoding_is_stable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let task = TaskSpec {
            deps: vec![],
            description: None,
            inputs: vec!["src/missing.txt".to_string()],
            outputs: vec!["dist/out".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("echo hi".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
        };
        let resolved = vec![PathBuf::from("src/missing.txt")];

        let env = BTreeMap::new();
        let secret = BTreeSet::new();
        let fp = compute_fingerprint(temp.path(), "build", &task, &resolved, &env, &secret, &[])
            .expect("fingerprint");
        assert!(fp.manifest.contains_key("input:src/missing.txt"));
        assert!(!fp.fingerprint.0.is_empty());
    }

    #[test]
    fn directory_hash_order_is_stable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("src/nested")).expect("create nested");
        fs::write(root.join("src/nested/b.txt"), "b").expect("write b");
        fs::write(root.join("src/a.txt"), "a").expect("write a");

        let task = TaskSpec {
            deps: vec![],
            description: None,
            inputs: vec!["src".to_string()],
            outputs: vec!["dist/out".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("echo hi".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
        };

        let resolved = vec![PathBuf::from("src")];
        let env = BTreeMap::new();
        let secret = BTreeSet::new();
        let first = compute_fingerprint(root, "build", &task, &resolved, &env, &secret, &[])
            .expect("first");
        let second = compute_fingerprint(root, "build", &task, &resolved, &env, &secret, &[])
            .expect("second");
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.manifest.get("input:src"), second.manifest.get("input:src"));
    }
}
