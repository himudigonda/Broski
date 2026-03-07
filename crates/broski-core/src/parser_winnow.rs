use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use miette::{miette, LabeledSpan};

use crate::model::{BroskiFile, BroskiSection, RunSpec, TaskMode, TaskParamSpec, TaskSpec};
use crate::resolver::normalize_relative_path;

const IMPORT_DEPTH_LIMIT: usize = 10;

thread_local! {
    static DSL_SOURCE: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[derive(Debug)]
struct DslSourceGuard;

impl DslSourceGuard {
    fn set(source: String) -> Self {
        DSL_SOURCE.with(|slot| {
            *slot.borrow_mut() = Some(source);
        });
        Self
    }
}

impl Drop for DslSourceGuard {
    fn drop(&mut self) {
        DSL_SOURCE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

#[derive(Debug, Clone, Default)]
struct TaskDraft {
    deps: Vec<String>,
    params: Vec<TaskParamSpec>,
    description: Option<String>,
    resolved_variables: BTreeMap<String, String>,
    inputs: Vec<String>,
    stage_ro: Vec<String>,
    outputs: Vec<String>,
    env: BTreeMap<String, String>,
    env_inherit: Vec<String>,
    secret_env: Vec<String>,
    isolation: Option<crate::model::IsolationMode>,
    mode: Option<TaskMode>,
    working_dir: Option<String>,
    private: bool,
    confirm: Option<String>,
    requires: Vec<String>,
    run_lines: Vec<String>,
}

#[derive(Debug, Clone)]
struct VariableDef {
    kind: VariableKind,
    line_no: usize,
}

#[derive(Debug, Clone)]
enum VariableKind {
    Static(String),
    DynamicCommand(String),
}

pub fn parse_broskifile_dsl(content: &str) -> Result<BroskiFile> {
    parse_broskifile_dsl_with_workspace(content, None)
}

pub fn parse_broskifile_dsl_with_workspace(
    content: &str,
    workspace_root: Option<&Path>,
) -> Result<BroskiFile> {
    let expanded_content = expand_imports(content, workspace_root, 0, &mut Vec::new())?;
    let _source_guard = DslSourceGuard::set(expanded_content.clone());
    let content = expanded_content.as_str();

    let mut version: Option<String> = None;
    let mut aliases = BTreeMap::new();
    let mut load_env = Vec::new();
    let mut variable_defs: BTreeMap<String, VariableDef> = BTreeMap::new();
    let mut resolved_variables: BTreeMap<String, String> = BTreeMap::new();
    let mut tasks: BTreeMap<String, TaskDraft> = BTreeMap::new();
    let mut current_task: Option<String> = None;
    let mut pending_task_comments: Vec<String> = Vec::new();
    let mut seen_task_header = false;

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
                if aliases.contains_key(&alias) {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!("name collision: alias '{}' is already defined", alias),
                    ));
                }
                if variable_defs.contains_key(&alias) || tasks.contains_key(&alias) {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!(
                            "name collision: '{}' conflicts with existing variable/task",
                            alias
                        ),
                    ));
                }
                aliases.insert(alias, target);
                pending_task_comments.clear();
                continue;
            }

            if let Some((name, variable_def)) = parse_variable_line(trimmed, line_no)? {
                if seen_task_header {
                    return Err(parse_error(
                        line_no,
                        1,
                        "variable declarations must appear before task headers".to_string(),
                    ));
                }
                if variable_defs.contains_key(&name) {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!("name collision: variable '{}' is already defined", name),
                    ));
                }
                if aliases.contains_key(&name) || tasks.contains_key(&name) {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!("name collision: '{}' conflicts with existing alias/task", name),
                    ));
                }
                variable_defs.insert(name, variable_def);
                pending_task_comments.clear();
                continue;
            }

            let (header_line, used_vars) = interpolate_template_with_defs(
                trimmed,
                line_no,
                &variable_defs,
                &mut resolved_variables,
                workspace_root,
                None,
            )?;
            if let Some((task_name, params, deps)) = parse_task_header(&header_line, line_no)? {
                seen_task_header = true;
                if tasks.contains_key(&task_name) {
                    return Err(parse_error(line_no, 1, format!("duplicate task '{}'", task_name)));
                }
                if aliases.contains_key(&task_name) || variable_defs.contains_key(&task_name) {
                    return Err(parse_error(
                        line_no,
                        1,
                        format!(
                            "name collision: task '{}' conflicts with existing alias/variable",
                            task_name
                        ),
                    ));
                }
                let description = if pending_task_comments.is_empty() {
                    None
                } else {
                    Some(pending_task_comments.join(" "))
                };
                tasks.insert(
                    task_name.clone(),
                    TaskDraft {
                        deps,
                        params,
                        description,
                        resolved_variables: used_vars
                            .into_iter()
                            .map(|name| {
                                let value =
                                    resolved_variables.get(&name).cloned().unwrap_or_default();
                                (name, value)
                            })
                            .collect(),
                        ..TaskDraft::default()
                    },
                );
                pending_task_comments.clear();
                current_task = Some(task_name);
                continue;
            }

            return Err(parse_error(
                line_no,
                1,
                "expected 'version = \"0.5\"', variable declaration, '@load', 'alias', or '<task>: ...'"
                    .to_string(),
            ));
        }

        let task_name = current_task
            .as_ref()
            .ok_or_else(|| parse_error(line_no, 1, "internal parser state error".to_string()))?;

        let body = trimmed;
        let Some(task) = tasks.get_mut(task_name) else {
            return Err(parse_error(line_no, 1, "internal parser state error".to_string()));
        };
        let allowed_params = task_param_names(task);
        let (body, used_vars) = interpolate_template_with_defs(
            body,
            line_no,
            &variable_defs,
            &mut resolved_variables,
            workspace_root,
            Some(&allowed_params),
        )?;
        for name in used_vars {
            if let Some(value) = resolved_variables.get(&name) {
                task.resolved_variables.insert(name, value.clone());
            }
        }

        if let Some(rest) = body.strip_prefix("@in") {
            let values = split_items(rest, line_no, "@in")?;
            task.inputs.extend(values);
            continue;
        }

        if let Some(rest) = body.strip_prefix("@stage_ro") {
            let values = split_items(rest, line_no, "@stage_ro")?;
            task.stage_ro.extend(values);
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

        if let Some(rest) = body.strip_prefix("@requires") {
            let values = split_items(rest, line_no, "@requires")?;
            task.requires.extend(values);
            continue;
        }

        if body == "@private" {
            task.private = true;
            continue;
        }

        if let Some(rest) = body.strip_prefix("@confirm") {
            let msg = rest.trim();
            if msg.is_empty() {
                return Err(parse_error(
                    line_no,
                    1,
                    "@confirm requires a prompt message".to_string(),
                ));
            }
            let normalized = msg.trim_matches('"').trim_matches('\'').to_string();
            task.confirm = Some(normalized);
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
        parse_error(1, 1, "missing required top-level line: version = \"0.5\"".to_string())
    })?;

    if version != "0.3" && version != "0.4" && version != "0.5" {
        bail!("DSL broskifile requires version = \"0.3\", \"0.4\", or \"0.5\"; found '{version}'");
    }

    if tasks.is_empty() {
        bail!("broskifile must define at least one task");
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
                resolved_variables: draft.resolved_variables,
                inputs: draft.inputs,
                stage_ro: draft.stage_ro,
                outputs: draft.outputs,
                env: draft.env,
                env_inherit: draft.env_inherit,
                secret_env: draft.secret_env,
                run: RunSpec::Shell(draft.run_lines.join("\n")),
                isolation: draft.isolation,
                mode: draft.mode,
                working_dir: draft.working_dir,
                params: draft.params,
                private: draft.private,
                confirm: draft.confirm,
                shell_override: None,
                requires: draft.requires,
            },
        );
    }

    Ok(BroskiFile { broski: BroskiSection { version }, task: task_specs, alias: aliases, load_env })
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

