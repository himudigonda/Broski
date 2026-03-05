use std::fs;
use std::path::Path;
use std::sync::Once;

use anyhow::{Context, Result};

use crate::model::PleaseFile;
use crate::parser_winnow::parse_pleasefile_dsl_with_workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserMode {
    Auto,
    Toml,
    Dsl,
}

impl ParserMode {
    pub fn from_env() -> Self {
        match std::env::var("PLEASE_PARSER_MODE") {
            Ok(value) if value.eq_ignore_ascii_case("toml") => ParserMode::Toml,
            Ok(value) if value.eq_ignore_ascii_case("dsl") => ParserMode::Dsl,
            _ => ParserMode::Auto,
        }
    }
}

pub fn load_pleasefile(workspace_root: &Path) -> Result<PleaseFile> {
    let path = workspace_root.join("pleasefile");
    let content = fs::read_to_string(&path)
        .with_context(|| format!("reading pleasefile from '{}'", path.display()))?;
    parse_pleasefile_with_mode_at(&content, ParserMode::from_env(), Some(workspace_root))
}

pub fn parse_pleasefile(content: &str) -> Result<PleaseFile> {
    parse_pleasefile_with_mode(content, ParserMode::from_env())
}

pub fn parse_pleasefile_with_mode(content: &str, mode: ParserMode) -> Result<PleaseFile> {
    parse_pleasefile_with_mode_at(content, mode, None)
}

fn parse_pleasefile_with_mode_at(
    content: &str,
    mode: ParserMode,
    workspace_root: Option<&Path>,
) -> Result<PleaseFile> {
    match mode {
        ParserMode::Toml => parse_toml(content),
        ParserMode::Dsl => {
            let parsed = parse_pleasefile_dsl_with_workspace(content, workspace_root)?;
            warn_dsl_03_deprecated_if_needed(&parsed);
            Ok(parsed)
        }
        ParserMode::Auto => {
            if looks_like_toml(content) {
                warn_toml_deprecated();
                parse_toml(content)
            } else {
                let parsed = parse_pleasefile_dsl_with_workspace(content, workspace_root)?;
                warn_dsl_03_deprecated_if_needed(&parsed);
                Ok(parsed)
            }
        }
    }
}

fn parse_toml(content: &str) -> Result<PleaseFile> {
    toml::from_str(content).context("parsing pleasefile TOML")
}

fn looks_like_toml(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return trimmed == "[please]"
            || trimmed.starts_with("[task.")
            || trimmed.starts_with("[alias")
            || trimmed.starts_with("[load");
    }
    false
}

fn warn_toml_deprecated() {
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        eprintln!(
            "warning: TOML pleasefile is deprecated; migrate to DSL (version = \"0.3\") before v0.5"
        );
    });
}

fn warn_dsl_03_deprecated_if_needed(parsed: &PleaseFile) {
    if parsed.please.version == "0.3" {
        static WARN_ONCE: Once = Once::new();
        WARN_ONCE.call_once(|| {
            eprintln!(
                "warning: pleasefile DSL version \"0.3\" is deprecated; migrate to version = \"0.4\" before v0.5"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_dsl_file() {
        let input = r#"
            version = "0.3"

            echo:
                @out dist/out.txt
                echo hi > dist/out.txt
        "#;

        let parsed =
            parse_pleasefile_with_mode(input, ParserMode::Dsl).expect("parse minimal file");
        assert_eq!(parsed.please.version, "0.3");
        assert!(parsed.task.contains_key("echo"));
    }

    #[test]
    fn parses_toml_file_with_explicit_mode() {
        let input = r#"
            [please]
            version = "0.2"

            [task.echo]
            inputs = ["src/main.rs"]
            outputs = ["dist/out.txt"]
            run = "echo hi"
        "#;

        let parsed = parse_pleasefile_with_mode(input, ParserMode::Toml).expect("parse toml");
        assert_eq!(parsed.please.version, "0.2");
        assert!(parsed.task.contains_key("echo"));
    }

    #[test]
    fn autodetect_prefers_dsl_when_no_toml_sections() {
        let input = r#"
            version = "0.3"

            hello:
                echo hi
        "#;

        let parsed = parse_pleasefile_with_mode(input, ParserMode::Auto).expect("parse dsl auto");
        assert_eq!(parsed.please.version, "0.3");
        assert!(parsed.task.contains_key("hello"));
    }

    #[test]
    fn autodetect_supports_toml_fallback() {
        let input = r#"
            [please]
            version = "0.2"

            [task.hello]
            inputs = ["src/main.rs"]
            outputs = ["dist/out.txt"]
            run = "echo hi"
        "#;

        let parsed = parse_pleasefile_with_mode(input, ParserMode::Auto).expect("parse toml auto");
        assert_eq!(parsed.please.version, "0.2");
        assert!(parsed.task.contains_key("hello"));
    }
}
