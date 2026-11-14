use super::ScanMsg;
use crate::config::{Config, CustomPath};
use crate::model::{Entry, EntryKind};
use jwalk::WalkDir;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::SystemTime;

struct Acc {
    /// dir path -> (total subtree size of files, deepest file mtime)
    dir_size: HashMap<PathBuf, (u64, Option<SystemTime>)>,
    files: Vec<(PathBuf, u64, Option<SystemTime>)>,
    empty_dirs: Vec<PathBuf>,
    broken_links: Vec<(PathBuf, Option<SystemTime>)>,
    skipped: u64,
    walked: AtomicU64,
}

pub fn run(req: &super::ScanRequest, tx: &Sender<ScanMsg>) -> Result<Vec<Entry>, String> {
    let cfg = &req.config;
    let protected = cfg.protected();
    let ignore: HashSet<String> = cfg.ignore_names.iter().cloned().collect();

    let mut acc = Acc {
        dir_size: HashMap::new(),
        files: Vec::new(),
        empty_dirs: Vec::new(),
        broken_links: Vec::new(),
        skipped: 0,
        walked: AtomicU64::new(0),
    };

    let roots: Vec<(PathBuf, Option<&CustomPath>)> = match req.mode {
        0 => {
            let root = req
                .path
                .as_ref()
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir())
                .ok_or("could not resolve home directory")?;
            vec![(root, None)]
        }
        1 => {
            let mut v = Vec::new();
            if let Some(p) = &req.path {
                v.push((PathBuf::from(p), None));
            } else {
                // junk mode focuses on cache + trash roots
                if cfg.scan_cache {
                    if let Some(c) = dirs::cache_dir() {
                        v.push((c, None));
                    }
                }
                if cfg.scan_trash {
                    if let Some(t) = trash_root() {
                        v.push((t, None));
                    }
                }
            }
            if v.is_empty() {
                return Err("no cache/trash roots to scan".into());
            }
            v
        }
        2 => cfg
            .custom_paths
            .iter()
            .map(|cp| (PathBuf::from(&cp.path), Some(cp)))
            .collect(),
        _ => return Err("invalid mode".into()),
    };

    for (root, custom) in &roots {
        if super::is_protected(root, &protected) {
            continue;
        }
        walk_root(root, *custom, cfg, &ignore, &protected, &mut acc, tx);
    }

    let mut entries = build_entries(&acc, req.mode, cfg, &roots, tx);
    entries.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(entries)
}

fn trash_root() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Trash").join("files"))
}

fn ignored_name(name: &str, ignore: &HashSet<String>, custom_ignores: &[String]) -> bool {
    if ignore.contains(name) {
        return true;
    }
    custom_ignores.iter().any(|ig| name.contains(ig.as_str()))
}

