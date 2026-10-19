//! Safe filesystem primitives.
//!
//! Rules enforced here:
//! * never truncate a file before the replacement validates;
//! * write to a temporary file in the same directory, then replace atomically;
//! * preserve the original file permissions where practical;
//! * refuse to follow unexpected symbolic links;
//! * detect concurrent external edits by comparing content hashes;
//! * verify the file after writing by reading it back.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub path: PathBuf,
    pub content: String,
    pub sha256: String,
    pub size: u64,
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub struct WriteOutcome {
    pub wrote: bool,
    pub previous_hash: Option<String>,
    pub new_hash: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Refuse to operate on symlinks: a config path must be a real file we control.
pub fn reject_symlink(path: &Path) -> AppResult<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(AppError::permission_denied(format!(
            "{} is a symbolic link; refusing to modify it",
            path.display()
        ))),
        _ => Ok(()),
    }
}

pub fn read_snapshot(path: &Path) -> AppResult<Option<FileSnapshot>> {
    reject_symlink(path)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::locked(format!("cannot read {}: {err}", path.display()))
        } else {
            AppError::io(format!("cannot read {}: {err}", path.display()))
        }
    })?;
    let read_only = fs::metadata(path).map(|meta| meta.permissions().readonly()).unwrap_or(false);
    Ok(Some(FileSnapshot {
        path: path.to_path_buf(),
        content: String::from_utf8_lossy(&bytes).to_string(),
        sha256: sha256_hex(&bytes),
        size: bytes.len() as u64,
        read_only,
    }))
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.json".to_string());
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let candidate = format!(".{file_name}.umm-{unique}.tmp");
    path.with_file_name(candidate)
}

/// Write `content` to `path` safely.
///
/// `expected_hash` guards against concurrent external edits: when provided it
/// must match the current file hash (or the file must not exist yet).
pub fn write_atomic(path: &Path, content: &str, expected_hash: Option<&str>) -> AppResult<WriteOutcome> {
    reject_symlink(path)?;

    let existing = read_snapshot(path)?;
    if let (Some(expected), Some(current)) = (expected_hash, existing.as_ref()) {
        if current.sha256 != expected {
            return Err(AppError::file_changed(format!(
                "{} changed on disk after the preview was generated",
                path.display()
            )));
        }
    }

    if let Some(current) = existing.as_ref() {
        if current.read_only {
            return Err(AppError::permission_denied(format!(
                "{} is read-only",
                path.display()
            )));
        }
        if current.content == content {
            // Idempotency: identical content means no write and no mtime change.
            return Ok(WriteOutcome {
                wrote: false,
                previous_hash: Some(current.sha256.clone()),
                new_hash: current.sha256.clone(),
            });
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp = temp_path_for(path);
    {
        let mut file = fs::File::create(&temp).map_err(|err| {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                AppError::permission_denied(format!("cannot create {}: {err}", temp.display()))
            } else {
                AppError::io(format!("cannot create temporary file: {err}"))
            }
        })?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
    }

    // Preserve existing permissions where practical before replacing.
    if let Some(current) = existing.as_ref() {
        if let Ok(meta) = fs::metadata(&current.path) {
            let _ = fs::set_permissions(&temp, meta.permissions());
        }
    }

    fs::rename(&temp, path).map_err(|err| {
        let _ = fs::remove_file(&temp);
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::locked(format!(
                "cannot replace {} (it may be locked by another process): {err}",
                path.display()
            ))
        } else {
            AppError::io(format!("cannot replace {}: {err}", path.display()))
        }
    })?;

    // Verify by reading the file back.
    let written = fs::read(path)?;
    let new_hash = sha256_hex(&written);
    let expected_bytes = content.as_bytes();
    if written != expected_bytes {
        return Err(AppError::io(format!(
            "verification failed after writing {}: file contents differ",
            path.display()
        ))
        .with_remediation("The previous content is preserved in the backup directory."));
    }

    Ok(WriteOutcome {
        wrote: true,
        previous_hash: existing.map(|snapshot| snapshot.sha256),
        new_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_overwrite_when_hash_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{\"a\":1}").unwrap();
        let stale_hash = sha256_hex(b"{\"b\":2}");
        let error = write_atomic(&path, "{\"c\":3}", Some(&stale_hash)).unwrap_err();
        assert_eq!(error.code, "FILE_CHANGED_ON_DISK");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn identical_content_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let content = "{\n  \"a\": 1\n}";
        fs::write(&path, content).unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let outcome = write_atomic(&path, content, None).unwrap();
        assert!(!outcome.wrote);
        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn missing_file_is_created_and_verified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("opencode.json");
        let outcome = write_atomic(&path, "{\"mcp\":{}}", None).unwrap();
        assert!(outcome.wrote);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"mcp\":{}}");
    }
}
