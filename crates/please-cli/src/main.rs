use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use please_cache::LocalArtifactStore;
use please_core::{
    load_pleasefile, sweep_runtime_state, validate_pleasefile, Executor, IsolationMode, RunOptions,
    TaskGraph,
};
use please_store::ArtifactStore;

#[derive(Debug, Parser)]
#[command(name = "please")]
#[command(about = "Deterministic task runner powered by pleasefile")]
struct Cli {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        task: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        force_isolation: bool,
        #[arg(long)]
        jobs: Option<usize>,
    },
    List,
    Graph {
        task: String,
        #[arg(long, value_enum, default_value = "text")]
        format: GraphFormat,
    },
    Doctor {
        #[arg(long, conflicts_with = "no_repair")]
        repair: bool,
        #[arg(long = "no-repair")]
        no_repair: bool,
    },
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CacheCommand {
    Prune {
        #[arg(long, default_value_t = 512)]
        max_size: u64,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum GraphFormat {
    Text,
    Dot,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:?}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let workspace = cli
        .workspace
        .canonicalize()
        .or_else(|_| Ok::<PathBuf, anyhow::Error>(cli.workspace.clone()))?;

    match cli.command {
        Command::Doctor { repair, no_repair } => run_doctor(&workspace, repair || !no_repair),
        Command::Cache { command } => run_cache_command(&workspace, command),
        Command::List => {
            let config = load_and_validate(&workspace)?;
            let graph = TaskGraph::build(&config.task)?;
            for task in graph.all_tasks_sorted() {
                println!("{task}");
            }
            Ok(())
        }
        Command::Graph { task, format } => {
            let config = load_and_validate(&workspace)?;
            let graph = TaskGraph::build(&config.task)?;
            match format {
                GraphFormat::Text => {
                    let layers = graph.layers_for_target(&task)?;
                    for (index, layer) in layers.iter().enumerate() {
                        println!("layer {}: {}", index, layer.join(", "));
                    }
                }
                GraphFormat::Dot => {
                    println!("{}", graph.dot_for_target(&task)?);
                }
            }
            Ok(())
        }
        Command::Run { task, dry_run, force, no_cache, force_isolation, jobs } => {
            let config = load_and_validate(&workspace)?;
            let cache = LocalArtifactStore::new(cache_root(&workspace))?;
            let executor = Executor::new(&workspace, config, Arc::new(cache))?;

            let mut options =
                RunOptions { dry_run, force, no_cache, force_isolation, ..RunOptions::default() };
            if let Some(j) = jobs {
                options.jobs = j.max(1);
            }

            let summary = executor.run_target(&task, &options)?;

            if !summary.cache_hits.is_empty() {
                println!("cache hits: {}", summary.cache_hits.join(", "));
            }
            if !summary.executed.is_empty() {
                println!("executed: {}", summary.executed.join(", "));
            }
            if !summary.dry_run.is_empty() {
                println!("dry-run: {}", summary.dry_run.join(", "));
            }

            Ok(())
        }
    }
}

fn load_and_validate(workspace: &Path) -> Result<please_core::PleaseFile> {
    let config = load_pleasefile(workspace).with_context(|| {
        format!("loading pleasefile at '{}'", workspace.join("pleasefile").display())
    })?;
    validate_pleasefile(&config, workspace)?;
    Ok(config)
}

fn run_doctor(workspace: &Path, repair: bool) -> Result<()> {
    let config = load_pleasefile(workspace).with_context(|| {
        format!("loading pleasefile at '{}'", workspace.join("pleasefile").display())
    })?;

    validate_pleasefile(&config, workspace)?;

    let sweep = sweep_runtime_state(workspace, repair)?;
    if sweep.active_lock_detected {
        return Err(anyhow!(
            "another Please execution is active; cannot run doctor sweep while lock is live"
        ));
    }

    let mut strict_tasks = Vec::new();
    for (name, task) in &config.task {
        if task.effective_isolation() == IsolationMode::Strict {
            strict_tasks.push(name.clone());
        }
    }

    if cfg!(target_os = "linux") && !strict_tasks.is_empty() {
        probe_linux_bwrap().context("strict isolation doctor probe failed")?;
    }
    if cfg!(target_os = "macos") && !strict_tasks.is_empty() {
        return Err(anyhow!(
            "strict isolation tasks are configured but strict sandboxing is unsupported on macOS"
        ));
    }

    println!("doctor: ok");
    println!("workspace: {}", workspace.display());
    println!("tasks: {}", config.task.len());
    println!("repair mode: {}", if repair { "enabled" } else { "disabled" });
    if sweep.stale_lock_detected {
        println!(
            "runtime lock: stale detected (removed: {})",
            if sweep.stale_lock_removed { "yes" } else { "no" }
        );
    }
    println!(
        "sweep cleanup: stage={} tx={}",
        sweep.stage_entries_removed, sweep.tx_entries_removed
    );
    if strict_tasks.is_empty() {
        println!("isolation: no strict tasks declared");
    } else {
        println!("strict isolation tasks: {}", strict_tasks.join(", "));
    }

    Ok(())
}

fn probe_linux_bwrap() -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }

    let bwrap = which::which("bwrap").context("strict isolation requires `bwrap` on PATH")?;
    let output = ProcessCommand::new(bwrap)
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--unshare-net")
        .arg("--ro-bind")
        .arg("/")
        .arg("/")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("/bin/sh")
        .arg("-lc")
        .arg("echo PLEASE_BWRAP_OK")
        .output()
        .context("executing bwrap probe")?;

    if !output.status.success() {
        return Err(anyhow!("bwrap probe command failed with status {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("PLEASE_BWRAP_OK") {
        return Err(anyhow!("bwrap probe output missing token; got: {}", stdout.trim()));
    }

    Ok(())
}

fn run_cache_command(workspace: &Path, command: CacheCommand) -> Result<()> {
    let store = LocalArtifactStore::new(cache_root(workspace))?;

    match command {
        CacheCommand::Prune { max_size } => {
            let report = store.prune(max_size)?;
            println!(
                "pruned objects: {} (freed {} bytes), remaining {} bytes",
                report.removed_objects, report.removed_bytes, report.remaining_bytes
            );
        }
    }

    Ok(())
}

fn cache_root(workspace: &Path) -> PathBuf {
    workspace.join(".please/cache")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_force_isolation_flag() {
        let cli = Cli::try_parse_from([
            "please",
            "--workspace",
            ".",
            "run",
            "build",
            "--force-isolation",
        ])
        .expect("parse cli");

        match cli.command {
            Command::Run { force_isolation, .. } => assert!(force_isolation),
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn doctor_repairs_orphaned_tx_entries() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let workspace = temp.path();

        fs::write(
            workspace.join("pleasefile"),
            r#"
                [please]
                version = "0.1"

                [task.example]
                inputs = ["src/input.txt"]
                outputs = ["dist/out.txt"]
                run = "echo test > dist/out.txt"
            "#,
        )
        .expect("write pleasefile");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::write(workspace.join("src/input.txt"), "x").expect("write input");
        fs::create_dir_all(workspace.join(".please/tx/orphan")).expect("create orphan tx");

        run_doctor(workspace, true).expect("doctor should succeed");
        assert!(!workspace.join(".please/tx/orphan").exists());
    }

    #[test]
    fn doctor_defaults_to_repair_enabled() {
        let cli =
            Cli::try_parse_from(["please", "--workspace", ".", "doctor"]).expect("parse doctor");
        match cli.command {
            Command::Doctor { repair, no_repair } => {
                let effective_repair = repair || !no_repair;
                assert!(effective_repair);
            }
            _ => panic!("expected doctor command"),
        }
    }
}
