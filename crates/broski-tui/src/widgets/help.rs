//! Static keybind hint footer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::theme::Palette;

pub struct HelpFooter<'s> {
    palette: &'s Palette,
}

impl<'s> HelpFooter<'s> {
    pub fn new(palette: &'s Palette) -> Self {
        Self { palette }
    }
}

impl Widget for HelpFooter<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = Line::from(Span::styled(
            " q quit · ↑/↓ select · c clear logs · r redraw ",
            Style::default().fg(self.palette.help),
        ));
        Paragraph::new(line).render(area, buf);
    }
}
