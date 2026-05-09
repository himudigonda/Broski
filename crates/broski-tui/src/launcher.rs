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

/// Result of folding an action into the launcher state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherDecision {
    /// Stay on the launcher screen.
    Continue,
    /// Run the parsed command, then return here.
    Run(ParsedCommand),
    /// Tear down the TUI and exit.
    Quit,
}

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

/// The launcher's full state. Held by the app loop and rendered each tick.
#[derive(Debug, Clone)]
pub struct LauncherState {
    /// Free-form input. May contain `-- arg arg`.
    pub input: String,
    /// All non-private task names in display order.
    pub all_tasks: Vec<String>,
    /// Index into [`filtered_tasks`] of the currently-highlighted match.
    /// `None` when there are no matches.
    pub selected: Option<usize>,
    /// Recent runs from this launcher session, newest first.
    pub history: Vec<RunHistoryEntry>,
    /// Transient banner ("ran X in 1.2s", "no task matches 'fooo'", etc.).
    pub status: Option<String>,
}

impl LauncherState {
    pub fn new(all_tasks: Vec<String>) -> Self {
        let selected = if all_tasks.is_empty() { None } else { Some(0) };
        Self { input: String::new(), all_tasks, selected, history: Vec::new(), status: None }
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

    /// Clamp `selected` into `0..filtered.len()` after an input edit.
    fn reclamp_selection(&mut self) {
        let len = self.filtered_tasks().len();
        if len == 0 {
            self.selected = None;
        } else {
            self.selected = Some(self.selected.map_or(0, |i| i.min(len - 1)));
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
                self.reclamp_selection();
                LauncherDecision::Continue
            }
            LauncherAction::Backspace => {
                self.input.pop();
                self.reclamp_selection();
                LauncherDecision::Continue
            }
            LauncherAction::ClearInput => {
                if self.input.is_empty() {
                    LauncherDecision::Continue
                } else {
                    self.input.clear();
                    self.reclamp_selection();
                    LauncherDecision::Continue
                }
            }
            LauncherAction::Up => {
                let len = self.filtered_tasks().len();
                if len > 0 {
                    self.selected = Some(match self.selected {
                        Some(i) if i > 0 => i - 1,
                        _ => len - 1,
                    });
                }
                LauncherDecision::Continue
            }
            LauncherAction::Down => {
                let len = self.filtered_tasks().len();
                if len > 0 {
                    self.selected = Some(match self.selected {
                        Some(i) if i + 1 < len => i + 1,
                        _ => 0,
                    });
                }
                LauncherDecision::Continue
            }
            LauncherAction::Home => {
                if !self.filtered_tasks().is_empty() {
                    self.selected = Some(0);
                }
                LauncherDecision::Continue
            }
            LauncherAction::End => {
                let len = self.filtered_tasks().len();
                if len > 0 {
                    self.selected = Some(len - 1);
                }
                LauncherDecision::Continue
            }
            LauncherAction::Complete => {
                if let Some(target) = self.highlighted_task().map(str::to_string) {
                    self.input = target;
                    self.reclamp_selection();
                }
                LauncherDecision::Continue
            }
            LauncherAction::Enter => match self.parse_command() {
                Some(cmd) => {
                    if self.all_tasks.contains(&cmd.target) {
                        return LauncherDecision::Run(cmd);
                    }
                    // Typed target doesn't exactly match a task, but the
                    // filter may have narrowed to one — fall back to the
                    // highlighted match, preserving any `-- args` the user
                    // typed.
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
            },
            LauncherAction::Quit => {
                if self.input.is_empty() {
                    LauncherDecision::Quit
                } else {
                    self.input.clear();
                    self.reclamp_selection();
                    LauncherDecision::Continue
                }
            }
            LauncherAction::Ignore => LauncherDecision::Continue,
        }
    }

    /// Record a finished run, then clear the input box so the user can
    /// pick a new target. Newest entries land at index 0 and the list is
    /// capped at 16.
    pub fn record_run(&mut self, target: String, outcome: RunOutcome, duration: Duration) {
        let entry = RunHistoryEntry { target: target.clone(), outcome: outcome.clone(), duration };
        let label = match outcome {
            RunOutcome::Success => "ran",
            RunOutcome::Failed => "failed",
            RunOutcome::Cancelled => "cancelled",
        };
        self.status = Some(format!("{} {} in {}", label, target, format_dur(duration)));
        self.history.insert(0, entry);
        if self.history.len() > 16 {
            self.history.truncate(16);
        }
        self.input.clear();
        self.reclamp_selection();
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
}
