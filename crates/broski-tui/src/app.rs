//! TUI application loop.
//!
//! Threading model:
//! - Background thread owns the `Executor` and runs the target. It pipes
//!   `ProgressEvent`s to the foreground via mpsc.
//! - Foreground thread owns the terminal: drains events, polls keys, redraws.
//! - On user quit, foreground tears down the terminal and joins the bg thread
//!   (executor finishes naturally; we don't yet have cancellation).

use std::collections::BTreeMap;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use broski_core::cancel::{CancelLevel, CancellationToken};
use broski_core::{
    load_broskifile, validate_broskifile, BroskiFile, Executor, ProgressEvent, RunOptions,
    RunSummary, TaskGraph, TaskMode,
};
use broski_store::ArtifactStore;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Terminal;

use crate::keys::{map_key, Action};
use crate::launcher::{
    LauncherAction, LauncherCtx, LauncherDecision, LauncherState, ParsedCommand, RunOutcome,
    TaskMeta,
};
use crate::state::{CancelState, RerunRequest, TuiState};
use crate::theme::{Palette, Theme};
use crate::widgets::dag::DagWidget;
use crate::widgets::help::HelpFooter;
use crate::widgets::launcher::LauncherWidget;
use crate::widgets::logs::LogsWidget;
use crate::widgets::summary::SummaryWidget;

/// Outcome returned by `drive_loop` to the run-orchestration caller.
enum LoopDecision {
    /// User pressed `q` / Esc / second Ctrl-C.
    Quit,
    /// User pressed `x` or `X` after the run finished.
    Rerun(RerunRequest),
}

/// Foreground poll cadence. Keys come in via `event::poll` so this also
/// caps the redraw rate when no events are arriving.
const TICK_MS: u64 = 75;

/// Window during which a second Ctrl-C escalates from soft to hard cancel.
const CANCEL_WINDOW: Duration = Duration::from_secs(2);

/// Launch the TUI for a given target. Blocks until the user quits.
///
/// The terminal is restored even if the inner work panics. ETAs for the
/// resolved task graph are prefetched from the artifact store before any
/// pixels are drawn, so the dashboard shows estimates from frame zero.
pub fn run(
    workspace: PathBuf,
    config: BroskiFile,
    store: Arc<dyn ArtifactStore>,
    target: String,
    base_options: RunOptions,
    theme: Theme,
) -> Result<RunSummary> {
    let mut terminal = enter_terminal().context("entering alt screen / raw mode")?;
    let palette = theme.palette();
    let result = run_target_in_terminal(
        &mut terminal,
        workspace,
        config,
        store,
        &target,
        base_options,
        &palette,
    );
    let _ = leave_terminal(&mut terminal);
    let (summary, _outcome) = result?;
    Ok(summary)
}

/// Launch the TUI in launcher / REPL mode. The user picks targets one at a
/// time; each finished run pops back to the launcher with a banner. The TUI
/// only exits on `q` or Ctrl-C from the launcher screen.
pub fn run_launcher(
    workspace: PathBuf,
    config: BroskiFile,
    store: Arc<dyn ArtifactStore>,
    base_options: RunOptions,
    theme: Theme,
) -> Result<()> {
    let mut terminal = enter_terminal().context("entering alt screen / raw mode")?;
    let result = drive_launcher(&mut terminal, &workspace, &config, &store, &base_options, theme);
    let _ = leave_terminal(&mut terminal);
    result
}

