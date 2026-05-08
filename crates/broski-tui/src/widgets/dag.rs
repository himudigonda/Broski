//! DAG widget: layered task list with status icons.
//!
//! Rendering is stateless and reads from [`TuiState`] — every redraw rebuilds
//! the lines. With task counts in the dozens that's fine.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget};

use crate::state::{TaskState, TuiState};
use crate::theme::Palette;

pub struct DagWidget<'s> {
    state: &'s TuiState,
    palette: &'s Palette,
}

impl<'s> DagWidget<'s> {
    pub fn new(state: &'s TuiState, palette: &'s Palette) -> Self {
        Self { state, palette }
    }
}

impl Widget for DagWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = match self.state.target.as_deref() {
            Some(t) => format!(" DAG · {t} "),
            None => " DAG ".to_string(),
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.palette.accent));

        let mut items: Vec<ListItem> = Vec::new();
        for (layer_idx, layer) in self.state.layers.iter().enumerate() {
            for name in layer {
                let info = self.state.tasks.get(name);
                let (icon, color) = task_icon(info.map(|i| i.state), self.palette);
                let mut spans = vec![
                    Span::styled(format!("{icon} "), Style::default().fg(color)),
                    Span::styled(name.clone(), Style::default().fg(self.palette.fg)),
                ];
                if let Some(info) = info {
                    if !info.duration.is_zero() {
                        spans.push(Span::styled(
                            format!("  {}", format_duration(info.duration)),
                            Style::default().fg(self.palette.help),
                        ));
                    }
                    if matches!(info.state, TaskState::Running | TaskState::Queued) {
                        if let Some(eta) = self.state.etas.get(name) {
                            spans.push(Span::styled(
                                format!("  ~{}", format_duration(*eta)),
                                Style::default().fg(self.palette.eta),
                            ));
                        }
                    }
                }
                items.push(ListItem::new(Line::from(spans)));
            }
            if layer_idx + 1 < self.state.layers.len() {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  ↓",
                    Style::default().fg(self.palette.help),
                ))));
            }
        }

        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "  (waiting for run start…)",
                Style::default().fg(self.palette.help),
            ))));
        }

        let mut list_state = ListState::default();
        list_state.select(self.state.selected.map(|sel| visible_index(self.state, sel)));

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default().fg(self.palette.accent).add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        StatefulWidget::render(list, area, buf, &mut list_state);
    }
}

fn task_icon(state: Option<TaskState>, palette: &Palette) -> (&'static str, Color) {
    match state {
        Some(TaskState::Running) => ("●", palette.running),
        Some(TaskState::Done) => ("✓", palette.done),
        Some(TaskState::Cached) => ("⊙", palette.cached),
        Some(TaskState::Failed) => ("✗", palette.failed),
        Some(TaskState::DryRun) => ("·", palette.queued),
        Some(TaskState::Skipped) => ("⊘", palette.queued),
        Some(TaskState::Queued) | None => ("○", palette.queued),
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

/// Translate a flat `task_order` index into the visible list index, accounting
/// for the layer-divider rows we inject between layers.
fn visible_index(state: &TuiState, flat_idx: usize) -> usize {
    let mut count = 0usize;
    let mut visible = 0usize;
    for (layer_idx, layer) in state.layers.iter().enumerate() {
        for _ in layer {
            if count == flat_idx {
                return visible;
            }
            count += 1;
            visible += 1;
        }
        if layer_idx + 1 < state.layers.len() {
            visible += 1;
        }
    }
    visible.saturating_sub(1)
}
