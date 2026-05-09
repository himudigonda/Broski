//! TUI state machine.
//!
//! Pure: every external input is folded into state through [`TuiState::apply`].
//! No I/O happens here — easy to unit-test, easy to reason about.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;

use broski_core::{LogStream, ProgressEvent, TaskPhase, TaskStatus};

/// Per-task lifecycle status as observed by the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Cached,
    Done,
    Failed,
    DryRun,
    Skipped,
}

/// One captured log line (kept in a per-task ring buffer).
#[derive(Debug, Clone)]
pub struct LogLineRecord {
    pub stream: LogStream,
    pub line: String,
}

/// Per-task UI record.
#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub state: TaskState,
    pub current_phase: Option<TaskPhase>,
    pub phase_count: u32,
    pub duration: Duration,
    pub error: Option<String>,
    pub logs: VecDeque<LogLineRecord>,
    /// Distance the user has scrolled UP from the bottom of [`logs`], in
    /// log lines. `0` means "follow the tail"; any positive value means
    /// the user is reviewing older output and we should NOT auto-snap to
    /// the bottom on new lines.
    pub scrollback: usize,
    /// True while [`scrollback`] is `0` *and* we should keep snapping to
    /// the bottom on new lines. Flipped off the moment the user scrolls
    /// up; flipped back on when they hit `End` or scroll past the bottom.
    pub follow_tail: bool,
    /// Cache hit/miss explain reasons captured on `TaskFinished`. Empty
    /// unless the run was started with `RunOptions::explain` set.
    pub cache_reasons: Vec<String>,
}

impl Default for TaskInfo {
    fn default() -> Self {
        Self {
            state: TaskState::Queued,
            current_phase: None,
            phase_count: 0,
            duration: Duration::ZERO,
            error: None,
            logs: VecDeque::new(),
            scrollback: 0,
            follow_tail: true,
            cache_reasons: Vec::new(),
        }
    }
}

/// Per-task ring buffer cap. Older lines are dropped with a sentinel marker.
pub const LOG_CAPACITY: usize = 4096;

/// User-driven cancellation state, advanced by the two-stage Ctrl-C handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CancelState {
    #[default]
    Idle,
    /// First Ctrl-C received: pending tasks will be skipped, in-flight
    /// children are left alone.
    Soft,
    /// Second Ctrl-C received within the cancellation window: in-flight
    /// children have been signaled.
    Hard,
}

/// A pending force-rerun request, set by `x` / `X` after a run finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunRequest {
    /// Target to pass to the next `run_target` call.
    /// For `x` this is the selected task; for `X` this is the original target.
    pub task: String,
    /// When true, set `RunOptions::force = true` (re-run everything).
    /// When false, set `RunOptions::force_tasks = {task}` (re-run only that node).
    pub force_all: bool,
}

/// Full TUI state.
#[derive(Debug, Default, Clone)]
pub struct TuiState {
    /// `Some` once `RunStarted` is received.
    pub target: Option<String>,
    /// Topo layers from `RunStarted`. Drives the DAG widget order.
    pub layers: Vec<Vec<String>>,
    /// Flat tasks-in-order (left-to-right within layer, layer-by-layer).
    pub task_order: Vec<String>,
    /// Per-task UI state.
    pub tasks: HashMap<String, TaskInfo>,
    /// User cursor — index into `task_order`. None until first task seen.
    pub selected: Option<usize>,
    /// True after `RunFinished`.
    pub run_finished: bool,
    /// Aggregate counters for the summary widget.
    pub running_count: u32,
    pub done_count: u32,
    pub cached_count: u32,
    pub failed_count: u32,
    /// Estimated duration per task, prefetched from the artifact-store
    /// history before the run starts. Empty when no history exists yet.
    pub etas: BTreeMap<String, Duration>,
    /// Two-stage cancellation state, advanced by the app's Ctrl-C handler.
    pub cancel: CancelState,
    /// Pending force-rerun request, set by `x` / `X` after `run_finished`.
    /// Consumed once by the app loop to trigger a new executor run.
    pub pending_rerun: Option<RerunRequest>,
}

