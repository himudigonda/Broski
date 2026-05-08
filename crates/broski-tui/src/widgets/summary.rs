//! One-line summary: counts + run state.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::state::TuiState;
use crate::theme;

pub struct SummaryWidget<'s> {
    state: &'s TuiState,
}

impl<'s> SummaryWidget<'s> {
    pub fn new(state: &'s TuiState) -> Self {
        Self { state }
    }
}

impl Widget for SummaryWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let total = self.state.task_order.len() as u32;
        let running = self.state.running_count;
        let done = self.state.done_count;
        let cached = self.state.cached_count;
        let failed = self.state.failed_count;
        let pending = total
            .saturating_sub(running)
            .saturating_sub(done)
            .saturating_sub(cached)
            .saturating_sub(failed);

        let status = if self.state.run_finished {
            if failed > 0 {
                ("FAILED", theme::FAILED)
            } else {
                ("DONE", theme::DONE)
            }
        } else if running > 0 {
            ("RUNNING", theme::RUNNING)
        } else {
            ("IDLE", theme::HELP)
        };

        let line = Line::from(vec![
            Span::styled(status.0, Style::default().fg(status.1)),
            Span::raw("  "),
            Span::styled(format!("{}/{} done", done + cached, total), Style::default().fg(theme::FG)),
            Span::raw("   "),
            Span::styled(format!("running:{running}"), Style::default().fg(theme::RUNNING)),
            Span::raw("  "),
            Span::styled(format!("cached:{cached}"), Style::default().fg(theme::CACHED)),
            Span::raw("  "),
            Span::styled(format!("failed:{failed}"), Style::default().fg(theme::FAILED)),
            Span::raw("  "),
            Span::styled(format!("queued:{pending}"), Style::default().fg(theme::QUEUED)),
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ACCENT));
        Paragraph::new(line).block(block).render(area, buf);
    }
}
