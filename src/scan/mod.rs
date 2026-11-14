pub mod walker;

use crate::config::Config;
use crate::model::Entry;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// Messages sent from the scan thread to the UI.
pub enum ScanMsg {
    /// Paths walked so far + current path being visited.
    Progress(u64, String),
    /// Scan finished with findings (or an error message).
    Done(Result<Vec<Entry>, String>),
}

pub struct ScanRequest {
    /// Mode 0/1/2 (stale / junk / custom).
    pub mode: usize,
    pub config: Config,
    /// Override scan root for stale mode.
    pub path: Option<String>,
}

/// Launch a scan on a background thread. Results arrive via `tx`.
pub fn spawn_scan(req: ScanRequest, tx: Sender<ScanMsg>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let result = walker::run(&req, &tx);
        let _ = tx.send(ScanMsg::Done(result));
    })
}

/// True if `p` is a protected path or lives under one.
pub fn is_protected(p: &PathBuf, protected: &[PathBuf]) -> bool {
    protected.iter().any(|pp| p == pp || p.starts_with(pp))
}

/// Refuse to act on anything dangerous.
pub fn action_allowed(p: &PathBuf, protected: &[PathBuf]) -> Result<(), String> {
    if p == &PathBuf::from("/") {
        return Err("refusing to act on filesystem root".into());
    }
    if is_protected(p, protected) {
        return Err(format!("protected path: {}", p.display()));
    }
    if p.components().count() <= 1 {
        return Err("refusing to act on a top-level path".into());
    }
    Ok(())
}
