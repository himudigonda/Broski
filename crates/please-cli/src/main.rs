use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use please_cache::LocalArtifactStore;
use please_core::{
    load_pleasefile, sweep_runtime_state, validate_pleasefile, Executor, IsolationMode, RunOptions,
    TaskGraph,
};
use please_store::ArtifactStore;

#[derive(Debug, Parser)]
#[command(name = "please")]
#[command(about = "Deterministic task runner powered by pleasefile")]
#[command(version)]
struct Cli {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        task: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        explain: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        watch: bool,
        #[arg(long)]
        force_isolation: bool,
        #[arg(long)]
        jobs: Option<usize>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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
    #[command(external_subcommand)]
    Task(Vec<String>),
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
        if let Some(report) = error.downcast_ref::<miette::Report>() {
            eprintln!("{report:?}");
        } else {
            eprintln!("error: {error:#}");
        }
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
        None => {
            let mut command = Cli::command();
            command.print_long_help().context("printing help output")?;
            println!();
            Ok(())
        }
        Some(Command::Doctor { repair, no_repair }) => run_doctor(&workspace, repair || !no_repair),
        Some(Command::Cache { command }) => run_cache_command(&workspace, command),
        Some(Command::List) => {
            let config = load_and_validate(&workspace)?;
            let graph = TaskGraph::build(&config.task)?;
            for task in graph.all_tasks_sorted() {
                if config.task.get(&task).is_some_and(|spec| spec.private) {
                    continue;
                }
                match config.task.get(&task).and_then(|spec| spec.description.as_deref()) {
                    Some(description) => println!("{task}\t- {description}"),
                    None => println!("{task}"),
                }
            }
            for (alias, target) in &config.alias {
                if config.task.get(target).is_some_and(|spec| spec.private) {
                    continue;
                }
                println!("alias {alias} -> {target}");
            }
            Ok(())
        }
        Some(Command::Graph { task, format }) => {
            let config = load_and_validate(&workspace)?;
            let graph = TaskGraph::build(&config.task)?;
            let resolved_task = config.resolve_task_name(&task)?;
            match format {
                GraphFormat::Text => {
                    let layers = graph.layers_for_target(&resolved_task)?;
                    for (index, layer) in layers.iter().enumerate() {
                        println!("layer {}: {}", index, layer.join(", "));
                    }
                }
                GraphFormat::Dot => {
                    println!("{}", graph.dot_for_target(&resolved_task)?);
                }
            }
            Ok(())
        }
        Some(Command::Run {
            task,
            dry_run,
            explain,
            force,
            no_cache,
            watch,
            force_isolation,
            jobs,
            args,
        }) => {
            let config = load_and_validate(&workspace)?;
            let cache = LocalArtifactStore::new(cache_root(&workspace))?;
            let executor = Executor::new(&workspace, config, Arc::new(cache))?;

            let mut options = RunOptions {
                dry_run,
                explain,
                force,
                no_cache,
                watch,
                force_isolation,
                passthrough_args: args,
                ..RunOptions::default()
            };
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
            if options.explain {
                for (task_name, reasons) in &summary.cache_miss_reasons {
                    println!("explain {}:", task_name);
                    for reason in reasons.iter().take(10) {
                        println!("- {}", reason);
                    }
                    if reasons.len() > 10 {
                        println!("- +{} more changes", reasons.len() - 10);
                    }
                }
            }

            Ok(())
        }
        Some(Command::Task(raw)) => {
            let invocation = parse_implicit_task_args(raw)?;
            let config = load_and_validate(&workspace)?;
            let cache = LocalArtifactStore::new(cache_root(&workspace))?;
            let executor = Executor::new(&workspace, config, Arc::new(cache))?;
            let options = RunOptions {
                watch: invocation.watch,
                passthrough_args: invocation.args,
                ..RunOptions::default()
            };

            let summary = executor.run_target(&invocation.task, &options)?;
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

struct ImplicitTaskInvocation {
    task: String,
    watch: bool,
    args: Vec<String>,
}

fn parse_implicit_task_args(raw: Vec<String>) -> Result<ImplicitTaskInvocation> {
    let mut iter = raw.into_iter();
    let task =
        iter.next().ok_or_else(|| anyhow!("implicit task execution expected a task name"))?;
    let mut watch = false;
    let mut args = Vec::new();
    let mut passthrough_mode = false;
    for token in iter {
        if token == "--" {
            passthrough_mode = true;
            continue;
        }
        if !passthrough_mode && token == "--watch" {
            watch = true;
            continue;
        }
        args.push(token);
    }
    Ok(ImplicitTaskInvocation { task, watch, args })
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

    let mut strict_probe_warning: Option<String> = None;
    if cfg!(target_os = "linux") && !strict_tasks.is_empty() {
        if let Err(error) = probe_linux_bwrap() {
            strict_probe_warning = Some(format!("strict isolation doctor probe failed: {}", error));
        }
    }
    if cfg!(target_os = "macos") && !strict_tasks.is_empty() {
        strict_probe_warning = Some(
            "strict isolation tasks are configured but strict sandboxing is unsupported on macOS"
                .to_string(),
        );
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
        if let Some(warning) = strict_probe_warning {
            println!("strict isolation probe: warning");
            println!("- {}", warning);
        } else if cfg!(target_os = "linux") {
            println!("strict isolation probe: ok");
        }
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
            "--watch",
            "--force-isolation",
        ])
        .expect("parse cli");

        match cli.command {
            Some(Command::Run { force_isolation, watch, .. }) => {
                assert!(force_isolation);
                assert!(watch);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_explain_flag() {
        let cli = Cli::try_parse_from(["please", "--workspace", ".", "run", "build", "--explain"])
            .expect("parse cli");
        match cli.command {
            Some(Command::Run { explain, .. }) => assert!(explain),
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
                version = "0.3"

                example:
                    @in src/input.txt
                    @out dist/out.txt
                    @isolation off
                    echo test > dist/out.txt
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
            Some(Command::Doctor { repair, no_repair }) => {
                let effective_repair = repair || !no_repair;
                assert!(effective_repair);
            }
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn parses_passthrough_args() {
        let cli = Cli::try_parse_from([
            "please",
            "--workspace",
            ".",
            "run",
            "test",
            "--",
            "--watch",
            "--grep",
            "slow suite",
        ])
        .expect("parse cli");

        match cli.command {
            Some(Command::Run { args, .. }) => {
                assert_eq!(args, vec!["--watch", "--grep", "slow suite"]);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_passthrough_args_without_separator() {
        let cli = Cli::try_parse_from([
            "please",
            "--workspace",
            ".",
            "run",
            "test",
            "-v",
            "--grep",
            "slow",
        ])
        .expect("parse cli");

        match cli.command {
            Some(Command::Run { args, .. }) => {
                assert_eq!(args, vec!["-v", "--grep", "slow"]);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_implicit_task_invocation() {
        let cli = Cli::try_parse_from(["please", "--workspace", ".", "build"]).expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => assert_eq!(raw, vec!["build"]),
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn parses_implicit_task_with_passthrough() {
        let cli =
            Cli::try_parse_from(["please", "--workspace", ".", "test", "--", "--grep", "slow"])
                .expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => {
                let parsed = parse_implicit_task_args(raw).expect("normalized args");
                assert_eq!(parsed.task, "test");
                assert!(!parsed.watch);
                assert_eq!(parsed.args, vec!["--grep", "slow"]);
            }
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn parses_implicit_task_watch_flag() {
        let cli = Cli::try_parse_from(["please", "--workspace", ".", "test", "--watch"])
            .expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => {
                let parsed = parse_implicit_task_args(raw).expect("normalized args");
                assert_eq!(parsed.task, "test");
                assert!(parsed.watch);
                assert!(parsed.args.is_empty());
            }
            _ => panic!("expected external subcommand task"),
        }
    }
}