fn drive_launcher(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace: &Path,
    config: &BroskiFile,
    store: &Arc<dyn ArtifactStore>,
    base_options: &RunOptions,
    theme: Theme,
) -> Result<()> {
    // The user's *requested* theme (e.g. Auto). Stays put across live
    // switches — we only update the resolved theme/palette below.
    let requested_theme = theme;
    let mut current_theme = theme.resolved();
    let mut palette = current_theme.palette();
    let workspace_display = workspace.display().to_string();
    let mut current_config = config.clone();
    let mut launcher = LauncherState::new(visible_task_names(&current_config));
    let mut ctx = build_launcher_ctx(
        workspace,
        &workspace_display,
        &current_config,
        store.as_ref(),
        requested_theme,
        current_theme,
    );
    let tick = Duration::from_millis(TICK_MS);
    let mut dirty = true;

    loop {
        if dirty {
            redraw_launcher(terminal, &launcher, &ctx, &palette)?;
            dirty = false;
        }

        let has_input = event::poll(tick).context("polling launcher events")?;
        if !has_input {
            continue;
        }
        match event::read().context("reading launcher event")? {
            Event::Key(key) => {
                let action = map_launcher_key(key);
                match launcher.apply(action) {
                    LauncherDecision::Continue => dirty = true,
                    LauncherDecision::Quit => return Ok(()),
                    LauncherDecision::Run(cmd) => {
                        let started = Instant::now();
                        let outcome = match run_target_in_terminal(
                            terminal,
                            workspace.to_path_buf(),
                            current_config.clone(),
                            store.clone(),
                            &cmd.target,
                            options_with_passthrough(base_options, &cmd),
                            &palette,
                        ) {
                            Ok((_, outcome)) => outcome,
                            Err(_) => RunOutcome::Failed,
                        };
                        launcher.record_run(cmd.target, outcome, started.elapsed());
                        // Refresh task_meta + cache_stats: a successful
                        // run wrote a new ExecutionRecord and may have
                        // grown the cache.
                        ctx = build_launcher_ctx(
                            workspace,
                            &workspace_display,
                            &current_config,
                            store.as_ref(),
                            requested_theme,
                            current_theme,
                        );
                        dirty = true;
                    }
                    LauncherDecision::SwitchTheme(requested) => {
                        // Resolved live: terminal-light toggles raw mode
                        // internally for the OSC 11 round-trip, so this
                        // is safe even mid-session.
                        let resolved = requested.resolved();
                        current_theme = resolved;
                        palette = resolved.palette();
                        ctx.theme_resolved_name = resolved.name().to_string();
                        ctx.theme_requested_name = if requested == resolved {
                            None
                        } else {
                            Some(requested.name().to_string())
                        };
                        launcher.record_status(format!(
                            "theme: {} (was {})",
                            resolved.name(),
                            requested.name()
                        ));
                        dirty = true;
                    }
                    LauncherDecision::Refresh => {
                        match reload_broskifile(workspace) {
                            Ok(reloaded) => {
                                current_config = reloaded;
                                launcher.replace_tasks(visible_task_names(&current_config));
                                ctx = build_launcher_ctx(
                                    workspace,
                                    &workspace_display,
                                    &current_config,
                                    store.as_ref(),
                                    requested_theme,
                                    current_theme,
                                );
                                launcher.record_status("reloaded broskifile");
                            }
                            Err(err) => {
                                launcher.record_status(format!("refresh failed: {err}"));
                            }
                        }
                        dirty = true;
                    }
                    LauncherDecision::PruneCache(mb) => {
                        match store.prune(mb) {
                            Ok(report) => {
                                let mb_freed = report.removed_bytes / (1024 * 1024);
                                launcher.record_status(format!(
                                    "pruned {} object(s), freed ~{} MB, remaining ~{} MB",
                                    report.removed_objects,
                                    mb_freed,
                                    report.remaining_bytes / (1024 * 1024),
                                ));
                                if let Ok(stats) = store.stats() {
                                    ctx.cache_stats = stats;
                                }
                            }
                            Err(err) => {
                                launcher.record_status(format!("prune failed: {err}"));
                            }
                        }
                        dirty = true;
                    }
                }
            }
            Event::Resize(_, _) => dirty = true,
            _ => {}
        }
    }
}

fn reload_broskifile(workspace: &Path) -> Result<BroskiFile> {
    let path = workspace.join("broskifile");
    let config = load_broskifile(&path).with_context(|| format!("reloading {}", path.display()))?;
    validate_broskifile(&config, workspace)
        .with_context(|| format!("validating {}", path.display()))?;
    Ok(config)
}

fn options_with_passthrough(base: &RunOptions, cmd: &ParsedCommand) -> RunOptions {
    let mut options = base.clone();
    if !cmd.passthrough.is_empty() {
        options.passthrough_args = cmd.passthrough.clone();
    }
    options
}

fn visible_task_names(config: &BroskiFile) -> Vec<String> {
    config.task.iter().filter(|(_, spec)| !spec.private).map(|(name, _)| name.clone()).collect()
}

