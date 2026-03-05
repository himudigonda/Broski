use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::graph::TaskGraph;
use crate::model::{PleaseFile, TaskMode};
use crate::resolver::normalize_relative_path;

pub const RESERVED_COMMAND_NAMES: &[&str] =
    &["run", "list", "graph", "doctor", "cache", "help", "version"];

pub fn validate_pleasefile(config: &PleaseFile, workspace_root: &Path) -> Result<()> {
    if config.please.version != "0.1"
        && config.please.version != "0.2"
        && config.please.version != "0.3"
        && config.please.version != "0.4"
    {
        return Err(anyhow!(
            "unsupported pleasefile version '{}'; expected '0.1', '0.2', '0.3', or '0.4'",
            config.please.version
        ));
    }

    if config.task.is_empty() {
        return Err(anyhow!("pleasefile must define at least one task"));
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
    }

    TaskGraph::build(&config.task).context("validating dependency graph")?;

    Ok(())
}

fn validate_aliases(config: &PleaseFile) -> Result<()> {
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

    use super::*;
    use crate::model::{PleaseFile, PleaseSection, RunSpec, TaskSpec};

    fn base_task() -> TaskSpec {
        TaskSpec {
            deps: Vec::new(),
            description: None,
            inputs: vec!["src/main.rs".to_string()],
            outputs: vec!["dist/out.txt".to_string()],
            env: BTreeMap::new(),
            env_inherit: Vec::new(),
            secret_env: Vec::new(),
            run: RunSpec::Shell("echo hello".to_string()),
            isolation: None,
            mode: None,
            working_dir: None,
        }
    }

    #[test]
    fn rejects_unknown_dep() {
        let mut task = base_task();
        task.deps = vec!["missing".to_string()];

        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), task);

        let config = PleaseFile {
            please: PleaseSection { version: "0.2".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };

        let result = validate_pleasefile(&config, Path::new("."));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_alias_shadowing_task() {
        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), base_task());
        let mut alias = BTreeMap::new();
        alias.insert("build".to_string(), "build".to_string());

        let config = PleaseFile {
            please: PleaseSection { version: "0.3".to_string() },
            task: tasks,
            alias,
            load_env: Vec::new(),
        };

        assert!(validate_pleasefile(&config, Path::new(".")).is_err());
    }

    #[test]
    fn rejects_reserved_task_name() {
        let mut tasks = BTreeMap::new();
        tasks.insert("list".to_string(), base_task());

        let config = PleaseFile {
            please: PleaseSection { version: "0.3".to_string() },
            task: tasks,
            alias: BTreeMap::new(),
            load_env: Vec::new(),
        };

        let error = validate_pleasefile(&config, Path::new(".")).expect_err("should fail");
        assert!(error.to_string().contains("reserved CLI command"));
    }
}
