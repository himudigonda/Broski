use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use broski_cache::LocalArtifactStore;
use broski_core::{
    load_broskifile, sweep_runtime_state, validate_broskifile, Executor, IsolationMode, RunOptions,
    TaskGraph,
};
use broski_store::ArtifactStore;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "broski")]
#[command(about = "Deterministic task runner powered by broskifile")]
#[command(after_help = "Stuck? Visit the Docs Portal: https://himudigonda.me/broski_docs/")]
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
        /// Launch the ratatui dashboard for this run.
        #[arg(long)]
        tui: bool,
        /// Theme name for the dashboard (default, dark, light, high-contrast).
        /// Falls back to the `BROSKI_THEME` env var, then the default theme.
        #[arg(long, value_name = "NAME")]
        theme: Option<String>,
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
    /// Show recent task execution history from the artifact store.
    History {
        /// Optional task to scope to. When omitted, prints the most recent
        /// run per known task (one row per task).
        task: Option<String>,
        /// Maximum number of rows to print.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Launch the live ratatui dashboard for a task.
    Tui {
        task: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        force_isolation: bool,
        #[arg(long)]
        jobs: Option<usize>,
        /// Theme name (default, dark, light, high-contrast). Falls back to
        /// the `BROSKI_THEME` env var, then the default theme.
        #[arg(long, value_name = "NAME")]
        theme: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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
        eprintln!("help: Docs Portal -> https://himudigonda.me/broski_docs/");
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
        Some(Command::History { task, limit }) => run_history_command(&workspace, task, limit),
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
            tui,
            theme,
            args,
        }) => {
            let config = load_and_validate(&workspace)?;
            let cache = LocalArtifactStore::new(cache_root(&workspace))?;

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

            if tui {
                if dry_run || explain || watch {
                    return Err(anyhow!(
                        "--tui cannot be combined with --dry-run, --explain, or --watch"
                    ));
                }
                let theme = resolve_theme(theme.as_deref())?;
                let summary = broski_tui::run(
                    workspace.clone(),
                    config,
                    Arc::new(cache),
                    task,
                    options,
                    theme,
                )?;
                emit_run_summary(&summary, false);
                return Ok(());
            }

            let executor = Executor::new(&workspace, config, Arc::new(cache))?;
            let summary = executor.run_target(&task, &options)?;
            emit_run_summary(&summary, options.explain);
            Ok(())
        }
        Some(Command::Tui { task, force, no_cache, force_isolation, jobs, theme, args }) => {
            let config = load_and_validate(&workspace)?;
            let cache = LocalArtifactStore::new(cache_root(&workspace))?;
            let mut options = RunOptions {
                force,
                no_cache,
                force_isolation,
                passthrough_args: args,
                ..RunOptions::default()
            };
            if let Some(j) = jobs {
                options.jobs = j.max(1);
            }
            let theme = resolve_theme(theme.as_deref())?;
            let summary = broski_tui::run(
                workspace.clone(),
                config,
                Arc::new(cache),
                task,
                options,
                theme,
            )?;
            if !summary.cache_hits.is_empty() {
                println!("cache hits: {}", summary.cache_hits.join(", "));
            }
            if !summary.executed.is_empty() {
                println!("executed: {}", summary.executed.join(", "));
            }
            Ok(())
        }
        Some(Command::Task(raw)) => {
            let invocation = parse_implicit_task_args(raw)?;
            let config = load_and_validate(&workspace)?;
            let cache = LocalArtifactStore::new(cache_root(&workspace))?;
            let executor = Executor::new(&workspace, config, Arc::new(cache))?;
            let mut options = RunOptions {
                dry_run: invocation.dry_run,
                explain: invocation.explain,
                force: invocation.force,
                no_cache: invocation.no_cache,
                watch: invocation.watch,
                force_isolation: invocation.force_isolation,
                passthrough_args: invocation.args,
                ..RunOptions::default()
            };
            if let Some(j) = invocation.jobs {
                options.jobs = j.max(1);
            }

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
    }
}

#[derive(Debug)]
struct ImplicitTaskInvocation {
    task: String,
    dry_run: bool,
    explain: bool,
    force: bool,
    no_cache: bool,
    watch: bool,
    force_isolation: bool,
    jobs: Option<usize>,
    args: Vec<String>,
}

