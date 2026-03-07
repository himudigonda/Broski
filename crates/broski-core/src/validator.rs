use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::graph::TaskGraph;
use crate::model::{BroskiFile, TaskMode};
use crate::resolver::normalize_relative_path;

pub const RESERVED_COMMAND_NAMES: &[&str] =
    &["run", "list", "graph", "doctor", "cache", "help", "version"];

pub fn validate_broskifile(config: &BroskiFile, workspace_root: &Path) -> Result<()> {
    if config.broski.version != "0.1"
        && config.broski.version != "0.2"
        && config.broski.version != "0.3"
        && config.broski.version != "0.4"
        && config.broski.version != "0.5"
    {
        return Err(anyhow!(
            "unsupported broskifile version '{}'; expected '0.1', '0.2', '0.3', '0.4', or '0.5'",
            config.broski.version
        ));
    }

    if config.task.is_empty() {
        return Err(anyhow!("broskifile must define at least one task"));
    }
    validate_aliases(config)?;

    for (task_name, task) in &config.task {
        match task.inferred_mode() {
            TaskMode::Graph if task.outputs.is_empty() => {
                return Err(anyhow!(
                    "task '{}' in graph mode must declare at least one output",
                    task_name
                ));
            }
            TaskMode::Interactive if !task.outputs.is_empty() => {
                return Err(anyhow!(
                    "task '{}' in interactive mode cannot declare outputs",
                    task_name
                ));
            }
            TaskMode::Interactive if !task.inputs.is_empty() => {
                return Err(anyhow!(
                    "task '{}' in interactive mode cannot declare inputs",
                    task_name
                ));
            }
            TaskMode::Interactive if !task.stage_ro.is_empty() => {
                return Err(anyhow!(
                    "task '{}' in interactive mode cannot declare @stage_ro paths",
                    task_name
                ));
            }
            _ => {}
        }

        match &task.run {
            crate::model::RunSpec::Shell(command) if command.trim().is_empty() => {
                return Err(anyhow!("task '{}' run command cannot be empty", task_name));
            }
            crate::model::RunSpec::Args(args) if args.is_empty() => {
                return Err(anyhow!("task '{}' run args cannot be empty", task_name));
            }
            _ => {}
        }

        for dep in &task.deps {
            if !config.task.contains_key(dep) {
                return Err(anyhow!("task '{}' has unknown dependency '{}'", task_name, dep));
            }
        }

        for input in &task.inputs {
            let _ = normalize_relative_path(input)
                .with_context(|| format!("task '{}' invalid input path '{}'", task_name, input))?;
        }

        for stage_ro in &task.stage_ro {
            let stage_ro_path = normalize_relative_path(stage_ro).with_context(|| {
                format!("task '{}' invalid @stage_ro path '{}'", task_name, stage_ro)
            })?;
            let absolute = workspace_root.join(&stage_ro_path);
            if !absolute.exists() {
                return Err(anyhow!(
                    "task '{}' @stage_ro path '{}' does not exist",
                    task_name,
                    stage_ro
                ));
            }
            for output in &task.outputs {
                let output_path = normalize_relative_path(output).with_context(|| {
                    format!("task '{}' invalid output path '{}'", task_name, output)
                })?;
                if output_path == stage_ro_path
                    || output_path.starts_with(&stage_ro_path)
                    || stage_ro_path.starts_with(&output_path)
                {
                    return Err(anyhow!(
                        "task '{}' @stage_ro path '{}' overlaps output '{}'",
                        task_name,
                        stage_ro,
                        output
                    ));
                }
            }
        }

        for output in &task.outputs {
            let normalized = normalize_relative_path(output).with_context(|| {
                format!("task '{}' invalid output path '{}'", task_name, output)
            })?;

            let absolute = workspace_root.join(&normalized);
            if !absolute.starts_with(workspace_root) {
                return Err(anyhow!(
                    "task '{}' output '{}' escapes workspace root",
                    task_name,
                    output
                ));
            }
        }

        if let Some(dir) = &task.working_dir {
            let _ = normalize_relative_path(dir)
                .with_context(|| format!("task '{}' invalid working_dir '{}'", task_name, dir))?;
        }

        let mut seen_params = std::collections::BTreeSet::new();
        for param in &task.params {
            if param.name.trim().is_empty() {
                return Err(anyhow!("task '{}' has parameter with empty name", task_name));
            }
            if !seen_params.insert(param.name.clone()) {
                return Err(anyhow!(
                    "task '{}' has duplicate parameter '{}'",
                    task_name,
                    param.name
                ));
            }
        }
        if task.confirm.as_ref().is_some_and(|value| value.trim().is_empty()) {
            return Err(anyhow!(
                "task '{}' has empty @confirm prompt; provide a non-empty message",
                task_name
            ));
        }
    }

    TaskGraph::build(&config.task).context("validating dependency graph")?;

    Ok(())
}

