use crate::app::App;
use crate::model::{fmt_age, fmt_size, EntryKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    canvas::{Canvas, Circle, Points},
    Block, Borders, Cell, Row, Table,
};
use ratatui::Frame;
use std::path::Path;

const ARCH_BLUE: Color = Color::Rgb(23, 147, 209);

/// (used, total) bytes of the filesystem containing `path`.
pub fn fs_usage(path: &Path) -> Option<(u64, u64)> {
    let s = rustix::fs::statvfs(path).ok()?;
    let total = s.f_blocks.saturating_mul(s.f_frsize);
    let free = s.f_bfree.saturating_mul(s.f_frsize);
    Some((total.saturating_sub(free), total))
}

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Length(28), Constraint::Min(30)]).split(area);
    draw_usage_panel(f, app, cols[0]);
    draw_exp_table(f, app, cols[1]);
}

fn usage_color(frac: f64) -> Color {
    if frac >= 0.85 {
        Color::Red
    } else if frac >= 0.6 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn draw_usage_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::new()
        .borders(Borders::ALL)
        .title(Span::styled(
            " usage ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(block, area);
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));

    let (used, total) = fs_usage(&app.exp_path).unwrap_or((0, 0));
    let frac = if total > 0 { used as f64 / total as f64 } else { 0.0 };
    let pct = (frac * 100.0).round() as u64;
    let color = usage_color(frac);

    // ring canvas (top ~70% of panel)
    let ring_h = inner.height.saturating_sub(6).max(5);
    let ring_area = Rect::new(inner.x, inner.y, inner.width, ring_h);
    let canvas = Canvas::default()
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([-1.3, 1.3])
        .y_bounds([-1.3, 1.3])
        .paint(move |ctx| {
            // full ring in dark gray (multiple radii = thickness)
            for r in [0.82, 0.88, 0.94, 1.0] {
                ctx.draw(&Circle { x: 0.0, y: 0.0, radius: r, color: Color::DarkGray });
            }
            // used sweep, starting at 12 o'clock, clockwise
            let steps = 240;
            let coords: Vec<(f64, f64)> = (0..=steps)
                .filter(|i| (*i as f64 / steps as f64) <= frac)
                .flat_map(|i| {
                    let a = std::f64::consts::FRAC_PI_2
                        - (i as f64 / steps as f64) * 2.0 * std::f64::consts::PI;
                    [0.82, 0.88, 0.94, 1.0].into_iter().map(move |r| (a.cos() * r, a.sin() * r))
                })
                .collect();
            ctx.draw(&Points { coords: &coords, color });
        });
    f.render_widget(canvas, ring_area);

    // % in the ring center
    if ring_area.height >= 5 {
        let label = format!("{pct}%");
        let lw = label.chars().count() as u16;
        let lbl_area = Rect::new(
            ring_area.x + ring_area.width.saturating_sub(lw + 1) / 2,
            ring_area.y + ring_area.height / 2,
            lw + 1,
            1,
        );
        f.render_widget(
            ratatui::widgets::Paragraph::new(Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
            lbl_area,
        );
    }

    // stats under the ring
    let stats_y = inner.y + ring_h;
    let stats = vec![
        Line::from(vec![
            Span::styled("used  ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt_size(used), Style::default().fg(color)),
        ]),
        Line::from(vec![
            Span::styled("total ", Style::default().fg(Color::DarkGray)),
            Span::raw(fmt_size(total)),
        ]),
        Line::from(vec![
            Span::styled("free  ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt_size(total.saturating_sub(used)), Style::default().fg(Color::Green)),
        ]),
    ];
    let stats_area = Rect::new(inner.x + 1, stats_y, inner.width.saturating_sub(1), 5);
    f.render_widget(ratatui::widgets::Paragraph::new(stats.clone()), stats_area);

    // top-3 children mini bars
    let mut tops: Vec<&crate::model::Entry> = app
        .exp_children
        .iter()
        .filter(|e| e.size > 0)
        .take(3)
        .collect();
    tops.sort_by(|a, b| b.size.cmp(&a.size));
    let max = tops.first().map(|e| e.size).unwrap_or(1).max(1);
    let by = stats_y + stats.len() as u16 + 1;
    for (i, e) in tops.iter().enumerate() {
        let name = e
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .take(9)
            .collect::<String>();
        let filled = ((e.size as f64 / max as f64) * 10.0).round() as usize;
        let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);
        let line = Line::from(vec![
            Span::styled(format!("{name:<9}"), Style::default().fg(Color::Reset)),
            Span::styled(bar, Style::default().fg(ARCH_BLUE)),
        ]);
        let bar_area = Rect::new(inner.x, by + i as u16, inner.width, 1);
        if by + (i as u16) < inner.y + inner.height {
            f.render_widget(ratatui::widgets::Paragraph::new(line), bar_area);
        }
    }
}

fn breadcrumb(p: &Path, max: usize) -> String {
    let home = dirs::home_dir();
    let full = match &home {
        Some(h) if p.starts_with(h) => format!("~{}", &p.display().to_string()[h.display().to_string().len()..]),
        _ => p.display().to_string(),
    };
    let bc = full.replace('/', " ▸ ");
    shorten(&bc, max)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn draw_exp_table(f: &mut Frame, app: &mut App, area: Rect) {
    let rows_height = area.height.saturating_sub(2) as usize;
    app.exp_offset = app
        .exp_offset
        .min(app.exp_cursor.saturating_sub(rows_height.saturating_sub(1)).max(0));

    let total_child: u64 = app.exp_children.iter().map(|e| e.size).sum::<u64>().max(1);
    let name_w = area.width.saturating_sub(22) as usize;
    let header = Row::new(["", "SIZE", "AGE", "BAR", "NAME"])
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = app
        .exp_visible
        .iter()
        .skip(app.exp_offset)
        .take(rows_height)
        .map(|&i| {
            let Some(e) = app.exp_children.get(i) else { return Row::new([""]) };
            let checked = if app.exp_selected.contains(&i) { "[x] " } else { "[ ] " };
            let is_dir = e.kind == EntryKind::Dir;
            let name = e.path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let disp = if is_dir { format!("{name}/") } else { name };
            let share = (e.size as f64 / total_child as f64) * 100.0;
            let filled = ((e.size as f64 / total_child as f64) * 12.0).round() as usize;
            let bar = "█".repeat(filled) + &"░".repeat(12 - filled);
            let arrow = if is_dir { "▸ " } else { "  " };
            let is_cursor = app.exp_visible.get(app.exp_cursor) == Some(&i);
            let mut cells = vec![
                Cell::from(checked),
                Cell::from(Span::styled(
                    fmt_size(e.size),
                    Style::default().fg(if e.size >= 64 * 1024 * 1024 { Color::Red } else { Color::Green }),
                )),
                Cell::from(fmt_age(e.age_days())),
                Cell::from(Span::styled(bar, Style::default().fg(ARCH_BLUE))),
                Cell::from(Line::from(vec![
                    Span::styled(arrow, Style::default().fg(Color::Blue)),
                    Span::styled(
                        shorten(&disp, name_w.saturating_sub(2)),
                        Style::default().fg(e.kind.color()).add_modifier(if is_dir {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                    Span::styled(format!(" {}", share as u64), Style::default().fg(Color::DarkGray)),
                    Span::styled("%", Style::default().fg(Color::DarkGray)),
                ])),
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
            Constraint::Length(13),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(
        Block::new()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {} ", breadcrumb(&app.exp_path, area.width.saturating_sub(6) as usize)),
                Style::default().fg(ARCH_BLUE).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::DarkGray)),
    )
    .column_spacing(1);

    f.render_widget(table, area);
}