#[allow(clippy::too_many_arguments)]
fn walk_root(
    root: &Path,
    custom: Option<&CustomPath>,
    cfg: &Config,
    ignore: &HashSet<String>,
    protected: &[PathBuf],
    acc: &mut Acc,
    tx: &Sender<ScanMsg>,
) {
    let min_age = custom.map(|c| c.min_age_days).unwrap_or(cfg.min_age_days);
    let _ = min_age; // filtering happens in build_entries
    let custom_ignores: Vec<String> = custom.map(|c| c.ignore.clone()).unwrap_or_default();

    let custom_ignores: Vec<String> = custom.map(|c| c.ignore.clone()).unwrap_or_default();
    let protected_prune: Vec<PathBuf> = protected.to_vec();
    let ignore_prune: HashSet<String> = ignore.iter().map(|s| s.to_string()).collect();
    let custom_ignores_prune = custom_ignores.clone();

    let walker = WalkDir::new(root)
        .parallelism(jwalk::Parallelism::RayonDefaultPool {
            busy_timeout: std::time::Duration::from_millis(50),
        })
        .follow_links(false)
        .skip_hidden(false)
        .process_read_dir(move |_depth, _path, _state, children| {
            // prune ignored and protected directories so we never descend into them
            children.retain(|entry| {
                let Ok(e) = entry else { return false };
                if e.depth == 0 {
                    return true; // keep the root itself
                }
                let name = e.file_name().to_string_lossy().to_string();
                if e.file_type.is_dir() {
                    if ignored_name(&name, &ignore_prune, &custom_ignores_prune) {
                        return false;
                    }
                    let full = _path.join(&name);
                    if super::is_protected(&full, &protected_prune) {
                        return false;
                    }
                }
                true
            });
        });

    for entry in walker {
        let Ok(entry) = entry else {
            acc.skipped += 1;
            continue;
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        let n = acc.walked.fetch_add(1, Ordering::Relaxed) + 1;
        if n % 512 == 0 {
            let _ = tx.send(ScanMsg::Progress(n, path.display().to_string()));
        }

        if super::is_protected(&path, protected) && path != root {
            continue;
        }
        let depth_ok = path != root; // never flag the root itself
        if !depth_ok {
            continue;
        }

        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            acc.skipped += 1;
            continue;
        };

        if meta.file_type().is_symlink() {
            if std::fs::metadata(&path).is_err() {
                if !ignored_name(&name, ignore, &custom_ignores) {
                    acc.broken_links.push((path, None));
                }
            }
            continue;
        }

        if meta.is_dir() {
            // dir sizes accumulate from files below; record base mtime later via files map
            acc.dir_size.entry(path).or_insert((0, None));
            continue;
        }

        if ignored_name(&name, ignore, &custom_ignores) {
            continue;
        }

        let size = meta.len();
        let mtime = meta.modified().ok();

        // record against every ancestor up to root
        let mut anc: Option<&Path> = path.parent();
        while let Some(a) = anc {
            let e = acc.dir_size.entry(a.to_path_buf()).or_insert((0, None));
            e.0 += size;
            if let Some(m) = mtime {
                if e.1.map(|cur| m > cur).unwrap_or(true) {
                    e.1 = Some(m);
                }
            }
            if a == root {
                break;
            }
            anc = a.parent();
        }

        acc.files.push((path, size, mtime));
    }

    // second pass: empty dirs (dirs we recorded with zero files)
    for (dir, (size, _)) in acc.dir_size.clone() {
        if size == 0 && std::fs::symlink_metadata(&dir).map(|m| m.is_dir()).unwrap_or(false) {
            // ensure it truly has no children (e.g. only ignored files)
            if std::fs::read_dir(&dir).map(|mut rd| rd.next().is_none()).unwrap_or(false) {
                acc.empty_dirs.push(dir);
            }
        }
    }
}

