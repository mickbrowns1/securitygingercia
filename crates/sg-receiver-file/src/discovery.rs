use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

/// One `include` entry split into a directory to (non-recursively) list
/// and a filename glob to match within it. Covers the common case
/// (`"/var/log/myapp/*.log"`) without a full recursive glob-walk
/// implementation.
pub struct DiscoveryPattern {
    dir: PathBuf,
    file_glob: globset::GlobMatcher,
}

pub fn split_pattern(pattern: &str) -> DiscoveryPattern {
    let path = Path::new(pattern);
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("*")
        .to_string();
    let file_glob = Glob::new(&file_name)
        .unwrap_or_else(|_| Glob::new("*").unwrap())
        .compile_matcher();
    DiscoveryPattern { dir, file_glob }
}

pub fn build_globset(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).map_err(|e| format!("invalid exclude pattern '{p}': {e}"))?;
        builder.add(glob);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Lists files currently matching `includes`, minus anything matched by
/// `excludes`. Non-recursive: only the exact directory named in each
/// include pattern is scanned.
pub fn discover_files(includes: &[DiscoveryPattern], excludes: &GlobSet) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for inc in includes {
        let Ok(entries) = std::fs::read_dir(&inc.dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|f| f.to_str()) else {
                continue;
            };
            if inc.file_glob.is_match(name) && !excludes.is_match(&path) {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_matching_files_and_respects_exclude() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.log"), "a").unwrap();
        std::fs::write(dir.path().join("app.log.gz"), "b").unwrap();
        std::fs::write(dir.path().join("other.txt"), "c").unwrap();

        let pattern = format!("{}/*.log", dir.path().display());
        let includes = vec![split_pattern(&pattern)];
        let exclude_pattern = format!("{}/*.log.gz", dir.path().display());
        let excludes = build_globset(&[exclude_pattern]).unwrap();

        let found = discover_files(&includes, &excludes);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "app.log");
    }
}
