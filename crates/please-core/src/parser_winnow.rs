use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};

use crate::model::{PleaseFile, PleaseSection, RunSpec, TaskMode, TaskSpec};

#[derive(Debug, Clone, Default)]
struct TaskDraft {
    deps: Vec<String>,
    description: Option<String>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    env: BTreeMap<String, String>,
    env_inherit: Vec<String>,
    secret_env: Vec<String>,
    isolation: Option<crate::model::IsolationMode>,
    mode: Option<TaskMode>,
    working_dir: Option<String>,
    run_lines: Vec<String>,
}

pub fn parse_pleasefile_dsl(content: &str) -> Result<PleaseFile> {
    let mut version: Option<String> = None;
    let mut aliases = BTreeMap::new();
    let mut load_env = Vec::new();
    let mut tasks: BTreeMap<String, TaskDraft> = BTreeMap::new();
    let mut current_task: Option<String> = None;
    let mut pending_task_comments: Vec<String> = Vec::new();

    for (line_no, raw_line) in content.lines().enumerate() {
        let line_no = line_no + 1;
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        let indented = line.starts_with(' ') || line.starts_with('\t');

        if trimmed.is_empty() {
            pending_task_comments.clear();
            continue;
        }

        if trimmed.starts_with('#') {
            if current_task.is_none() {
                let comment = trimmed.trim_start_matches('#').trim().to_string();
                if !comment.is_empty() {
                    pending_task_comments.push(comment);
                }
            }
            continue;
        }

        if !current_task.as_ref().is_some_and(|_| indented) {
            current_task = None;

            if let Some(value) = parse_version_line(trimmed, line_no)? {
                version = Some(value);
                pending_task_comments.clear();
                continue;
            }

            if let Some(path) = parse_load_line(trimmed, line_no)? {
                load_env.push(path);
                pending_task_comments.clear();
                continue;
            }

            if let Some((alias, target)) = parse_alias_line(trimmed, line_no)? {
                aliases.insert(alias, target);
                pending_task_comments.clear();
                continue;
            }

            if let Some((task_name, deps)) = parse_task_header(trimmed, line_no)? {
                if tasks.contains_key(&task_name) {
                    return Err(parse_error(line_no, 1, format!("duplicate task '{}'", task_name)));
                }
                let description = if pending_task_comments.is_empty() {
                    None
                } else {
                    Some(pending_task_comments.join(" "))
                };
                tasks.insert(
                    task_name.clone(),
                    TaskDraft { deps, description, ..TaskDraft::default() },
                );
                pending_task_comments.clear();
                current_task = Some(task_name);
                continue;
            }

            return Err(parse_error(
                line_no,
                1,
                "expected 'version = \"0.3\"', '@load', 'alias', or '<task>: ...'".to_string(),
            ));
        }

        let task_name = current_task
            .as_ref()
            .ok_or_else(|| parse_error(line_no, 1, "internal parser state error".to_string()))?;

        let body = trimmed;
        let Some(task) = tasks.get_mut(task_name) else {
            return Err(parse_error(line_no, 1, "internal parser state error".to_string()));
        };

        if let Some(rest) = body.strip_prefix("@in") {
            let values = split_items(rest, line_no, "@in")?;
            task.inputs.extend(values);
            continue;
        }

        if let Some(rest) = body.strip_prefix("@out") {
            let values = split_items(rest, line_no, "@out")?;
            task.outputs.extend(values);
            continue;
        }

        if let Some(rest) = body.strip_prefix("@env") {
            let entries = split_items(rest, line_no, "@env")?;
            for entry in entries {
                if let Some((key, value)) = entry.split_once('=') {
                    let key = key.trim();
                    if key.is_empty() {
                        return Err(parse_error(line_no, 1, "@env has empty key".to_string()));
                    }
                    task.env.insert(key.to_string(), value.to_string());
                } else {
                    task.env_inherit.push(entry);
                }
            }
            continue;
        }

        if let Some(rest) = body.strip_prefix("@secret_env") {
            let values = split_items(rest, line_no, "@secret_env")?;
            task.secret_env.extend(values);
            continue;
        }

        if let Some(rest) = body.strip_prefix("@dir") {
            let values = split_items(rest, line_no, "@dir")?;
            if values.len() != 1 {
                return Err(parse_error(line_no, 1, "@dir accepts exactly one path".to_string()));
            }
            task.working_dir = Some(values[0].clone());
            continue;
        }

        if let Some(rest) = body.strip_prefix("@mode") {
            let values = split_items(rest, line_no, "@mode")?;
            if values.len() != 1 {
                return Err(parse_error(
                    line_no,
                    1,
                    "@mode accepts exactly one value: graph|interactive".to_string(),
                ));
            }
            task.mode = match values[0].as_str() {
                "graph" => Some(TaskMode::Graph),
                "interactive" => Some(TaskMode::Interactive),
                other => {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!("unknown @mode value '{}'; expected graph|interactive", other),
                    ));
                }
            };
            continue;
        }

        if let Some(rest) = body.strip_prefix("@isolation") {
            let values = split_items(rest, line_no, "@isolation")?;
            if values.len() != 1 {
                return Err(parse_error(
                    line_no,
                    1,
                    "@isolation accepts exactly one value: strict|best_effort|off".to_string(),
                ));
            }
            task.isolation = match values[0].as_str() {
                "strict" => Some(crate::model::IsolationMode::Strict),
                "best_effort" => Some(crate::model::IsolationMode::BestEffort),
                "off" => Some(crate::model::IsolationMode::Off),
                other => {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!(
                            "unknown @isolation value '{}'; expected strict|best_effort|off",
                            other
                        ),
                    ));
                }
            };
            continue;
        }

        task.run_lines.push(body.to_string());
    }

    let version = version.ok_or_else(|| {
        parse_error(1, 1, "missing required top-level line: version = \"0.3\"".to_string())
    })?;

    if version != "0.3" {
        bail!("DSL pleasefile requires version = \"0.3\"; found '{version}'");
    }

    if tasks.is_empty() {
        bail!("pleasefile must define at least one task");
    }

    let mut task_specs = BTreeMap::new();
    for (name, draft) in tasks {
        if draft.run_lines.is_empty() {
            bail!("task '{}' has no command lines", name);
        }
        task_specs.insert(
            name,
            TaskSpec {
                deps: draft.deps,
                description: draft.description,
                inputs: draft.inputs,
                outputs: draft.outputs,
                env: draft.env,
                env_inherit: draft.env_inherit,
                secret_env: draft.secret_env,
                run: RunSpec::Shell(draft.run_lines.join("\n")),
                isolation: draft.isolation,
                mode: draft.mode,
                working_dir: draft.working_dir,
            },
        );
    }

    Ok(PleaseFile { please: PleaseSection { version }, task: task_specs, alias: aliases, load_env })
}

