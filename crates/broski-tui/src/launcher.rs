//! Pure state machine for the `broski tui` launcher screen.
//!
//! When the user runs `broski tui` with no task argument, they land on a
//! launcher: a filterable task list plus a free-form input box. They pick or
//! type a target, hit Enter, the dashboard runs that target, and on completion
//! they pop back here. This module encodes that screen as a pure data type so
//! the UI layer is a thin renderer and every transition is unit-testable.
//!
//! The launcher does not perform IO. It does not own a terminal. It folds key
//! events ([`LauncherAction`]) into a new state plus an outward
//! [`LauncherDecision`] that the app loop acts on.

use std::time::Duration;

use crate::theme::Theme;

/// Result of folding an action into the launcher state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherDecision {
    /// Stay on the launcher screen.
    Continue,
    /// Run the parsed command, then return here.
    Run(ParsedCommand),
    /// Tear down the TUI and exit.
    Quit,
    /// Switch the active theme without exiting.
    SwitchTheme(Theme),
    /// Re-load `broskifile` from disk; the app loop is responsible for
    /// rebuilding the task list and refreshing cache stats.
    Refresh,
    /// Invoke `ArtifactStore::prune(mb)`; the app loop reports the result
    /// back via [`LauncherState::record_status`].
    PruneCache(u64),
}

/// One of the launcher's two text-input modes:
/// - **Filter**: typed characters narrow the task list; Enter runs the
///   highlighted (or typed) task.
/// - **Slash**: input begins with `/`; the right column shows the
///   slash-command suggestion list and Enter dispatches that command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherMode {
    Filter,
    Slash,
}

/// One of the slash commands the launcher recognizes. Every invocation
/// returns either a [`LauncherDecision`] or a status banner — the parser
/// here is exhaustive so unknown commands surface a clear message rather
/// than silently no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Theme(Theme),
    About,
    Clear,
    Refresh,
    PruneCache(u64),
    Quit,
}

/// Static catalog of slash commands shown in the suggestion panel and used
/// for Tab completion.
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "show key map and slash commands"),
    ("/theme", "switch theme: default | dark | light | high-contrast | auto"),
    ("/about", "show broski version, workspace, git rev"),
    ("/clear", "clear the in-session run history"),
    ("/refresh", "reload broskifile and refresh cache stats"),
    ("/cache prune <MB>", "prune cache to a size budget"),
    ("/quit", "exit the launcher (alias: /q)"),
];

/// A target name plus optional `-- <args>` passthrough, parsed from the
/// input box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub target: String,
    pub passthrough: Vec<String>,
}

/// Outcome of a previous run, surfaced on the launcher between sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHistoryEntry {
    pub target: String,
    pub outcome: RunOutcome,
    pub duration: Duration,
}

/// Single keystroke or directive routed into the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherAction {
    InsertChar(char),
    Backspace,
    Up,
    Down,
    Home,
    End,
    Enter,
    /// Esc / Ctrl-U: clear the input box but stay on the launcher.
    ClearInput,
    /// Ctrl-C / `q` (only when input is empty): quit.
    Quit,
    /// Tab: complete the input from the highlighted task.
    Complete,
    /// Anything we don't care about.
    Ignore,
}

/// Outcome counters surfaced in the launcher's stats panel and used for
/// the "5 runs · 4✓ 1✗" string.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub total_runs: usize,
    pub successes: usize,
    pub failures: usize,
    pub cancellations: usize,
    pub total_duration: Duration,
}

/// The launcher's full state. Held by the app loop and rendered each tick.
#[derive(Debug, Clone)]
pub struct LauncherState {
    /// Free-form input. May contain `-- arg arg`.
    pub input: String,
    /// All non-private task names in display order.
    pub all_tasks: Vec<String>,
    /// Index into [`filtered_tasks`] (or [`filtered_slash_commands`] in
    /// `Slash` mode) of the currently-highlighted match. `None` when the
    /// filtered list is empty.
    pub selected: Option<usize>,
    /// Recent runs from this launcher session, newest first.
    pub history: Vec<RunHistoryEntry>,
    /// Transient banner ("ran X in 1.2s", "no task matches 'fooo'", etc.).
    pub status: Option<String>,
    /// Current input mode (filter vs. slash command).
    pub mode: LauncherMode,
    /// Aggregated session counters.
    pub stats: SessionStats,
}

