mod popups;

use crate::app::{App, Popup};
use crate::model::{fmt_age, fmt_size, MODES};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Row, Table, Tabs};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(9), // logo / header
        Constraint::Min(5),    // table
        Constraint::Length(1), // status
        Constraint::Length(1), // keybinds
    ])
    .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_table(f, app, chunks[1]);
    draw_status(f, app, chunks[2]);
    draw_keybinds(f, app, chunks[3]);

    match &app.popup {
        Popup::None => {}
        Popup::Help => popups::draw_help(f),
        Popup::About => popups::draw_about(f),
        Popup::Confirm { .. } => popups::draw_confirm(f, app),
        Popup::Details(_) => popups::draw_details(f, app),
    }
}

/// The Arch Linux logo, neofetch style.
const ARCH_LOGO: [&str; 7] = [
    r"      /\",
    r"     /  \",
    r"     /\   \",
    r"    /      \",
    r"   /   ,,   \",
    r"  /   |  |  -.",
    r" /_-''    ''-_\",
];

const ARCH_BLUE: Color = Color::Rgb(23, 147, 209);

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(block, area);
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));

    // logo (left)
    let logo_w = 16u16;
    let logo_area = Rect::new(
        inner.x,
        inner.y,
        logo_w.min(inner.width),
        inner.height.min(ARCH_LOGO.len() as u16),
    );
    let logo: Vec<Line> = ARCH_LOGO
        .iter()
        .map(|l| Line::from(*l).style(Style::default().fg(ARCH_BLUE)))
        .collect();
    f.render_widget(ratatui::widgets::Paragraph::new(logo), logo_area);

    // right column: title, mode tabs, scan gauge / status
    let rx = inner.x + logo_w.min(inner.width) + 1;
    let rw = inner.width.saturating_sub(logo_w.min(inner.width) + 1);
    if rw < 12 {
        return;
    }
    let right = Rect::new(rx, inner.y, rw, inner.height);
    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // tabs
        Constraint::Length(1), // gauge or status
        Constraint::Min(0),
    ])
    .split(right);

    let total: u64 = app.entries.iter().map(|e| e.size).sum();
    let title = Line::from(vec![
        Span::styled(
            "⚡ ArchFinder ",
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("v{}  ", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{} reclaimable", fmt_size(total)),
            Style::default().fg(Color::Green),
        ),
    ]);
    f.render_widget(ratatui::widgets::Paragraph::new(title), rows[0]);

    let titles: Vec<Line> = MODES
        .iter()
        .enumerate()
        .map(|(i, m)| {
            Line::from(format!(" {} {} ", i + 1, m)).style(if i == app.mode {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            })
        })
        .collect();
    let tabs = Tabs::new(titles).select(app.mode).highlight_style(Style::default());
    f.render_widget(tabs, rows[1]);

    if app.scanning {
        let pct = (app.progress.0 % 100) as u16; // indeterminate-style shimmer
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(ARCH_BLUE))
            .percent(pct)
            .label(format!(
                "scanning… {} paths, {}",
                app.progress.0,
                truncate_tail(&app.progress.1, rw.saturating_sub(20) as usize)
            ));
        f.render_widget(gauge, rows[2]);
    } else {
        let line = Line::from(app.status.clone()).style(Style::default().fg(Color::DarkGray));
        f.render_widget(ratatui::widgets::Paragraph::new(line), rows[2]);
    }
}

fn truncate_tail(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn draw_table(f: &mut Frame, app: &mut App, area: Rect) {
    let rows_height = area.height.saturating_sub(2) as usize;
    app.offset = app.offset.min(app.cursor.saturating_sub(rows_height.saturating_sub(1)).max(0));

    let max_w = area.width.saturating_sub(24) as usize;
    let header = Row::new(["", "SIZE", "AGE", "TYPE", "PATH"])
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = app
        .visible
        .iter()
        .skip(app.offset)
        .take(rows_height)
        .map(|&i| {
            let Some(e) = app.entries.get(i) else {
                return Row::new([""]);
            };
            let checked = if app.selected.contains(&i) { "[x] " } else { "[ ] " };
            let path_str = shorten(&e.path.display().to_string(), max_w);
            let is_cursor = app.visible.get(app.cursor) == Some(&i);
            let mut cells = vec![
                Cell::from(checked),
                Cell::from(Span::styled(
                    fmt_size(e.size),
                    Style::default().fg(size_color(e.size)),
                )),
                Cell::from(fmt_age(e.age_days())),
                Cell::from(Span::styled(
                    e.kind.label(),
                    Style::default().fg(e.kind.color()),
                )),
                Cell::from(path_str),
            ];
            if is_cursor {
                for c in &mut cells {
                    *c = c.clone().style(Style::default().bg(Color::DarkGray));
                }
            }
            Row::new(cells)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::new()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(
                    " {} | sort: {} | filter: {} | {} shown ",
                    MODES[app.mode],
                    app.sort.label(),
                    if app.filter.is_empty() { "none".into() } else { app.filter.clone() },
                    app.visible.len()
                ),
                Style::default().fg(Color::DarkGray),
            ))
            .border_style(Style::default().fg(Color::DarkGray)),
    )
    .column_spacing(1);

    f.render_widget(table, area);
}

fn size_color(b: u64) -> Color {
    if b >= 512 * 1024 * 1024 {
        Color::Red
    } else if b >= 64 * 1024 * 1024 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head)
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let line = app.status_line();
    f.render_widget(
        ratatui::widgets::Paragraph::new(line).style(Style::default().fg(Color::Blue)),
        area,
    );
}

fn draw_keybinds(f: &mut Frame, app: &App, area: Rect) {
    let sel = app.selected.len();
    let binds = format!(
        "1/2/3 mode · r scan · j/k move · space select ({sel}) · a all · A none · s sort · / filter · t trash · d details · i about · ? help · q quit"
    );
    f.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(binds))
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
