use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ignore::WalkBuilder;

pub fn resolve_inputs(workspace_root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>> {
    let mut resolved = BTreeSet::new();

    for pattern in patterns {
        validate_pattern(pattern)?;

        if looks_like_glob(pattern) {
            let matcher = glob::Pattern::new(pattern)
                .with_context(|| format!("invalid input glob '{}'", pattern))?;
            let mut builder = WalkBuilder::new(workspace_root);
            builder.git_ignore(true).git_exclude(true).git_global(true).require_git(false);
            let walker = builder.build();
            let mut found = false;
            for entry in walker {
                let entry = entry.with_context(|| format!("expanding input glob '{}'", pattern))?;
                let path = entry.path();
                if path == workspace_root {
                    continue;
                }
                let rel = path
                    .strip_prefix(workspace_root)
                    .with_context(|| {
                        format!(
                            "input path '{}' escaped workspace root '{}'",
                            path.display(),
                            workspace_root.display()
                        )
                    })?
                    .to_path_buf();
                if matcher.matches_path(&rel) {
                    resolved.insert(rel);
                    found = true;
                }
            }

            if !found {
                resolved.insert(PathBuf::from(pattern));
            }
        } else {
            let normalized = normalize_relative_path(pattern)?;
            resolved.insert(normalized);
        }
    }

    Ok(resolved.into_iter().collect())
}

pub fn normalize_relative_path(raw: &str) -> Result<PathBuf> {
    if raw.trim().is_empty() {
        return Err(anyhow!("path cannot be empty"));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(anyhow!("path '{}' must be relative", raw));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                return Err(anyhow!("path '{}' cannot contain '..' segments in broskifile", raw))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("path '{}' must be relative", raw))
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(anyhow!("path '{}' cannot resolve to current directory", raw));
    }

    Ok(normalized)
}

fn validate_pattern(pattern: &str) -> Result<()> {
    if pattern.trim().is_empty() {
        return Err(anyhow!("input pattern cannot be empty"));
    }
    if pattern.starts_with('/') {
        return Err(anyhow!("input pattern '{}' must be relative to workspace", pattern));
    }
    if pattern.split('/').any(|segment| segment == "..") {
        return Err(anyhow!("input pattern '{}' cannot contain '..' segments", pattern));
    }
    Ok(())
}

fn looks_like_glob(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[') || pattern.contains('{')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn normalizes_valid_relative_paths() {
        let normalized = normalize_relative_path("./src//main.rs").expect("normalize path");
        assert_eq!(normalized, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn rejects_parent_segments() {
        assert!(normalize_relative_path("../secret").is_err());
    }

    #[test]
    fn resolves_recursive_glob_inputs() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let workspace = tmp.path();

        fs::create_dir_all(workspace.join("src/nested")).expect("create nested dirs");
        fs::write(
            workspace.join("src/a.rs"),
            "fn a() {}
",
        )
        .expect("write src/a.rs");
        fs::write(
            workspace.join("src/nested/b.rs"),
            "fn b() {}
",
        )
        .expect("write src/nested/b.rs");
        fs::write(workspace.join("src/nested/c.txt"), "ignore").expect("write src/nested/c.txt");

        let patterns = vec!["src/**/*.rs".to_string()];
        let resolved = resolve_inputs(workspace, &patterns).expect("resolve glob inputs");

        assert_eq!(resolved, vec![PathBuf::from("src/a.rs"), PathBuf::from("src/nested/b.rs")]);
    }

    #[test]
    fn single_star_glob_respects_gitignore_like_double_star() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let workspace = tmp.path();

        fs::create_dir_all(workspace.join("gen")).expect("create gen dir");
        fs::write(workspace.join(".gitignore"), "gen/ignored.rs\n").expect("write .gitignore");
        fs::write(workspace.join("gen/kept.rs"), "fn kept() {}\n").expect("write gen/kept.rs");
        fs::write(workspace.join("gen/ignored.rs"), "fn ignored() {}\n")
            .expect("write gen/ignored.rs");

        let single_star =
            resolve_inputs(workspace, &["gen/*.rs".to_string()]).expect("resolve single-star glob");
        let double_star = resolve_inputs(workspace, &["gen/**/*.rs".to_string()])
            .expect("resolve double-star glob");

        assert_eq!(single_star, vec![PathBuf::from("gen/kept.rs")]);
        assert_eq!(single_star, double_star);
    }
}
