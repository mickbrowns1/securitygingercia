//! Offset persistence for rotation-aware file tailing. Identity is keyed
//! by `(device, inode)` rather than path, so a rotated/renamed file keeps
//! its progress until it's actually deleted. Writes are atomic
//! (write-to-temp + rename) so a crash never corrupts the previous good
//! checkpoint -- worst case a restart re-reads a handful of recent lines,
//! which is acceptable at-least-once behavior for a log collector.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

impl FileIdentity {
    fn key(&self) -> String {
        format!("{}:{}", self.device, self.inode)
    }

    fn parse_key(s: &str) -> Option<Self> {
        let (d, i) = s.split_once(':')?;
        Some(Self {
            device: d.parse().ok()?,
            inode: i.parse().ok()?,
        })
    }
}

#[cfg(unix)]
pub fn identity_of(meta: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: meta.dev(),
        inode: meta.ino(),
    }
}

#[cfg(windows)]
pub fn identity_of(meta: &std::fs::Metadata) -> FileIdentity {
    use std::os::windows::fs::MetadataExt;
    FileIdentity {
        device: meta.volume_serial_number().unwrap_or(0) as u64,
        inode: meta.file_index().unwrap_or(0),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOffsetEntry {
    /// Last known path, kept only for diagnostics -- identity is the key.
    pub path: PathBuf,
    pub offset: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointFileFormat {
    version: u32,
    entries: HashMap<String, FileOffsetEntry>,
}

impl Default for CheckpointFileFormat {
    fn default() -> Self {
        Self {
            version: 1,
            entries: HashMap::new(),
        }
    }
}

pub struct CheckpointStore {
    path: PathBuf,
    data: CheckpointFileFormat,
}

impl CheckpointStore {
    /// Loads an existing checkpoint file, or starts empty if it doesn't
    /// exist yet. A malformed file is treated as empty rather than a
    /// fatal error -- losing a checkpoint costs a few re-read lines, not
    /// correctness.
    pub fn load(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CheckpointFileFormat::default(),
            Err(e) => return Err(e),
        };
        Ok(Self { path, data })
    }

    pub fn get(&self, id: FileIdentity) -> Option<u64> {
        self.data.entries.get(&id.key()).map(|e| e.offset)
    }

    pub fn set(&mut self, id: FileIdentity, path: &Path, offset: u64) {
        self.data.entries.insert(
            id.key(),
            FileOffsetEntry {
                path: path.to_path_buf(),
                offset,
                updated_at: Utc::now(),
            },
        );
    }

    /// Drops entries whose identity no longer corresponds to any file this
    /// caller cares about (e.g. a rotated file that has aged out).
    pub fn retain(&mut self, keep: impl Fn(FileIdentity) -> bool) {
        self.data
            .entries
            .retain(|k, _| FileIdentity::parse_key(k).map(&keep).unwrap_or(false));
    }

    pub fn flush(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let tmp_path = self.path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(&self.data)?;
        {
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(&json)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_missing_file_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let store = CheckpointStore::load(&path).unwrap();
        assert_eq!(store.get(FileIdentity { device: 1, inode: 2 }), None);
    }

    #[test]
    fn round_trips_through_flush_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/checkpoint.json");
        let id = FileIdentity {
            device: 42,
            inode: 7,
        };

        let mut store = CheckpointStore::load(&path).unwrap();
        store.set(id, Path::new("/var/log/app.log"), 1234);
        store.flush().unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());

        let reloaded = CheckpointStore::load(&path).unwrap();
        assert_eq!(reloaded.get(id), Some(1234));
    }

    #[test]
    fn survives_rotation_by_tracking_new_identity_separately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let old_id = FileIdentity {
            device: 1,
            inode: 100,
        };
        let new_id = FileIdentity {
            device: 1,
            inode: 200,
        };

        let mut store = CheckpointStore::load(&path).unwrap();
        store.set(old_id, Path::new("/var/log/app.log"), 500);
        store.flush().unwrap();

        let mut store = CheckpointStore::load(&path).unwrap();
        assert_eq!(store.get(old_id), Some(500));
        store.set(new_id, Path::new("/var/log/app.log"), 10);
        store.flush().unwrap();

        let store = CheckpointStore::load(&path).unwrap();
        assert_eq!(store.get(old_id), Some(500));
        assert_eq!(store.get(new_id), Some(10));
    }

    #[test]
    fn retain_drops_stale_identities() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let keep_id = FileIdentity { device: 1, inode: 1 };
        let drop_id = FileIdentity { device: 1, inode: 2 };

        let mut store = CheckpointStore::load(&path).unwrap();
        store.set(keep_id, Path::new("/a"), 1);
        store.set(drop_id, Path::new("/b"), 2);
        store.retain(|id| id == keep_id);

        assert_eq!(store.get(keep_id), Some(1));
        assert_eq!(store.get(drop_id), None);
    }
}
