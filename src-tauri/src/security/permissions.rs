//! Filesystem permission policy.
//!
//! The frontend never supplies arbitrary write targets. Paths are checked
//! against an allow-list built from:
//! * the application data/backup directories,
//! * each detected harness configuration location,
//! * generic profiles explicitly configured by the user,
//! * paths the user picks through the OS file dialog (import/export, backups).

use std::path::{Component, Path, PathBuf};

use crate::errors::{AppError, AppResult};
use crate::services::safe_io::reject_symlink;

#[derive(Debug, Clone, Default)]
pub struct PathPolicy {
    allowed_files: Vec<PathBuf>,
    allowed_dirs: Vec<PathBuf>,
}

impl PathPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_file(&mut self, path: impl Into<PathBuf>) {
        self.allowed_files.push(normalize(path.into()));
    }

    pub fn allow_dir(&mut self, path: impl Into<PathBuf>) {
        self.allowed_dirs.push(normalize(path.into()));
    }

    pub fn is_allowed(&self, path: &Path) -> bool {
        let candidate = normalize(path.to_path_buf());
        if self.allowed_files.iter().any(|allowed| *allowed == candidate) {
            return true;
        }
        self.allowed_dirs
            .iter()
            .any(|dir| candidate.starts_with(dir))
    }

    /// Validate a write target. Rejects traversal tricks, symlinks and paths
    /// outside the allow-list.
    pub fn check_write_target(&self, path: &Path) -> AppResult<()> {
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(AppError::permission_denied(
                "paths containing '..' are rejected",
            ));
        }
        if !self.is_allowed(path) {
            return Err(AppError::permission_denied(format!(
                "{} is outside the set of allowed configuration locations",
                path.display()
            )));
        }
        reject_symlink(path)
    }

    /// Looser check used for read operations and dialog-picked files.
    pub fn check_readable(&self, path: &Path) -> AppResult<()> {
        if !path.is_absolute() {
            return Err(AppError::permission_denied("an absolute path is required"));
        }
        reject_symlink(path)
    }
}

fn normalize(path: PathBuf) -> PathBuf {
    // Best-effort canonicalisation without requiring the file to exist.
    if let Ok(canonical) = std::fs::canonicalize(&path) {
        return canonical;
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_allow_list() {
        let mut policy = PathPolicy::new();
        policy.allow_file("C:/configs/opencode.json");
        assert!(policy.check_write_target(Path::new("C:/other/evil.json")).is_err());
        assert!(policy.check_write_target(Path::new("C:/configs/opencode.json")).is_ok());
    }

    #[test]
    fn rejects_parent_dir_traversal() {
        let mut policy = PathPolicy::new();
        policy.allow_dir("C:/configs");
        assert!(policy
            .check_write_target(Path::new("C:/configs/../secrets.txt"))
            .is_err());
    }
}
