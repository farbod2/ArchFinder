use crate::model::Entry;
use crate::scan;
use std::path::PathBuf;

/// Trash the given entries. Returns (trashed, failed) with error strings.
pub fn trash_entries(
    entries: &[&Entry],
    protected: &[PathBuf],
) -> (usize, Vec<(String, String)>) {
    let mut ok = 0;
    let mut failed = Vec::new();
    for e in entries {
        if let Err(err) = scan::action_allowed(&e.path, protected) {
            failed.push((e.path.display().to_string(), err));
            continue;
        }
        match trash::delete(&e.path) {
            Ok(()) => ok += 1,
            Err(err) => failed.push((e.path.display().to_string(), err.to_string())),
        }
    }
    (ok, failed)
}
