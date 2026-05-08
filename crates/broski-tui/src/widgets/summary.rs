//! One-line summary: counts + run state + remaining ETA.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::state::TuiState;
use crate::theme::Palette;

pub struct SummaryWidget<'s> {
    state: &'s TuiState,
    palette: &'s Palette,
}

impl<'s> SummaryWidget<'s> {
    pub fn new(state: &'s TuiState, palette: &'s Palette) -> Self {
        Self { state, palette }
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

        let (status_text, status_color) = if self.state.run_finished {
            if failed > 0 {
                ("FAILED", self.palette.failed)
            } else {
                ("DONE", self.palette.done)
            }
        } else if running > 0 {
            ("RUNNING", self.palette.running)
        } else {
            ("IDLE", self.palette.help)
        };

        let mut spans = vec![
            Span::styled(status_text, Style::default().fg(status_color)),
            Span::raw("  "),
            Span::styled(
                format!("{}/{} done", done + cached, total),
                Style::default().fg(self.palette.fg),
            ),
            Span::raw("   "),
            Span::styled(format!("running:{running}"), Style::default().fg(self.palette.running)),
            Span::raw("  "),
            Span::styled(format!("cached:{cached}"), Style::default().fg(self.palette.cached)),
            Span::raw("  "),
            Span::styled(format!("failed:{failed}"), Style::default().fg(self.palette.failed)),
            Span::raw("  "),
            Span::styled(format!("queued:{pending}"), Style::default().fg(self.palette.queued)),
        ];

        if !self.state.run_finished {
            let remaining_eta = self.state.remaining_eta();
            if !remaining_eta.is_zero() {
                spans.push(Span::raw("   "));
                spans.push(Span::styled(
                    format!("eta ~{}", format_duration(remaining_eta)),
                    Style::default().fg(self.palette.eta),
                ));
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.palette.accent));
        Paragraph::new(Line::from(spans)).block(block).render(area, buf);
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f32();
    if secs < 1.0 {
        format!("{}ms", d.as_millis())
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let m = (secs / 60.0).floor() as u64;
        let s = secs - (m * 60) as f32;
        format!("{}m{:.0}s", m, s)
    }
}
