use std::path::{Path, PathBuf};

use crate::error::MemoryError;

pub struct StoreInfo {
    pub name: String,
    pub size_bytes: u64,
}

struct ActiveStore {
    name: String,
    conn: rusqlite::Connection,
}

pub struct StoreManager {
    base_dir: PathBuf,
    active_store: Option<ActiveStore>,
}

fn validate_name(name: &str) -> Result<(), MemoryError> {
    if name.is_empty() || name.len() > 64 {
        return Err(MemoryError::InvalidName(name.into()));
    }
    let mut chars = name.chars();
    if !chars.next().unwrap().is_ascii_alphanumeric() {
        return Err(MemoryError::InvalidName(name.into()));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(MemoryError::InvalidName(name.into()));
    }
    let upper = name.to_uppercase();
    let base = upper.split('.').next().unwrap();
    if matches!(
        base,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(MemoryError::InvalidName(name.into()));
    }
    Ok(())
}

fn check_not_symlink(path: &Path) -> Result<(), MemoryError> {
    if path.exists() {
        let meta = std::fs::symlink_metadata(path).map_err(|e| {
            MemoryError::InvalidPath(format!("cannot read metadata for {}: {e}", path.display()))
        })?;
        if meta.file_type().is_symlink() {
            return Err(MemoryError::InvalidPath("store path is a symlink".into()));
        }
    }
    Ok(())
}

fn resolve_and_verify(base_dir: &Path, name: &str) -> Result<PathBuf, MemoryError> {
    let path = base_dir.join(format!("{name}.db"));
    check_not_symlink(&path)?;
    let canonical_base = std::fs::canonicalize(base_dir).map_err(|e| {
        tracing::error!(
            "Cannot canonicalize base directory {}: {e}",
            base_dir.display()
        );
        MemoryError::InvalidPath("cannot canonicalize base directory".into())
    })?;
    let resolved = canonical_base.join(format!("{name}.db"));
    if !resolved.starts_with(&canonical_base) {
        return Err(MemoryError::InvalidPath(
            "resolved path escapes base directory".into(),
        ));
    }
    Ok(resolved)
}

#[cfg(unix)]
fn has_bad_prefix(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.starts_with("/dev/") || s.starts_with("/proc/") || s.starts_with("/sys/")
}

#[cfg(windows)]
fn has_bad_prefix(p: &Path) -> bool {
    let s = p.to_string_lossy();
    // Reject UNC network paths (\\server\share) but allow the extended-length
    // path prefix (\\?\) that std::fs::canonicalize() produces on Windows.
    s.starts_with("\\\\") && !s.starts_with("\\\\?\\")
}

#[cfg(not(any(unix, windows)))]
fn has_bad_prefix(_p: &Path) -> bool {
    false
}

impl StoreManager {
    pub fn new() -> Result<Self, MemoryError> {
        let base_dir = match std::env::var("LOCAL_MEMORY_HOME") {
            Ok(val) if !val.is_empty() => {
                let p = PathBuf::from(&val);
                if !p.is_absolute() {
                    return Err(MemoryError::InvalidPath(format!(
                        "LOCAL_MEMORY_HOME must be absolute: {val}"
                    )));
                }
                if p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(MemoryError::InvalidPath(format!(
                        "LOCAL_MEMORY_HOME must not contain '..': {val}"
                    )));
                }
                if has_bad_prefix(&p) {
                    return Err(MemoryError::InvalidPath(format!(
                        "LOCAL_MEMORY_HOME points to a restricted path: {val}"
                    )));
                }
                if p.exists() {
                    let canon = std::fs::canonicalize(&p).map_err(|e| {
                        MemoryError::InvalidPath(format!(
                            "cannot canonicalize LOCAL_MEMORY_HOME: {e}"
                        ))
                    })?;
                    if has_bad_prefix(&canon) {
                        return Err(MemoryError::InvalidPath(format!(
                            "LOCAL_MEMORY_HOME resolves to a restricted path: {}",
                            canon.display()
                        )));
                    }
                    canon
                } else {
                    p
                }
            }
            _ => {
                #[cfg(unix)]
                {
                    dirs::home_dir()
                        .map(|h| h.join(".local-memory-mcp"))
                        .ok_or_else(|| {
                            MemoryError::InvalidPath(
                                "Cannot determine home directory. Set LOCAL_MEMORY_HOME environment variable.".into(),
                            )
                        })?
                }
                #[cfg(windows)]
                {
                    dirs::data_local_dir()
                        .map(|d| d.join("local-memory-mcp"))
                        .ok_or_else(|| {
                            MemoryError::InvalidPath(
                                "Cannot determine home directory. Set LOCAL_MEMORY_HOME environment variable.".into(),
                            )
                        })?
                }
                #[cfg(not(any(unix, windows)))]
                {
                    return Err(MemoryError::InvalidPath(
                        "Cannot determine home directory. Set LOCAL_MEMORY_HOME environment variable.".into(),
                    ));
                }
            }
        };