fn build_entries(
    acc: &Acc,
    mode: usize,
    cfg: &Config,
    roots: &[(PathBuf, Option<&CustomPath>)],
    tx: &Sender<ScanMsg>,
) -> Vec<Entry> {
    let mut entries = Vec::new();
    let cutoff = |days: u64| SystemTime::now() - std::time::Duration::from_secs(days * 86_400);

    // ancestor dirs already reported -> suppress children
    let mut reported_dirs: HashSet<PathBuf> = HashSet::new();

    let is_under_reported = |p: &Path, reported: &HashSet<PathBuf>| -> bool {
        p.ancestors()
            .skip(1)
            .any(|a| reported.contains(a))
    };

    match mode {
        0 => {
            // stale files & dirs under home
            let cut = cutoff(cfg.min_age_days);
            // dirs first so children can be suppressed (shallowest first)
            let mut stale_dirs: Vec<(&PathBuf, &(u64, Option<SystemTime>))> = acc
                .dir_size
                .iter()
                .filter(|(d, _)| d.as_path() != Path::new("/"))
                .collect();
            stale_dirs.sort_by_key(|(d, _)| d.components().count());
            for (dir, (size, newest_file)) in stale_dirs {
                let dir_mtime = std::fs::symlink_metadata(dir).and_then(|m| m.modified()).ok();
                let stale = dir_mtime.map(|m| m <= cut).unwrap_or(false);
                if stale && !is_under_reported(dir, &reported_dirs) {
                    // suppress everything *inside* this stale dir
                    reported_dirs.insert(dir.clone());
                    // keep the "newest file inside" as modified so age shown is honest
                    entries.push(Entry {
                        path: dir.clone(),
                        kind: EntryKind::Dir,
                        size: *size,
                        modified: dir_mtime.or(*newest_file),
                        mode,
                    });
                }
            }
            for (path, size, mtime) in &acc.files {
                if mtime.map(|m| m <= cut).unwrap_or(false) && !is_under_reported(path, &reported_dirs) {
                    entries.push(Entry {
                        path: path.clone(),
                        kind: EntryKind::File,
                        size: *size,
                        modified: *mtime,
                        mode,
                    });
                }
            }
        }
        1 => {
            // junk: cache top-level children, trash contents, empty dirs, broken links
            for (root, _) in roots {
                let is_trash = root.file_name().map(|f| f == "files").unwrap_or(false)
                    && root.parent().map(|p| p.file_name().map(|f| f == "Trash").unwrap_or(false)).unwrap_or(false);
                let cut = cutoff(cfg.min_age_days);
                let children = acc
                    .dir_size
                    .keys()
                    .filter(|d| d.parent() == Some(root.as_path()))
                    .cloned()
                    .collect::<Vec<_>>();
                for child in children {
                    let (size, newest) = acc.dir_size.get(&child).cloned().unwrap_or((0, None));
                    let mtime = std::fs::symlink_metadata(&child).and_then(|m| m.modified()).ok();
                    if is_trash {
                        // trash is always reclaimable
                        entries.push(Entry {
                            path: child,
                            kind: EntryKind::Trash,
                            size,
                            modified: mtime.or(newest),
                            mode,
                        });
                    } else {
                        // cache: only if stale enough
                        let stale = mtime
                            .map(|m| m <= cut)
                            .unwrap_or(false)
                            || newest.map(|m| m <= cut).unwrap_or(false);
                        if stale {
                            entries.push(Entry {
                                path: child,
                                kind: EntryKind::Cache,
                                size,
                                modified: mtime.or(newest),
                                mode,
                            });
                        }
                    }
                }
                // stray stale files directly under cache/trash root
                for (path, size, mtime) in &acc.files {
                    if path.parent() == Some(root.as_path()) {
                        if is_trash || mtime.map(|m| m <= cut).unwrap_or(false) {
                            entries.push(Entry {
                                path: path.clone(),
                                kind: if is_trash { EntryKind::Trash } else { EntryKind::Cache },
                                size: *size,
                                modified: *mtime,
                                mode,
                            });
                        }
                    }
                }
            }
            if cfg.detect_empty_and_broken {
                for d in &acc.empty_dirs {
                    if d.parent().map(|p| roots.iter().any(|(r, _)| p == r)).unwrap_or(false) {
                        entries.push(Entry {
                            path: d.clone(),
                            kind: EntryKind::EmptyDir,
                            size: 0,
                            modified: std::fs::symlink_metadata(d).and_then(|m| m.modified()).ok(),
                            mode,
                        });
                    }
                }
                for (p, _) in &acc.broken_links {
                    if p.parent().map(|p| roots.iter().any(|(r, _)| p == r)).unwrap_or(false) {
                        entries.push(Entry {
                            path: p.clone(),
                            kind: EntryKind::BrokenSymlink,
                            size: 0,
                            modified: None,
                            mode,
                        });
                    }
                }
            }
        }
        2 => {
            for (root, custom) in roots {
                let Some(cp) = custom else { continue };
                let cut = cutoff(cp.min_age_days);
                for (path, size, mtime) in &acc.files {
                    if mtime.map(|m| m <= cut).unwrap_or(false) && !is_under_reported(path, &reported_dirs) {
                        entries.push(Entry {
                            path: path.clone(),
                            kind: EntryKind::File,
                            size: *size,
                            modified: *mtime,
                            mode,
                        });
                    }
                }
                let mut custom_dirs: Vec<(&PathBuf, &(u64, Option<SystemTime>))> = acc
                    .dir_size
                    .iter()
                    .filter(|(d, _)| d.as_path() != root.as_path())
                    .collect();
                custom_dirs.sort_by_key(|(d, _)| d.components().count());
                for (dir, (size, newest)) in custom_dirs {
                    let dir_mtime = std::fs::symlink_metadata(dir).and_then(|m| m.modified()).ok();
                    if dir_mtime.map(|m| m <= cut).unwrap_or(false)
                        && !is_under_reported(dir, &reported_dirs)
                    {
                        reported_dirs.insert(dir.clone());
                        entries.push(Entry {
                            path: dir.clone(),
                            kind: EntryKind::Dir,
                            size: *size,
                            modified: dir_mtime.or(*newest),
                            mode,
                        });
                    }
                }
            }
        }
        _ => {}
    }

    let _ = tx.send(ScanMsg::Progress(acc.walked.load(Ordering::Relaxed), String::new()));
    entries
}