fn split_items(rest: &str, line_no: usize, directive: &str) -> Result<Vec<String>> {
    let values: Vec<String> = rest
        .split_whitespace()
        .map(|value| value.trim_matches('"').trim_matches('\'').to_string())
        .filter(|value| !value.is_empty())
        .collect();

    if values.is_empty() {
        return Err(parse_error(line_no, 1, format!("{} requires at least one value", directive)));
    }

    Ok(values)
}

fn parse_version_line(line: &str, line_no: usize) -> Result<Option<String>> {
    if !line.starts_with("version") {
        return Ok(None);
    }

    let Some((left, right)) = line.split_once('=') else {
        return Err(parse_error(
            line_no,
            1,
            "invalid version declaration; expected version = \"0.3\"".to_string(),
        ));
    };

    if left.trim() != "version" {
        return Ok(None);
    }

    let raw = right.trim();
    let value = raw.trim_matches('"');
    Ok(Some(value.to_string()))
}

fn parse_load_line(line: &str, line_no: usize) -> Result<Option<String>> {
    let Some(rest) = line.strip_prefix("@load") else {
        return Ok(None);
    };
    let values = split_items(rest, line_no, "@load")?;
    if values.len() != 1 {
        return Err(parse_error(line_no, 1, "@load accepts exactly one file path".to_string()));
    }
    Ok(Some(values[0].clone()))
}

