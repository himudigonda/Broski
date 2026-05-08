//! Ratatui-based terminal dashboard for live broski runs.
//!
//! The TUI is a thin client over `broski_core::Executor`'s event stream
//! ([`ProgressEvent`](broski_core::ProgressEvent)). It spawns the executor on
//! a background thread, consumes events through an mpsc channel, folds them
//! into [`TuiState`], and redraws on demand.
//!
//! Public entrypoint: [`run`]. The CLI's `broski tui <task>` calls into it.

pub mod keys;
pub mod state;
pub mod theme;
pub mod widgets;

mod app;

pub use app::run;
pub use state::{LogLineRecord, TaskState, TuiState};
