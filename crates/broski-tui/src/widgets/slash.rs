//! Slash-command suggestion list rendered in the launcher's right column
//! when the user is in [`LauncherMode::Slash`].
//!
//! Static command catalog comes from [`crate::launcher::SLASH_COMMANDS`];
//! filtering follows the launcher state's current input.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, StatefulWidget};

use crate::launcher::{LauncherState, SLASH_COMMANDS};
use crate::theme::Palette;

pub struct SlashPanel<'s> {
    state: &'s LauncherState,
    palette: &'s Palette,
}

impl<'s> SlashPanel<'s> {
    pub fn new(state: &'s LauncherState, palette: &'s Palette) -> Self {
        Self { state, palette }
    }
}

impl ratatui::widgets::Widget for SlashPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let filtered = self.state.filtered_slash_commands();
        let title = format!(" Commands ({}/{}) ", filtered.len(), SLASH_COMMANDS.len());
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.palette.accent));

        if filtered.is_empty() {
            let line = Line::from(Span::styled(
                "  (no command matches)",
                Style::default().fg(self.palette.help),
            ));
            ratatui::widgets::Paragraph::new(line).block(block).render(area, buf);
            return;
        }

        let items: Vec<ListItem> = filtered
            .iter()
            .map(|label| {
                let desc =
                    SLASH_COMMANDS.iter().find(|(l, _)| l == label).map(|(_, d)| *d).unwrap_or("");
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<22}", label),
                        Style::default().fg(self.palette.fg).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(desc.to_string(), Style::default().fg(self.palette.help)),
                ]))
            })
            .collect();

        let mut list_state = ListState::default();
        list_state.select(self.state.selected);
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(self.palette.accent).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
        StatefulWidget::render(list, area, buf, &mut list_state);
    }
}
