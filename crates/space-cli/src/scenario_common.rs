use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct SizeStats {
    pub disk_bytes: u64,
    pub apparent_bytes: u64,
    pub files: u64,
}

#[derive(Default)]
struct HardlinkSet {
    #[cfg(unix)]
    seen: HashSet<(u64, u64)>,
}

impl HardlinkSet {
    fn count(&mut self, meta: &fs::Metadata) -> (u64, u64) {
        let len = meta.len();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() > 1 && !self.seen.insert((meta.dev(), meta.ino())) {
                return (0, 0);
            }
            let allocated = meta.blocks().saturating_mul(512);
            let disk = if allocated == 0 && len <= 4096 {
                len
            } else {
                len.min(allocated)
            };
            (disk, len)
        }
        #[cfg(not(unix))]
        {
            (len, len)
        }
    }
}

pub fn user_home() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let raw = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .ok_or("HOME/USERPROFILE is not set")?;
    Ok(PathBuf::from(raw))
}

pub fn dev_project_roots(home: &Path) -> Vec<PathBuf> {
    let rels = [
        "Projects",
        "Source",
        "Sources",
        "Developer",
        "src",
        "repos",
        "code",
        "work",
        "workspace",
        "Documents/Projects",
        "Documents/Source",
        "Documents/Sources",
        "RiderProjects",
        "IdeaProjects",
        "WebstormProjects",
        "PycharmProjects",
        "CLionProjects",
        "GolandProjects",
    ];
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for rel in rels {
        let p = home.join(rel);
        if !p.is_dir() {
            continue;
        }
        let key = p.canonicalize().unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            roots.push(p);
        }
    }
    roots
}

/// Measure allocated on-disk bytes (with hardlink dedup) and apparent bytes
/// without following symlinks.
pub fn measure_path(path: &Path) -> Option<SizeStats> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    let mut hardlinks = HardlinkSet::default();
    let mut stats = SizeStats::default();
    if meta.is_file() {
        let (disk, apparent) = hardlinks.count(&meta);
        return Some(SizeStats {
            disk_bytes: disk,
            apparent_bytes: apparent,
            files: 1,
        });
    }
    if !meta.is_dir() {
        return None;
    }

    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(m) = entry.metadata() {
                    let (disk, apparent) = hardlinks.count(&m);
                    stats.disk_bytes = stats.disk_bytes.saturating_add(disk);
                    stats.apparent_bytes = stats.apparent_bytes.saturating_add(apparent);
                    stats.files = stats.files.saturating_add(1);
                }
            }
        }
    }
    Some(stats)
}
