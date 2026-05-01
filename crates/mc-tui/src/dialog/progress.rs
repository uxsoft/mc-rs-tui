use mc_config::ColorScheme;
use mc_jobs::{JobHandle, JobOutcome, Progress};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph};
use ratatui::Frame;

use super::centered_rect;
use crate::theme::rtc;

pub struct ProgressDialog {
    pub handle: JobHandle,
    pub description: String,
    pub status: String,
    pub progress: Progress,
    pub finished: Option<JobOutcome>,
}

impl ProgressDialog {
    #[must_use]
    pub fn new(handle: JobHandle, description: String) -> Self {
        Self {
            handle,
            description,
            status: String::new(),
            progress: Progress::default(),
            finished: None,
        }
    }

    pub fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(72, 9, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default().fg(rtc(scheme.dialog_fg)).bg(rtc(scheme.dialog_bg));

        let title = match &self.finished {
            None => format!(" {} ", self.description),
            Some(JobOutcome::Success) => format!(" {} (done) ", self.description),
            Some(JobOutcome::Cancelled) => format!(" {} (cancelled) ", self.description),
            Some(JobOutcome::Failed(e)) => format!(" {} (failed: {e}) ", self.description),
        };
        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default().fg(rtc(scheme.dialog_title_fg)).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(rtc(scheme.dialog_border)).bg(rtc(scheme.dialog_bg)))
            .style(dlg);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(1), // status
                ratatui::layout::Constraint::Length(1), // items gauge
                ratatui::layout::Constraint::Length(1), // bytes gauge
                ratatui::layout::Constraint::Length(1), // spacer
                ratatui::layout::Constraint::Length(1), // hint
            ])
            .split(inner);

        let status_line = Paragraph::new(Line::from(vec![
            Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(self.status.clone()),
        ])).style(dlg);
        f.render_widget(status_line, layout[0]);

        let items_pct = pct(self.progress.items_done, self.progress.items_total);
        let items_label = format!(
            "items {}/{}",
            self.progress.items_done, self.progress.items_total
        );
        let items_gauge = Gauge::default()
            .ratio(items_pct)
            .label(items_label)
            .gauge_style(Style::default().fg(rtc(scheme.dialog_focus_fg)).bg(rtc(scheme.dialog_focus_bg)));
        f.render_widget(items_gauge, layout[1]);

        let bytes_pct = pct(self.progress.bytes_done, self.progress.bytes_total);
        let bytes_label = format!(
            "bytes {} / {}",
            human(self.progress.bytes_done),
            human(self.progress.bytes_total),
        );
        let bytes_gauge = Gauge::default()
            .ratio(bytes_pct)
            .label(bytes_label)
            .gauge_style(Style::default().fg(rtc(scheme.diff_add_fg)).bg(rtc(scheme.diff_add_bg)));
        f.render_widget(bytes_gauge, layout[2]);

        let hint = if self.finished.is_some() {
            "Enter / Esc: close"
        } else {
            "Esc: cancel"
        };
        f.render_widget(
            Paragraph::new(hint).style(Style::default().fg(rtc(scheme.panel_dim_fg)).bg(rtc(scheme.dialog_bg))),
            layout[4],
        );
    }
}

fn pct(done: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let v = done as f64 / total as f64;
    v.clamp(0.0, 1.0)
}

fn human(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut size = n as f64;
    let mut idx = 0;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{n}")
    } else {
        format!("{size:.1}{}", UNITS[idx])
    }
}