fn parse_implicit_task_args(raw: Vec<String>) -> Result<ImplicitTaskInvocation> {
    let mut iter = raw.into_iter();
    let task =
        iter.next().ok_or_else(|| anyhow!("implicit task execution expected a task name"))?;
    let mut dry_run = false;
    let mut explain = false;
    let mut force = false;
    let mut no_cache = false;
    let mut watch = false;
    let mut force_isolation = false;
    let mut jobs: Option<usize> = None;
    let mut args = Vec::new();
    let mut passthrough_mode = false;
    for token in iter {
        if token == "--" {
            passthrough_mode = true;
            continue;
        }
        if !passthrough_mode {
            match token.as_str() {
                "--dry-run" => {
                    dry_run = true;
                    continue;
                }
                "--explain" => {
                    explain = true;
                    continue;
                }
                "--force" => {
                    force = true;
                    continue;
                }
                "--no-cache" => {
                    no_cache = true;
                    continue;
                }
                "--watch" => {
                    watch = true;
                    continue;
                }
                "--force-isolation" => {
                    force_isolation = true;
                    continue;
                }
                _ => {}
            }

            if let Some(value) = token.strip_prefix("--jobs=") {
                let parsed = value
                    .parse::<usize>()
                    .with_context(|| format!("invalid value for --jobs: {}", value))?;
                jobs = Some(parsed.max(1));
                continue;
            }

            if token == "--jobs" {
                return Err(anyhow!(
                    "implicit task invocation requires --jobs=<n>; use explicit `broski run {task} --jobs <n>` for space-separated jobs"
                ));
            }
        }
        args.push(token);
    }
    Ok(ImplicitTaskInvocation {
        task,
        dry_run,
        explain,
        force,
        no_cache,
        watch,
        force_isolation,
        jobs,
        args,
    })
}

fn load_and_validate(workspace: &Path) -> Result<broski_core::BroskiFile> {
    let config = load_broskifile(workspace).with_context(|| {
        format!("loading broskifile at '{}'", workspace.join("broskifile").display())
    })?;
    validate_broskifile(&config, workspace)?;
    Ok(config)
}

