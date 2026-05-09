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
use broski_core::{BroskiFile, Executor, ProgressEvent, RunOptions, RunSummary, TaskGraph};
use broski_store::ArtifactStore;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Terminal;

use crate::keys::{map_key, Action};
use crate::launcher::{LauncherAction, LauncherDecision, LauncherState, ParsedCommand, RunOutcome};
use crate::state::{CancelState, TuiState};
use crate::theme::{Palette, Theme};
use crate::widgets::dag::DagWidget;
use crate::widgets::help::HelpFooter;
use crate::widgets::launcher::LauncherWidget;
use crate::widgets::logs::LogsWidget;
use crate::widgets::summary::SummaryWidget;

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
    let palette = theme.palette();
    let theme_name = theme.name().to_string();
    let workspace_display = workspace.display().to_string();
    let mut launcher = LauncherState::new(visible_task_names(config));
    let tick = Duration::from_millis(TICK_MS);
    let mut dirty = true;

    loop {
        if dirty {
            redraw_launcher(terminal, &launcher, &palette, &workspace_display, &theme_name)?;
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
                            config.clone(),
                            store.clone(),
                            &cmd.target,
                            options_with_passthrough(base_options, &cmd),
                            &palette,
                        ) {
                            Ok((_, outcome)) => outcome,
                            Err(_) => RunOutcome::Failed,
                        };
                        launcher.record_run(cmd.target, outcome, started.elapsed());
                        dirty = true;
                    }
                }
            }
            Event::Resize(_, _) => dirty = true,
            _ => {}
        }
    }
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

/// Run a single target inside an already-active raw-mode terminal. Returns
/// the executor's [`RunSummary`] plus a [`RunOutcome`] for the launcher's
/// history table.
fn run_target_in_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    workspace: PathBuf,
    config: BroskiFile,
    store: Arc<dyn ArtifactStore>,
    target: &str,
    mut base_options: RunOptions,
    palette: &Palette,
) -> Result<(RunSummary, RunOutcome)> {
    let etas = prefetch_etas(&config, target, store.as_ref());

    let (event_tx, event_rx) = mpsc::channel::<ProgressEvent>();
    let cancellation = CancellationToken::new();
    base_options.event_sink = Some(event_tx);
    base_options.capture_output = true;
    base_options.cancellation = Some(cancellation.clone());

    let executor_workspace = workspace.clone();
    let executor_target = target.to_string();
    let executor_options = base_options.clone();
    let executor_handle = thread::spawn(move || -> Result<RunSummary> {
        let executor = Executor::new(executor_workspace, config, store)
            .context("constructing executor for TUI run")?;
        executor.run_target(&executor_target, &executor_options)
    });

    let drive_result = drive_loop(terminal, event_rx, palette, etas, &cancellation);
    let summary_result =
        executor_handle.join().map_err(|_| anyhow::anyhow!("executor thread panicked"))?;

    match (drive_result, summary_result) {
        (Ok(_), Ok(summary)) => {
            // The executor returns Err on a task failure, so a successful
            // summary at this point means every required task either ran
            // cleanly, hit the cache, or was skipped by cancellation.
            let outcome = if !summary.skipped.is_empty() {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Success
            };
            Ok((summary, outcome))
        }
        (Err(e), _) => Err(e),
        (_, Err(e)) => Err(e),
    }
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
) -> Result<()> {
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
                        return Ok(());
                    }
                    dirty = true;
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
        Action::Redraw | Action::Ignore => false,
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
    palette: &Palette,
    workspace: &str,
    theme_name: &str,
) -> Result<()> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            frame.render_widget(LauncherWidget::new(state, palette, workspace, theme_name), area);
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
    execute!(stdout, EnterAlternateScreen).context("entering alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("creating terminal")
}

fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
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
}