impl TuiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with prefetched per-task ETAs (typically the most recent
    /// successful run's wall-clock duration).
    pub fn with_etas(etas: BTreeMap<String, Duration>) -> Self {
        Self { etas, ..Self::default() }
    }

    /// Sum of ETAs for tasks still queued or running. Returns
    /// `Duration::ZERO` when no relevant ETAs exist.
    pub fn remaining_eta(&self) -> Duration {
        self.task_order
            .iter()
            .filter_map(|name| {
                let info = self.tasks.get(name)?;
                if matches!(info.state, TaskState::Queued | TaskState::Running) {
                    self.etas.get(name).copied()
                } else {
                    None
                }
            })
            .sum()
    }

    /// Apply one event. Pure: returns nothing, mutates state.
    pub fn apply(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::RunStarted { target, layers } => {
                self.target = Some(target);
                self.layers = layers.clone();
                self.task_order = layers.into_iter().flatten().collect();
                for name in &self.task_order {
                    self.tasks.entry(name.clone()).or_default();
                }
                if !self.task_order.is_empty() && self.selected.is_none() {
                    self.selected = Some(0);
                }
            }
            ProgressEvent::TaskQueued { task } => {
                let info = self.tasks.entry(task.clone()).or_default();
                info.state = TaskState::Queued;
                if !self.task_order.contains(&task) {
                    self.task_order.push(task);
                }
            }
            ProgressEvent::TaskStarted { task, .. } => {
                let info = self.tasks.entry(task).or_default();
                info.state = TaskState::Running;
                self.running_count += 1;
            }
            ProgressEvent::TaskPhase { task, phase, elapsed } => {
                let info = self.tasks.entry(task).or_default();
                info.current_phase = Some(phase);
                info.phase_count += 1;
                info.duration += elapsed;
            }
            ProgressEvent::LogLine { task, stream, line } => {
                let info = self.tasks.entry(task).or_default();
                let was_full = info.logs.len() >= LOG_CAPACITY;
                if was_full {
                    info.logs.pop_front();
                    // We dropped the oldest line, so any positive
                    // scrollback offset has effectively shifted by one.
                    // Decrement so the user keeps looking at the same
                    // visual region rather than silently sliding upward.
                    info.scrollback = info.scrollback.saturating_sub(1);
                }
                info.logs.push_back(LogLineRecord { stream, line });
                // When following the tail, keep scrollback pinned to 0;
                // otherwise leave the user where they were.
                if info.follow_tail {
                    info.scrollback = 0;
                }
            }
            ProgressEvent::TaskFinished { task, status, duration, error, cache_reasons } => {
                let info = self.tasks.entry(task).or_default();
                if info.state == TaskState::Running {
                    self.running_count = self.running_count.saturating_sub(1);
                }
                info.duration = duration;
                info.error = error;
                info.cache_reasons = cache_reasons;
                info.current_phase = None;
                info.state = match status {
                    TaskStatus::Executed => {
                        self.done_count += 1;
                        TaskState::Done
                    }
                    TaskStatus::CacheHit => {
                        self.cached_count += 1;
                        TaskState::Cached
                    }
                    TaskStatus::DryRun => TaskState::DryRun,
                    TaskStatus::Failed => {
                        self.failed_count += 1;
                        TaskState::Failed
                    }
                    TaskStatus::Skipped => TaskState::Skipped,
                };
            }
            ProgressEvent::RunFinished => {
                self.run_finished = true;
            }
        }
    }

    /// Move the cursor by `delta` (saturating, no-wrap).
    pub fn move_selection(&mut self, delta: i32) {
        if self.task_order.is_empty() {
            self.selected = None;
            return;
        }
        let last = (self.task_order.len() - 1) as i32;
        let current = self.selected.map_or(0, |s| s as i32);
        let next = (current + delta).clamp(0, last) as usize;
        self.selected = Some(next);
    }

    /// Currently-selected task name, if any.
    pub fn selected_task(&self) -> Option<&str> {
        let idx = self.selected?;
        self.task_order.get(idx).map(String::as_str)
    }

    /// Clear logs for the selected task. Used by the `c` keybind.
    pub fn clear_selected_logs(&mut self) {
        if let Some(name) = self.selected_task().map(str::to_string) {
            if let Some(info) = self.tasks.get_mut(&name) {
                info.logs.clear();
                info.scrollback = 0;
                info.follow_tail = true;
            }
        }
    }

    /// Scroll the selected task's log pane UP by `lines`. Disengages
    /// follow-tail (so new lines arriving don't yank the user back to
    /// the bottom). `lines == 0` is a no-op.
    pub fn scroll_logs_up(&mut self, lines: usize) {
        let Some(info) = self.selected_task_info_mut() else {
            return;
        };
        let total = info.logs.len();
        // Maximum scrollback is `total - 1` so at least one line stays
        // visible at the very top.
        let max = total.saturating_sub(1);
        info.scrollback = info.scrollback.saturating_add(lines).min(max);
        info.follow_tail = info.scrollback == 0;
    }

    /// Scroll the selected task's log pane DOWN by `lines`. Hitting the
    /// bottom (`scrollback == 0`) re-enables follow-tail.
    pub fn scroll_logs_down(&mut self, lines: usize) {
        let Some(info) = self.selected_task_info_mut() else {
            return;
        };
        info.scrollback = info.scrollback.saturating_sub(lines);
        if info.scrollback == 0 {
            info.follow_tail = true;
        }
    }

    /// Jump to the very top of the selected task's log buffer.
    pub fn scroll_logs_home(&mut self) {
        let Some(info) = self.selected_task_info_mut() else {
            return;
        };
        let total = info.logs.len();
        info.scrollback = total.saturating_sub(1);
        info.follow_tail = false;
    }

    /// Jump back to the tail (bottom) and resume follow-tail mode.
    pub fn scroll_logs_end(&mut self) {
        let Some(info) = self.selected_task_info_mut() else {
            return;
        };
        info.scrollback = 0;
        info.follow_tail = true;
    }

    /// Set a pending force-rerun only when the run has already finished.
    /// Silently ignored while a run is still in progress.
    pub fn request_rerun(&mut self, task: String, force_all: bool) {
        if self.run_finished {
            self.pending_rerun = Some(RerunRequest { task, force_all });
        }
    }

    fn selected_task_info_mut(&mut self) -> Option<&mut TaskInfo> {
        let name = self.selected_task().map(str::to_string)?;
        self.tasks.get_mut(&name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev_run_started(layers: Vec<Vec<&str>>) -> ProgressEvent {
        ProgressEvent::RunStarted {
            target: "ci".to_string(),
            layers: layers
                .into_iter()
                .map(|layer| layer.into_iter().map(String::from).collect())
                .collect(),
        }
    }

    #[test]
    fn run_started_seeds_task_order_and_selection() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["fmt"], vec!["lint"], vec!["test"]]));
        assert_eq!(s.task_order, vec!["fmt", "lint", "test"]);
        assert_eq!(s.selected, Some(0));
        assert_eq!(s.target.as_deref(), Some("ci"));
    }

    #[test]
    fn task_started_then_finished_transitions_state_and_counters() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["fmt"]]));
        s.apply(ProgressEvent::TaskStarted {
            task: "fmt".to_string(),
            mode: broski_core::TaskMode::Graph,
        });
        assert_eq!(s.running_count, 1);
        assert_eq!(s.tasks["fmt"].state, TaskState::Running);
        s.apply(ProgressEvent::TaskFinished {
            task: "fmt".to_string(),
            status: TaskStatus::Executed,
            duration: Duration::from_millis(42),
            error: None,
            cache_reasons: Vec::new(),
        });
        assert_eq!(s.running_count, 0);
        assert_eq!(s.done_count, 1);
        assert_eq!(s.tasks["fmt"].state, TaskState::Done);
        assert_eq!(s.tasks["fmt"].duration, Duration::from_millis(42));
    }

    #[test]
    fn cache_hit_increments_cached_counter_only() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["build"]]));
        s.apply(ProgressEvent::TaskStarted {
            task: "build".to_string(),
            mode: broski_core::TaskMode::Graph,
        });
        s.apply(ProgressEvent::TaskFinished {
            task: "build".to_string(),
            status: TaskStatus::CacheHit,
            duration: Duration::from_millis(1),
            error: None,
            cache_reasons: Vec::new(),
        });
        assert_eq!(s.cached_count, 1);
        assert_eq!(s.done_count, 0);
        assert_eq!(s.tasks["build"].state, TaskState::Cached);
    }

    #[test]
    fn failed_task_records_error_and_increments_failed() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["test"]]));
        s.apply(ProgressEvent::TaskStarted {
            task: "test".to_string(),
            mode: broski_core::TaskMode::Graph,
        });
        s.apply(ProgressEvent::TaskFinished {
            task: "test".to_string(),
            status: TaskStatus::Failed,
            duration: Duration::from_millis(99),
            error: Some("assertion failed".to_string()),
            cache_reasons: vec!["cache miss: input changed: src/lib.rs".into()],
        });
        assert_eq!(s.failed_count, 1);
        assert_eq!(s.tasks["test"].state, TaskState::Failed);
        assert_eq!(s.tasks["test"].error.as_deref(), Some("assertion failed"));
    }

    #[test]
    fn log_lines_are_buffered_and_capped() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["x"]]));
        for i in 0..(LOG_CAPACITY + 50) {
            s.apply(ProgressEvent::LogLine {
                task: "x".to_string(),
                stream: LogStream::Stdout,
                line: format!("line {i}"),
            });
        }
        let logs = &s.tasks["x"].logs;
        assert_eq!(logs.len(), LOG_CAPACITY);
        // Oldest 50 lines were evicted.
        assert_eq!(logs.front().unwrap().line, format!("line {}", 50));
        assert_eq!(logs.back().unwrap().line, format!("line {}", LOG_CAPACITY + 50 - 1));
    }

    #[test]
    fn selection_cursor_clamps_at_bounds() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"], vec!["b"], vec!["c"]]));
        s.move_selection(-5);
        assert_eq!(s.selected, Some(0));
        s.move_selection(10);
        assert_eq!(s.selected, Some(2));
        s.move_selection(-1);
        assert_eq!(s.selected, Some(1));
        assert_eq!(s.selected_task(), Some("b"));
    }

    #[test]
    fn run_finished_sets_flag() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"]]));
        s.apply(ProgressEvent::RunFinished);
        assert!(s.run_finished);
    }

    #[test]
    fn remaining_eta_sums_only_queued_and_running_tasks() {
        let mut etas = BTreeMap::new();
        etas.insert("a".to_string(), Duration::from_secs(2));
        etas.insert("b".to_string(), Duration::from_secs(3));
        etas.insert("c".to_string(), Duration::from_secs(5));
        let mut s = TuiState::with_etas(etas);
        s.apply(ev_run_started(vec![vec!["a", "b", "c"]]));
        // Initially all queued -> sum is 2+3+5 = 10.
        assert_eq!(s.remaining_eta(), Duration::from_secs(10));

        // Mark `a` running: still counts toward remaining.
        s.apply(ProgressEvent::TaskStarted {
            task: "a".to_string(),
            mode: broski_core::TaskMode::Graph,
        });
        assert_eq!(s.remaining_eta(), Duration::from_secs(10));

        // Finish `a`: should drop from remaining.
        s.apply(ProgressEvent::TaskFinished {
            task: "a".to_string(),
            status: TaskStatus::Executed,
            duration: Duration::from_secs(2),
            error: None,
            cache_reasons: Vec::new(),
        });
        assert_eq!(s.remaining_eta(), Duration::from_secs(8));

        // Cache hit on `b`: also drops.
        s.apply(ProgressEvent::TaskStarted {
            task: "b".to_string(),
            mode: broski_core::TaskMode::Graph,
        });
        s.apply(ProgressEvent::TaskFinished {
            task: "b".to_string(),
            status: TaskStatus::CacheHit,
            duration: Duration::from_millis(5),
            error: None,
            cache_reasons: Vec::new(),
        });
        assert_eq!(s.remaining_eta(), Duration::from_secs(5));
    }

    #[test]
    fn remaining_eta_is_zero_with_no_history() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"]]));
        assert_eq!(s.remaining_eta(), Duration::ZERO);
    }

    fn push_log(s: &mut TuiState, task: &str, line: &str) {
        s.apply(ProgressEvent::LogLine {
            task: task.to_string(),
            stream: LogStream::Stdout,
            line: line.into(),
        });
    }

    #[test]
    fn new_log_line_pins_scrollback_to_zero_when_following_tail() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"]]));
        push_log(&mut s, "a", "one");
        push_log(&mut s, "a", "two");
        let info = &s.tasks["a"];
        assert!(info.follow_tail);
        assert_eq!(info.scrollback, 0);
    }

    #[test]
    fn scroll_up_disengages_follow_and_clamps_at_top() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"]]));
        for i in 0..10 {
            push_log(&mut s, "a", &format!("line {i}"));
        }
        s.scroll_logs_up(3);
        let info = &s.tasks["a"];
        assert_eq!(info.scrollback, 3);
        assert!(!info.follow_tail);

        // New lines should NOT snap back to bottom while user is scrolled up.
        push_log(&mut s, "a", "line 10");
        let info = &s.tasks["a"];
        assert_eq!(info.scrollback, 3);
        assert!(!info.follow_tail);

        // Asking for way more than available clamps to len-1.
        s.scroll_logs_up(10_000);
        let info = &s.tasks["a"];
        assert_eq!(info.scrollback, info.logs.len() - 1);
    }

    #[test]
    fn scroll_down_to_zero_resumes_follow_tail() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"]]));
        for i in 0..5 {
            push_log(&mut s, "a", &format!("line {i}"));
        }
        s.scroll_logs_up(2);
        assert!(!s.tasks["a"].follow_tail);
        s.scroll_logs_down(2);
        let info = &s.tasks["a"];
        assert_eq!(info.scrollback, 0);
        assert!(info.follow_tail);

        // Once follow is back on, new lines stay pinned at 0.
        push_log(&mut s, "a", "line 5");
        assert_eq!(s.tasks["a"].scrollback, 0);
    }

    #[test]
    fn scroll_home_jumps_to_top_and_end_jumps_to_tail() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"]]));
        for i in 0..7 {
            push_log(&mut s, "a", &format!("line {i}"));
        }
        s.scroll_logs_home();
        assert_eq!(s.tasks["a"].scrollback, 6);
        assert!(!s.tasks["a"].follow_tail);
        s.scroll_logs_end();
        assert_eq!(s.tasks["a"].scrollback, 0);
        assert!(s.tasks["a"].follow_tail);
    }

    #[test]
    fn scroll_methods_are_no_op_with_no_selection() {
        let mut s = TuiState::new();
        // No tasks, no panic.
        s.scroll_logs_up(3);
        s.scroll_logs_down(3);
        s.scroll_logs_home();
        s.scroll_logs_end();
    }

    #[test]
    fn evicting_oldest_line_keeps_user_view_stable() {
        // When the ring buffer is full and a new line is pushed, the
        // oldest line is dropped; if the user was scrolled up, their
        // visual position should slide along with the buffer (i.e.
        // scrollback decrements by 1) so they're still looking at the
        // same lines, not a silently-shifted view.
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["x"]]));
        // Fill the ring exactly.
        for i in 0..LOG_CAPACITY {
            push_log(&mut s, "x", &format!("line {i}"));
        }
        s.scroll_logs_up(50);
        let before = s.tasks["x"].scrollback;
        // Push 10 more lines — each evicts one.
        for i in 0..10 {
            push_log(&mut s, "x", &format!("evictor {i}"));
        }
        let after = s.tasks["x"].scrollback;
        assert_eq!(after, before.saturating_sub(10), "scrollback must slide with eviction");
        assert!(!s.tasks["x"].follow_tail, "follow_tail must stay off while user is scrolled up");
    }

    #[test]
    fn request_rerun_sets_pending_when_run_is_finished() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["lint"]]));
        s.apply(ProgressEvent::RunFinished);
        assert!(s.run_finished);
        s.request_rerun("lint".to_string(), false);
        assert_eq!(
            s.pending_rerun,
            Some(RerunRequest { task: "lint".to_string(), force_all: false })
        );
    }

    #[test]
    fn request_rerun_ignored_when_run_in_progress() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["lint"]]));
        // run_finished is still false
        s.request_rerun("lint".to_string(), true);
        assert!(s.pending_rerun.is_none(), "should not set pending_rerun while run is in progress");
    }

    #[test]
    fn clear_selected_logs_only_clears_selected_task() {
        let mut s = TuiState::new();
        s.apply(ev_run_started(vec![vec!["a"], vec!["b"]]));
        s.apply(ProgressEvent::LogLine {
            task: "a".to_string(),
            stream: LogStream::Stdout,
            line: "alpha".into(),
        });
        s.apply(ProgressEvent::LogLine {
            task: "b".to_string(),
            stream: LogStream::Stdout,
            line: "beta".into(),
        });
        s.move_selection(1); // select "b"
        s.clear_selected_logs();
        assert_eq!(s.tasks["a"].logs.len(), 1);
        assert_eq!(s.tasks["b"].logs.len(), 0);
    }
}
