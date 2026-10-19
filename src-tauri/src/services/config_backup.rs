//! Timestamped backups and restore with integrity checks.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::database::Db;
use crate::errors::{AppError, AppResult};
use crate::models::{BackupInfo, RestoreResult};
use crate::services::safe_io::{read_snapshot, sha256_hex, write_atomic};

pub struct BackupManager {
    dir: PathBuf,
}

impl BackupManager {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    /// Copy `path` into the backup directory. Returns `None` when the source
    /// does not exist yet (nothing to back up).
    pub fn backup_file(
        &self,
        harness_id: &str,
        path: &Path,
        reason: &str,
    ) -> AppResult<Option<BackupInfo>> {
        let Some(snapshot) = read_snapshot(path)? else {
            return Ok(None);
        };

        fs::create_dir_all(&self.dir)?;
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "config.json".into());
        let stamp = Utc::now().format("%Y%m%d-%H%M%S-%3f");
        let backup_path = self.dir.join(format!("{harness_id}__{stamp}__{file_name}"));
        fs::write(&backup_path, &snapshot.content)?;

        Ok(Some(BackupInfo {
            id: 0,
            harness_id: harness_id.to_string(),
            original_path: path.to_string_lossy().to_string(),
            backup_path: backup_path.to_string_lossy().to_string(),
            sha256: snapshot.sha256,
            size: snapshot.size,
            reason: reason.to_string(),
            created_at: Utc::now().to_rfc3339(),
            exists: true,
        }))
    }

    /// Record a backup in the database, returning the stored entry.
    pub fn store_backup(&self, db: &Db, info: BackupInfo) -> AppResult<BackupInfo> {
        let id = db.insert_backup(&info)?;
        Ok(BackupInfo { id, ..info })
    }

    /// Restore a backup over its original path.
    ///
    /// The current file is backed up again first ("restore-safety") so a bad
    /// restore can be undone. Integrity of the backup file is verified against
    /// the hash recorded when it was created.
    pub fn restore(&self, db: &Db, backup: &BackupInfo) -> AppResult<RestoreResult> {
        let backup_path = PathBuf::from(&backup.backup_path);
        if !backup_path.exists() {
            return Err(AppError::not_found(format!(
                "backup file {} no longer exists",
                backup_path.display()
            )));
        }
        let bytes = fs::read(&backup_path)?;
        let actual = sha256_hex(&bytes);
        if actual != backup.sha256 {
            return Err(AppError::validation(format!(
                "backup integrity check failed for {}",
                backup_path.display()
            ))
            .with_remediation("The backup file was modified after it was created; refusing to restore it."));
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| AppError::validation("backup is not valid UTF-8 text"))?;

        let target = PathBuf::from(&backup.original_path);
        let safety = self.backup_file(&backup.harness_id, &target, "restore-safety")?;
        let safety = match safety {
            Some(info) => Some(self.store_backup(db, info)?),
            None => None,
        };

        let expected = read_snapshot(&target)?.map(|snapshot| snapshot.sha256);
        let outcome = write_atomic(&target, &content, expected.as_deref())?;

        Ok(RestoreResult {
            restored: true,
            backup_path: backup.backup_path.clone(),
            target_path: target.to_string_lossy().to_string(),
            safety_backup_path: safety.map(|info| info.backup_path),
            message: if outcome.wrote {
                "Configuration restored from backup and verified.".into()
            } else {
                "Configuration already matched the backup; no write was necessary.".into()
            },
        })
    }
}
