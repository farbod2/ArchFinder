use std::path::PathBuf;
use std::time::SystemTime;

/// What kind of entry a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    EmptyDir,
    BrokenSymlink,
    Cache,
    Trash,
}

impl EntryKind {
    pub fn label(&self) -> &'static str {
        match self {
            EntryKind::File => "file",
            EntryKind::Dir => "dir",
            EntryKind::EmptyDir => "empty",
            EntryKind::BrokenSymlink => "deadlnk",
            EntryKind::Cache => "cache",
            EntryKind::Trash => "trash",
        }
    }

    /// Color used in the table.
    pub fn color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            EntryKind::File => Color::Reset,
            EntryKind::Dir => Color::Blue,
            EntryKind::EmptyDir => Color::DarkGray,
            EntryKind::BrokenSymlink => Color::Magenta,
            EntryKind::Cache => Color::Yellow,
            EntryKind::Trash => Color::Red,
        }
    }
}

/// A single finding: a path we consider unused / reclaimable.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: EntryKind,
    /// Total size in bytes (for dirs, recursive).
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Entry {
    pub fn age_days(&self) -> Option<u64> {
        self.modified
            .map(|m| m.elapsed().ok())
            .flatten()
            .map(|d| d.as_secs() / 86_400)
    }
}

/// The three detection modes.
pub const MODES: [&str; 3] = ["Stale files", "Junk & leftovers", "Custom paths"];

/// Sort order for the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Size,
    Age,
    Name,
    Path,
}

impl SortBy {
    pub fn label(&self) -> &'static str {
        match self {
            SortBy::Size => "size",
            SortBy::Age => "age",
            SortBy::Name => "name",
            SortBy::Path => "path",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            SortBy::Size => SortBy::Age,
            SortBy::Age => SortBy::Name,
            SortBy::Name => SortBy::Path,
            SortBy::Path => SortBy::Size,
        }
    }
}

/// Human-readable byte size, e.g. `1.2 GB`.
pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if val >= 100.0 {
        format!("{val:.0} {}", UNITS[unit])
    } else if val >= 10.0 {
        format!("{val:.1} {}", UNITS[unit])
    } else {
        format!("{val:.2} {}", UNITS[unit])
    }
}

/// Human-readable age, e.g. `3mo 12d`.
pub fn fmt_age(days: Option<u64>) -> String {
    match days {
        None => "—".into(),
        Some(d) => {
            if d < 1 {
                "<1d".into()
            } else if d < 30 {
                format!("{d}d")
            } else if d < 365 {
                format!("{}mo {}d", d / 30, d % 30)
            } else {
                format!("{}y {}d", d / 365, (d % 365) / 30)
            }
        }
    }
}
