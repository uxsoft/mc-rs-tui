//! Frame rendering: `App::render` (panels + buttonbar + modal overlay) and
//! the small free helpers it dispatches to.

use mc_config::ColorScheme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::dialog::Dialog;
use crate::panel::{PanelDecor, panel_body_rect, render_panel};
use crate::theme::rtc;

use super::{App, ButtonSegment, Modal};

impl App {
    pub fn render(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        self.layout.frame = area;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
        self.layout.menubar = chunks[0];
        self.layout.buttonbar = chunks[2];

        let menu_focused = matches!(self.modal, Modal::Menu);
        self.menubar
            .render_titles(f, chunks[0], &self.scheme, menu_focused);

        let left_pct = self.config.layout.left_pct.clamp(1, 99);
        let panels_dir = if self.config.layout.vertical {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };
        let panels = Layout::default()
            .direction(panels_dir)
            .constraints([
                Constraint::Percentage(left_pct as u16),
                Constraint::Percentage((100 - left_pct as u16).max(1)),
            ])
            .split(chunks[1]);
        self.layout.left_body = panel_body_rect(panels[0]);
        self.layout.right_body = panel_body_rect(panels[1]);

        let decor = PanelDecor {
            icons: self.config.options.icons,
            git_status: self.config.options.git_status,
        };
        render_panel(
            f,
            panels[0],
            &mut self.left,
            &self.highlight,
            &self.scheme,
            decor,
        );
        render_panel(
            f,
            panels[1],
            &mut self.right,
            &self.highlight,
            &self.scheme,
            decor,
        );
        if let Some(status) = self.current_status() {
            let p = Paragraph::new(Line::from(format!(" {status} "))).style(
                Style::default()
                    .fg(rtc(self.scheme.op_status_fg))
                    .bg(rtc(self.scheme.op_status_bg))
                    .add_modifier(ratatui::style::Modifier::BOLD),
            );
            f.render_widget(p, chunks[2]);
            self.layout.button_segments.clear();
        } else {
            self.layout.button_segments = render_buttonbar(f, chunks[2], &self.scheme);
        }

        let scheme = &self.scheme;
        match &mut self.modal {
            Modal::None | Modal::PrefixCtrlX | Modal::Menu => {}
            Modal::Mkdir(d) | Modal::Rename(d, _) => d.render(f, area, scheme),
            Modal::CopyMove { dlg, .. } => dlg.render(f, area, scheme),
            Modal::SelectGroup { dlg, .. } | Modal::Chmod { dlg, .. } => {
                dlg.render(f, area, scheme)
            }
            Modal::DeleteConfirm(d, _) => d.render(f, area, scheme),
            Modal::Viewer(v) => v.render(f, area, scheme),
            Modal::Progress(d) => d.render(f, area, scheme),
            Modal::Hotlist(d) => d.render(f, area, scheme),
            Modal::FindForm(d) => d.render(f, area, scheme),
            Modal::FindResults(d) => d.render(f, area, scheme),
            Modal::CmdLine(d) => d.render(f, area, scheme),
            Modal::UserMenu(d) => d.render(f, area, scheme),
            Modal::Diff(d) => d.render(f, area, scheme),
            Modal::Help(d) => d.render(f, area, scheme),
            Modal::QuickCd(d) => d.render(f, area, scheme),
            Modal::LearnKeys(d) => d.render(f, area, scheme),
            Modal::JobsView(d) => d.render(f, area, scheme),
            Modal::Password { dlg, .. } => dlg.render(f, area, scheme),
            Modal::HostKeyConfirm { dlg, .. } => dlg.render(f, area, scheme),
            Modal::QuickSearch(filter) => render_quick_search(f, area, filter, scheme),
            Modal::Chown { dlg, .. }
            | Modal::Chattr { dlg, .. }
            | Modal::Hardlink { dlg, .. }
            | Modal::Symlink { dlg, .. }
            | Modal::EditSymlink { dlg, .. }
            | Modal::ExternalPanelize(dlg)
            | Modal::Filter(dlg) => dlg.render(f, area, scheme),
            Modal::VfsList(d) => d.render(f, area, scheme),
            Modal::Configuration(d) | Modal::Confirmation(d) | Modal::VirtualFs(d) => {
                d.render(f, area, scheme)
            }
            Modal::Layout(d) => d.render(f, area, scheme),
            Modal::Theme(d) => d.render(f, area, scheme),
            Modal::QuitConfirm(d) => d.render(f, area, scheme),
        }
        if menu_focused {
            let body = Rect::new(
                area.x,
                area.y + 1,
                area.width,
                area.height.saturating_sub(1),
            );
            self.menubar.render_dropdown(f, body, scheme);
        }
        if matches!(self.modal, Modal::PrefixCtrlX) {
            let hint = Line::from(" C-x: c=chmod   (Esc to cancel) ");
            let p = Paragraph::new(hint).style(
                Style::default()
                    .fg(rtc(self.scheme.op_status_fg))
                    .bg(rtc(self.scheme.op_status_bg)),
            );
            let rect = Rect::new(
                area.x,
                area.y + area.height.saturating_sub(2),
                area.width,
                1,
            );
            f.render_widget(p, rect);
        }
    }
}

fn render_quick_search(f: &mut Frame<'_>, area: Rect, filter: &str, scheme: &ColorScheme) {
    let line = Line::from(vec![
        Span::raw(" Search: "),
        Span::styled(
            filter.to_string(),
            Style::default()
                .fg(rtc(scheme.search_fg))
                .bg(rtc(scheme.search_bg))
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
    ]);
    let p = Paragraph::new(line).style(
        Style::default()
            .fg(rtc(scheme.statusbar_fg))
            .bg(rtc(scheme.statusbar_bg)),
    );
    let rect = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(2),
        area.width,
        1,
    );
    f.render_widget(p, rect);
}

fn render_buttonbar(f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) -> Vec<ButtonSegment> {
    let labels: [(u8, &str); 10] = [
        (1, "Help"),
        (2, "Menu"),
        (3, "View"),
        (4, "Edit"),
        (5, "Copy"),
        (6, "RenMov"),
        (7, "Mkdir"),
        (8, "Delete"),
        (9, "PullDn"),
        (10, "Quit"),
    ];
    let mut spans: Vec<Span> = Vec::with_capacity(labels.len() * 3);
    let mut segments: Vec<ButtonSegment> = Vec::with_capacity(labels.len());
    let mut x = area.x;
    for (i, (n, name)) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
            x = x.saturating_add(1);
        }
        let f_label = format!(" ");
        let f_w = u16::try_from(f_label.len()).unwrap_or(0);
        spans.push(Span::styled(
            f_label,
            Style::default()
                .fg(rtc(scheme.buttonbar_fg))
                .bg(rtc(scheme.buttonbar_bg)),
        ));
        let name_label = format!(" F{n} {name} ");
        let name_w = u16::try_from(name_label.len()).unwrap_or(0);
        spans.push(Span::styled(
            name_label,
            Style::default()
                .fg(rtc(scheme.buttonbar_label_fg))
                .bg(rtc(scheme.buttonbar_label_bg)),
        ));
        let total = f_w.saturating_add(name_w);
        segments.push(ButtonSegment {
            fkey: *n,
            x,
            w: total,
        });
        x = x.saturating_add(total);
    }
    let line = Line::from(spans);
    let p = Paragraph::new(line).style(Style::default().bg(rtc(scheme.buttonbar_bg)));
    f.render_widget(p, area);
    segments
}
