use std::collections::BTreeMap;

use anyhow::{anyhow, Context, Result};
use winnow::token::rest;

use crate::model::PleaseFile;

#[derive(Debug, Clone, Default)]
struct AstDocument {
    please: BTreeMap<String, toml::Value>,
    tasks: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Clone)]
enum Section {
    None,
    Please,
    Task(String),
}

#[derive(Debug, Clone)]
enum SectionHeader {
    Please,
    Task(String),
}

pub fn parse_pleasefile_winnow(content: &str) -> Result<PleaseFile> {
    let mut ast = AstDocument::default();
    let mut section = Section::None;

    let mut iter = content.lines();
    while let Some(line) = iter.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(header) = parse_section_header(trimmed)? {
            section = match header {
                SectionHeader::Please => Section::Please,
                SectionHeader::Task(task_name) => {
                    ast.tasks.entry(task_name.clone()).or_default();
                    Section::Task(task_name)
                }
            };
            continue;
        }

        let (key, mut raw_value) = parse_key_value(trimmed)?
            .ok_or_else(|| anyhow!("invalid line in pleasefile: '{}'", line.trim()))?;

        if needs_multiline_capture(&raw_value) {
            for next_line in iter.by_ref() {
                raw_value.push('\n');
                raw_value.push_str(next_line);
                if multiline_complete(&raw_value) {
                    break;
                }
            }
        }

        let value = parse_toml_value(&raw_value)
            .with_context(|| format!("parsing value for key '{}'", key))?;

        match &section {
            Section::Please => {
                ast.please.insert(key, value);
            }
            Section::Task(task_name) => {
                let task = ast.tasks.entry(task_name.clone()).or_default();
                task.insert(key, value);
            }
            Section::None => {
                return Err(anyhow!("key-value pair found outside a section: '{}'", line.trim()));
            }
        }
    }

    ast_to_model(ast)
}

fn ast_to_model(ast: AstDocument) -> Result<PleaseFile> {
    let mut root = toml::map::Map::new();

    let mut please_table = toml::map::Map::new();
    for (key, value) in ast.please {
        please_table.insert(key, value);
    }
    root.insert("please".to_string(), toml::Value::Table(please_table));

    let mut tasks_table = toml::map::Map::new();
    for (task_name, values) in ast.tasks {
        let mut entry_table = toml::map::Map::new();
        for (key, value) in values {
            entry_table.insert(key, value);
        }
        tasks_table.insert(task_name, toml::Value::Table(entry_table));
    }
    root.insert("task".to_string(), toml::Value::Table(tasks_table));

    toml::Value::Table(root).try_into().context("mapping winnow AST into PleaseFile model")
}

fn parse_section_header(input_line: &str) -> Result<Option<SectionHeader>> {
    if !input_line.starts_with('[') {
        return Ok(None);
    }

    if !input_line.ends_with(']') {
        return Err(anyhow!("invalid section header '{}': missing closing ']'", input_line));
    }
    let body = input_line[1..input_line.len() - 1].trim();
    if body.is_empty() {
        return Err(anyhow!("invalid section header '{}': empty body", input_line));
    }
    if body.contains('[') || body.contains(']') {
        return Err(anyhow!(
            "invalid section header '{}': unexpected trailing content",
            input_line
        ));
    }

    if body == "please" {
        return Ok(Some(SectionHeader::Please));
    }

    if let Some(task_name) = body.strip_prefix("task.") {
        if task_name.is_empty() {
            return Err(anyhow!("task section must include a task name"));
        }
        return Ok(Some(SectionHeader::Task(task_name.to_string())));
    }

    Err(anyhow!("unknown section header '[{}]'", body))
}

fn parse_key_value(line: &str) -> Result<Option<(String, String)>> {
    if line.starts_with('[') {
        return Ok(None);
    }

    let Some((raw_key, raw_value)) = line.split_once('=') else {
        return Err(anyhow!("invalid key-value line '{}': missing '='", line));
    };

    let key = raw_key.trim();
    if key.is_empty() {
        return Err(anyhow!("invalid key-value line '{}': missing key", line));
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(anyhow!("invalid key-value line '{}': unexpected key content", line));
    }

    let mut value_input = raw_value;
    let value: &str = rest::<_, winnow::error::ContextError>(&mut value_input)
        .map_err(|_| anyhow!("invalid key-value line '{}': missing value", line))?;

    Ok(Some((key.to_string(), value.trim().to_string())))
}

fn parse_toml_value(raw: &str) -> Result<toml::Value> {
    let snippet = format!("value = {}", raw);
    let parsed: toml::Value = toml::from_str(&snippet).context("parsing TOML value expression")?;
    parsed
        .get("value")
        .cloned()
        .ok_or_else(|| anyhow!("value expression did not produce a TOML value"))
}

fn needs_multiline_capture(raw_value: &str) -> bool {
    raw_value.trim_start().starts_with("\"\"\"") && !multiline_complete(raw_value)
}

fn multiline_complete(raw_value: &str) -> bool {
    raw_value.matches("\"\"\"").count() >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_document() {
        let input = r#"
            [please]
            version = "0.1"

            [task.build]
            deps = ["prep"]
            inputs = ["src/main.rs"]
            outputs = ["dist/out"]
            env = { MODE = "dev" }
            run = "echo hi"
        "#;

        let parsed = parse_pleasefile_winnow(input).expect("parse with winnow");
        assert_eq!(parsed.please.version, "0.1");
        assert!(parsed.task.contains_key("build"));
    }

    #[test]
    fn parses_multiline_run_block() {
        let input = r#"
            [please]
            version = "0.1"

            [task.echo]
            inputs = ["src/main.rs"]
            outputs = ["dist/out.txt"]
            run = """
              echo hello
              echo world
            """
        "#;

        let parsed = parse_pleasefile_winnow(input).expect("parse with winnow");
        let run = parsed.task.get("echo").expect("task echo").run_as_shell();
        assert!(run.contains("echo hello"));
        assert!(run.contains("echo world"));
    }
}