/// Snapshot the contextual data the launcher renders alongside the live
/// state. `requested` is what the user asked for (e.g. `Theme::Auto`);
/// `resolved` is what we ended up using.
fn build_launcher_ctx(
    workspace: &Path,
    workspace_display: &str,
    config: &BroskiFile,
    store: &dyn ArtifactStore,
    requested: Theme,
    resolved: Theme,
) -> LauncherCtx {
    use std::collections::BTreeMap;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut task_meta = BTreeMap::new();
    for (name, spec) in &config.task {
        if spec.private {
            continue;
        }
        let mut meta = TaskMeta {
            description: spec.description.clone(),
            deps: spec.deps.clone(),
            inputs_count: spec.inputs.len(),
            outputs_count: spec.outputs.len(),
            ..TaskMeta::default()
        };
        if let Ok(Some(record)) = store.fetch_latest_execution(name) {
            if record.duration_ms > 0 {
                meta.last_duration_ms = Some(record.duration_ms);
            }
            meta.last_run_ago_secs = Some(now_secs.saturating_sub(record.created_at));
        }
        task_meta.insert(name.clone(), meta);
    }
    let cache_stats = store.stats().unwrap_or_default();
    LauncherCtx {
        workspace_display: workspace_display.to_string(),
        version: env!("CARGO_PKG_VERSION"),
        git_rev: git_short_rev(workspace),
        theme_resolved_name: resolved.name().to_string(),
        theme_requested_name: if requested == resolved {
            None
        } else {
            Some(requested.name().to_string())
        },
        cache_stats,
        task_meta,
    }
}

/// Best-effort short git SHA. Returns `None` when the workspace is not a
/// git repo, when git isn't installed, or when the command otherwise
/// fails. Never panics, never blocks longer than `git rev-parse` itself.
fn git_short_rev(workspace: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let rev = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if rev.is_empty() {
        None
    } else {
        Some(rev)
    }
}

/// Run a single target inside an already-active raw-mode terminal. Returns
/// the executor's [`RunSummary`] plus a [`RunOutcome`] for the launcher's
/// history table.
fn run_target_in_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace: PathBuf,
    config: BroskiFile,
    store: Arc<dyn ArtifactStore>,
    target: &str,
    base_options: RunOptions,
    palette: &Palette,
) -> Result<(RunSummary, RunOutcome)> {
    // If the resolved DAG includes any `@mode interactive` task, run with
    // the TUI suspended: leave raw mode, drop mouse capture, hand the
    // terminal to the child via `Stdio::inherit`, then re-enter the
    // alternate screen when it exits. The dashboard isn't useful for
    // interactive tasks (dev servers, REPLs, prompts) — they need a real
    // TTY, and any captured-pipe path would deadlock on prompts.
    if target_has_interactive_task(&config, target) {
        return run_target_suspended(terminal, workspace, config, store, target, base_options);
    }
    run_target_with_dashboard(terminal, workspace, config, store, target, base_options, palette)
}

fn run_target_with_dashboard(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace: PathBuf,
    config: BroskiFile,
    store: Arc<dyn ArtifactStore>,
    original_target: &str,
    base_options: RunOptions,
    palette: &Palette,
) -> Result<(RunSummary, RunOutcome)> {
    let mut current_target = original_target.to_string();
    let mut pending_rerun: Option<RerunRequest> = None;

    let summary = loop {
        // Build per-iteration options: apply force_tasks / force from any
        // pending rerun request, preserving the original CLI options otherwise.
        let mut opts = base_options.clone();
        if let Some(ref req) = pending_rerun {
            if req.force_all {
                opts.force = true;
            } else {
                opts.force_tasks = std::iter::once(req.task.clone()).collect();
            }
        }

        let etas = prefetch_etas(&config, &current_target, store.as_ref());
        let (event_tx, event_rx) = mpsc::channel::<ProgressEvent>();
        let cancellation = CancellationToken::new();
        opts.event_sink = Some(event_tx);
        opts.capture_output = true;
        opts.cancellation = Some(cancellation.clone());

        let exec_workspace = workspace.clone();
        let exec_config = config.clone();
        let exec_store = store.clone();
        let exec_target = current_target.clone();
        let exec_opts = opts.clone();
        let executor_handle = thread::spawn(move || -> Result<RunSummary> {
            Executor::new(exec_workspace, exec_config, exec_store)
                .context("constructing executor for TUI run")?
                .run_target(&exec_target, &exec_opts)
        });

        let decision = drive_loop(terminal, event_rx, palette, etas, &cancellation);
        let iter_summary = executor_handle
            .join()
            .map_err(|_| anyhow::anyhow!("executor thread panicked"))??;

        match decision? {
            LoopDecision::Quit => break iter_summary,
            LoopDecision::Rerun(req) => {
                current_target = req.task.clone();
                pending_rerun = Some(req);
                // drive_loop creates a fresh TuiState each call, so no
                // manual state reset is needed here.
            }
        }
    };
    let outcome =
        if !summary.skipped.is_empty() { RunOutcome::Cancelled } else { RunOutcome::Success };
    Ok((summary, outcome))
}

