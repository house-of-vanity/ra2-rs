//! Filesystem traversal shared by native game content loaders.
use std::path::{Path, PathBuf};
pub fn scan(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            scan(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) mod robots;

pub(crate) mod arenas;
pub(crate) mod introduction;
