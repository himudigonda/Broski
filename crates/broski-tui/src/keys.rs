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
    Ignore,
}

pub fn map_key(event: KeyEvent) -> Action {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    match event.code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if ctrl => Action::Interrupt,
        KeyCode::Char('c') => Action::ClearLogs,
        KeyCode::Char('r') | KeyCode::Char('R') => Action::Redraw,
        KeyCode::Down | KeyCode::Char('j') => Action::SelectNext,
        KeyCode::Up | KeyCode::Char('k') => Action::SelectPrev,
        KeyCode::Home | KeyCode::Char('g') => Action::SelectFirst,
        KeyCode::End | KeyCode::Char('G') => Action::SelectLast,
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
        assert_eq!(map_key(key(KeyCode::Char('x'))), Action::Ignore);
        assert_eq!(map_key(key(KeyCode::F(1))), Action::Ignore);
    }
}
