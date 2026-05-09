//! Stats panel rendered in the launcher's right column when the user is
//! filtering tasks (Filter mode). Four stacked cards:
//! - Workspace (path, version, git rev, theme resolution)
//! - Cache (object count, total MB)
//! - Session (run counts, total time)
//! - Highlighted task detail (description, last run, glob counts)

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::launcher::{LauncherCtx, LauncherState, RunOutcome};
use crate::theme::Palette;

pub struct StatsPanel<'s> {
    state: &'s LauncherState,
    ctx: &'s LauncherCtx,
    palette: &'s Palette,
}

impl<'s> StatsPanel<'s> {
    pub fn new(state: &'s LauncherState, ctx: &'s LauncherCtx, palette: &'s Palette) -> Self {
        Self { state, ctx, palette }
    }
}

impl Widget for StatsPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let highlight_task = self.state.highlighted_task().map(str::to_string);
        let show_task_card = highlight_task.is_some();

        // Card heights: workspace=4, cache=3, session=3, task=4 (when shown),
        // recent runs takes the remainder.
        let constraints: Vec<Constraint> = if show_task_card {
            vec![
                Constraint::Length(4),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Min(3),
            ]
        } else {
            vec![
                Constraint::Length(4),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(3),
            ]
        };
        let cells =
            Layout::default().direction(Direction::Vertical).constraints(constraints).split(area);

        render_workspace(self.ctx, self.palette, cells[0], buf);
        render_cache(self.ctx, self.palette, cells[1], buf);
        render_session(self.state, self.palette, cells[2], buf);
        if show_task_card {
            render_task_detail(highlight_task.as_deref(), self.ctx, self.palette, cells[3], buf);
            render_recent_runs(self.state, self.palette, cells[4], buf);
        } else {
            render_recent_runs(self.state, self.palette, cells[3], buf);
        }
    }
}

fn render_workspace(ctx: &LauncherCtx, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let theme_label = match &ctx.theme_requested_name {
        Some(req) if req != &ctx.theme_resolved_name => {
            format!("{} → {}", req, ctx.theme_resolved_name)
        }
        _ => ctx.theme_resolved_name.clone(),
    };
    let mut version_line = format!("broski {}", ctx.version);
    if let Some(rev) = &ctx.git_rev {
        version_line.push_str(&format!(" · git@{}", rev));
    }
    let lines = vec![
        Line::from(Span::styled(ctx.workspace_display.clone(), Style::default().fg(palette.fg))),
        Line::from(Span::styled(version_line, Style::default().fg(palette.help))),
        Line::from(vec![
            Span::styled("theme: ", Style::default().fg(palette.help)),
            Span::styled(theme_label, Style::default().fg(palette.eta)),
        ]),
    ];
    Paragraph::new(lines).block(card_block(" Workspace ", palette)).render(area, buf);
}

fn render_cache(ctx: &LauncherCtx, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let line = Line::from(vec![
        Span::styled(
            format!(
                " {} object{}",
                ctx.cache_stats.object_count,
                if ctx.cache_stats.object_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(palette.fg),
        ),
        Span::styled(" · ", Style::default().fg(palette.help)),
        Span::styled(
            format_bytes(ctx.cache_stats.total_bytes),
            Style::default().fg(palette.cached),
        ),
    ]);
    Paragraph::new(line).block(card_block(" Cache ", palette)).render(area, buf);
}

fn render_session(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let stats = &state.stats;
    let line = Line::from(vec![
        Span::styled(format!(" {} run", stats.total_runs), Style::default().fg(palette.fg)),
        Span::styled(
            if stats.total_runs == 1 { " " } else { "s " },
            Style::default().fg(palette.fg),
        ),
        Span::styled(format!("{}", stats.successes), Style::default().fg(palette.done)),
        Span::styled("✓ ", Style::default().fg(palette.done)),
        Span::styled(format!("{}", stats.failures), Style::default().fg(palette.failed)),
        Span::styled("✗ ", Style::default().fg(palette.failed)),
        Span::styled(format!("{}", stats.cancellations), Style::default().fg(palette.queued)),
        Span::styled("⊘  · ", Style::default().fg(palette.queued)),
        Span::styled(format_dur(stats.total_duration), Style::default().fg(palette.eta)),
    ]);
    Paragraph::new(line).block(card_block(" Session ", palette)).render(area, buf);
}

fn render_task_detail(
    task: Option<&str>,
    ctx: &LauncherCtx,
    palette: &Palette,
    area: Rect,
    buf: &mut Buffer,
) {
    let Some(task) = task else { return };
    let title = format!(" Task: {} ", task);
    let block = card_block(&title, palette);
    let meta = ctx.meta_for(task);
    let mut lines = Vec::new();
    if let Some(meta) = meta {
        if let Some(desc) = &meta.description {
            lines.push(Line::from(Span::styled(desc.clone(), Style::default().fg(palette.fg))));
        } else {
            lines.push(Line::from(Span::styled(
                "(no description)",
                Style::default().fg(palette.help).add_modifier(Modifier::ITALIC),
            )));
        }
        let mut detail = format!(
            " deps: {}  · in: {}  · out: {}",
            if meta.deps.is_empty() { "—".to_string() } else { meta.deps.join(", ") },
            meta.inputs_count,
            meta.outputs_count,
        );
        if let (Some(ago), Some(ms)) = (meta.last_run_ago_secs, meta.last_duration_ms) {
            detail.push_str(&format!("  · last: {} ({} ago)", format_ms(ms), format_ago(ago)));
        }
        lines.push(Line::from(Span::styled(detail, Style::default().fg(palette.help))));
    } else {
        lines.push(Line::from(Span::styled(
            "(missing metadata — try /refresh)",
            Style::default().fg(palette.help),
        )));
    }
    Paragraph::new(lines).block(block).render(area, buf);
}

fn render_recent_runs(state: &LauncherState, palette: &Palette, area: Rect, buf: &mut Buffer) {
    let block = card_block(" Recent runs ", palette);
    if state.history.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "  (no runs yet this session)",
            Style::default().fg(palette.help),
        )))
        .block(block)
        .render(area, buf);
        return;
    }
    let lines: Vec<Line> = state
        .history
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|entry| {
            let (icon, color) = match entry.outcome {
                RunOutcome::Success => ("✓", palette.done),
                RunOutcome::Failed => ("✗", palette.failed),
                RunOutcome::Cancelled => ("⊘", palette.queued),
            };
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color)),
                Span::styled(format!("{:<20}", entry.target), Style::default().fg(palette.fg)),
                Span::styled(format_dur(entry.duration), Style::default().fg(palette.help)),
            ])
        })
        .collect();
    Paragraph::new(lines).block(block).render(area, buf);
}

fn card_block<'s>(title: &'s str, palette: &Palette) -> Block<'s> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent))
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes < KB {
        format!("{} B", bytes)
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    }
}

fn format_dur(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1_000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

fn format_ms(ms: u64) -> String {
    format_dur(Duration::from_millis(ms))
}

fn format_ago(secs: i64) -> String {
    let s = secs.max(0) as u64;
    if s < 60 {
        format!("{}s", s)
    } else if s < 3_600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3_600)
    } else {
        format!("{}d", s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_picks_human_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GB");
    }

    #[test]
    fn format_ago_buckets_into_units() {
        assert_eq!(format_ago(-5), "0s");
        assert_eq!(format_ago(45), "45s");
        assert_eq!(format_ago(120), "2m");
        assert_eq!(format_ago(7_200), "2h");
        assert_eq!(format_ago(2 * 86_400), "2d");
    }
}