impl LauncherState {
    pub fn new(all_tasks: Vec<String>) -> Self {
        let selected = if all_tasks.is_empty() { None } else { Some(0) };
        Self {
            input: String::new(),
            all_tasks,
            selected,
            history: Vec::new(),
            status: None,
            mode: LauncherMode::Filter,
            stats: SessionStats::default(),
        }
    }

    /// The visible task list given the current input filter.
    /// The filter matches any task whose name *contains* the input prefix
    /// (case-insensitive). `--` and anything after it is treated as run args
    /// and excluded from the filter substring.
    pub fn filtered_tasks(&self) -> Vec<&str> {
        let needle = filter_needle(&self.input).to_ascii_lowercase();
        if needle.is_empty() {
            return self.all_tasks.iter().map(String::as_str).collect();
        }
        self.all_tasks
            .iter()
            .filter(|t| t.to_ascii_lowercase().contains(&needle))
            .map(String::as_str)
            .collect()
    }

    /// Best-effort parse of the input box into a target plus passthrough args.
    /// Returns `None` when the input is empty or only whitespace, and falls
    /// back to the highlighted task when the input has no explicit target.
    pub fn parse_command(&self) -> Option<ParsedCommand> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            // Empty input: dispatch the currently-highlighted task.
            return self
                .highlighted_task()
                .map(|t| ParsedCommand { target: t.to_string(), passthrough: Vec::new() });
        }

        // Split on the first `--` token.
        let mut parts = trimmed.splitn(2, " -- ");
        let head = parts.next().unwrap_or("").trim();
        let tail = parts.next().unwrap_or("").trim();

        let mut head_tokens = head.split_whitespace();
        let target = match head_tokens.next() {
            Some(t) => t.to_string(),
            None => return None,
        };

        let mut passthrough: Vec<String> = head_tokens.map(str::to_string).collect();
        if !tail.is_empty() {
            passthrough.extend(tail.split_whitespace().map(str::to_string));
        }

        Some(ParsedCommand { target, passthrough })
    }

    /// The task name under the cursor (i.e. the one Enter would pick when
    /// the input is empty).
    pub fn highlighted_task(&self) -> Option<&str> {
        let filtered = self.filtered_tasks();
        let idx = self.selected?;
        filtered.get(idx).copied()
    }

    /// The slash-command label under the cursor (only meaningful in
    /// `Slash` mode).
    pub fn highlighted_slash(&self) -> Option<&'static str> {
        let filtered = self.filtered_slash_commands();
        let idx = self.selected?;
        filtered.get(idx).copied()
    }

    /// Slash-command suggestions filtered by the input (which always
    /// starts with `/`). The leading `/` is part of the needle, so typing
    /// `/th` narrows to `/theme`.
    pub fn filtered_slash_commands(&self) -> Vec<&'static str> {
        let needle = self.input.trim().to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|(label, _)| {
                let label = (*label).to_ascii_lowercase();
                needle.is_empty() || label.starts_with(&needle) || needle.starts_with(&label)
            })
            .map(|(label, _)| *label)
            .collect()
    }

    /// Length of whichever filtered list is active in the current mode.
    fn active_list_len(&self) -> usize {
        match self.mode {
            LauncherMode::Filter => self.filtered_tasks().len(),
            LauncherMode::Slash => self.filtered_slash_commands().len(),
        }
    }

    /// Clamp `selected` into `0..len` for the active mode after an edit.
    fn reclamp_selection(&mut self) {
        let len = self.active_list_len();
        if len == 0 {
            self.selected = None;
        } else {
            self.selected = Some(self.selected.map_or(0, |i| i.min(len - 1)));
        }
    }

    /// Update [`mode`] and reset the cursor based on whether `input`
    /// currently begins with `/`.
    fn refresh_mode(&mut self) {
        let new_mode =
            if self.input.starts_with('/') { LauncherMode::Slash } else { LauncherMode::Filter };
        if new_mode != self.mode {
            self.mode = new_mode;
            self.selected = if self.active_list_len() == 0 { None } else { Some(0) };
        } else {
            self.reclamp_selection();
        }
    }

    /// Fold one action into the state and return the resulting decision.
    pub fn apply(&mut self, action: LauncherAction) -> LauncherDecision {
        // Any user action clears a stale status banner.
        if !matches!(action, LauncherAction::Ignore) {
            self.status = None;
        }
        match action {
            LauncherAction::InsertChar(c) => {
                self.input.push(c);
                self.refresh_mode();
                LauncherDecision::Continue
            }
            LauncherAction::Backspace => {
                self.input.pop();
                self.refresh_mode();
                LauncherDecision::Continue
            }
            LauncherAction::ClearInput => {
                if self.input.is_empty() {
                    LauncherDecision::Continue
                } else {
                    self.input.clear();
                    self.refresh_mode();
                    LauncherDecision::Continue
                }
            }
            LauncherAction::Up => {
                let len = self.active_list_len();
                if len > 0 {
                    self.selected = Some(match self.selected {
                        Some(i) if i > 0 => i - 1,
                        _ => len - 1,
                    });
                }
                LauncherDecision::Continue
            }
            LauncherAction::Down => {
                let len = self.active_list_len();
                if len > 0 {
                    self.selected = Some(match self.selected {
                        Some(i) if i + 1 < len => i + 1,
                        _ => 0,
                    });
                }
                LauncherDecision::Continue
            }
            LauncherAction::Home => {
                if self.active_list_len() > 0 {
                    self.selected = Some(0);
                }
                LauncherDecision::Continue
            }
            LauncherAction::End => {
                let len = self.active_list_len();
                if len > 0 {
                    self.selected = Some(len - 1);
                }
                LauncherDecision::Continue
            }
            LauncherAction::Complete => match self.mode {
                LauncherMode::Filter => {
                    if let Some(target) = self.highlighted_task().map(str::to_string) {
                        self.input = target;
                        self.refresh_mode();
                    }
                    LauncherDecision::Continue
                }
                LauncherMode::Slash => {
                    if let Some(label) = self.highlighted_slash() {
                        self.input = canonical_slash_input(label);
                        self.refresh_mode();
                    }
                    LauncherDecision::Continue
                }
            },
            LauncherAction::Enter => match self.mode {
                LauncherMode::Filter => self.dispatch_filter_enter(),
                LauncherMode::Slash => self.dispatch_slash_enter(),
            },
            LauncherAction::Quit => {
                if self.input.is_empty() {
                    LauncherDecision::Quit
                } else {
                    self.input.clear();
                    self.refresh_mode();
                    LauncherDecision::Continue
                }
            }
            LauncherAction::Ignore => LauncherDecision::Continue,
        }
    }

    fn dispatch_filter_enter(&mut self) -> LauncherDecision {
        match self.parse_command() {
            Some(cmd) => {
                if self.all_tasks.contains(&cmd.target) {
                    return LauncherDecision::Run(cmd);
                }
                if let Some(highlighted) = self.highlighted_task().map(str::to_string) {
                    return LauncherDecision::Run(ParsedCommand {
                        target: highlighted,
                        passthrough: cmd.passthrough,
                    });
                }
                self.status =
                    Some(format!("no task matches '{}' — pick one from the list", cmd.target));
                LauncherDecision::Continue
            }
            None => {
                self.status = Some("type a task name or pick one with ↑/↓".to_string());
                LauncherDecision::Continue
            }
        }
    }

    fn dispatch_slash_enter(&mut self) -> LauncherDecision {
        match parse_slash_command(&self.input) {
            Ok(SlashCommand::Help) => {
                self.status = Some(slash_help_text());
                self.input.clear();
                self.refresh_mode();
                LauncherDecision::Continue
            }
            Ok(SlashCommand::Theme(theme)) => {
                self.input.clear();
                self.refresh_mode();
                LauncherDecision::SwitchTheme(theme)
            }
            Ok(SlashCommand::About) => {
                self.status = Some(format!("broski {}", env!("CARGO_PKG_VERSION")));
                self.input.clear();
                self.refresh_mode();
                LauncherDecision::Continue
            }
            Ok(SlashCommand::Clear) => {
                self.history.clear();
                self.stats = SessionStats::default();
                self.status = Some("cleared session history".to_string());
                self.input.clear();
                self.refresh_mode();
                LauncherDecision::Continue
            }
            Ok(SlashCommand::Refresh) => {
                self.input.clear();
                self.refresh_mode();
                LauncherDecision::Refresh
            }
            Ok(SlashCommand::PruneCache(mb)) => {
                self.input.clear();
                self.refresh_mode();
                LauncherDecision::PruneCache(mb)
            }
            Ok(SlashCommand::Quit) => LauncherDecision::Quit,
            Err(msg) => {
                self.status = Some(msg);
                LauncherDecision::Continue
            }
        }
    }

    /// Set the status banner. Used by the app loop to surface results of
    /// `Refresh` / `PruneCache` decisions back to the user.
    pub fn record_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    /// Replace the task list (called after `/refresh`).
    pub fn replace_tasks(&mut self, all_tasks: Vec<String>) {
        self.all_tasks = all_tasks;
        self.refresh_mode();
    }

    /// Record a finished run, then clear the input box so the user can
    /// pick a new target. Newest entries land at index 0 and the list is
    /// capped at 16. Also bumps [`SessionStats`].
    pub fn record_run(&mut self, target: String, outcome: RunOutcome, duration: Duration) {
        let entry = RunHistoryEntry { target: target.clone(), outcome: outcome.clone(), duration };
        self.stats.total_runs += 1;
        self.stats.total_duration =
            self.stats.total_duration.checked_add(duration).unwrap_or(self.stats.total_duration);
        let label = match outcome {
            RunOutcome::Success => {
                self.stats.successes += 1;
                "ran"
            }
            RunOutcome::Failed => {
                self.stats.failures += 1;
                "failed"
            }
            RunOutcome::Cancelled => {
                self.stats.cancellations += 1;
                "cancelled"
            }
        };
        self.status = Some(format!("{} {} in {}", label, target, format_dur(duration)));
        self.history.insert(0, entry);
        if self.history.len() > 16 {
            self.history.truncate(16);
        }
        self.input.clear();
        self.refresh_mode();
    }
}

