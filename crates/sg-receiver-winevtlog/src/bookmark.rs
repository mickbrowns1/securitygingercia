use std::io::Write;
use std::path::Path;

/// Reads a previously persisted bookmark XML blob, if any. A missing or
/// empty file just means "start fresh per `start_at`" -- not an error.
pub fn load(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// Atomically persists the bookmark XML (write-to-temp + rename, same
/// pattern as `sg-checkpoint`) so a crash mid-write can't corrupt the
/// last good bookmark.
pub fn save(path: &Path, xml: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp_path = path.with_extension("xml.tmp");
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(xml.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_loads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("no-such-file.xml")).is_none());
    }

    #[test]
    fn round_trips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/bookmark.xml");
        save(&path, "<BookmarkList><Bookmark Channel='Security'/></BookmarkList>").unwrap();
        assert!(!path.with_extension("xml.tmp").exists());
        assert_eq!(
            load(&path).unwrap(),
            "<BookmarkList><Bookmark Channel='Security'/></BookmarkList>"
        );
    }
}
