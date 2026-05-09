//! Keyboard mapping. Pure: a `KeyEvent` becomes an `Action`; the app loop
//! decides what to do with it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Plain quit. Exits without cancelling the executor.
    Quit,
    /// User pressed Ctrl-C. The app loop interprets this as a soft cancel
    /// on first press and a hard cancel + quit on a second press within
    /// the cancellation window.
    Interrupt,
    SelectNext,
    SelectPrev,
    SelectFirst,
    SelectLast,
    ClearLogs,
    Redraw,
    /// Scroll the log pane up by N lines (toward older output).
    LogScrollUp(usize),
    /// Scroll the log pane down by N lines (toward newer output).
    LogScrollDown(usize),
    /// Jump the log pane to the very top of the buffered output.
    LogScrollHome,
    /// Jump the log pane back to the tail and resume follow-tail.
    LogScrollEnd,
    /// Force-rerun only the selected task (deps still use cache).
    ForceRerunSelected,
    /// Force-rerun the entire original target (all tasks bypass cache).
    ForceRerunAll,
    Ignore,
}

/// Number of lines a single PageUp/PageDown moves through the log pane.
const PAGE_SCROLL: usize = 10;

pub fn map_key(event: KeyEvent) -> Action {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let shift = event.modifiers.contains(KeyModifiers::SHIFT);
    match event.code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if ctrl => Action::Interrupt,
        KeyCode::Char('c') => Action::ClearLogs,
        KeyCode::Char('r') | KeyCode::Char('R') => Action::Redraw,
        KeyCode::PageUp => Action::LogScrollUp(PAGE_SCROLL),
        KeyCode::PageDown => Action::LogScrollDown(PAGE_SCROLL),
        // Shift+Up/Down scroll the LOGS by 1 line. Plain Up/Down still
        // navigate the task list so the existing UX is unchanged.
        KeyCode::Up if shift => Action::LogScrollUp(1),
        KeyCode::Down if shift => Action::LogScrollDown(1),
        KeyCode::Home if shift => Action::LogScrollHome,
        KeyCode::End if shift => Action::LogScrollEnd,
        KeyCode::Down | KeyCode::Char('j') => Action::SelectNext,
        KeyCode::Up | KeyCode::Char('k') => Action::SelectPrev,
        KeyCode::Home | KeyCode::Char('g') => Action::SelectFirst,
        KeyCode::End | KeyCode::Char('G') => Action::SelectLast,
        KeyCode::Char('x') => Action::ForceRerunSelected,
        KeyCode::Char('X') => Action::ForceRerunAll,
        _ => Action::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn key_with_mods(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn quit_keys() {
        assert_eq!(map_key(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(map_key(key(KeyCode::Char('Q'))), Action::Quit);
        assert_eq!(map_key(key(KeyCode::Esc)), Action::Quit);
    }

    #[test]
    fn ctrl_c_is_interrupt_not_quit() {
        // The app loop interprets Interrupt as soft cancel on first press,
        // hard cancel on second. Plain `q` still terminates without
        // touching the executor.
        assert_eq!(
            map_key(key_with_mods(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Interrupt
        );
    }

    #[test]
    fn clear_vs_quit_disambiguates_by_ctrl() {
        // Lowercase 'c' alone is "clear logs", not quit.
        assert_eq!(map_key(key(KeyCode::Char('c'))), Action::ClearLogs);
    }

    #[test]
    fn navigation_keys() {
        assert_eq!(map_key(key(KeyCode::Down)), Action::SelectNext);
        assert_eq!(map_key(key(KeyCode::Up)), Action::SelectPrev);
        assert_eq!(map_key(key(KeyCode::Char('j'))), Action::SelectNext);
        assert_eq!(map_key(key(KeyCode::Char('k'))), Action::SelectPrev);
        assert_eq!(map_key(key(KeyCode::Home)), Action::SelectFirst);
        assert_eq!(map_key(key(KeyCode::End)), Action::SelectLast);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        assert_eq!(map_key(key(KeyCode::F(1))), Action::Ignore);
        assert_eq!(map_key(key(KeyCode::Char('z'))), Action::Ignore);
    }

    #[test]
    fn x_maps_to_force_rerun_selected() {
        assert_eq!(map_key(key(KeyCode::Char('x'))), Action::ForceRerunSelected);
    }

    #[test]
    fn shift_x_maps_to_force_rerun_all() {
        assert_eq!(map_key(key(KeyCode::Char('X'))), Action::ForceRerunAll);
    }

    #[test]
    fn pageup_pagedown_scroll_logs_by_page() {
        match map_key(key(KeyCode::PageUp)) {
            Action::LogScrollUp(n) => assert!(n > 1),
            other => panic!("PageUp should map to LogScrollUp(>1), got {:?}", other),
        }
        match map_key(key(KeyCode::PageDown)) {
            Action::LogScrollDown(n) => assert!(n > 1),
            other => panic!("PageDown should map to LogScrollDown(>1), got {:?}", other),
        }
    }

    #[test]
    fn shift_arrow_scrolls_logs_by_one_line() {
        assert_eq!(
            map_key(key_with_mods(KeyCode::Up, KeyModifiers::SHIFT)),
            Action::LogScrollUp(1)
        );
        assert_eq!(
            map_key(key_with_mods(KeyCode::Down, KeyModifiers::SHIFT)),
            Action::LogScrollDown(1)
        );
    }

    #[test]
    fn shift_home_end_jump_to_top_and_tail() {
        assert_eq!(
            map_key(key_with_mods(KeyCode::Home, KeyModifiers::SHIFT)),
            Action::LogScrollHome
        );
        assert_eq!(map_key(key_with_mods(KeyCode::End, KeyModifiers::SHIFT)), Action::LogScrollEnd);
    }

    #[test]
    fn unshifted_arrows_still_navigate_tasks() {
        // Make sure adding shift-arrow handling didn't break the
        // status-quo task-cursor behavior on plain arrows.
        assert_eq!(map_key(key(KeyCode::Up)), Action::SelectPrev);
        assert_eq!(map_key(key(KeyCode::Down)), Action::SelectNext);
        assert_eq!(map_key(key(KeyCode::Home)), Action::SelectFirst);
        assert_eq!(map_key(key(KeyCode::End)), Action::SelectLast);
    }
}
