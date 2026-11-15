use crate::app::{App, Popup};
use crate::model::{fmt_age, fmt_size};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

fn centered(width: u16, height: u16, f: &Frame) -> ratatui::layout::Rect {
    let area = f.area();
    let v = Layout::vertical([Constraint::Length(height)]).flex(ratatui::layout::Flex::Center);
    let h = Layout::horizontal([Constraint::Length(width)]).flex(ratatui::layout::Flex::Center);
    let [row] = v.areas(area);
    let [col] = h.areas(row);
    col
}

fn box_block(title: &str) -> Block<'_> {
    Block::new()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Cyan))
}

pub fn draw_help(f: &mut Frame) {
    let area = centered(64, 20, f);
    let text = Text::from(vec![
        Line::from(Span::styled("Navigation", Style::default().fg(Color::Yellow))),
        Line::from("  j / k / arrows     move cursor"),
        Line::from("  g / G              jump to top / bottom"),
        Line::from("  PgUp / PgDn        page"),
        Line::from(""),
        Line::from(Span::styled("Selection & action", Style::default().fg(Color::Yellow))),
        Line::from("  space              toggle select item"),
        Line::from("  a / A              select all shown / clear selection"),
        Line::from("  t                  trash selected (with confirmation)"),
        Line::from("  d                  item details"),
        Line::from(""),
        Line::from(Span::styled("View", Style::default().fg(Color::Yellow))),
        Line::from("  Tab                switch Scanner / Explorer tabs"),
        Line::from("  1 / 2 / 3          scanner mode, r scans"),
        Line::from("  s                  cycle sort (size/age/name/path)"),
        Line::from("  /                  filter, Esc clears filter"),
        Line::from(""),
        Line::from(Span::styled("Explorer tab", Style::default().fg(Color::Yellow))),
        Line::from("  p                  open any path (default: /)"),
        Line::from("  Enter / l          descend into directory"),
        Line::from("  h / Backspace      go up one level"),
        Line::from("  i                  about"),
        Line::from("  q                  quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Items are moved to the system trash — nothing is deleted.",
            Style::default().fg(Color::Green),
        )),
    ]);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text)
            .block(box_block("ArchFinder help"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub fn draw_confirm(f: &mut Frame, app: &App) {
    let Popup::Confirm { entries, total } = &app.popup else {
        return;
    };
    let shown = entries.iter().take(8);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Move {} items ({}) to trash?", entries.len(), fmt_size(*total)),
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for e in shown {
        lines.push(Line::from(format!("  {} ({})", e.path.display(), fmt_size(e.size))));
    }
    if entries.len() > 8 {
        lines.push(Line::from(format!("  … and {} more", entries.len() - 8)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Items go to the system trash and can be restored.",
        Style::default().fg(Color::Green),
    )));
    lines.push(Line::from("Enter / y = trash · Esc / n = cancel"));

    let area = centered(80, 16, f);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(box_block("Confirm"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub fn draw_details(f: &mut Frame, app: &App) {
    let Popup::Details(idx) = &app.popup else {
        return;
    };
    let e = if app.tab == 1 {
        app.exp_visible.get(*idx).and_then(|&i| app.exp_children.get(i))
    } else {
        app.visible.get(*idx).and_then(|&i| app.entries.get(i))
    };
    let Some(e) = e else {
        return;
    };
    let mut lines = vec![
        Line::from(format!("Path:  {}", e.path.display())),
        Line::from(format!("Type:  {}", e.kind.label())),
        Line::from(format!("Size:  {}", fmt_size(e.size))),
        Line::from(format!("Age:   {}", fmt_age(e.age_days()))),
    ];
    if let Some(m) = e.modified {
        if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
            lines.push(Line::from(format!(
                "Modified: {}",
                chrono_fmt(d.as_secs() as i64)
            )));
        }
    }
    if e.kind == crate::model::EntryKind::BrokenSymlink {
        if let Ok(target) = std::fs::read_link(&e.path) {
            lines.push(Line::from(format!("Points to (missing): {}", target.display())));
        }
    }

    let area = centered(90, 10, f);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(box_block("Details — Esc to close"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Minimal UTC timestamp formatting without chrono.
fn chrono_fmt(secs: i64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days algorithm (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

pub fn draw_path_input(f: &mut Frame, app: &App) {
    let area = centered(70, 3, f);
    f.render_widget(Clear, area);
    let default = format!(" (default: {})  ", app.exp_path.display());
    f.render_widget(
        Paragraph::new(format!("{}{}", app.path_input, default))
            .block(box_block("Open path — Enter to explore, Esc to cancel")),
        area,
    );
}

pub fn draw_about(f: &mut Frame) {
    let version = env!("CARGO_PKG_VERSION");
    let area = centered(62, 12, f);
    let pad = "  ";
    let mut lines = Vec::new();
    // centered title
    let title = "⚡ ArchFinder";
    let tw = (area.width as usize).saturating_sub(4);
    let pad_title = " ".repeat((tw.saturating_sub(title.chars().count())) / 2);
    lines.push(Line::from(Span::styled(
        format!("{pad_title}{title}"),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let about = "Find and reclaim unused files and folders on Arch Linux.";
    lines.push(Line::from(format!("{pad}{about}")));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(format!("{pad}Author   "), Style::default().fg(Color::DarkGray)),
        Span::raw("farbod2"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("{pad}GitHub   "), Style::default().fg(Color::DarkGray)),
        Span::styled(
            "github.com/farbod2/ArchFinder",
            Style::default().fg(Color::Cyan),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("{pad}Version  "), Style::default().fg(Color::DarkGray)),
        Span::raw(format!("v{version}")),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("{pad}License  "), Style::default().fg(Color::DarkGray)),
        Span::raw("GPL-3.0-or-later"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("{pad}This program comes with ABSOLUTELY NO WARRANTY;"),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        format!("{pad}redistribute it under the GNU GPL v3+."),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "press i / Esc to close",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(box_block("About"))
            .wrap(Wrap { trim: false }),
        area,
    );
}


