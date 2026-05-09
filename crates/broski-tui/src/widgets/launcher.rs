//! Launcher screen: filterable task list, free-form input box, slash
//! command palette, plus a side-by-side stats / suggestion column.
//!
//! Rendering is mode-aware: in [`LauncherMode::Filter`] the right column
//! shows the stats panel; in [`LauncherMode::Slash`] it shows the slash
//! command suggestion list. The widget is a thin view over
//! [`LauncherState`] + [`LauncherCtx`]; all logic stays in those types.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, StatefulWidget, Widget,
};
use ratatui::Frame;

use crate::launcher::{LauncherCtx, LauncherMode, LauncherState};
use crate::theme::Palette;
use crate::widgets::slash::SlashPanel;
use crate::widgets::stats::StatsPanel;

/// Minimum terminal width below which we collapse the right column and
/// show only the task list / input box / help. Below this, the stats and
/// slash panels are noise.
const NARROW_TERMINAL_THRESHOLD: u16 = 80;

pub struct LauncherWidget<'s> {
    state: &'s LauncherState,
    ctx: &'s LauncherCtx,
    palette: &'s Palette,
}

impl<'s> LauncherWidget<'s> {
    pub fn new(state: &'s LauncherState, ctx: &'s LauncherCtx, palette: &'s Palette) -> Self {
        Self { state, ctx, palette }
    }

    /// Render the launcher into `area` and place the terminal cursor at
    /// the end of the input box. We can't do this from a plain `Widget`
    /// impl because cursor positioning needs `&mut Frame`; the app loop
    /// calls this directly.
    pub fn render_into(&self, frame: &mut Frame, area: Rect) {
        let buf = frame.buffer_mut();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header
                Constraint::Min(8),    // body (split horizontally)
                Constraint::Length(1), // status banner
                Constraint::Length(3), // input box
                Constraint::Length(1), // help
            ])
            .split(area);

        render_header(self.ctx, self.palette, layout[0], buf);
        let body = layout[1];
        if body.width >= NARROW_TERMINAL_THRESHOLD {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(body);
            render_tasks(self.state, self.palette, cols[0], buf);
            match self.state.mode {
                LauncherMode::Filter => {
                    StatsPanel::new(self.state, self.ctx, self.palette).render(cols[1], buf);
                }
                LauncherMode::Slash => {
                    SlashPanel::new(self.state, self.palette).render(cols[1], buf);
                }
            }
        } else {
            // Narrow terminal: hide right column, show only tasks (plus
            // the slash panel inline if the user is composing a command).
            match self.state.mode {
                LauncherMode::Filter => {
                    render_tasks(self.state, self.palette, body, buf);
                }
                LauncherMode::Slash => {
                    SlashPanel::new(self.state, self.palette).render(body, buf);
                }
            }
        }

        render_status(self.state, self.palette, layout[2], buf);
        let cursor = render_input(self.state, self.palette, layout[3], buf);
        render_help(self.state.mode, self.palette, layout[4], buf);

        if let Some((x, y)) = cursor {
            frame.set_cursor_position((x, y));
        }
    }
}

fn render_header(ctx: &LauncherCtx, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" broski tui ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent));
    let theme_label = match &ctx.theme_requested_name {
        Some(req) if req != &ctx.theme_resolved_name => {
            format!("{} → {}", req, ctx.theme_resolved_name)
        }
        _ => ctx.theme_resolved_name.clone(),
    };
    let lines = vec![Line::from(vec![
        Span::styled("workspace: ", Style::default().fg(palette.help)),
        Span::styled(ctx.workspace_display.clone(), Style::default().fg(palette.fg)),
        Span::styled("   theme: ", Style::default().fg(palette.help)),
        Span::styled(theme_label, Style::default().fg(palette.eta)),
        Span::styled(format!("   broski {}", ctx.version), Style::default().fg(palette.help)),
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
    if state.mode == LauncherMode::Filter && !filtered.is_empty() {
        list_state.select(state.selected);
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    StatefulWidget::render(list, area, buf, &mut list_state);
}

fn render_status(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let line = match &state.status {
        Some(msg) => Line::from(Span::styled(format!(" {msg}"), Style::default().fg(palette.eta))),
        None => Line::from(Span::raw("")),
    };
    Paragraph::new(line).render(area, buf);
}

/// Render the input box and return the absolute (x, y) cell position
/// where the terminal cursor should be drawn at end-of-input. The caller
/// passes that to `Frame::set_cursor_position` so the user gets a real
/// blinker instead of a static glyph.
fn render_input(
    state: &LauncherState,
    palette: &Palette,
    area: Rect,
    buf: &mut Buffer,
) -> Option<(u16, u16)> {
    let prompt_glyph = match state.mode {
        LauncherMode::Slash => "⟫ ",
        LauncherMode::Filter => "❯ ",
    };
    let block =
        Block::default().borders(Borders::ALL).border_style(Style::default().fg(palette.accent));
    let line = Line::from(vec![
        Span::styled(
            format!(" {}", prompt_glyph),
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(state.input.clone(), Style::default().fg(palette.fg)),
    ]);
    Paragraph::new(line).block(block).render(area, buf);

    // Cursor coords inside the input box: 1 cell border + 1 leading space
    // + 2 cells for prompt glyph + input length. Clamp to the right
    // border so the cursor never spills outside the box on long input.
    let inside_x =
        1u16 + 1u16 + prompt_glyph.chars().count() as u16 + state.input.chars().count() as u16;
    let max_inside_x = area.width.saturating_sub(2); // -2 for the borders
    let x = area.x + inside_x.min(max_inside_x.max(1));
    let y = area.y + 1; // +1 to skip the top border
    Some((x, y))
}

fn render_help(mode: LauncherMode, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let text = match mode {
        LauncherMode::Filter => {
            " ↑/↓ select · Tab complete · Enter run · / commands · Esc clear · q exit "
        }
        LauncherMode::Slash => {
            " ↑/↓ select · Tab complete · Enter run command · Esc cancel · type to filter "
        }
    };
    let line = Line::from(Span::styled(text, Style::default().fg(palette.help)));
    Paragraph::new(line).render(area, buf);
}
