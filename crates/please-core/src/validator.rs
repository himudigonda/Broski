use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::graph::TaskGraph;
use crate::model::PleaseFile;
use crate::resolver::normalize_relative_path;

pub fn validate_pleasefile(config: &PleaseFile, workspace_root: &Path) -> Result<()> {
    if config.please.version != "0.1" && config.please.version != "0.2" {
        return Err(anyhow!(
            "unsupported pleasefile version '{}'; expected '0.1' or '0.2'",
            config.please.version
        ));
    }

    if config.task.is_empty() {
        return Err(anyhow!("pleasefile must define at least one task"));
    }

    for (task_name, task) in &config.task {
        if task.outputs.is_empty() {
            return Err(anyhow!("task '{}' must declare at least one output", task_name));
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
    }

    TaskGraph::build(&config.task).context("validating dependency graph")?;

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
            inputs: vec!["src/main.rs".to_string()],
            outputs: vec!["dist/out.txt".to_string()],
            env: BTreeMap::new(),
            run: RunSpec::Shell("echo hello".to_string()),
            isolation: None,
        }
    }

    #[test]
    fn rejects_unknown_dep() {
        let mut task = base_task();
        task.deps = vec!["missing".to_string()];

        let mut tasks = BTreeMap::new();
        tasks.insert("build".to_string(), task);

        let config =
            PleaseFile { please: PleaseSection { version: "0.2".to_string() }, task: tasks };

        let result = validate_pleasefile(&config, Path::new("."));
        assert!(result.is_err());
    }
}