/// Parse the input box's current text as a slash command, returning either
/// a [`SlashCommand`] or a human-readable error suitable for the status
/// banner.
pub fn parse_slash_command(input: &str) -> Result<SlashCommand, String> {
    let trimmed = input.trim();
    let body = trimmed
        .strip_prefix('/')
        .ok_or_else(|| "expected a slash command starting with '/'".to_string())?;
    let mut tokens = body.split_whitespace();
    let head = tokens.next().unwrap_or("").to_ascii_lowercase();
    match head.as_str() {
        "help" | "h" | "?" => Ok(SlashCommand::Help),
        "quit" | "q" | "exit" => Ok(SlashCommand::Quit),
        "about" | "version" => Ok(SlashCommand::About),
        "clear" => Ok(SlashCommand::Clear),
        "refresh" | "reload" => Ok(SlashCommand::Refresh),
        "theme" => match tokens.next() {
            Some(name) => name
                .parse::<Theme>()
                .map(SlashCommand::Theme)
                .map_err(|_| format!("unknown theme '{}' — try /help", name)),
            None => Err("usage: /theme <default|dark|light|high-contrast|auto>".to_string()),
        },
        "cache" => match tokens.next().map(str::to_ascii_lowercase).as_deref() {
            Some("prune") => match tokens.next() {
                Some(mb) => mb
                    .parse::<u64>()
                    .map(SlashCommand::PruneCache)
                    .map_err(|_| format!("expected a megabyte budget, got '{}'", mb)),
                None => Err("usage: /cache prune <MB>".to_string()),
            },
            Some(other) => Err(format!("unknown cache subcommand '{}' — try /help", other)),
            None => Err("usage: /cache prune <MB>".to_string()),
        },
        other => Err(format!("unknown command '/{}' — try /help", other)),
    }
}