fn run_doctor(workspace: &Path, repair: bool) -> Result<()> {
    let config = load_broskifile(workspace).with_context(|| {
        format!("loading broskifile at '{}'", workspace.join("broskifile").display())
    })?;

    validate_broskifile(&config, workspace)?;

    let sweep = sweep_runtime_state(workspace, repair)?;
    if sweep.active_lock_detected {
        return Err(anyhow!(
            "another Broski execution is active; cannot run doctor sweep while lock is live"
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
        .arg("echo BROSKI_BWRAP_OK")
        .output()
        .context("executing bwrap probe")?;

    if !output.status.success() {
        return Err(anyhow!("bwrap probe command failed with status {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("BROSKI_BWRAP_OK") {
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

fn run_history_command(
    workspace: &Path,
    task: Option<String>,
    limit: usize,
) -> Result<()> {
    use broski_store::ArtifactStore;
    let store = LocalArtifactStore::new(cache_root(workspace))?;
    let rows = store.fetch_history(task.as_deref(), limit)?;
    if rows.is_empty() {
        println!("(no history)");
        return Ok(());
    }
    if task.is_some() {
        let header_when = "WHEN";
        let header_duration = "DURATION";
        let header_fp = "FINGERPRINT";
        println!("{header_when:<24} {header_duration:>10}  {header_fp}");
        for r in rows {
            let when = format_unix_ts(r.created_at);
            let dur = format_duration_ms(r.duration_ms);
            let fp = short_fingerprint(&r.fingerprint);
            println!("{when:<24} {dur:>10}  {fp}");
        }
    } else {
        let header_task = "TASK";
        let header_when = "LAST RUN";
        let header_duration = "DURATION";
        println!("{header_task:<32} {header_when:<24} {header_duration:>10}");
        for r in rows {
            let task = r.task_name;
            let when = format_unix_ts(r.created_at);
            let dur = format_duration_ms(r.duration_ms);
            println!("{task:<32} {when:<24} {dur:>10}");
        }
    }
    Ok(())
}

fn format_unix_ts(unix_secs: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    if unix_secs <= 0 {
        return "?".to_string();
    }
    let when = UNIX_EPOCH + Duration::from_secs(unix_secs as u64);
    let now = std::time::SystemTime::now();
    match now.duration_since(when) {
        Ok(elapsed) => format_relative(elapsed),
        Err(_) => format!("ts:{}", unix_secs), // future timestamp; clock skew
    }
}

fn format_relative(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn format_duration_ms(ms: u64) -> String {
    if ms == 0 {
        return "?".to_string();
    }
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let m = ms / 60_000;
        let s = (ms % 60_000) / 1000;
        format!("{}m{:02}s", m, s)
    }
}

fn short_fingerprint(fp: &str) -> String {
    if fp.len() <= 12 {
        fp.to_string()
    } else {
        format!("{}…", &fp[..12])
    }
}

fn resolve_theme(flag: Option<&str>) -> Result<broski_tui::Theme> {
    if let Some(value) = flag {
        return value
            .parse::<broski_tui::Theme>()
            .with_context(|| format!("parsing --theme value '{}'", value));
    }
    if let Ok(value) = std::env::var("BROSKI_THEME") {
        if !value.trim().is_empty() {
            return value
                .parse::<broski_tui::Theme>()
                .with_context(|| format!("parsing BROSKI_THEME value '{}'", value));
        }
    }
    Ok(broski_tui::Theme::Default)
}

fn emit_run_summary(summary: &broski_core::RunSummary, explain: bool) {
    if !summary.cache_hits.is_empty() {
        println!("cache hits: {}", summary.cache_hits.join(", "));
    }
    if !summary.executed.is_empty() {
        println!("executed: {}", summary.executed.join(", "));
    }
    if !summary.dry_run.is_empty() {
        println!("dry-run: {}", summary.dry_run.join(", "));
    }
    if explain {
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
}

fn cache_root(workspace: &Path) -> PathBuf {
    workspace.join(".broski/cache")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_force_isolation_flag() {
        let cli = Cli::try_parse_from([
            "broski",
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
        let cli = Cli::try_parse_from(["broski", "--workspace", ".", "run", "build", "--explain"])
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
            workspace.join("broskifile"),
            r#"
                version = "0.3"

                example:
                    @in src/input.txt
                    @out dist/out.txt
                    @isolation off
                    echo test > dist/out.txt
            "#,
        )
        .expect("write broskifile");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::write(workspace.join("src/input.txt"), "x").expect("write input");
        fs::create_dir_all(workspace.join(".broski/tx/orphan")).expect("create orphan tx");

        run_doctor(workspace, true).expect("doctor should succeed");
        assert!(!workspace.join(".broski/tx/orphan").exists());
    }

    #[test]
    fn doctor_defaults_to_repair_enabled() {
        let cli =
            Cli::try_parse_from(["broski", "--workspace", ".", "doctor"]).expect("parse doctor");
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
            "broski",
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
            "broski",
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
        let cli = Cli::try_parse_from(["broski", "--workspace", ".", "build"]).expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => assert_eq!(raw, vec!["build"]),
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn parses_implicit_task_with_passthrough() {
        let cli =
            Cli::try_parse_from(["broski", "--workspace", ".", "test", "--", "--grep", "slow"])
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
    fn parses_implicit_task_explain_and_dry_run_flags() {
        let cli =
            Cli::try_parse_from(["broski", "--workspace", ".", "setup", "--explain", "--dry-run"])
                .expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => {
                let parsed = parse_implicit_task_args(raw).expect("normalized args");
                assert_eq!(parsed.task, "setup");
                assert!(parsed.explain);
                assert!(parsed.dry_run);
                assert!(parsed.args.is_empty());
            }
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn implicit_passthrough_separator_keeps_broski_flags_as_task_args() {
        let cli = Cli::try_parse_from([
            "broski",
            "--workspace",
            ".",
            "setup",
            "--",
            "--explain",
            "--dry-run",
        ])
        .expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => {
                let parsed = parse_implicit_task_args(raw).expect("normalized args");
                assert!(!parsed.explain);
                assert!(!parsed.dry_run);
                assert_eq!(parsed.args, vec!["--explain", "--dry-run"]);
            }
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn parses_implicit_task_jobs_equals_form() {
        let cli = Cli::try_parse_from(["broski", "--workspace", ".", "test", "--jobs=4"])
            .expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => {
                let parsed = parse_implicit_task_args(raw).expect("normalized args");
                assert_eq!(parsed.jobs, Some(4));
            }
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn rejects_implicit_task_jobs_space_form() {
        let cli = Cli::try_parse_from(["broski", "--workspace", ".", "test", "--jobs", "4"])
            .expect("parse cli");
        match cli.command {
            Some(Command::Task(raw)) => {
                let error = parse_implicit_task_args(raw).expect_err("jobs should fail");
                assert!(error.to_string().contains("--jobs=<n>"));
            }
            _ => panic!("expected external subcommand task"),
        }
    }

    #[test]
    fn parses_implicit_task_watch_flag() {
        let cli = Cli::try_parse_from(["broski", "--workspace", ".", "test", "--watch"])
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
