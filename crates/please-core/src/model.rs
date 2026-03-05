use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PleaseFile {
    pub please: PleaseSection,
    #[serde(default)]
    pub task: BTreeMap<String, TaskSpec>,
    #[serde(default)]
    pub alias: BTreeMap<String, String>,
    #[serde(default)]
    pub load_env: Vec<String>,
}

impl PleaseFile {
    pub fn resolve_task_name(&self, input: &str) -> Result<String> {
        if self.task.contains_key(input) {
            return Ok(input.to_string());
        }

        let mut current = input;
        let mut seen = std::collections::BTreeSet::new();

        while let Some(next) = self.alias.get(current) {
            if !seen.insert(current.to_string()) {
                return Err(anyhow!("alias cycle detected while resolving '{}'", input));
            }
            if self.task.contains_key(next) {
                return Ok(next.clone());
            }
            current = next;
        }

        Err(anyhow!("task '{}' not found", input))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PleaseSection {
    pub version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub resolved_variables: BTreeMap<String, String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub env_inherit: Vec<String>,
    #[serde(default)]
    pub secret_env: Vec<String>,
    pub run: RunSpec,
    #[serde(default)]
    pub isolation: Option<IsolationMode>,
    #[serde(default)]
    pub mode: Option<TaskMode>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub params: Vec<TaskParamSpec>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub confirm: Option<String>,
    #[serde(default)]
    pub shell_override: Option<ShellSpec>,
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskParamSpec {
    pub name: String,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RunSpec {
    Shell(String),
    Args(Vec<String>),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode {
    Strict,
    BestEffort,
    Off,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskMode {
    Graph,
    Interactive,
}

impl TaskSpec {
    pub fn effective_isolation(&self) -> IsolationMode {
        if let Some(mode) = self.isolation {
            return mode;
        }

        if cfg!(target_os = "linux") {
            IsolationMode::Strict
        } else {
            IsolationMode::BestEffort
        }
    }

    pub fn run_as_shell(&self) -> String {
        match &self.run {
            RunSpec::Shell(command) => command.clone(),
            RunSpec::Args(args) => {
                args.iter().map(|part| shell_escape(part)).collect::<Vec<String>>().join(" ")
            }
        }
    }

    pub fn inferred_mode(&self) -> TaskMode {
        match self.mode {
            Some(mode) => mode,
            None if self.outputs.is_empty() => TaskMode::Interactive,
            None => TaskMode::Graph,
        }
    }
}

fn shell_escape(input: &str) -> String {
    if input.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))
    {
        return input.to_string();
    }

    format!("'{}'", input.replace('\'', "'\"'\"'"))
}