        std::fs::create_dir_all(&base_dir).map_err(|e| {
            MemoryError::ConnectionFailed(format!(
                "failed to create base directory {}: {e}",
                base_dir.display()
            ))
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(&base_dir, perms).map_err(|e| {
                MemoryError::ConnectionFailed(format!(
                    "failed to set permissions on {}: {e}",
                    base_dir.display()
                ))
            })?;
        }

        Ok(StoreManager {
            base_dir,
            active_store: None,
        })
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Result<Self, MemoryError> {
        std::fs::create_dir_all(&base_dir).map_err(|e| {
            MemoryError::ConnectionFailed(format!(
                "failed to create base directory {}: {e}",
                base_dir.display()
            ))
        })?;
        Ok(StoreManager {
            base_dir,
            active_store: None,
        })
    }

    pub fn open_default(&mut self) -> Result<(), MemoryError> {
        self.open_store("default")
    }

    fn open_store(&mut self, name: &str) -> Result<(), MemoryError> {
        validate_name(name)?;
        let path = resolve_and_verify(&self.base_dir, name)?;
        let conn = crate::db::open(&path)?;
        self.active_store = Some(ActiveStore {
            name: name.to_string(),
            conn,
        });
        Ok(())
    }

    pub fn db(&self) -> Result<&dyn crate::db::Db, MemoryError> {
        match self.active_store {
            Some(ref s) => Ok(&s.conn),
            None => Err(MemoryError::Disconnected),
        }
    }

    pub fn active_name(&self) -> Option<&str> {
        self.active_store.as_ref().map(|s| s.name.as_str())
    }

    pub fn close_active(&mut self) -> Result<(), MemoryError> {
        if let Some(ref store) = self.active_store {
            store
                .conn
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .map_err(|e| {
                    tracing::error!("WAL checkpoint failed: {e}");
                    MemoryError::ConnectionFailed(format!("WAL checkpoint failed: {e}"))
                })?;
            if let Err(e) = store.conn.execute_batch("PRAGMA optimize") {
                tracing::warn!("PRAGMA optimize failed: {e}");
            }
            self.active_store = None;
        }
        Ok(())
    }

    fn close_active_best_effort(&mut self) {
        if let Some(ref store) = self.active_store {
            if let Err(e) = store.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
                tracing::warn!("WAL checkpoint failed during drop: {e}");
            }
            if let Err(e) = store.conn.execute_batch("PRAGMA optimize") {
                tracing::warn!("PRAGMA optimize failed during drop: {e}");
            }
        }
        self.active_store = None;
    }

    pub fn switch(&mut self, name: &str) -> Result<(), MemoryError> {
        validate_name(name)?;
        if self.active_name() == Some(name) {
            return Ok(());
        }
        self.close_active()?;
        self.open_store(name)
    }

