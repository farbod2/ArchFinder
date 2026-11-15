use crate::actions;
use crate::config::Config;
use crate::model::{fmt_size, Entry, SortBy, MODES};
use crate::scan::{self, ScanMsg, ScanRequest};
use crossterm::event::{Event as CEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

pub enum TerminalEvent {
    Key(KeyEvent),
    Mouse,
    Resize,
}

pub enum Popup {
    None,
    Help,
    About,
    Confirm { entries: Vec<Entry>, total: u64 },
    Details(usize),
    PathInput,
}

pub struct App {
    pub config: Config,
    pub entries: Vec<Entry>,
    /// indices into entries selected for action
    pub selected: HashSet<usize>,
    /// cursor row in the visible (filtered/sorted) list
    pub cursor: usize,
    /// scroll offset within visible list
    pub offset: usize,
    pub mode: usize,
    pub sort: SortBy,
    pub filter: String,
    pub filter_input: String,
    pub collecting_filter: bool,
    pub popup: Popup,
    pub scanning: bool,
    pub progress: (u64, String),
    pub status: String,
    pub scan_rx: Option<Receiver<ScanMsg>>,
    pub scan_tx: Option<Sender<ScanMsg>>,
    pub should_quit: bool,
    pub last_trashed: Vec<PathBuf>,
    pub protected: Vec<PathBuf>,
    pub visible: Vec<usize>,
    // ---- Explorer tab ----
    /// 0 = Scanner, 1 = Explorer
    pub tab: usize,
    /// current directory shown in the Explorer
    pub exp_path: PathBuf,
    /// explorer children (dirs first, size desc)
    pub exp_children: Vec<Entry>,
    /// selected indices into exp_children
    pub exp_selected: HashSet<usize>,
    /// explorer visible (after filter)
    pub exp_visible: Vec<usize>,
    pub exp_cursor: usize,
    pub exp_offset: usize,
    pub exp_scanning: bool,
    pub exp_rx: Option<Receiver<ScanMsg>>,
    pub path_input: String,
}

impl App {
    pub fn new(config: Config) -> Self {
        let protected = config.protected();
        Self {
            config,
            entries: Vec::new(),
            selected: HashSet::new(),
            cursor: 0,
            offset: 0,
            mode: 0,
            sort: SortBy::Size,
            filter: String::new(),
            filter_input: String::new(),
            collecting_filter: false,
            popup: Popup::None,
            scanning: false,
            progress: (0, String::new()),
            status: "Press 1/2/3 to pick a mode, then r to scan".into(),
            scan_rx: None,
            scan_tx: None,
            should_quit: false,
            last_trashed: Vec::new(),
            protected,
            visible: Vec::new(),
            tab: 0,
            exp_path: PathBuf::from("/"),
            exp_children: Vec::new(),
            exp_selected: HashSet::new(),
            exp_visible: Vec::new(),
            exp_cursor: 0,
            exp_offset: 0,
            exp_scanning: false,
            exp_rx: None,
            path_input: String::new(),
        }
    }

    pub fn start_scan(&mut self) {
        if self.scanning {
            self.status = "Scan already in progress…".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.scan_tx = Some(tx.clone());
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.entries.clear();
        self.selected.clear();
        self.visible.clear();
        self.cursor = 0;
        self.offset = 0;
        self.status = "Scanning…".into();
        let req = ScanRequest { mode: self.mode, config: self.config.clone(), path: None };
        scan::spawn_scan(req, tx);
    }

    pub fn poll_scan(&mut self) {
        if self.scan_rx.is_none() {
            return;
        }
        let rx = self.scan_rx.take().unwrap();
        loop {
            match rx.try_recv() {
                Ok(ScanMsg::Progress(n, cur)) => self.progress = (n, cur),
                Ok(ScanMsg::Done(Ok(entries))) => {
                    let total: u64 = entries.iter().map(|e| e.size).sum();
                    self.entries = entries;
                    self.rebuild_visible();
                    self.scanning = false;
                    self.status = format!(
                        "Found {} items, {} reclaimable",
                        self.entries.len(),
                        fmt_size(total)
                    );
                    break;
                }
                Ok(ScanMsg::Done(Err(e))) => {
                    self.scanning = false;
                    self.status = format!("Scan failed: {e}");
                    break;
                }
                Err(_) => {
                    // scan still running; put the receiver back
                    self.scan_rx = Some(rx);
                    break;
                }
            }
        }
    }

    // ---------- Explorer tab ----------

    pub fn start_explore(&mut self, path: PathBuf) {
        if self.exp_scanning {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.exp_rx = Some(rx);
        self.exp_scanning = true;
        self.exp_path = path;
        self.exp_children.clear();
        self.exp_visible.clear();
        self.exp_selected.clear();
        self.exp_cursor = 0;
        self.exp_offset = 0;
        self.status = format!("Exploring {}…", self.exp_path.display());
        let ignore: HashSet<String> = self.config.ignore_names.iter().cloned().collect();
        scan::explore::spawn_explore(self.exp_path.clone(), ignore, tx);
    }

    pub fn poll_explore(&mut self) {
        if self.exp_rx.is_none() {
            return;
        }
        let rx = self.exp_rx.take().unwrap();
        loop {
            match rx.try_recv() {
                Ok(ScanMsg::Progress(n, cur)) => {
                    self.progress = (n, cur);
                }
                Ok(ScanMsg::Done(Ok(children))) => {
                    let total: u64 = children.iter().map(|e| e.size).sum();
                    self.exp_children = children;
                    self.exp_rebuild_visible();
                    self.exp_scanning = false;
                    self.status = format!(
                        "{}: {} items, {}",
                        self.exp_path.display(),
                        self.exp_children.len(),
                        fmt_size(total)
                    );
                    break;
                }
                Ok(ScanMsg::Done(Err(e))) => {
                    self.exp_scanning = false;
                    self.status = format!("Explore failed: {e}");
                    break;
                }
                Err(_) => {
                    self.exp_rx = Some(rx);
                    break;
                }
            }
        }
    }

    pub fn exp_rebuild_visible(&mut self) {
        let filter = self.filter.to_lowercase();
        let mut vis: Vec<usize> = self
            .exp_children
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                filter.is_empty()
                    || e.path.to_string_lossy().to_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect();
        if self.sort == SortBy::Name {
            vis.sort_by(|&a, &b| {
                let an = self.exp_children[a]
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let bn = self.exp_children[b]
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                an.cmp(&bn)
            });
        }
        self.exp_visible = vis;
        self.exp_clamp();
    }

    fn exp_clamp(&mut self) {
        let len = self.exp_visible.len();
        if len == 0 {
            self.exp_cursor = 0;
            self.exp_offset = 0;
        } else {
            self.exp_cursor = self.exp_cursor.min(len - 1);
            if self.exp_cursor < self.exp_offset {
                self.exp_offset = self.exp_cursor;
            }
            if self.exp_cursor >= self.exp_offset + 200 {
                self.exp_offset = self.exp_cursor - 199;
            }
        }
    }

    pub fn exp_cursor_entry(&self) -> Option<&Entry> {
        self.exp_visible
            .get(self.exp_cursor)
            .and_then(|&i| self.exp_children.get(i))
    }

    pub fn exp_toggle_select(&mut self) {
        if let Some(&i) = self.exp_visible.get(self.exp_cursor) {
            if !self.exp_selected.insert(i) {
                self.exp_selected.remove(&i);
            }
        }
    }

    pub fn exp_select_all(&mut self) {
        for &i in &self.exp_visible {
            self.exp_selected.insert(i);
        }
        self.status = format!("Selected {} items", self.exp_visible.len());
    }

    /// Open the directory under the explorer cursor (if it is a dir).
    pub fn exp_open(&mut self) {
        let Some(e) = self.exp_cursor_entry().cloned() else { return };
        if e.kind == crate::model::EntryKind::Dir {
            self.start_explore(e.path);
        } else {
            self.status = "Not a directory".into();
        }
    }

    pub fn exp_up(&mut self) {
        if let Some(parent) = self.exp_path.parent() {
            if parent != self.exp_path {
                self.start_explore(parent.to_path_buf());
            }
        }
    }

    pub fn open_path_input(&mut self) {
        self.path_input.clear();
        self.popup = Popup::PathInput;
    }

    pub fn submit_path_input(&mut self) {
        if let Popup::PathInput = std::mem::replace(&mut self.popup, Popup::None) {
            let raw = self.path_input.trim().to_string();
            if raw.is_empty() {
                return;
            }
            let expanded = if let Some(rest) = raw.strip_prefix('~') {
                match dirs::home_dir() {
                    Some(home) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(&raw),
                }
            } else {
                PathBuf::from(&raw)
            };
            if !expanded.exists() {
                self.status = format!("No such path: {}", expanded.display());
                return;
            }
            self.tab = 1;
            self.start_explore(expanded);
        }
    }


    pub fn rebuild_visible(&mut self) {
        let filter = self.filter.to_lowercase();
        let mut vis: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                filter.is_empty()
                    || e.path.to_string_lossy().to_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect();
        match self.sort {
            SortBy::Size => vis.sort_by(|&a, &b| self.entries[b].size.cmp(&self.entries[a].size)),
            SortBy::Age => vis.sort_by(|&a, &b| {
                let ad = self.entries[a].age_days().unwrap_or(0);
                let bd = self.entries[b].age_days().unwrap_or(0);
                bd.cmp(&ad)
            }),
            SortBy::Name => vis.sort_by(|&a, &b| {
                let an = self.entries[a]
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let bn = self.entries[b]
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                an.cmp(&bn)
            }),
            SortBy::Path => vis.sort_by(|&a, &b| {
                self.entries[a]
                    .path
                    .as_os_str()
                    .cmp(self.entries[b].path.as_os_str())
            }),
        }
        self.visible = vis;
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible.len();
        if len == 0 {
            self.cursor = 0;
            self.offset = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
            if self.cursor < self.offset {
                self.offset = self.cursor;
            }
            if self.cursor >= self.offset + 200 {
                self.offset = self.cursor - 199;
            }
        }
    }

    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.visible
            .get(self.cursor)
            .and_then(|&i| self.entries.get(i))
    }

    pub fn toggle_select(&mut self) {
        if let Some(&i) = self.visible.get(self.cursor) {
            if !self.selected.insert(i) {
                self.selected.remove(&i);
            }
        }
    }

    pub fn select_all(&mut self) {
        for &i in &self.visible {
            self.selected.insert(i);
        }
        self.status = format!("Selected {} items", self.visible.len());
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.status = "Selection cleared".into();
    }

    pub fn open_confirm(&mut self) {
        if self.selected.is_empty() {
            self.status = "Nothing selected — use space to select items".into();
            return;
        }
        let picked: Vec<Entry> = self
            .selected
            .iter()
            .filter_map(|&i| self.entries.get(i).cloned())
            .collect();
        let total = picked.iter().map(|e| e.size).sum();
        self.popup = Popup::Confirm { entries: picked, total };
    }

    pub fn confirm_trash(&mut self) {
        if let Popup::Confirm { entries, .. } = std::mem::replace(&mut self.popup, Popup::None) {
            let refs: Vec<&Entry> = entries.iter().collect();
            let (ok, failed) = actions::trash_entries(&refs, &self.protected);
            self.last_trashed = entries.iter().map(|e| e.path.clone()).collect();
            // drop trashed from the active tab's list
            let trashed: HashSet<PathBuf> = self.last_trashed.iter().cloned().collect();
            if self.tab == 1 {
                self.exp_children.retain(|e| !trashed.contains(&e.path));
                self.exp_selected.clear();
                self.exp_rebuild_visible();
                self.start_explore(self.exp_path.clone()); // refresh sizes
            } else {
                self.entries.retain(|e| !trashed.contains(&e.path));
                self.selected.clear();
                self.rebuild_visible();
            }
            let mut msg = format!("Trashed {ok} items");
            if !failed.is_empty() {
                msg.push_str(&format!(", {} failed: {}", failed.len(), failed[0].1));
            }
            self.status = msg;
        }
    }

    pub fn show_details(&mut self) {
        let idx = if self.tab == 1 { self.exp_cursor } else { self.cursor };
        let has = if self.tab == 1 {
            self.exp_cursor_entry().is_some()
        } else {
            self.cursor_entry().is_some()
        };
        if has {
            self.popup = Popup::Details(idx);
        } else {
            self.status = "No item under cursor".into();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.collecting_filter {
            match key.code {
                KeyCode::Esc => {
                    self.collecting_filter = false;
                    self.filter_input.clear();
                }
                KeyCode::Enter => {
                    self.filter = self.filter_input.clone();
                    self.collecting_filter = false;
                    if self.tab == 1 {
                        self.exp_rebuild_visible();
                    } else {
                        self.rebuild_visible();
                    }
                }
                KeyCode::Backspace => {
                    self.filter_input.pop();
                }
                KeyCode::Char(c) => self.filter_input.push(c),
                _ => {}
            }
            return;
        }

        if matches!(self.popup, Popup::PathInput) {
            match key.code {
                KeyCode::Esc => self.popup = Popup::None,
                KeyCode::Enter => self.submit_path_input(),
                KeyCode::Backspace => {
                    self.path_input.pop();
                }
                KeyCode::Char(c) => self.path_input.push(c),
                _ => {}
            }
            return;
        }

        // Tab toggles Scanner <-> Explorer (works even with popups closed only)
        if key.code == KeyCode::Tab && matches!(self.popup, Popup::None) {
            self.tab = 1 - self.tab;
            self.status = if self.tab == 1 {
                "Explorer — r to scan, p to open a path, Tab to switch back".into()
            } else {
                "Scanner".into()
            };
            return;
        }

        match self.popup {
            Popup::None => self.handle_main_key(key),
            Popup::Help => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char('?') => {
                    self.popup = Popup::None
                }
                _ => {}
            },
            Popup::About => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char('i') => {
                    self.popup = Popup::None
                }
                _ => {}
            },
            Popup::Confirm { .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.popup = Popup::None,
                KeyCode::Enter | KeyCode::Char('y') => self.confirm_trash(),
                _ => {}
            },
            Popup::Details(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('d') | KeyCode::Char('q') | KeyCode::Enter) {
                    self.popup = Popup::None;
                }
            }
            Popup::PathInput => {} // handled above
        }
    }

    fn handle_main_key(&mut self, key: KeyEvent) {
        if self.tab == 1 {
            self.handle_explorer_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.cursor + 1 < self.visible.len() {
                    self.cursor += 1;
                    self.clamp_cursor();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                self.clamp_cursor();
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.cursor = 0;
                self.clamp_cursor();
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.cursor = self.visible.len().saturating_sub(1);
                self.clamp_cursor();
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 20).min(self.visible.len().saturating_sub(1));
                self.clamp_cursor();
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(20);
                self.clamp_cursor();
            }
            KeyCode::Char(' ') => self.toggle_select(),
            KeyCode::Char('a') => self.select_all(),
            KeyCode::Char('A') => self.clear_selection(),
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.rebuild_visible();
                self.status = format!("Sorted by {}", self.sort.label());
            }
            KeyCode::Char('/') => {
                self.collecting_filter = true;
                self.filter_input.clear();
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.rebuild_visible();
            }
            KeyCode::Char('1') => self.switch_mode(0),
            KeyCode::Char('2') => self.switch_mode(1),
            KeyCode::Char('3') => self.switch_mode(2),
            KeyCode::Char('r') => self.start_scan(),
            KeyCode::Char('t') => self.open_confirm(),
            KeyCode::Char('d') => self.show_details(),
            KeyCode::Char('?') => self.popup = Popup::Help,
            KeyCode::Char('i') => self.popup = Popup::About,
            _ => {}
        }
    }

    fn handle_explorer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.exp_cursor + 1 < self.exp_visible.len() {
                    self.exp_cursor += 1;
                    self.exp_clamp();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.exp_cursor = self.exp_cursor.saturating_sub(1);
                self.exp_clamp();
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.exp_cursor = 0;
                self.exp_clamp();
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.exp_cursor = self.exp_visible.len().saturating_sub(1);
                self.exp_clamp();
            }
            KeyCode::PageDown => {
                self.exp_cursor = (self.exp_cursor + 20).min(self.exp_visible.len().saturating_sub(1));
                self.exp_clamp();
            }
            KeyCode::PageUp => {
                self.exp_cursor = self.exp_cursor.saturating_sub(20);
                self.exp_clamp();
            }
            KeyCode::Enter | KeyCode::Char('l') => self.exp_open(),
            KeyCode::Char('h') | KeyCode::Backspace => self.exp_up(),
            KeyCode::Char('p') => self.open_path_input(),
            KeyCode::Char('r') => self.start_explore(self.exp_path.clone()),
            KeyCode::Char(' ') => self.exp_toggle_select(),
            KeyCode::Char('a') => self.exp_select_all(),
            KeyCode::Char('A') => {
                self.exp_selected.clear();
                self.status = "Selection cleared".into();
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.exp_rebuild_visible();
                self.status = format!("Sorted by {}", self.sort.label());
            }
            KeyCode::Char('/') => {
                self.collecting_filter = true;
                self.filter_input.clear();
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.exp_rebuild_visible();
            }
            KeyCode::Char('t') => {
                if self.exp_selected.is_empty() {
                    self.status = "Nothing selected — use space to select items".into();
                } else {
                    let picked: Vec<Entry> = self
                        .exp_selected
                        .iter()
                        .filter_map(|&i| self.exp_children.get(i).cloned())
                        .collect();
                    let total = picked.iter().map(|e| e.size).sum();
                    self.popup = Popup::Confirm { entries: picked, total };
                }
            }
            KeyCode::Char('d') => {
                if self.exp_cursor_entry().is_some() {
                    self.popup = Popup::Details(self.exp_cursor);
                }
            }
            KeyCode::Char('?') => self.popup = Popup::Help,
            KeyCode::Char('i') => self.popup = Popup::About,
            _ => {}
        }
    }

    fn switch_mode(&mut self, mode: usize) {
        if self.mode != mode {
            self.mode = mode;
            self.status = format!("Mode: {} — press r to scan", MODES[mode]);
        }
    }

    /// Block briefly for events; used by the main loop.
    pub fn next_event(timeout: Duration) -> Option<TerminalEvent> {
        let poll = crossterm::event::poll(timeout);
        match poll {
            Ok(true) => match crossterm::event::read() {
                Ok(CEvent::Key(k)) => Some(TerminalEvent::Key(k)),
                Ok(CEvent::Mouse(_)) => Some(TerminalEvent::Mouse),
                Ok(CEvent::Resize(_, _)) => Some(TerminalEvent::Resize),
                Ok(_) => None,
                Err(_) => None,
            },
            _ => None,
        }
    }

    pub fn status_line(&self) -> Line<'static> {
        Line::from(self.status.clone())
    }
}