fn expand_imports(
    content: &str,
    workspace_root: Option<&Path>,
    depth: usize,
    import_stack: &mut Vec<PathBuf>,
) -> Result<String> {
    if depth > IMPORT_DEPTH_LIMIT {
        return Err(anyhow!(
            "import depth limit exceeded (>{}); chain: {}",
            IMPORT_DEPTH_LIMIT,
            import_stack
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<String>>()
                .join(" -> ")
        ));
    }

    let mut expanded = String::new();
    let mut in_task_block = false;
    for (line_idx, raw_line) in content.lines().enumerate() {
        let line_no = line_idx + 1;
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        let indented = line.starts_with(' ') || line.starts_with('\t');

        if in_task_block && indented {
            expanded.push_str(line);
            expanded.push('\n');
            continue;
        }
        in_task_block = false;

        if let Some((_task_name, _params, _deps)) = parse_task_header(trimmed, line_no)? {
            in_task_block = true;
            expanded.push_str(line);
            expanded.push('\n');
            continue;
        }

        if let Some(path) = parse_import_line(trimmed, line_no)? {
            let workspace_root = workspace_root.ok_or_else(|| {
                parse_error(
                    line_no,
                    1,
                    "@import requires workspace context; parse from a workspace root".to_string(),
                )
            })?;
            let rel = normalize_relative_path(&path)
                .with_context(|| format!("invalid @import path '{}'", path))?;
            let import_path = workspace_root.join(rel);
            let canonical = fs::canonicalize(&import_path).unwrap_or_else(|_| import_path.clone());

            if import_stack.contains(&canonical) {
                let mut chain: Vec<String> =
                    import_stack.iter().map(|item| item.display().to_string()).collect();
                chain.push(canonical.display().to_string());
                return Err(anyhow!("circular import detected: {}", chain.join(" -> ")));
            }

            if depth == IMPORT_DEPTH_LIMIT {
                return Err(anyhow!("import depth limit exceeded at '{}'", canonical.display()));
            }

            let imported_content = fs::read_to_string(&import_path).with_context(|| {
                format!("reading imported broskifile '{}'", import_path.display())
            })?;
            import_stack.push(canonical);
            let nested =
                expand_imports(&imported_content, Some(workspace_root), depth + 1, import_stack)?;
            import_stack.pop();

            expanded.push_str(&strip_imported_version_lines(&nested, true));
            if !expanded.ends_with('\n') {
                expanded.push('\n');
            }
            continue;
        }

        expanded.push_str(line);
        expanded.push('\n');
    }

    Ok(expanded)
}