/// Run the target with the TUI fully suspended: leave raw mode + alt
/// screen + mouse capture, run the executor with default streaming
/// settings (no event_sink / no capture, so `Stdio::inherit` flows the
/// child's stdio straight to the user), then restore the dashboard.
fn run_target_suspended(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace: PathBuf,
    config: BroskiFile,
    store: Arc<dyn ArtifactStore>,
    target: &str,
    base_options: RunOptions,
) -> Result<(RunSummary, RunOutcome)> {
    suspend_terminal(terminal)?;
    println!("[broski tui] running interactive task '{target}' — TUI paused");

    let result: Result<RunSummary> = (|| {
        let executor = Executor::new(workspace, config, store)
            .context("constructing executor for suspended interactive run")?;
        executor.run_target(target, &base_options)
    })();

    // Always try to restore the alternate screen + mouse + raw mode,
    // even when the run failed. Otherwise the launcher would draw on
    // top of the user's normal scrollback.
    if let Err(restore_err) = restore_terminal(terminal) {
        eprintln!("[broski tui] failed to restore terminal: {restore_err}");
    }

    let summary = result?;
    let outcome =
        if !summary.skipped.is_empty() { RunOutcome::Cancelled } else { RunOutcome::Success };
    Ok((summary, outcome))
}

fn suspend_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("disabling raw mode for interactive task")?;
    execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)
        .context("leaving alt screen for interactive task")?;
    let _ = terminal.show_cursor();
    Ok(())
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    enable_raw_mode().context("re-enabling raw mode after interactive task")?;
    execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)
        .context("re-entering alt screen after interactive task")?;
    terminal.clear().context("clearing terminal after interactive task")?;
    Ok(())
}

/// Resolve the target's full task graph and report whether any
/// transitively-required task runs in [`TaskMode::Interactive`].
/// Best-effort: when graph resolution fails we conservatively return
/// `false` and let the dashboard try its luck.
fn target_has_interactive_task(config: &BroskiFile, target: &str) -> bool {
    let resolved = match config.resolve_task_name(target) {
        Ok(name) => name,
        Err(_) => return false,
    };
    let graph = match TaskGraph::build(&config.task) {
        Ok(g) => g,
        Err(_) => return false,
    };
    let required = match graph.required_tasks_for_target(&resolved) {
        Ok(tasks) => tasks,
        Err(_) => return false,
    };
    required.iter().any(|name| {
        config
            .task
            .get(name)
            .map(|spec| spec.inferred_mode() == TaskMode::Interactive)
            .unwrap_or(false)
    })
}

/// Walk the resolved task graph and ask the artifact store for the most
/// recent successful execution per task. Returns durations keyed by task name.
/// Any failure (no graph, missing record, store error) is silently absorbed —
/// ETAs are a UX nicety, not a correctness requirement.
fn prefetch_etas(
    config: &BroskiFile,
    target: &str,
    store: &dyn ArtifactStore,
) -> BTreeMap<String, Duration> {
    let mut etas = BTreeMap::new();
    let resolved = match config.resolve_task_name(target) {
        Ok(name) => name,
        Err(_) => return etas,
    };
    let graph = match TaskGraph::build(&config.task) {
        Ok(g) => g,
        Err(_) => return etas,
    };
    let required = match graph.required_tasks_for_target(&resolved) {
        Ok(tasks) => tasks,
        Err(_) => return etas,
    };
    for name in required {
        if let Ok(Some(record)) = store.fetch_latest_execution(&name) {
            if record.duration_ms > 0 {
                etas.insert(name, Duration::from_millis(record.duration_ms));
            }
        }
    }
    etas
}

