//! Cooperative cancellation for executor runs.
//!
//! A [`CancellationToken`] is a cheap, cloneable handle that holders can poll
//! via [`is_cancelled`](CancellationToken::is_cancelled). The executor checks
//! it between layers and before each task; on a hard cancel it also signals
//! every registered child process.
//!
//! Two cancel levels exist so the TUI can implement the familiar
//! "first Ctrl-C asks nicely, second Ctrl-C insists" UX:
//!
//! - [`CancelLevel::Soft`] — sets the cancel flag; pending tasks are skipped
//!   but in-flight children run to completion.
//! - [`CancelLevel::Hard`] — sets the cancel flag *and* sends `SIGTERM` to
//!   every currently-registered child PID. cargo, make, npm and friends
//!   propagate that signal to their own descendants.
//!
//! Cancellation is always opt-in: when `RunOptions::cancellation` is `None`,
//! behavior is byte-identical to today.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Caller-controlled cancel handle. Cheap to clone — internally
/// `Arc<...>`.
#[derive(Debug, Default, Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

#[derive(Debug, Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    children: Mutex<Vec<u32>>,
}

/// How aggressively to cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelLevel {
    /// Skip remaining tasks. Currently-running children are left alone.
    Soft,
    /// Skip remaining tasks **and** send `SIGTERM` to every registered child.
    Hard,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` once any cancel has been issued. Cheap; safe to poll
    /// in hot loops.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Relaxed)
    }

    /// Issue a cancel at the given level. Idempotent.
    pub fn cancel(&self, level: CancelLevel) {
        self.inner.cancelled.store(true, Ordering::Relaxed);
        if level == CancelLevel::Hard {
            self.signal_registered_children();
        }
    }

    /// Register a child PID so a future hard cancel can signal it.
    /// Used by the executor; not part of the public API surface beyond
    /// allowing cross-crate calls.
    pub fn register_child(&self, pid: u32) {
        if let Ok(mut guard) = self.inner.children.lock() {
            guard.push(pid);
        }
    }

    /// Remove a previously-registered child PID (typically once the child
    /// has been waited on).
    pub fn unregister_child(&self, pid: u32) {
        if let Ok(mut guard) = self.inner.children.lock() {
            guard.retain(|registered| *registered != pid);
        }
    }

    /// Snapshot of currently-registered child PIDs. Useful for tests.
    pub fn registered_children(&self) -> Vec<u32> {
        self.inner.children.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn signal_registered_children(&self) {
        let pids = match self.inner.children.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => return,
        };
        for pid in pids {
            send_terminate(pid);
        }
    }
}

#[cfg(unix)]
fn send_terminate(pid: u32) {
    if pid == 0 {
        return;
    }
    // SAFETY: libc::kill is signal-safe; we treat any error (ESRCH after
    // child exited, EPERM on edge cases) as benign.
    unsafe {
        let _ = libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn send_terminate(_pid: u32) {
    // Cancellation is currently a Unix-only feature. On Windows the TUI's
    // cooperative skip still works; killing the child needs a different path
    // (Job Objects) that's out of scope for the .1 patch.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_starts_uncancelled() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
    }

    #[test]
    fn soft_cancel_flips_flag_without_signaling() {
        let t = CancellationToken::new();
        t.register_child(42);
        t.cancel(CancelLevel::Soft);
        assert!(t.is_cancelled());
        assert_eq!(t.registered_children(), vec![42]);
    }

    #[test]
    fn hard_cancel_flips_flag_and_attempts_signal() {
        let t = CancellationToken::new();
        // PID 0 is a no-op in send_terminate so this exercises the path
        // without actually killing anything.
        t.register_child(0);
        t.cancel(CancelLevel::Hard);
        assert!(t.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let t = CancellationToken::new();
        t.cancel(CancelLevel::Soft);
        t.cancel(CancelLevel::Hard);
        t.cancel(CancelLevel::Soft);
        assert!(t.is_cancelled());
    }

    #[test]
    fn child_registration_roundtrips() {
        let t = CancellationToken::new();
        t.register_child(101);
        t.register_child(202);
        assert_eq!(t.registered_children(), vec![101, 202]);
        t.unregister_child(101);
        assert_eq!(t.registered_children(), vec![202]);
        t.unregister_child(999); // unknown pid is a no-op
        assert_eq!(t.registered_children(), vec![202]);
    }

    #[test]
    fn token_is_cheap_to_clone() {
        let a = CancellationToken::new();
        let b = a.clone();
        a.cancel(CancelLevel::Soft);
        assert!(b.is_cancelled());
    }
}