fn strip_imported_version_lines(content: &str, imported: bool) -> String {
    if !imported {
        return content.to_string();
    }
    let mut output = String::new();
    for raw_line in content.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented && parse_version_line(trimmed, 1).ok().flatten().is_some() {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn parse_version_line(line: &str, line_no: usize) -> Result<Option<String>> {
    if !line.starts_with("version") {
        return Ok(None);
    }

    let Some((left, right)) = line.split_once('=') else {
        return Err(parse_error(
            line_no,
            1,
            "invalid version declaration; expected version = \"0.5\"".to_string(),
        ));
    };

    if left.trim() != "version" {
        return Ok(None);
    }

    let raw = right.trim();
    let value = raw.trim_matches('"');
    Ok(Some(value.to_string()))
}

fn parse_import_line(line: &str, line_no: usize) -> Result<Option<String>> {
    let Some(rest) = line.strip_prefix("@import") else {
        return Ok(None);
    };
    let values = split_items(rest, line_no, "@import")?;
    if values.len() != 1 {
        return Err(parse_error(line_no, 1, "@import accepts exactly one file path".to_string()));
    }
    Ok(Some(values[0].clone()))
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

fn parse_variable_line(line: &str, line_no: usize) -> Result<Option<(String, VariableDef)>> {
    let Some((left, right)) = line.split_once('=') else {
        return Ok(None);
    };
    let key = left.trim();
    if key.is_empty() || key == "version" {
        return Ok(None);
    }
    if !is_variable_name(key) {
        return Ok(None);
    }

    let value = right.trim();
    if value.is_empty() {
        return Err(parse_error(
            line_no,
            key.len() + 2,
            format!("variable '{}' has empty value", key),
        ));
    }

    let kind = if value.starts_with("$(") && value.ends_with(')') {
        let command = value[2..value.len() - 1].trim().to_string();
        if command.is_empty() {
            return Err(parse_error(
                line_no,
                key.len() + 2,
                format!("dynamic variable '{}' has empty command", key),
            ));
        }
        VariableKind::DynamicCommand(command)
    } else {
        VariableKind::Static(value.trim_matches('"').trim_matches('\'').to_string())
    };

    Ok(Some((key.to_string(), VariableDef { kind, line_no })))
}

type ParsedTaskHeader = (String, Vec<TaskParamSpec>, Vec<String>);

fn parse_task_header(line: &str, line_no: usize) -> Result<Option<ParsedTaskHeader>> {
    let Some((name, deps_raw)) = line.split_once(':') else {
        return Ok(None);
    };

    let mut header_tokens = name.split_whitespace();
    let Some(task_name) = header_tokens.next().map(str::trim) else {
        return Err(parse_error(line_no, 1, "task name cannot be empty".to_string()));
    };
    if task_name.is_empty() {
        return Err(parse_error(line_no, 1, "task name cannot be empty".to_string()));
    }

    if !is_identifier(task_name) {
        return Err(parse_error(line_no, 1, format!("invalid task identifier '{}'", task_name)));
    }

    let mut params = Vec::new();
    let mut seen_params = BTreeSet::new();
    for token in header_tokens {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if !(token.starts_with('[') && token.ends_with(']')) {
            return Err(parse_error(
                line_no,
                1,
                format!(
                    "invalid task header token '{}'; expected [param] or [param=\"default\"]",
                    token
                ),
            ));
        }
        let inner = token[1..token.len() - 1].trim();
        if inner.is_empty() {
            return Err(parse_error(line_no, 1, "empty task parameter declaration".to_string()));
        }

        let (param_name, default) = if let Some((left, right)) = inner.split_once('=') {
            let param_name = left.trim();
            let default = right.trim().trim_matches('"').trim_matches('\'').to_string();
            (param_name, Some(default))
        } else {
            (inner, None)
        };

        if !is_variable_name(param_name) {
            return Err(parse_error(
                line_no,
                1,
                format!("invalid task parameter name '{}'", param_name),
            ));
        }
        if !seen_params.insert(param_name.to_string()) {
            return Err(parse_error(
                line_no,
                1,
                format!("duplicate task parameter '{}'", param_name),
            ));
        }
        params.push(TaskParamSpec { name: param_name.to_string(), default });
    }

    let deps = deps_raw
        .split_whitespace()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();

    Ok(Some((task_name.to_string(), params, deps)))
}

fn is_identifier(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn is_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn resolve_variable(
    name: &str,
    defs: &BTreeMap<String, VariableDef>,
    resolved: &mut BTreeMap<String, String>,
    resolving: &mut BTreeSet<String>,
    workspace_root: Option<&Path>,
    line_no: usize,
) -> Result<String> {
    if let Some(value) = resolved.get(name) {
        return Ok(value.clone());
    }
    if !resolving.insert(name.to_string()) {
        return Err(parse_error(
            line_no,
            1,
            format!("cyclic variable reference detected involving '{}'", name),
        ));
    }

    let def = defs
        .get(name)
        .ok_or_else(|| parse_error(line_no, 1, format!("unknown variable '{}'", name)))?;

    let resolved_value = match &def.kind {
        VariableKind::Static(value) => {
            let (interpolated, _) = interpolate_with_resolver(value, def.line_no, |ref_name| {
                resolve_variable(ref_name, defs, resolved, resolving, workspace_root, def.line_no)
                    .map(Some)
            })?;
            interpolated
        }
        VariableKind::DynamicCommand(command) => {
            let (interpolated_command, _) =
                interpolate_with_resolver(command, def.line_no, |ref_name| {
                    resolve_variable(
                        ref_name,
                        defs,
                        resolved,
                        resolving,
                        workspace_root,
                        def.line_no,
                    )
                    .map(Some)
                })?;
            run_dynamic_variable_command(&interpolated_command, workspace_root, def.line_no)?
        }
    };

    resolving.remove(name);
    resolved.insert(name.to_string(), resolved_value.clone());
    Ok(resolved_value)
}

fn interpolate_template_with_defs(
    input: &str,
    line_no: usize,
    defs: &BTreeMap<String, VariableDef>,
    resolved: &mut BTreeMap<String, String>,
    workspace_root: Option<&Path>,
    allow_deferred: Option<&BTreeSet<String>>,
) -> Result<(String, BTreeSet<String>)> {
    let mut resolving = BTreeSet::new();
    interpolate_with_resolver(input, line_no, |name| {
        if defs.contains_key(name) {
            return resolve_variable(name, defs, resolved, &mut resolving, workspace_root, line_no)
                .map(Some);
        }
        if allow_deferred.is_some_and(|allowed| allowed.contains(name)) {
            return Ok(None);
        }
        Err(parse_error(line_no, 1, format!("unknown variable '{}'", name)))
    })
}

fn interpolate_with_resolver<F>(
    input: &str,
    line_no: usize,
    mut resolver: F,
) -> Result<(String, BTreeSet<String>)>
where
    F: FnMut(&str) -> Result<Option<String>>,
{
    let mut output = String::with_capacity(input.len());
    let mut used = BTreeSet::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = input[cursor..].find("{{") {
        let start = cursor + rel_start;
        output.push_str(&input[cursor..start]);
        let open_end = start + 2;
        let Some(rel_close) = input[open_end..].find("}}") else {
            return Err(parse_error(
                line_no,
                start + 1,
                "unterminated variable interpolation; expected '}}'".to_string(),
            ));
        };
        let close = open_end + rel_close;
        let key = input[open_end..close].trim();
        if key.is_empty() {
            return Err(parse_error(
                line_no,
                start + 1,
                "empty variable interpolation; expected variable name".to_string(),
            ));
        }
        if let Some(value) = evaluate_builtin_expr(key, line_no, start + 1)? {
            output.push_str(&value);
            cursor = close + 2;
            continue;
        }
        if !is_variable_name(key) {
            return Err(parse_error(
                line_no,
                start + 1,
                format!("invalid variable name '{}'", key),
            ));
        }
        let value = resolver(key).with_context(|| {
            format!("resolving variable '{}' at {}:{}", key, line_no, start + 1)
        })?;
        if let Some(value) = value {
            used.insert(key.to_string());
            output.push_str(&value);
        } else {
            output.push_str(&format!("{{{{ {} }}}}", key));
        }
        cursor = close + 2;
    }

    output.push_str(&input[cursor..]);
    Ok((output, used))
}

fn evaluate_builtin_expr(expr: &str, line_no: usize, column: usize) -> Result<Option<String>> {
    if expr == "os()" {
        return Ok(Some(env::consts::OS.to_string()));
    }
    if expr == "arch()" {
        return Ok(Some(env::consts::ARCH.to_string()));
    }
    if !expr.starts_with("env(") {
        return Ok(None);
    }
    if !expr.ends_with(')') {
        return Err(parse_error(
            line_no,
            column,
            "invalid env() builtin; expected env(\"KEY\", \"default\")".to_string(),
        ));
    }
    let inner = &expr[4..expr.len() - 1];
    let args = parse_env_builtin_args(inner, line_no, column)?;
    if args.is_empty() || args.len() > 2 {
        return Err(parse_error(line_no, column, "env() expects 1 or 2 arguments".to_string()));
    }
    let key = &args[0];
    if key.is_empty() {
        return Err(parse_error(line_no, column, "env() key cannot be empty".to_string()));
    }
    if let Ok(value) = env::var(key) {
        return Ok(Some(value));
    }
    if let Some(default) = args.get(1) {
        return Ok(Some(default.clone()));
    }
    Ok(Some(String::new()))
}

fn parse_env_builtin_args(inner: &str, line_no: usize, column: usize) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for ch in inner.chars() {
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_quote = Some(ch),
            ',' => {
                args.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if in_quote.is_some() {
        return Err(parse_error(
            line_no,
            column,
            "unterminated quote in env() builtin".to_string(),
        ));
    }
    if !current.trim().is_empty() || inner.contains(',') {
        args.push(current.trim().to_string());
    }
    Ok(args)
}

fn task_param_names(task: &TaskDraft) -> BTreeSet<String> {
    task.params.iter().map(|param| param.name.clone()).collect()
}

fn run_dynamic_variable_command(
    command: &str,
    workspace_root: Option<&Path>,
    line_no: usize,
) -> Result<String> {
    const DYNAMIC_COMMAND_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

    let cwd = workspace_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut child_cmd = Command::new("/bin/sh");
    child_cmd.arg("-lc").arg(command).current_dir(cwd);
    child_cmd.env_clear();
    child_cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for key in ["PATH", "HOME", "USER", "SHELL", "TMPDIR", "TERM"] {
        if let Ok(value) = env::var(key) {
            child_cmd.env(key, value);
        }
    }

    let mut child = child_cmd.spawn().with_context(|| {
        format!("spawning dynamic variable command '{}' at line {}", command, line_no)
    })?;
    let stdout_pipe = child.stdout.take().ok_or_else(|| {
        parse_error(line_no, 1, "failed to capture dynamic command stdout".to_string())
    })?;
    let stderr_pipe = child.stderr.take().ok_or_else(|| {
        parse_error(line_no, 1, "failed to capture dynamic command stderr".to_string())
    })?;
    let stdout_reader = thread::spawn(move || {
        read_limited_dynamic_output_stream(stdout_pipe, DYNAMIC_COMMAND_OUTPUT_LIMIT_BYTES)
    });
    let stderr_reader = thread::spawn(move || {
        read_limited_dynamic_output_stream(stderr_pipe, DYNAMIC_COMMAND_OUTPUT_LIMIT_BYTES)
    });

    let timeout = Duration::from_secs(5);
    let start = Instant::now();
    let mut timed_out = false;
    let mut final_status: Option<std::process::ExitStatus> = None;
    loop {
        if let Some(exit_status) =
            child.try_wait().context("waiting for dynamic variable command")?
        {
            final_status = Some(exit_status);
            break;
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let stdout_result = stdout_reader
        .join()
        .map_err(|_| anyhow!("stdout reader thread panicked for dynamic variable command"))?
        .context("reading dynamic variable command stdout")?;
    let stderr_result = stderr_reader
        .join()
        .map_err(|_| anyhow!("stderr reader thread panicked for dynamic variable command"))?
        .context("reading dynamic variable command stderr")?;

    if timed_out {
        return Err(parse_error(
            line_no,
            1,
            format!("dynamic variable command timed out after {}s", timeout.as_secs()),
        ));
    }
    let status = final_status.ok_or_else(|| {
        parse_error(line_no, 1, "dynamic variable command exited without a status".to_string())
    })?;

    if stdout_result.exceeded_limit {
        return Err(parse_error(
            line_no,
            1,
            format!(
                "dynamic variable command stdout exceeded {} byte limit",
                DYNAMIC_COMMAND_OUTPUT_LIMIT_BYTES
            ),
        ));
    }
    if stderr_result.exceeded_limit {
        return Err(parse_error(
            line_no,
            1,
            format!(
                "dynamic variable command stderr exceeded {} byte limit",
                DYNAMIC_COMMAND_OUTPUT_LIMIT_BYTES
            ),
        ));
    }

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_result.bytes);
        return Err(parse_error(
            line_no,
            1,
            format!(
                "dynamic variable command failed with status {}: {}",
                status.code().map_or_else(|| "unknown".to_string(), |code| code.to_string()),
                stderr.trim()
            ),
        ));
    }

    let stdout = String::from_utf8_lossy(&stdout_result.bytes).to_string();
    Ok(stdout.trim_end_matches(['\r', '\n']).to_string())
}

struct LimitedDynamicOutput {
    bytes: Vec<u8>,
    exceeded_limit: bool,
}

fn read_limited_dynamic_output_stream<R: Read>(
    mut reader: R,
    limit_bytes: usize,
) -> std::io::Result<LimitedDynamicOutput> {
    let mut output = Vec::new();
    let mut exceeded_limit = false;
    let mut buffer = [0u8; 8 * 1024];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if !exceeded_limit {
            let remaining = limit_bytes.saturating_add(1).saturating_sub(output.len());
            if read <= remaining {
                output.extend_from_slice(&buffer[..read]);
            } else {
                output.extend_from_slice(&buffer[..remaining]);
                exceeded_limit = true;
            }
            if output.len() > limit_bytes {
                exceeded_limit = true;
            }
        }
    }

    Ok(LimitedDynamicOutput { bytes: output, exceeded_limit })
}

fn parse_error(line: usize, column: usize, message: String) -> anyhow::Error {
    let formatted = format!("DSL parse error at {}:{}: {}", line, column, message);

    DSL_SOURCE.with(|slot| {
        if let Some(source) = slot.borrow().as_ref() {
            let offset = line_col_to_offset(source, line, column);
            let span_end = (offset + 1).min(source.len().max(1));
            let report =
                miette!(labels = vec![LabeledSpan::at(offset..span_end, "here")], "{}", formatted)
                    .with_source_code(source.clone());
            anyhow!(report)
        } else {
            anyhow!(formatted)
        }
    })
}

fn line_col_to_offset(source: &str, line: usize, column: usize) -> usize {
    let mut offset = 0usize;
    let target_line = line.max(1);
    let target_col = column.max(1);

    for (idx, raw_line) in source.split('\n').enumerate() {
        let current_line = idx + 1;
        if current_line == target_line {
            let col_offset = target_col.saturating_sub(1).min(raw_line.len());
            return offset + col_offset;
        }
        offset += raw_line.len() + 1;
    }

    source.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_basic_document() {
        let input = r#"
            version = "0.3"
            @load .env
            alias b = build

            build: prep
                @in src/**/*.rs Cargo.toml
                @stage_ro frontend/node_modules backend/.venv
                @out dist/out.txt
                @env MODE=dev
                @env PATH
                @secret_env API_KEY
                @dir crates/broski-core
                @mode graph
                @isolation off
                @requires cargo rustc
                cargo build --release

            prep:
                echo prep
        "#;

        let parsed = parse_broskifile_dsl(input).expect("parse DSL");
        assert_eq!(parsed.broski.version, "0.3");
        assert_eq!(parsed.load_env, vec![".env".to_string()]);
        assert_eq!(parsed.alias.get("b"), Some(&"build".to_string()));
        let build = parsed.task.get("build").expect("build task");
        assert_eq!(build.inputs.len(), 2);
        assert_eq!(build.stage_ro.len(), 2);
        assert_eq!(build.outputs.len(), 1);
        assert_eq!(build.env.get("MODE"), Some(&"dev".to_string()));
        assert!(build.env_inherit.contains(&"PATH".to_string()));
        assert!(build.secret_env.contains(&"API_KEY".to_string()));
        assert_eq!(build.mode, Some(TaskMode::Graph));
        assert_eq!(build.isolation, Some(crate::model::IsolationMode::Off));
        assert_eq!(build.requires, vec!["cargo".to_string(), "rustc".to_string()]);
    }

    #[test]
    fn rejects_invalid_mode() {
        let input = r#"
            version = "0.3"
            dev:
                @mode invalid
                echo hi
        "#;

        let error = parse_broskifile_dsl(input).expect_err("invalid mode should fail");
        assert!(error.to_string().contains("unknown @mode value"));
    }

    #[test]
    fn requires_version_03_for_dsl() {
        let input = r#"
            version = "0.2"
            dev:
                echo hi
        "#;

        let error = parse_broskifile_dsl(input).expect_err("version mismatch should fail");
        assert!(error.to_string().contains("requires version"));
    }

    #[test]
    fn interpolates_static_variables() {
        let input = r#"
            version = "0.4"
            OUT = "dist/out.txt"

            build:
                @out {{ OUT }}
                echo hi > {{ OUT }}
        "#;

        let parsed = parse_broskifile_dsl(input).expect("parse DSL");
        let build = parsed.task.get("build").expect("build task");
        assert_eq!(build.outputs, vec!["dist/out.txt".to_string()]);
        assert_eq!(build.run_as_shell(), "echo hi > dist/out.txt");
        assert_eq!(build.resolved_variables.get("OUT"), Some(&"dist/out.txt".to_string()));
    }

    #[test]
    fn rejects_undefined_variable_interpolation() {
        let input = r#"
            version = "0.4"

            build:
                @out dist/out.txt
                echo {{ MISSING }} > dist/out.txt
        "#;

        let error = parse_broskifile_dsl(input).expect_err("undefined variable should fail");
        assert!(error.to_string().contains("MISSING"));
    }

    #[test]
    fn rejects_cyclic_variable_references() {
        let input = r#"
            version = "0.4"
            A = "{{ B }}"
            B = "{{ A }}"

            build:
                @out dist/out.txt
                echo {{ A }} > dist/out.txt
        "#;

        let error = parse_broskifile_dsl(input).expect_err("cycle should fail");
        assert!(error.to_string().contains("resolving variable"));
    }

    #[test]
    fn evaluates_dynamic_variable_when_referenced() {
        let input = r#"
            version = "0.4"
            SHA = $(printf abc123)

            build:
                @out dist/out.txt
                echo {{ SHA }} > dist/out.txt
        "#;

        let parsed = parse_broskifile_dsl(input).expect("parse DSL");
        let build = parsed.task.get("build").expect("build task");
        assert_eq!(build.resolved_variables.get("SHA"), Some(&"abc123".to_string()));
        assert_eq!(build.run_as_shell(), "echo abc123 > dist/out.txt");
    }

    #[test]
    fn skips_unused_dynamic_variable_commands() {
        let input = r#"
            version = "0.4"
            UNUSED = $(false)

            build:
                @out dist/out.txt
                echo hi > dist/out.txt
        "#;

        let parsed = parse_broskifile_dsl(input).expect("parse DSL");
        let build = parsed.task.get("build").expect("build task");
        assert!(!build.resolved_variables.contains_key("UNUSED"));
    }

    #[test]
    fn fails_when_dynamic_variable_command_times_out() {
        let input = r#"
            version = "0.4"
            SLOW = $(sleep 6)

            build:
                @out dist/out.txt
                echo {{ SLOW }} > dist/out.txt
        "#;

        let error = parse_broskifile_dsl(input).expect_err("timeout should fail");
        let chain = format!("{error:#}");
        assert!(chain.contains("timed out"));
    }

    #[test]
    fn fails_when_dynamic_variable_command_exceeds_output_limit() {
        let input = r#"
            version = "0.4"
            BIG = $(yes a | head -c 1048577)

            build:
                @out dist/out.txt
                echo {{ BIG }} > dist/out.txt
        "#;

        let error = parse_broskifile_dsl(input).expect_err("size limit should fail");
        let chain = format!("{error:#}");
        assert!(
            chain.contains("dynamic variable command stdout exceeded 1048576 byte limit"),
            "unexpected error chain: {chain}"
        );
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

        let parsed = parse_broskifile_dsl(input).expect("parse DSL");
        let build = parsed.task.get("build").expect("build task");
        assert_eq!(
            build.description.as_deref(),
            Some("Build backend artifacts Uses Cargo release profile")
        );
    }

    #[test]
    fn parses_imported_tasks() {
        let tmp = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(tmp.path().join("shared")).expect("create shared dir");
        fs::write(
            tmp.path().join("shared/broskifile"),
            "version = \"0.5\"\n\nprep:\n    echo prep\n",
        )
        .expect("write imported broskifile");

        let root = "version = \"0.5\"\n@import shared/broskifile\n\nbuild: prep\n    @out dist/out.txt\n    echo hi > dist/out.txt\n";

        let parsed = parse_broskifile_dsl_with_workspace(root, Some(tmp.path())).expect("parse");
        assert!(parsed.task.contains_key("prep"));
        assert!(parsed.task.contains_key("build"));
    }

    #[test]
    fn rejects_circular_imports() {
        let tmp = tempfile::tempdir().expect("temp dir");
        fs::write(
            tmp.path().join("a.broski"),
            "version = \"0.5\"\n@import b.broski\na:\n    echo a\n",
        )
        .expect("write a");
        fs::write(
            tmp.path().join("b.broski"),
            "version = \"0.5\"\n@import a.broski\nb:\n    echo b\n",
        )
        .expect("write b");

        let root = "version = \"0.5\"\n@import a.broski\n\nroot:\n    echo root\n";

        let error = parse_broskifile_dsl_with_workspace(root, Some(tmp.path()))
            .expect_err("circular imports should fail");
        assert!(error.to_string().contains("circular import detected"));
    }

    #[test]
    fn rejects_import_name_collisions() {
        let tmp = tempfile::tempdir().expect("temp dir");
        fs::write(tmp.path().join("first.broski"), "version = \"0.5\"\nbuild:\n    echo first\n")
            .expect("write first");
        fs::write(tmp.path().join("second.broski"), "version = \"0.5\"\nbuild:\n    echo second\n")
            .expect("write second");

        let root = "version = \"0.5\"\n@import first.broski\n@import second.broski\n\nroot:\n    echo root\n";

        let error = parse_broskifile_dsl_with_workspace(root, Some(tmp.path()))
            .expect_err("name collisions should fail");
        let message = error.to_string();
        assert!(message.contains("duplicate task") || message.contains("name collision"));
    }
}