fn drive_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    event_rx: mpsc::Receiver<ProgressEvent>,
    palette: &Palette,
    etas: BTreeMap<String, Duration>,
    cancellation: &CancellationToken,
) -> Result<LoopDecision> {
    let mut state = TuiState::with_etas(etas);
    let mut dirty = true;
    let mut channel_open = true;
    let mut last_interrupt: Option<Instant> = None;
    let tick = Duration::from_millis(TICK_MS);

    loop {
        if dirty {
            redraw(terminal, &state, palette)?;
            dirty = false;
        }

        let has_input = event::poll(tick).context("polling terminal events")?;
        if has_input {
            match event::read().context("reading terminal event")? {
                Event::Key(key) => {
                    let action = map_key(key);
                    if apply_action(&mut state, action, cancellation, &mut last_interrupt) {
                        return Ok(LoopDecision::Quit);
                    }
                    if let Some(req) = state.pending_rerun.take() {
                        return Ok(LoopDecision::Rerun(req));
                    }
                    dirty = true;
                }
                Event::Mouse(mouse) => {
                    if let Some(action) = map_mouse_to_log_scroll(mouse) {
                        if apply_action(&mut state, action, cancellation, &mut last_interrupt) {
                            return Ok(LoopDecision::Quit);
                        }
                        dirty = true;
                    }
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }

        // Drain whatever the executor produced this tick.
        if channel_open {
            loop {
                match event_rx.try_recv() {
                    Ok(ev) => {
                        state.apply(ev);
                        dirty = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        channel_open = false;
                        break;
                    }
                }
            }
        }
    }
}

/// Two-stage Ctrl-C handler. Returns the next `CancelState` plus whether the
/// app loop should terminate. Pure: easy to unit-test without a terminal.
pub(crate) fn next_interrupt_state(
    current: CancelState,
    last_press: Option<Instant>,
    now: Instant,
    window: Duration,
) -> (CancelState, bool) {
    match current {
        CancelState::Idle => (CancelState::Soft, false),
        CancelState::Soft => {
            let within_window = last_press.is_some_and(|t| now.duration_since(t) <= window);
            if within_window {
                (CancelState::Hard, true)
            } else {
                // The first press timed out; treat this one as a fresh first.
                (CancelState::Soft, false)
            }
        }
        CancelState::Hard => (CancelState::Hard, true),
    }
}

/// Returns true if the action should terminate the loop.
fn apply_action(
    state: &mut TuiState,
    action: Action,
    cancellation: &CancellationToken,
    last_interrupt: &mut Option<Instant>,
) -> bool {
    match action {
        Action::Quit => true,
        Action::Interrupt => {
            let now = Instant::now();
            let (next, should_quit) =
                next_interrupt_state(state.cancel, *last_interrupt, now, CANCEL_WINDOW);
            state.cancel = next;
            *last_interrupt = Some(now);
            match next {
                CancelState::Soft => cancellation.cancel(CancelLevel::Soft),
                CancelState::Hard => cancellation.cancel(CancelLevel::Hard),
                CancelState::Idle => {}
            }
            should_quit
        }
        Action::SelectNext => {
            state.move_selection(1);
            false
        }
        Action::SelectPrev => {
            state.move_selection(-1);
            false
        }
        Action::SelectFirst => {
            state.selected = state.task_order.first().map(|_| 0);
            false
        }
        Action::SelectLast => {
            state.selected =
                if state.task_order.is_empty() { None } else { Some(state.task_order.len() - 1) };
            false
        }
        Action::ClearLogs => {
            state.clear_selected_logs();
            false
        }
        Action::LogScrollUp(n) => {
            state.scroll_logs_up(n);
            false
        }
        Action::LogScrollDown(n) => {
            state.scroll_logs_down(n);
            false
        }
        Action::LogScrollHome => {
            state.scroll_logs_home();
            false
        }
        Action::LogScrollEnd => {
            state.scroll_logs_end();
            false
        }
        Action::ForceRerunSelected => {
            if let Some(name) = state.selected_task().map(str::to_string) {
                state.request_rerun(name, false);
            }
            false
        }
        Action::ForceRerunAll => {
            if let Some(name) = state.target.clone() {
                state.request_rerun(name, true);
            }
            false
        }
        Action::Redraw | Action::Ignore => false,
    }
}

/// Lines moved per mouse-wheel notch in the dashboard's log pane.
/// Three is the typical OS-default scroll step.
const MOUSE_WHEEL_LINES: usize = 3;

/// Translate a mouse event into a log-scroll action. Returns `None` for
/// non-wheel events (clicks, drag, etc.) so the caller can ignore them.
/// Pure: easy to unit-test on synthetic `MouseEvent`s.
pub(crate) fn map_mouse_to_log_scroll(mouse: MouseEvent) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollUp => Some(Action::LogScrollUp(MOUSE_WHEEL_LINES)),
        MouseEventKind::ScrollDown => Some(Action::LogScrollDown(MOUSE_WHEEL_LINES)),
        _ => None,
    }
}

