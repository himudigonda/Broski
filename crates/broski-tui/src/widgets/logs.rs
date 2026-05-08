//! Log pane for the selected task. Stdout/stderr are color-tagged.

use broski_core::LogStream;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::state::{TaskInfo, TuiState};
use crate::theme;

pub struct LogsWidget<'s> {
    state: &'s TuiState,
}

impl<'s> LogsWidget<'s> {
    pub fn new(state: &'s TuiState) -> Self {
        Self { state }
    }
}

impl Widget for LogsWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let task = self.state.selected_task();
        let title = match task {
            Some(name) => format!(" Logs · {name} "),
            None => " Logs ".to_string(),
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ACCENT));

        let mut lines: Vec<Line> = Vec::new();
        if let Some(name) = task {
            if let Some(info) = self.state.tasks.get(name) {
                if info.logs.is_empty() {
                    lines.push(empty_message(info));
                } else {
                    let max_visible = area.height.saturating_sub(2) as usize;
                    let total = info.logs.len();
                    let start = total.saturating_sub(max_visible);
                    for record in info.logs.iter().skip(start) {
                        let color = match record.stream {
                            LogStream::Stdout => theme::STDOUT,
                            LogStream::Stderr => theme::STDERR,
                        };
                        lines.push(Line::from(Span::styled(
                            record.line.clone(),
                            Style::default().fg(color),
                        )));
                    }
                }
                if let Some(error) = info.error.as_ref() {
                    lines.push(Line::from(Span::styled(
                        format!("error: {error}"),
                        Style::default().fg(theme::FAILED),
                    )));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  (no task selected)",
                Style::default().fg(theme::HELP),
            )));
        }

        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).render(area, buf);
    }
}

fn empty_message(info: &TaskInfo) -> Line<'static> {
    let detail = match info.current_phase {
        Some(phase) => format!("  (no logs yet · current phase: {:?})", phase),
        None => "  (no logs)".to_string(),
    };
    Line::from(Span::styled(detail, Style::default().fg(theme::HELP)))
}
