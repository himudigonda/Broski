//! Log pane for the selected task. Stdout/stderr are color-tagged.

use broski_core::LogStream;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::state::{TaskInfo, TuiState};
use crate::theme::Palette;

pub struct LogsWidget<'s> {
    state: &'s TuiState,
    palette: &'s Palette,
}

impl<'s> LogsWidget<'s> {
    pub fn new(state: &'s TuiState, palette: &'s Palette) -> Self {
        Self { state, palette }
    }
}

impl Widget for LogsWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let task = self.state.selected_task();
        let title = match task {
            Some(name) => match self.state.tasks.get(name) {
                Some(info) if info.scrollback > 0 => {
                    format!(" Logs · {name} · ↑{} ", info.scrollback)
                }
                _ => format!(" Logs · {name} "),
            },
            None => " Logs ".to_string(),
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.palette.accent));

        let mut lines: Vec<Line> = Vec::new();
        if let Some(name) = task {
            if let Some(info) = self.state.tasks.get(name) {
                if info.logs.is_empty() {
                    lines.push(empty_message(info, self.palette));
                } else {
                    let max_visible = area.height.saturating_sub(2) as usize;
                    let total = info.logs.len();
                    // The visible window's right edge is `total - scrollback`
                    // (exclusive). Subtract `max_visible` to find the start.
                    let end = total.saturating_sub(info.scrollback);
                    let start = end.saturating_sub(max_visible);
                    for record in info.logs.iter().skip(start).take(end - start) {
                        let color = match record.stream {
                            LogStream::Stdout => self.palette.stdout,
                            LogStream::Stderr => self.palette.stderr,
                        };
                        lines.push(Line::from(Span::styled(
                            record.line.clone(),
                            Style::default().fg(color),
                        )));
                    }
                }
                // Only print the trailing error when the user is actually
                // looking at the tail; otherwise it would awkwardly inject
                // itself between the lines they're reviewing.
                if info.scrollback == 0 {
                    if let Some(error) = info.error.as_ref() {
                        lines.push(Line::from(Span::styled(
                            format!("error: {error}"),
                            Style::default().fg(self.palette.failed),
                        )));
                    }
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  (no task selected)",
                Style::default().fg(self.palette.help),
            )));
        }

        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).render(area, buf);
    }
}

fn empty_message(info: &TaskInfo, palette: &Palette) -> Line<'static> {
    let detail = match info.current_phase {
        Some(phase) => format!("  (no logs yet · current phase: {:?})", phase),
        None => "  (no logs)".to_string(),
    };
    Line::from(Span::styled(detail, Style::default().fg(palette.help)))
}