/// Translate a raw key event into a [`LauncherAction`]. Pure: no IO, no
/// cancellation, easy to unit-test on Press events.
pub(crate) fn map_launcher_key(key: KeyEvent) -> LauncherAction {
    if matches!(key.kind, KeyEventKind::Release | KeyEventKind::Repeat)
        && key.kind != KeyEventKind::Press
    {
        return LauncherAction::Ignore;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, ctrl) {
        (KeyCode::Char('c'), true) => LauncherAction::Quit,
        (KeyCode::Char('u'), true) => LauncherAction::ClearInput,
        (KeyCode::Esc, _) => LauncherAction::ClearInput,
        (KeyCode::Enter, _) => LauncherAction::Enter,
        (KeyCode::Tab, _) => LauncherAction::Complete,
        (KeyCode::Backspace, _) => LauncherAction::Backspace,
        (KeyCode::Up, _) => LauncherAction::Up,
        (KeyCode::Down, _) => LauncherAction::Down,
        (KeyCode::Home, _) => LauncherAction::Home,
        (KeyCode::End, _) => LauncherAction::End,
        (KeyCode::Char(c), false) => LauncherAction::InsertChar(c),
        _ => LauncherAction::Ignore,
    }
}

fn redraw_launcher(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &LauncherState,
    ctx: &LauncherCtx,
    palette: &Palette,
) -> Result<()> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            LauncherWidget::new(state, ctx, palette).render_into(frame, area);
        })
        .context("drawing launcher frame")?;
    Ok(())
}

fn redraw(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &TuiState,
    palette: &Palette,
) -> Result<()> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(8), Constraint::Length(3), Constraint::Length(1)])
                .split(area);

            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                .split(outer[0]);

            frame.render_widget(DagWidget::new(state, palette), body[0]);
            frame.render_widget(LogsWidget::new(state, palette), body[1]);
            frame.render_widget(SummaryWidget::new(state, palette), outer[1]);
            frame.render_widget(HelpFooter::new(palette), outer[2]);
        })
        .context("drawing TUI frame")?;
    Ok(())
}