fn parse_alias_line(line: &str, line_no: usize) -> Result<Option<(String, String)>> {
    let Some(rest) = line.strip_prefix("alias ") else {
        return Ok(None);
    };

    let Some((name, target)) = rest.split_once('=') else {
        return Err(parse_error(
            line_no,
            1,
            "invalid alias; expected: alias <name> = <target>".to_string(),
        ));
    };

    let name = name.trim();
    let target = target.trim();
    if name.is_empty() || target.is_empty() {
        return Err(parse_error(
            line_no,
            1,
            "invalid alias; name/target cannot be empty".to_string(),
        ));
    }

    Ok(Some((name.to_string(), target.to_string())))
}

fn parse_task_header(line: &str, line_no: usize) -> Result<Option<(String, Vec<String>)>> {
    let Some((name, deps_raw)) = line.split_once(':') else {
        return Ok(None);
    };

    let task_name = name.trim();
    if task_name.is_empty() {
        return Err(parse_error(line_no, 1, "task name cannot be empty".to_string()));
    }

    if !is_identifier(task_name) {
        return Err(parse_error(line_no, 1, format!("invalid task identifier '{}'", task_name)));
    }

    let deps = deps_raw
        .split_whitespace()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();

    Ok(Some((task_name.to_string(), deps)))
}

fn is_identifier(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn parse_error(line: usize, column: usize, message: String) -> anyhow::Error {
    anyhow!("DSL parse error at {}:{}: {}", line, column, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_document() {
        let input = r#"
            version = "0.3"
            @load .env
            alias b = build

            build: prep
                @in src/**/*.rs Cargo.toml
                @out dist/out.txt
                @env MODE=dev
                @env PATH
                @secret_env API_KEY
                @dir crates/please-core
                @mode graph
                @isolation off
                cargo build --release

            prep:
                echo prep
        "#;

        let parsed = parse_pleasefile_dsl(input).expect("parse DSL");
        assert_eq!(parsed.please.version, "0.3");
        assert_eq!(parsed.load_env, vec![".env".to_string()]);
        assert_eq!(parsed.alias.get("b"), Some(&"build".to_string()));
        let build = parsed.task.get("build").expect("build task");
        assert_eq!(build.inputs.len(), 2);
        assert_eq!(build.outputs.len(), 1);
        assert_eq!(build.env.get("MODE"), Some(&"dev".to_string()));
        assert!(build.env_inherit.contains(&"PATH".to_string()));
        assert!(build.secret_env.contains(&"API_KEY".to_string()));
        assert_eq!(build.mode, Some(TaskMode::Graph));
        assert_eq!(build.isolation, Some(crate::model::IsolationMode::Off));
    }

    #[test]
    fn rejects_invalid_mode() {
        let input = r#"
            version = "0.3"
            dev:
                @mode invalid
                echo hi
        "#;

        let error = parse_pleasefile_dsl(input).expect_err("invalid mode should fail");
        assert!(error.to_string().contains("unknown @mode value"));
    }

    #[test]
    fn requires_version_03_for_dsl() {
        let input = r#"
            version = "0.2"
            dev:
                echo hi
        "#;

        let error = parse_pleasefile_dsl(input).expect_err("version mismatch should fail");
        assert!(error.to_string().contains("requires version"));
    }

    #[test]
    fn captures_task_description_from_comments() {
        let input = r#"
            version = "0.3"

            # Build backend artifacts
            # Uses Cargo release profile
            build:
                @out dist/out.txt
                echo hi > dist/out.txt
        "#;

        let parsed = parse_pleasefile_dsl(input).expect("parse DSL");
        let build = parsed.task.get("build").expect("build task");
        assert_eq!(
            build.description.as_deref(),
            Some("Build backend artifacts Uses Cargo release profile")
        );
    }
}
