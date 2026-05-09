//! Launcher screen widget: task picker + free-form input box.
//!
//! Rendered when `broski tui` is invoked without a task argument. The
//! launcher is a thin view over [`LauncherState`]; all logic lives there
//! and we just paint what it tells us.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget,
};

use crate::launcher::{LauncherState, RunOutcome};
use crate::theme::Palette;

pub struct LauncherWidget<'s> {
    state: &'s LauncherState,
    palette: &'s Palette,
    workspace: &'s str,
    theme_name: &'s str,
}

impl<'s> LauncherWidget<'s> {
    pub fn new(
        state: &'s LauncherState,
        palette: &'s Palette,
        workspace: &'s str,
        theme_name: &'s str,
    ) -> Self {
        Self { state, palette, workspace, theme_name }
    }
}

impl Widget for LauncherWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Vertical split: header / tasks / history / status / input / help
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header
                Constraint::Min(6),    // tasks
                Constraint::Length(7), // history
                Constraint::Length(1), // status banner
                Constraint::Length(3), // input box
                Constraint::Length(1), // help
            ])
            .split(area);

        render_header(self.workspace, self.theme_name, self.palette, layout[0], buf);
        render_tasks(self.state, self.palette, layout[1], buf);
        render_history(self.state, self.palette, layout[2], buf);
        render_status(self.state, self.palette, layout[3], buf);
        render_input(self.state, self.palette, layout[4], buf);
        render_help(self.palette, layout[5], buf);
    }
}

fn render_header(
    workspace: &str,
    theme_name: &str,
    palette: &Palette,
    area: Rect,
    buf: &mut Buffer,
) {
    let block = Block::default()
        .title(" broski tui ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let lines = vec![Line::from(vec![
        Span::styled("workspace: ", Style::default().fg(palette.help)),
        Span::styled(workspace.to_string(), Style::default().fg(palette.fg)),
        Span::styled("   theme: ", Style::default().fg(palette.help)),
        Span::styled(theme_name.to_string(), Style::default().fg(palette.fg)),
    ])];
    Paragraph::new(lines).block(block).render(area, buf);
}

fn render_tasks(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let filtered = state.filtered_tasks();
    let title = format!(" Tasks ({} of {}) ", filtered.len(), state.all_tasks.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));

    let items: Vec<ListItem> = if filtered.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (no tasks match this filter)",
            Style::default().fg(palette.help),
        )))]
    } else {
        filtered
            .iter()
            .map(|name| {
                ListItem::new(Line::from(Span::styled(
                    (*name).to_string(),
                    Style::default().fg(palette.fg),
                )))
            })
            .collect()
    };

    let mut list_state = ListState::default();
    if !filtered.is_empty() {
        list_state.select(state.selected);
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    StatefulWidget::render(list, area, buf, &mut list_state);
}

fn render_history(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" Recent runs ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));

    if state.history.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "  (no runs yet this session)",
            Style::default().fg(palette.help),
        )))
        .block(block)
        .render(area, buf);
        return;
    }

    let lines: Vec<Line> = state
        .history
        .iter()
        .take(5)
        .map(|entry| {
            let (icon, color) = match entry.outcome {
                RunOutcome::Success => ("✓", palette.done),
                RunOutcome::Failed => ("✗", palette.failed),
                RunOutcome::Cancelled => ("⊘", palette.queued),
            };
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color)),
                Span::styled(format!("{:<24}", entry.target), Style::default().fg(palette.fg)),
                Span::styled(format_dur(entry.duration), Style::default().fg(palette.help)),
            ])
        })
        .collect();

    Paragraph::new(lines).block(block).render(area, buf);
}

fn render_status(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let line = match &state.status {
        Some(msg) => Line::from(Span::styled(format!(" {msg}"), Style::default().fg(palette.eta))),
        None => Line::from(Span::raw("")),
    };
    Paragraph::new(line).render(area, buf);
}

fn render_input(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let block =
        Block::default().borders(Borders::ALL).border_style(Style::default().fg(palette.accent));
    let line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::styled(state.input.clone(), Style::default().fg(palette.fg)),
        Span::styled("▍", Style::default().fg(palette.accent)),
    ]);
    Paragraph::new(line).block(block).render(area, buf);
}

fn render_help(palette: &Palette, area: Rect, buf: &mut Buffer) {
    let line = Line::from(Span::styled(
        " ↑/↓ select · Tab complete · Enter run · Esc clear · q / Ctrl-C quit ",
        Style::default().fg(palette.help),
    ));
    Paragraph::new(line).render(area, buf);
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