fn enter_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = io::stdout();
    // Enable mouse capture so the dashboard can scroll the log pane on
    // wheel events. Crossterm emits these as `Event::Mouse` with kind
    // `ScrollUp` / `ScrollDown` once capture is on.
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("entering alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("creating terminal")
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_interrupt_goes_soft_without_quitting() {
        let now = Instant::now();
        let (next, quit) = next_interrupt_state(CancelState::Idle, None, now, CANCEL_WINDOW);
        assert_eq!(next, CancelState::Soft);
        assert!(!quit);
    }

    #[test]
    fn second_interrupt_within_window_escalates_to_hard_and_quits() {
        let first = Instant::now();
        let second = first + Duration::from_millis(500);
        let (next, quit) =
            next_interrupt_state(CancelState::Soft, Some(first), second, CANCEL_WINDOW);
        assert_eq!(next, CancelState::Hard);
        assert!(quit);
    }

    #[test]
    fn second_interrupt_after_window_resets_to_soft() {
        let first = Instant::now();
        let second = first + Duration::from_secs(10);
        let (next, quit) =
            next_interrupt_state(CancelState::Soft, Some(first), second, CANCEL_WINDOW);
        assert_eq!(next, CancelState::Soft);
        assert!(!quit);
    }

    #[test]
    fn third_interrupt_after_hard_keeps_hard_and_quits() {
        let now = Instant::now();
        let (next, quit) = next_interrupt_state(
            CancelState::Hard,
            Some(now - Duration::from_millis(100)),
            now,
            CANCEL_WINDOW,
        );
        assert_eq!(next, CancelState::Hard);
        assert!(quit);
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn launcher_plain_letter_inserts_char() {
        assert_eq!(map_launcher_key(key(KeyCode::Char('f'))), LauncherAction::InsertChar('f'));
    }

    #[test]
    fn launcher_ctrl_c_is_quit_not_insert() {
        assert_eq!(map_launcher_key(ctrl('c')), LauncherAction::Quit);
    }

    #[test]
    fn launcher_ctrl_u_clears_input() {
        assert_eq!(map_launcher_key(ctrl('u')), LauncherAction::ClearInput);
    }

    #[test]
    fn launcher_esc_clears_input() {
        assert_eq!(map_launcher_key(key(KeyCode::Esc)), LauncherAction::ClearInput);
    }

    #[test]
    fn launcher_arrows_and_tab_route_correctly() {
        assert_eq!(map_launcher_key(key(KeyCode::Up)), LauncherAction::Up);
        assert_eq!(map_launcher_key(key(KeyCode::Down)), LauncherAction::Down);
        assert_eq!(map_launcher_key(key(KeyCode::Home)), LauncherAction::Home);
        assert_eq!(map_launcher_key(key(KeyCode::End)), LauncherAction::End);
        assert_eq!(map_launcher_key(key(KeyCode::Tab)), LauncherAction::Complete);
        assert_eq!(map_launcher_key(key(KeyCode::Backspace)), LauncherAction::Backspace);
        assert_eq!(map_launcher_key(key(KeyCode::Enter)), LauncherAction::Enter);
    }

    #[test]
    fn launcher_unknown_keys_are_ignored() {
        assert_eq!(map_launcher_key(key(KeyCode::F(5))), LauncherAction::Ignore);
        assert_eq!(map_launcher_key(key(KeyCode::Insert)), LauncherAction::Ignore);
    }

    fn mouse_event(kind: MouseEventKind) -> MouseEvent {
        MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn mouse_wheel_up_scrolls_logs_up() {
        match map_mouse_to_log_scroll(mouse_event(MouseEventKind::ScrollUp)) {
            Some(Action::LogScrollUp(n)) => assert!(n > 0),
            other => panic!("ScrollUp should yield LogScrollUp, got {:?}", other),
        }
    }

    #[test]
    fn mouse_wheel_down_scrolls_logs_down() {
        match map_mouse_to_log_scroll(mouse_event(MouseEventKind::ScrollDown)) {
            Some(Action::LogScrollDown(n)) => assert!(n > 0),
            other => panic!("ScrollDown should yield LogScrollDown, got {:?}", other),
        }
    }

    #[test]
    fn mouse_clicks_and_drags_are_ignored() {
        use crossterm::event::MouseButton;
        assert!(
            map_mouse_to_log_scroll(mouse_event(MouseEventKind::Down(MouseButton::Left))).is_none()
        );
        assert!(
            map_mouse_to_log_scroll(mouse_event(MouseEventKind::Up(MouseButton::Left))).is_none()
        );
        assert!(
            map_mouse_to_log_scroll(mouse_event(MouseEventKind::Drag(MouseButton::Left))).is_none()
        );
    }

    #[test]
    fn visible_task_names_filters_private_tasks() {
        use broski_core::model::{BroskiSection, RunSpec, TaskSpec};
        let mut tasks = std::collections::BTreeMap::new();
        let mk = |private: bool| TaskSpec {
            deps: vec![],
            description: None,
            resolved_variables: Default::default(),
            inputs: vec![],
            stage_ro: vec![],
            outputs: vec![],
            env: Default::default(),
            env_inherit: vec![],
            secret_env: vec![],
            run: RunSpec::Shell("echo".into()),
            isolation: None,
            mode: None,
            working_dir: None,
            params: vec![],
            private,
            confirm: None,
            shell_override: None,
            requires: vec![],
        };
        tasks.insert("public_a".to_string(), mk(false));
        tasks.insert("_private".to_string(), mk(true));
        tasks.insert("public_b".to_string(), mk(false));
        let config = BroskiFile {
            broski: BroskiSection { version: "0.5".into() },
            task: tasks,
            alias: Default::default(),
            load_env: vec![],
        };
        let names = visible_task_names(&config);
        assert!(names.contains(&"public_a".to_string()));
        assert!(names.contains(&"public_b".to_string()));
        assert!(!names.contains(&"_private".to_string()));
    }

    #[test]
    fn target_with_only_graph_tasks_is_not_interactive() {
        use broski_core::model::{BroskiSection, RunSpec, TaskSpec};
        let mut tasks = std::collections::BTreeMap::new();
        tasks.insert(
            "build".to_string(),
            TaskSpec {
                deps: vec![],
                description: None,
                resolved_variables: Default::default(),
                inputs: vec!["src/lib.rs".into()],
                stage_ro: vec![],
                outputs: vec!["dist/out".into()],
                env: Default::default(),
                env_inherit: vec![],
                secret_env: vec![],
                run: RunSpec::Shell("echo build".into()),
                isolation: None,
                mode: None,
                working_dir: None,
                params: vec![],
                private: false,
                confirm: None,
                shell_override: None,
                requires: vec![],
            },
        );
        let config = BroskiFile {
            broski: BroskiSection { version: "0.5".into() },
            task: tasks,
            alias: Default::default(),
            load_env: vec![],
        };
        assert!(!target_has_interactive_task(&config, "build"));
    }

    #[test]
    fn target_with_interactive_task_is_detected() {
        use broski_core::model::{BroskiSection, RunSpec, TaskSpec};
        let mut tasks = std::collections::BTreeMap::new();
        // No outputs → inferred Interactive (per model::inferred_mode).
        tasks.insert(
            "dev".to_string(),
            TaskSpec {
                deps: vec![],
                description: None,
                resolved_variables: Default::default(),
                inputs: vec![],
                stage_ro: vec![],
                outputs: vec![],
                env: Default::default(),
                env_inherit: vec![],
                secret_env: vec![],
                run: RunSpec::Shell("npm run dev".into()),
                isolation: None,
                mode: None,
                working_dir: None,
                params: vec![],
                private: false,
                confirm: None,
                shell_override: None,
                requires: vec![],
            },
        );
        let config = BroskiFile {
            broski: BroskiSection { version: "0.5".into() },
            task: tasks,
            alias: Default::default(),
            load_env: vec![],
        };
        assert!(target_has_interactive_task(&config, "dev"));
    }

    #[test]
    fn unknown_target_falls_back_to_dashboard() {
        use broski_core::model::BroskiSection;
        let config = BroskiFile {
            broski: BroskiSection { version: "0.5".into() },
            task: Default::default(),
            alias: Default::default(),
            load_env: vec![],
        };
        // Unknown target → graph resolution fails → conservatively false.
        assert!(!target_has_interactive_task(&config, "ghost"));
    }

    #[test]
    fn options_with_passthrough_overrides_only_when_present() {
        let base = RunOptions { passthrough_args: vec!["base".into()], ..RunOptions::default() };
        // No passthrough → keep base.
        let cmd_empty = ParsedCommand { target: "t".into(), passthrough: vec![] };
        let opts = options_with_passthrough(&base, &cmd_empty);
        assert_eq!(opts.passthrough_args, vec!["base".to_string()]);

        // Passthrough → overwrite.
        let cmd_args =
            ParsedCommand { target: "t".into(), passthrough: vec!["x".into(), "y".into()] };
        let opts = options_with_passthrough(&base, &cmd_args);
        assert_eq!(opts.passthrough_args, vec!["x".to_string(), "y".to_string()]);
    }

    /// Helper: build a minimal `TuiState` with one task visible and the
    /// run already finished, so `x` / `X` are effective.
    fn finished_state_with_task(task: &str, target: &str) -> TuiState {
        use broski_core::{ProgressEvent, TaskStatus};
        use std::time::Duration;
        let mut s = TuiState::new();
        s.apply(ProgressEvent::RunStarted {
            target: target.to_string(),
            layers: vec![vec![task.to_string()]],
        });
        s.apply(ProgressEvent::TaskStarted {
            task: task.to_string(),
            mode: broski_core::TaskMode::Graph,
        });
        s.apply(ProgressEvent::TaskFinished {
            task: task.to_string(),
            status: TaskStatus::Executed,
            duration: Duration::from_millis(10),
            error: None,
            cache_reasons: Vec::new(),
        });
        s.apply(ProgressEvent::RunFinished);
        s
    }

    #[test]
    fn force_rerun_selected_sets_pending_rerun_when_finished() {
        let cancellation = CancellationToken::new();
        let mut last_interrupt = None;
        let mut state = finished_state_with_task("lint", "ci");
        // Cursor is on "lint" (index 0).
        assert_eq!(state.selected_task(), Some("lint"));

        let quit = apply_action(
            &mut state,
            Action::ForceRerunSelected,
            &cancellation,
            &mut last_interrupt,
        );
        assert!(!quit, "ForceRerunSelected should not quit the loop");
        assert_eq!(
            state.pending_rerun,
            Some(RerunRequest { task: "lint".to_string(), force_all: false })
        );
    }

    #[test]
    fn force_rerun_all_uses_original_target() {
        let cancellation = CancellationToken::new();
        let mut last_interrupt = None;
        let mut state = finished_state_with_task("fmt", "ci");

        let quit =
            apply_action(&mut state, Action::ForceRerunAll, &cancellation, &mut last_interrupt);
        assert!(!quit);
        assert_eq!(
            state.pending_rerun,
            Some(RerunRequest { task: "ci".to_string(), force_all: true })
        );
    }

    #[test]
    fn force_rerun_ignored_when_run_still_in_progress() {
        let cancellation = CancellationToken::new();
        let mut last_interrupt = None;
        use broski_core::ProgressEvent;
        let mut state = TuiState::new();
        state.apply(ProgressEvent::RunStarted {
            target: "ci".to_string(),
            layers: vec![vec!["lint".to_string()]],
        });
        // run_finished is still false

        apply_action(&mut state, Action::ForceRerunSelected, &cancellation, &mut last_interrupt);
        assert!(state.pending_rerun.is_none(), "x during a live run must be ignored");

        apply_action(&mut state, Action::ForceRerunAll, &cancellation, &mut last_interrupt);
        assert!(state.pending_rerun.is_none(), "X during a live run must be ignored");
    }
}
