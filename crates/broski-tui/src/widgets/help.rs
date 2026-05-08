//! Static keybind hint footer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::theme;

pub struct HelpFooter;

impl Widget for HelpFooter {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = Line::from(Span::styled(
            " q quit · ↑/↓ select · c clear logs · r redraw ",
            Style::default().fg(theme::HELP),
        ));
        Paragraph::new(line).render(area, buf);
    }
}