    pub fn list(&self) -> Result<Vec<StoreInfo>, MemoryError> {
        let mut stores = Vec::new();
        let entries = std::fs::read_dir(&self.base_dir).map_err(|e| {
            tracing::error!("read_dir failed: {e}");
            MemoryError::QueryFailed("failed to list stores".into())
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let Some(ext) = path.extension() else {
                continue;
            };
            if ext != "db" {
                continue;
            };
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let name = stem.to_string();
            let mut size = meta.len();
            for suffix in &["-wal", "-shm"] {
                let aux = self.base_dir.join(format!("{name}.db{suffix}"));
                if let Ok(m) = std::fs::metadata(&aux) {
                    size += m.len();
                }
            }
            stores.push(StoreInfo {
                name,
                size_bytes: size,
            });
        }
        stores.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(stores)
    }

    pub fn delete(&self, name: &str) -> Result<(), MemoryError> {
        validate_name(name)?;
        if self.active_name() == Some(name) {
            return Err(MemoryError::ActiveStoreDeletion(name.into()));
        }
        let db_path = self.base_dir.join(format!("{name}.db"));
        check_not_symlink(&db_path)?;
        if !db_path.exists() {
            return Err(MemoryError::NotFound(name.into()));
        }
        for suffix in &[".db-shm", ".db-wal"] {
            let aux = self.base_dir.join(format!("{name}{suffix}"));
            if aux.exists() {
                let _ = std::fs::remove_file(&aux);
            }
        }
        remove_with_retry(&db_path).map_err(|e| {
            tracing::error!("Failed to delete store {name}: {e}");
            MemoryError::DeleteFailed(format!("failed to delete store '{name}'"))
        })?;
        Ok(())
    }
}

fn remove_with_retry(path: &std::path::Path) -> std::io::Result<()> {
    let mut last_err = None;
    for _ in 0..3 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let retry = e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(32);
                if retry {
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(last_err.unwrap())
}

impl Drop for StoreManager {
    fn drop(&mut self) {
        self.close_active_best_effort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_open_default() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        assert_eq!(mgr.active_name(), Some("default"));
    }

    #[test]
    fn test_db_before_open() {
        let dir = TempDir::new().unwrap();
        let mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        assert!(matches!(mgr.db(), Err(MemoryError::Disconnected)));
    }

    #[test]
    fn test_env_var_relative_rejected() {
        std::env::set_var("LOCAL_MEMORY_HOME", "relative/path");
        let result = StoreManager::new();
        std::env::remove_var("LOCAL_MEMORY_HOME");
        assert!(matches!(result, Err(MemoryError::InvalidPath(_))));
    }

    #[test]
    fn test_validate_name_invalid() {
        assert!(validate_name("").is_err());
        assert!(validate_name("-bad").is_err());
        assert!(validate_name("_bad").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_name("CON").is_err());
        assert!(validate_name("nul").is_err());
        assert!(validate_name("com1").is_err());
    }

    #[test]
    fn test_switch_creates_new_store() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        mgr.switch("work").unwrap();
        assert_eq!(mgr.active_name(), Some("work"));
        assert!(dir.path().join("work.db").exists());
    }

    #[test]
    fn test_switch_invalid_name() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        assert!(matches!(
            mgr.switch("../evil"),
            Err(MemoryError::InvalidName(_))
        ));
        assert!(matches!(mgr.switch(""), Err(MemoryError::InvalidName(_))));
        assert!(matches!(
            mgr.switch(&"a".repeat(65)),
            Err(MemoryError::InvalidName(_))
        ));
    }

    #[test]
    fn test_delete_store() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        mgr.switch("temp").unwrap();
        mgr.switch("default").unwrap();
        mgr.delete("temp").unwrap();
        assert!(!dir.path().join("temp.db").exists());
    }

    #[test]
    fn test_delete_active_store() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        assert!(matches!(
            mgr.delete("default"),
            Err(MemoryError::ActiveStoreDeletion(_))
        ));
    }

    #[test]
    fn test_delete_nonexistent() {
        let dir = TempDir::new().unwrap();
        let mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        assert!(matches!(mgr.delete("nope"), Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_symlink_rejected() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        // Create a symlink posing as a store
        let target = dir.path().join("default.db");
        let link = dir.path().join("evil.db");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        let result = mgr.switch("evil");
        assert!(matches!(result, Err(MemoryError::InvalidPath(_))));
    }

    #[test]
    fn test_resolve_and_verify_rejects_missing_base() {
        // If base_dir doesn't exist, canonicalize fails and returns error (not silent fallback)
        let nonexistent = std::path::PathBuf::from("/tmp/nonexistent_test_dir_12345");
        assert!(!nonexistent.exists());
        let result = resolve_and_verify(&nonexistent, "test");
        assert!(matches!(result, Err(MemoryError::InvalidPath(_))));
    }
}
