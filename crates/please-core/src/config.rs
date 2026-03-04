use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::model::PleaseFile;
use crate::parser_winnow::parse_pleasefile_winnow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserMode {
    Toml,
    Winnow,
}

impl ParserMode {
    pub fn from_env() -> Self {
        match std::env::var("PLEASE_PARSER_MODE") {
            Ok(value) if value.eq_ignore_ascii_case("winnow") => ParserMode::Winnow,
            _ => ParserMode::Toml,
        }
    }
}

pub fn load_pleasefile(workspace_root: &Path) -> Result<PleaseFile> {
    let path = workspace_root.join("pleasefile");
    let content = fs::read_to_string(&path)
        .with_context(|| format!("reading pleasefile from '{}'", path.display()))?;
    parse_pleasefile_with_mode(&content, ParserMode::from_env())
}

pub fn parse_pleasefile(content: &str) -> Result<PleaseFile> {
    parse_pleasefile_with_mode(content, ParserMode::from_env())
}

pub fn parse_pleasefile_with_mode(content: &str, mode: ParserMode) -> Result<PleaseFile> {
    match mode {
        ParserMode::Toml => toml::from_str(content).context("parsing pleasefile TOML"),
        ParserMode::Winnow => parse_pleasefile_winnow(content).context("parsing pleasefile Winnow"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_file() {
        let input = r#"
            [please]
            version = "0.1"

            [task.echo]
            inputs = ["src/main.rs"]
            outputs = ["dist/out.txt"]
            run = "echo hi"
        "#;

        let parsed =
            parse_pleasefile_with_mode(input, ParserMode::Toml).expect("parse minimal file");
        assert_eq!(parsed.please.version, "0.1");
        assert!(parsed.task.contains_key("echo"));
    }

    #[test]
    fn rejects_unknown_keys() {
        let input = r#"
            [please]
            version = "0.1"

            [task.echo]
            outputs = ["dist/out.txt"]
            run = "echo hi"
            unknown = "nope"
        "#;

        assert!(parse_pleasefile_with_mode(input, ParserMode::Toml).is_err());
    }

    #[test]
    fn toml_and_winnow_parsers_are_model_equivalent() {
        let input = r#"
            [please]
            version = "0.1"

            [task.echo]
            deps = []
            inputs = ["src/main.rs"]
            outputs = ["dist/out.txt"]
            env = { MODE = "dev" }
            run = ["echo", "hi"]
            isolation = "best_effort"
        "#;

        let toml = parse_pleasefile_with_mode(input, ParserMode::Toml).expect("parse toml");
        let winnow = parse_pleasefile_with_mode(input, ParserMode::Winnow).expect("parse winnow");

        let toml_json = serde_json::to_value(toml).expect("serialize toml model");
        let winnow_json = serde_json::to_value(winnow).expect("serialize winnow model");
        assert_eq!(toml_json, winnow_json);
    }
}
