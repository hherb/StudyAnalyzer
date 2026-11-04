//! File grants: the webview can neither choose nor open a path (Tauri still
//! reports a dropped file's path to it). A file the user chose in the dialog
//! or dropped on the window becomes a grant, an id the webview passes back to
//! `open_file` / `attach_file`. A grant is good once and the next pick or drop
//! forgets it (one paper at a time), so the webview can only name a grant.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

use crate::state::lock;

/// A granted file as the webview sees it: the id to pass back and the file
/// name to show, never the path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrantedFile {
    /// The grant, passed to `open_file` / `attach_file`.
    pub id: u64,
    /// The file's name, for display.
    pub name: String,
}

/// The grants of this launch.
#[derive(Default)]
pub struct FileGrants(Mutex<Held>);

/// What [`FileGrants`] holds under its lock.
#[derive(Default)]
struct Held {
    /// The id the next grant gets; ids are never reused in a launch.
    next: u64,
    /// The grants still good, by id.
    paths: HashMap<u64, PathBuf>,
}

impl FileGrants {
    /// Forgets every earlier grant and grants each of `paths` a new id.
    pub fn replace(&self, paths: Vec<PathBuf>) -> Vec<GrantedFile> {
        let mut held = lock(&self.0);
        held.paths.clear();
        paths
            .into_iter()
            .map(|path| {
                held.next += 1;
                let id = held.next;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                held.paths.insert(id, path);
                GrantedFile { id, name }
            })
            .collect()
    }

    /// The path granted as `id`, once: `None` when it is unknown, used or
    /// replaced.
    pub fn take(&self, id: u64) -> Option<PathBuf> {
        lock(&self.0).paths.remove(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grant gives its path once.
    #[test]
    fn grant_is_good_once() {
        let g = FileGrants::default();
        let id = g.replace(vec![PathBuf::from("/a/p.pdf")])[0].id;
        assert_eq!(g.take(id), Some(PathBuf::from("/a/p.pdf")));
        assert_eq!(g.take(id), None);
    }

    /// A new pick or drop forgets the earlier grants, and ids are not reused.
    #[test]
    fn pick_replaces_grants() {
        let g = FileGrants::default();
        let first = g.replace(vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        let second = g.replace(vec![PathBuf::from("/c")]);
        assert!(first.iter().all(|f| g.take(f.id).is_none()));
        assert!(first.iter().all(|f| f.id != second[0].id));
        assert_eq!(g.take(second[0].id), Some(PathBuf::from("/c")));
    }

    /// The display name is the file name only.
    #[test]
    fn granted_name_is_file_name() {
        let g = FileGrants::default();
        assert_eq!(
            g.replace(vec![PathBuf::from("/a/b/paper.pdf")])[0].name,
            "paper.pdf"
        );
    }
}