fn validate_aliases(config: &BroskiFile) -> Result<()> {
    for task_name in config.task.keys() {
        if RESERVED_COMMAND_NAMES.contains(&task_name.as_str()) {
            return Err(anyhow!(
                "task '{}' shadows reserved CLI command; choose a different task name",
                task_name
            ));
        }
    }

    for (alias, target) in &config.alias {
        if RESERVED_COMMAND_NAMES.contains(&alias.as_str()) {
            return Err(anyhow!(
                "alias '{}' shadows reserved CLI command; choose a different alias name",
                alias
            ));
        }
        if config.task.contains_key(alias) {
            return Err(anyhow!(
                "alias '{}' shadows an existing task; alias shadowing is not allowed",
                alias
            ));
        }
        if target == alias {
            return Err(anyhow!("alias '{}' cannot point to itself", alias));
        }
    }

    for alias in config.alias.keys() {
        let mut seen = std::collections::BTreeSet::new();
        let mut current = alias.as_str();
        while let Some(next) = config.alias.get(current) {
            if !seen.insert(current.to_string()) {
                return Err(anyhow!("alias cycle detected starting at '{}'", alias));
            }
            current = next;
        }

        if !config.task.contains_key(current) {
            return Err(anyhow!("alias '{}' resolves to unknown task '{}'", alias, current));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    use super::*;
    use crate::model::{BroskiFile, BroskiSection, RunSpec, TaskSpec};

    fn base_task() -> TaskSpec {
        TaskSpec {
            deps: Vec::new(),
            description: None,
            resolved_variables: BTreeMap::new(),
            inputs: vec!["src/main.rs".to_string()],
            stage_ro: Vec::new(),
            outputs: vec!["dist/out.txt".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("echo hello".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
            params: Vec::new(),
            private: false,
            confirm: None,
            shell_override: None,
            requires: Vec::new(),
        }
    }

    #[test]
    fn rejects_unknown_dep() {
        let mut task = base_task();
        task.deps = vec!["missing".to_string()];

        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), task);

        let config = BroskiFile {
            broski: BroskiSection { version: "0.2".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };

        let result = validate_broskifile(&config, Path::new("."));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_alias_shadowing_task() {
        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), base_task());
        let mut alias = BTreeMap::new();
        alias.insert("build".to_string(), "build".to_string());

        let config = BroskiFile {
            broski: BroskiSection { version: "0.3".to_string() },
            task: tasks,
            alias,
            load_env: Vec::new(),
        };

        assert!(validate_broskifile(&config, Path::new(".")).is_err());
    }

    #[test]
    fn rejects_reserved_task_name() {
        let mut tasks = BTreeMap::new();
        tasks.insert("list".to_string(), base_task());

        let config = BroskiFile {
            broski: BroskiSection { version: "0.3".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };

        let error = validate_broskifile(&config, Path::new(".")).expect_err("should fail");
        assert!(error.to_string().contains("reserved CLI command"));
    }

    #[test]
    fn rejects_stage_ro_overlap_with_output() {
        let tmp = tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("frontend/node_modules")).expect("mkdir");

        let mut task = base_task();
        task.outputs = vec!["frontend".to_string()];
        task.stage_ro = vec!["frontend/node_modules".to_string()];
        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), task);

        let config = BroskiFile {
            broski: BroskiSection { version: "0.5".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };

        let error = validate_broskifile(&config, tmp.path()).expect_err("should fail");
        assert!(error.to_string().contains("overlaps output"));
    }
}