/// Multi-line key map shown by `/help`.
fn slash_help_text() -> String {
    let mut out = String::from("commands:");
    for (label, desc) in SLASH_COMMANDS {
        out.push_str(&format!("  {label} — {desc}"));
    }
    out
}

/// Canonical input form for a tab-completed slash command. `/cache prune`
/// keeps the trailing space so the user can type the MB budget right
/// away; commands with arguments end in `" "`, argument-less commands do
/// not.
fn canonical_slash_input(label: &str) -> String {
    if label.starts_with("/theme") {
        "/theme ".to_string()
    } else if label.starts_with("/cache") {
        "/cache prune ".to_string()
    } else {
        label.split_whitespace().next().unwrap_or(label).to_string()
    }
}

/// Strip an inline `-- ...` passthrough segment so the filter only matches
/// against the leading target token. Whitespace is trimmed on both sides so
/// filtering still hits exact task names when the user has typed a trailing
/// space.
fn filter_needle(input: &str) -> &str {
    let trimmed = input.trim();
    if let Some(idx) = trimmed.find(" -- ") {
        trimmed[..idx].trim()
    } else {
        trimmed
    }
}

fn format_dur(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1_000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(slice: &[&str]) -> Vec<String> {
        slice.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn empty_input_lists_all_tasks_in_order() {
        let l = LauncherState::new(s(&["fmt", "lint", "test"]));
        assert_eq!(l.filtered_tasks(), vec!["fmt", "lint", "test"]);
        assert_eq!(l.selected, Some(0));
    }

    #[test]
    fn empty_state_with_no_tasks_has_no_selection() {
        let l = LauncherState::new(vec![]);
        assert_eq!(l.selected, None);
        assert!(l.filtered_tasks().is_empty());
    }

    #[test]
    fn typing_filters_case_insensitively() {
        let mut l = LauncherState::new(s(&["fmt", "lint", "test", "FormatDocs"]));
        l.apply(LauncherAction::InsertChar('f'));
        let filtered = l.filtered_tasks();
        assert_eq!(filtered, vec!["fmt", "FormatDocs"]);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut l = LauncherState::new(s(&["fmt", "lint"]));
        l.apply(LauncherAction::InsertChar('f'));
        l.apply(LauncherAction::InsertChar('m'));
        assert_eq!(l.filtered_tasks(), vec!["fmt"]);
        l.apply(LauncherAction::Backspace);
        assert_eq!(l.filtered_tasks(), vec!["fmt"]);
        l.apply(LauncherAction::Backspace);
        assert_eq!(l.filtered_tasks(), vec!["fmt", "lint"]);
    }

    #[test]
    fn up_and_down_wrap_around() {
        let mut l = LauncherState::new(s(&["a", "b", "c"]));
        l.apply(LauncherAction::Down);
        assert_eq!(l.selected, Some(1));
        l.apply(LauncherAction::Down);
        assert_eq!(l.selected, Some(2));
        l.apply(LauncherAction::Down);
        assert_eq!(l.selected, Some(0));
        l.apply(LauncherAction::Up);
        assert_eq!(l.selected, Some(2));
    }

    #[test]
    fn home_and_end_jump_to_bounds() {
        let mut l = LauncherState::new(s(&["a", "b", "c"]));
        l.apply(LauncherAction::Down);
        l.apply(LauncherAction::End);
        assert_eq!(l.selected, Some(2));
        l.apply(LauncherAction::Home);
        assert_eq!(l.selected, Some(0));
    }

    #[test]
    fn selection_reclamps_when_filter_shrinks_list() {
        let mut l = LauncherState::new(s(&["fmt", "lint", "test"]));
        l.apply(LauncherAction::Down);
        l.apply(LauncherAction::Down); // selected = 2 (test)
        l.apply(LauncherAction::InsertChar('f')); // now only "fmt"
        assert_eq!(l.filtered_tasks(), vec!["fmt"]);
        assert_eq!(l.selected, Some(0));
    }

    #[test]
    fn empty_input_with_selection_dispatches_highlighted_task() {
        let mut l = LauncherState::new(s(&["fmt", "lint"]));
        l.apply(LauncherAction::Down); // highlight "lint"
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(
            decision,
            LauncherDecision::Run(ParsedCommand { target: "lint".into(), passthrough: vec![] })
        );
    }

    #[test]
    fn explicit_target_wins_over_highlight() {
        let mut l = LauncherState::new(s(&["fmt", "lint"]));
        l.apply(LauncherAction::InsertChar('l'));
        l.apply(LauncherAction::InsertChar('i'));
        l.apply(LauncherAction::InsertChar('n'));
        l.apply(LauncherAction::InsertChar('t'));
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(
            decision,
            LauncherDecision::Run(ParsedCommand { target: "lint".into(), passthrough: vec![] })
        );
    }

    #[test]
    fn enter_on_unknown_target_with_no_filter_match_stays_with_status_message() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "nope".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(decision, LauncherDecision::Continue);
        assert!(l.status.as_deref().unwrap_or("").contains("no task matches"));
    }

    #[test]
    fn enter_on_partial_input_falls_back_to_highlighted_task() {
        let mut l = LauncherState::new(s(&["fmt", "lint", "test"]));
        // Typed 'te' filters to ["test"], highlighted is "test".
        l.apply(LauncherAction::InsertChar('t'));
        l.apply(LauncherAction::InsertChar('e'));
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(
            decision,
            LauncherDecision::Run(ParsedCommand { target: "test".into(), passthrough: vec![] })
        );
    }

    #[test]
    fn enter_on_partial_input_with_passthrough_preserves_args() {
        let mut l = LauncherState::new(s(&["test"]));
        for c in "te -- --grep slow".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(
            decision,
            LauncherDecision::Run(ParsedCommand {
                target: "test".into(),
                passthrough: vec!["--grep".into(), "slow".into()],
            })
        );
    }

    #[test]
    fn enter_with_double_dash_extracts_passthrough() {
        let mut l = LauncherState::new(s(&["test"]));
        for c in "test -- --grep slow".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(
            decision,
            LauncherDecision::Run(ParsedCommand {
                target: "test".into(),
                passthrough: vec!["--grep".into(), "slow".into()],
            })
        );
    }

    #[test]
    fn enter_with_inline_args_collects_them() {
        let mut l = LauncherState::new(s(&["serve"]));
        for c in "serve --port 8080".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        let decision = l.apply(LauncherAction::Enter);
        assert_eq!(
            decision,
            LauncherDecision::Run(ParsedCommand {
                target: "serve".into(),
                passthrough: vec!["--port".into(), "8080".into()],
            })
        );
    }

    #[test]
    fn enter_with_only_whitespace_input_and_no_selection_does_not_run() {
        let mut l = LauncherState::new(vec![]);
        for c in "   ".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.status.is_some());
    }

    #[test]
    fn quit_action_quits_only_when_input_is_empty() {
        let mut l = LauncherState::new(s(&["fmt"]));
        l.apply(LauncherAction::InsertChar('f'));
        // 'q' typed while input has content shouldn't quit; we treat Quit as
        // "clear the buffer first".
        let dec = l.apply(LauncherAction::Quit);
        assert_eq!(dec, LauncherDecision::Continue);
        assert!(l.input.is_empty());
        // Now input is empty, Quit really quits.
        let dec = l.apply(LauncherAction::Quit);
        assert_eq!(dec, LauncherDecision::Quit);
    }

    #[test]
    fn clear_input_drops_typed_text_but_keeps_screen() {
        let mut l = LauncherState::new(s(&["fmt", "lint"]));
        l.apply(LauncherAction::InsertChar('f'));
        l.apply(LauncherAction::InsertChar('m'));
        let dec = l.apply(LauncherAction::ClearInput);
        assert_eq!(dec, LauncherDecision::Continue);
        assert!(l.input.is_empty());
        assert_eq!(l.filtered_tasks(), vec!["fmt", "lint"]);
    }

    #[test]
    fn complete_replaces_input_with_highlighted_task_name() {
        let mut l = LauncherState::new(s(&["fmt", "lint"]));
        l.apply(LauncherAction::InsertChar('l'));
        l.apply(LauncherAction::Complete);
        assert_eq!(l.input, "lint");
    }

    #[test]
    fn record_run_pushes_entry_to_front_and_caps_at_16() {
        let mut l = LauncherState::new(s(&["t"]));
        for i in 0..20 {
            l.record_run(format!("t{}", i), RunOutcome::Success, Duration::from_millis(10));
        }
        assert_eq!(l.history.len(), 16);
        assert_eq!(l.history[0].target, "t19");
        assert_eq!(l.history[15].target, "t4");
    }

    #[test]
    fn record_run_clears_input_and_sets_status_banner() {
        let mut l = LauncherState::new(s(&["fmt"]));
        l.apply(LauncherAction::InsertChar('f'));
        l.record_run("fmt".into(), RunOutcome::Success, Duration::from_millis(120));
        assert!(l.input.is_empty());
        let s = l.status.as_deref().unwrap();
        assert!(s.contains("ran fmt"));
        assert!(s.contains("120ms"));
    }

    #[test]
    fn status_banner_clears_on_next_user_action() {
        let mut l = LauncherState::new(s(&["fmt"]));
        l.record_run("fmt".into(), RunOutcome::Success, Duration::from_millis(50));
        assert!(l.status.is_some());
        l.apply(LauncherAction::InsertChar('x'));
        assert!(l.status.is_none());
    }

    #[test]
    fn duration_formatting_handles_ms_seconds_and_minutes() {
        assert_eq!(format_dur(Duration::from_millis(500)), "500ms");
        assert_eq!(format_dur(Duration::from_millis(1_500)), "1.5s");
        assert_eq!(format_dur(Duration::from_secs(125)), "2m5s");
    }

    #[test]
    fn filter_needle_strips_passthrough_segment() {
        assert_eq!(filter_needle("test -- --grep slow"), "test");
        assert_eq!(filter_needle("  fmt  "), "fmt");
        assert_eq!(filter_needle("--all"), "--all");
    }

    // -------- slash command tests --------

    #[test]
    fn slash_mode_engages_on_leading_slash() {
        let mut l = LauncherState::new(s(&["fmt", "lint"]));
        assert_eq!(l.mode, LauncherMode::Filter);
        l.apply(LauncherAction::InsertChar('/'));
        assert_eq!(l.mode, LauncherMode::Slash);
        assert!(!l.filtered_slash_commands().is_empty());
        l.apply(LauncherAction::Backspace);
        assert_eq!(l.mode, LauncherMode::Filter);
    }

    #[test]
    fn slash_typing_narrows_command_list() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/th".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        let filtered = l.filtered_slash_commands();
        assert_eq!(filtered, vec!["/theme"]);
    }

    #[test]
    fn slash_help_dispatches_help_decision_with_status() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/help".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        let banner = l.status.as_deref().unwrap_or("");
        assert!(banner.contains("/theme"), "banner should list commands; got: {banner}");
        assert!(l.input.is_empty());
    }

    #[test]
    fn slash_theme_dispatches_switch_with_parsed_theme() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/theme dark".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::SwitchTheme(Theme::Dark));
        assert_eq!(l.mode, LauncherMode::Filter);
        assert!(l.input.is_empty());
    }

    #[test]
    fn slash_theme_unknown_returns_status_banner() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/theme neon".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.status.as_deref().unwrap_or("").contains("unknown theme"));
        // Input is preserved so the user can edit it.
        assert!(l.input.starts_with("/theme"));
    }

    #[test]
    fn slash_theme_without_arg_shows_usage() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/theme".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.status.as_deref().unwrap_or("").contains("usage:"));
    }

    #[test]
    fn slash_quit_dispatches_quit() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/quit".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Quit);
        // /q alias too.
        let mut l2 = LauncherState::new(s(&["fmt"]));
        for c in "/q".chars() {
            l2.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l2.apply(LauncherAction::Enter), LauncherDecision::Quit);
    }

    #[test]
    fn slash_clear_clears_history_and_resets_stats() {
        let mut l = LauncherState::new(s(&["fmt"]));
        l.record_run("fmt".into(), RunOutcome::Success, Duration::from_millis(10));
        l.record_run("fmt".into(), RunOutcome::Failed, Duration::from_millis(5));
        assert_eq!(l.history.len(), 2);
        assert_eq!(l.stats.total_runs, 2);
        for c in "/clear".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.history.is_empty());
        assert_eq!(l.stats, SessionStats::default());
    }

    #[test]
    fn slash_refresh_dispatches_refresh_decision() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/refresh".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Refresh);
    }

    #[test]
    fn slash_cache_prune_parses_megabytes() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/cache prune 256".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::PruneCache(256));
    }

    #[test]
    fn slash_cache_prune_invalid_arg_shows_status() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/cache prune lots".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.status.as_deref().unwrap_or("").contains("expected"));
    }

    #[test]
    fn slash_cache_without_subcommand_shows_usage() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/cache".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.status.as_deref().unwrap_or("").contains("usage:"));
    }

    #[test]
    fn slash_unknown_command_shows_banner() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/wat".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        assert!(l.status.as_deref().unwrap_or("").contains("unknown command"));
    }

    #[test]
    fn slash_about_emits_status_with_version() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/about".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        assert_eq!(l.apply(LauncherAction::Enter), LauncherDecision::Continue);
        let banner = l.status.as_deref().unwrap_or("");
        assert!(banner.starts_with("broski "), "banner = {banner}");
    }

    #[test]
    fn slash_tab_completes_to_canonical_form() {
        let mut l = LauncherState::new(s(&["fmt"]));
        for c in "/th".chars() {
            l.apply(LauncherAction::InsertChar(c));
        }
        l.apply(LauncherAction::Complete);
        assert_eq!(l.input, "/theme ");
        // Cache prune completes with prune already filled.
        let mut l2 = LauncherState::new(s(&["fmt"]));
        for c in "/cac".chars() {
            l2.apply(LauncherAction::InsertChar(c));
        }
        l2.apply(LauncherAction::Complete);
        assert_eq!(l2.input, "/cache prune ");
    }

    #[test]
    fn slash_arrow_navigates_command_list() {
        let mut l = LauncherState::new(s(&["fmt"]));
        l.apply(LauncherAction::InsertChar('/'));
        let total = l.filtered_slash_commands().len();
        assert!(total > 1);
        l.apply(LauncherAction::Down);
        assert_eq!(l.selected, Some(1));
        l.apply(LauncherAction::Down);
        assert_eq!(l.selected, Some(2));
        l.apply(LauncherAction::Up);
        assert_eq!(l.selected, Some(1));
    }

    #[test]
    fn session_stats_increment_on_record_run() {
        let mut l = LauncherState::new(s(&["a"]));
        l.record_run("a".into(), RunOutcome::Success, Duration::from_millis(50));
        l.record_run("a".into(), RunOutcome::Success, Duration::from_millis(150));
        l.record_run("a".into(), RunOutcome::Failed, Duration::from_millis(75));
        l.record_run("a".into(), RunOutcome::Cancelled, Duration::from_millis(25));
        assert_eq!(l.stats.total_runs, 4);
        assert_eq!(l.stats.successes, 2);
        assert_eq!(l.stats.failures, 1);
        assert_eq!(l.stats.cancellations, 1);
        assert_eq!(l.stats.total_duration, Duration::from_millis(50 + 150 + 75 + 25));
    }

    #[test]
    fn parse_slash_command_handles_aliases() {
        assert_eq!(parse_slash_command("/h"), Ok(SlashCommand::Help));
        assert_eq!(parse_slash_command("/?"), Ok(SlashCommand::Help));
        assert_eq!(parse_slash_command("/exit"), Ok(SlashCommand::Quit));
        assert_eq!(parse_slash_command("/version"), Ok(SlashCommand::About));
        assert_eq!(parse_slash_command("/reload"), Ok(SlashCommand::Refresh));
    }

    #[test]
    fn parse_slash_command_rejects_non_slash_input() {
        let err = parse_slash_command("foo").expect_err("non-slash should fail");
        assert!(err.contains("starting with '/'"));
    }
}
