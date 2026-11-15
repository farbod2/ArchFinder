//! Explorer tab: sizes of immediate children of a directory, computed in a
//! background thread with rayon parallelism.

use crate::model::{Entry, EntryKind};
use crate::scan::ScanMsg;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::SystemTime;

/// Size of one path: files = own size; dirs = recursive subtree size.
///
/// Uses a plain iterative walk (NOT jwalk/rayon-inside-rayon — nesting rayon
/// pools with a blocking wait deadlocks, which froze the Explorer). Metadata
/// is only stat'ed, never read, so even kernel filesystems (/proc, /sys, /dev,
/// /run) terminate quickly. Every mount is checked — nothing is skipped.
fn subtree_size(path: &Path, walked: &AtomicU64) -> (u64, Option<SystemTime>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, None);
    };
    if !meta.is_dir() {
        return (meta.len(), meta.modified().ok());
    }
    let mut total = 0u64;
    let mut newest: Option<SystemTime> = None;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            walked.fetch_add(1, Ordering::Relaxed);
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
                if let Ok(m) = meta.modified() {
                    if newest.map(|cur| m > cur).unwrap_or(true) {
                        newest = Some(m);
                    }
                }
            }
        }
    }
    (total, newest)
}

/// Launch an explorer scan of `path`'s immediate children.
pub fn spawn_explore(
    path: PathBuf,
    ignore_names: HashSet<String>,
    tx: Sender<ScanMsg>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let result = run(&path, ignore_names, &tx);
        let _ = tx.send(ScanMsg::Done(result));
    })
}

fn run(path: &Path, ignore_names: HashSet<String>, tx: &Sender<ScanMsg>) -> Result<Vec<Entry>, String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    let children: Vec<(PathBuf, std::fs::Metadata)> = std::fs::read_dir(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .filter_map(|rd| rd.ok())
        .filter_map(|de| {
            let p = de.path();
            let name = de.file_name().to_string_lossy().to_string();
            if ignore_names.contains(&name) {
                return None;
            }
            std::fs::symlink_metadata(&p).ok().map(|m| (p, m))
        })
        .collect();

    let walked = AtomicU64::new(0);
    let entries: Vec<Entry> = children
        .into_par_iter()
        .map(|(p, m)| {
            let kind = if m.file_type().is_symlink() {
                if std::fs::metadata(&p).is_err() {
                    EntryKind::BrokenSymlink
                } else {
                    EntryKind::File
                }
            } else if m.is_dir() {
                EntryKind::Dir
            } else if m.len() == 0 {
                EntryKind::EmptyDir
            } else {
                EntryKind::File
            };
            let (size, modified) = match kind {
                EntryKind::BrokenSymlink => (0, None),
                _ => subtree_size(&p, &walked),
            };
            let modified = modified.or_else(|| m.modified().ok());
            Entry { path: p, kind, size, modified }
        })
        .collect();

    let _ = tx.send(ScanMsg::Progress(walked.load(Ordering::Relaxed), path.display().to_string()));
    let mut entries = entries;
    // dirs first, then by size desc
    entries.sort_by(|a, b| {
        let da = a.kind == EntryKind::Dir;
        let db = b.kind == EntryKind::Dir;
        db.cmp(&da).then(b.size.cmp(&a.size))
    });
    Ok(entries)
}
