//! DAG widget: layered task list with status icons.
//!
//! Rendering is stateless and reads from [`TuiState`] — every redraw rebuilds
//! the lines. This keeps the redraw logic dirt-simple at the cost of a small
//! amount of allocation per frame; with task counts in the dozens that's fine.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget, Widget};

use crate::state::{TaskState, TuiState};
use crate::theme;

pub struct DagWidget<'s> {
    state: &'s TuiState,
}

impl<'s> DagWidget<'s> {
    pub fn new(state: &'s TuiState) -> Self {
        Self { state }
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
            .border_style(Style::default().fg(theme::ACCENT));

        let mut items: Vec<ListItem> = Vec::new();
        for (layer_idx, layer) in self.state.layers.iter().enumerate() {
            for name in layer {
                let info = self.state.tasks.get(name);
                let (icon, color) = task_icon(info.map(|i| i.state));
                let mut spans = vec![
                    Span::styled(format!("{icon} "), Style::default().fg(color)),
                    Span::styled(name.clone(), Style::default().fg(theme::FG)),
                ];
                if let Some(info) = info {
                    if !info.duration.is_zero() {
                        spans.push(Span::styled(
                            format!("  {:.1}s", info.duration.as_secs_f32()),
                            Style::default().fg(theme::HELP),
                        ));
                    }
                }
                items.push(ListItem::new(Line::from(spans)));
            }
            if layer_idx + 1 < self.state.layers.len() {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  ↓",
                    Style::default().fg(theme::HELP),
                ))));
            }
        }

        if items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "  (waiting for run start…)",
                Style::default().fg(theme::HELP),
            ))));
        }

        let mut list_state = ListState::default();
        list_state.select(self.state.selected.map(|sel| visible_index(self.state, sel)));

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");

        StatefulWidget::render(list, area, buf, &mut list_state);
    }
}

fn task_icon(state: Option<TaskState>) -> (&'static str, ratatui::style::Color) {
    match state {
        Some(TaskState::Running) => ("●", theme::RUNNING),
        Some(TaskState::Done) => ("✓", theme::DONE),
        Some(TaskState::Cached) => ("⊙", theme::CACHED),
        Some(TaskState::Failed) => ("✗", theme::FAILED),
        Some(TaskState::DryRun) => ("·", theme::QUEUED),
        Some(TaskState::Skipped) => ("⊘", theme::QUEUED),
        Some(TaskState::Queued) | None => ("○", theme::QUEUED),
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
            visible += 1; // the divider row
        }
    }
    visible.saturating_sub(1)
}
