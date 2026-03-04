use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PleaseFile {
    pub please: PleaseSection,
    #[serde(default)]
    pub task: BTreeMap<String, TaskSpec>,
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
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub run: RunSpec,
    #[serde(default)]
    pub isolation: Option<IsolationMode>,
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
}

fn shell_escape(input: &str) -> String {
    if input.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))
    {
        return input.to_string();
    }

    format!("'{}'", input.replace('\'', "'\"'\"'"))
}
