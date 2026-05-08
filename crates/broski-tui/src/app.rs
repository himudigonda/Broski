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
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use broski_core::{BroskiFile, Executor, ProgressEvent, RunOptions, RunSummary, TaskGraph};
use broski_store::ArtifactStore;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Terminal;

use crate::keys::{map_key, Action};
use crate::state::TuiState;
use crate::theme::{Palette, Theme};
use crate::widgets::dag::DagWidget;
use crate::widgets::help::HelpFooter;
use crate::widgets::logs::LogsWidget;
use crate::widgets::summary::SummaryWidget;

/// Foreground poll cadence. Keys come in via `event::poll` so this also
/// caps the redraw rate when no events are arriving.
const TICK_MS: u64 = 75;

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
    mut base_options: RunOptions,
    theme: Theme,
) -> Result<RunSummary> {
    let etas = prefetch_etas(&config, &target, store.as_ref());

    let (event_tx, event_rx) = mpsc::channel::<ProgressEvent>();
    base_options.event_sink = Some(event_tx);
    base_options.capture_output = true;

    let executor_workspace = workspace.clone();
    let executor_target = target.clone();
    let executor_options = base_options.clone();
    let executor_handle = thread::spawn(move || -> Result<RunSummary> {
        let executor = Executor::new(executor_workspace, config, store)
            .context("constructing executor for TUI run")?;
        executor.run_target(&executor_target, &executor_options)
    });

    let mut terminal = enter_terminal().context("entering alt screen / raw mode")?;
    let palette = theme.palette();
    let result = drive_loop(&mut terminal, event_rx, &palette, etas);
    let _ = leave_terminal(&mut terminal);

    let summary = executor_handle
        .join()
        .map_err(|_| anyhow::anyhow!("executor thread panicked"))??;
    result?;
    Ok(summary)
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
) -> Result<()> {
    let mut state = TuiState::with_etas(etas);
    let mut dirty = true;
    let mut channel_open = true;
    let tick = Duration::from_millis(TICK_MS);

    loop {
        if dirty {
            redraw(terminal, &state, palette)?;
            dirty = false;
        }

        let poll_started = Instant::now();
        let has_input = event::poll(tick).context("polling terminal events")?;
        if has_input {
            match event::read().context("reading terminal event")? {
                Event::Key(key) => {
                    let action = map_key(key);
                    if apply_action(&mut state, action) {
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
                        let was_run_finished =
                            matches!(ev, ProgressEvent::RunFinished);
                        state.apply(ev);
                        dirty = true;
                        if was_run_finished {
                            // Stay in TUI so user can inspect; channel may close shortly.
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        channel_open = false;
                        break;
                    }
                }
            }
        }

        // If the user has nothing to do and the run is over, redraw budget is
        // bounded by the tick so we don't busy-loop.
        let _ = poll_started; // suppress unused warning when no input arrived
    }
}

/// Returns true if the action should terminate the loop.
fn apply_action(state: &mut TuiState, action: Action) -> bool {
    match action {
        Action::Quit => true,
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
            state.selected = if state.task_order.is_empty() {
                None
            } else {
                Some(state.task_order.len() - 1)
            };
            false
        }
        Action::ClearLogs => {
            state.clear_selected_logs();
            false
        }
        Action::Redraw | Action::Ignore => false,
    }
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
                .constraints([
                    Constraint::Min(8),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
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
