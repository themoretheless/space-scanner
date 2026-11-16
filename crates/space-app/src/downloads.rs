//! Lightweight scanner & selection state for `~/Downloads`.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DEFAULT_MIN_INSTALLER_BYTES: u64 = 1 << 20;
const DEFAULT_MIN_LARGE_BYTES: u64 = 50 << 20;
const MAX_DOWNLOAD_ITEMS: usize = 200;
const MAX_DIR_WALK_ENTRIES: usize = 15_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadKind {
    Installer,
    LargeItem,
}

#[derive(Clone, Debug)]
pub struct DownloadItem {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub is_dir: bool,
    pub kind: DownloadKind,
    pub age_secs: Option<u64>,
}

#[derive(Default)]
pub struct DownloadsState {
    pub loading: bool,
    pub open: bool,
    items: Vec<DownloadItem>,
    selected: Vec<bool>,
    selected_count: usize,
    selected_bytes: u128,
    selection_version: u64,
}

impl DownloadsState {
    pub fn selection_version(&self) -> u64 {
        self.selection_version
    }

    pub fn set_items(&mut self, items: Vec<DownloadItem>) {
        self.loading = false;
        self.selected = vec![false; items.len()];
        self.selected_count = 0;
        self.selected_bytes = 0;
        self.items = items;
        self.selection_version = self.selection_version.wrapping_add(1);
    }

    pub fn items(&self) -> &[DownloadItem] {
        &self.items
    }

    pub fn total_bytes(&self) -> u64 {
        self.items
            .iter()
            .fold(0_u64, |sum, item| sum.saturating_add(item.bytes))
    }

    pub fn selected_count(&self) -> usize {
        self.selected_count
    }

    pub fn selected_bytes(&self) -> u64 {
        self.selected_bytes.min(u128::from(u64::MAX)) as u64
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.get(index).copied().unwrap_or(false)
    }

    pub fn set_selected(&mut self, index: usize, on: bool) {
        let Some(current) = self.selected.get_mut(index) else {
            return;
        };
        if *current == on {
            return;
        }
        *current = on;
        self.selection_version = self.selection_version.wrapping_add(1);
        let bytes = u128::from(self.items[index].bytes);
        if on {
            self.selected_count += 1;
            self.selected_bytes += bytes;
        } else {
            self.selected_count -= 1;
            self.selected_bytes -= bytes;
        }
    }

    pub fn set_all(&mut self, on: bool) {
        for index in 0..self.items.len() {
            self.set_selected(index, on);
        }
    }

    pub fn select_installers(&mut self) {
        for index in 0..self.items.len() {
            if self.items[index].kind == DownloadKind::Installer {
                self.set_selected(index, true);
            }
        }
    }

    pub fn has_installers(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.kind == DownloadKind::Installer)
    }

    pub fn clear_selection(&mut self) {
        if self.selected_count > 0 {
            self.selection_version = self.selection_version.wrapping_add(1);
        }
        self.selected.fill(false);
        self.selected_count = 0;
        self.selected_bytes = 0;
    }

    pub fn selected_items(&self) -> Vec<DownloadItem> {
        self.items
            .iter()
            .enumerate()
            .filter(|(index, _)| self.is_selected(*index))
            .map(|(_, item)| item.clone())
            .collect()
    }

    pub fn exclude_paths(&mut self, paths: &[PathBuf]) {
        for index in 0..self.items.len() {
            let item_path = &self.items[index].path;
            if paths.iter().any(|path| item_path.starts_with(path)) {
                self.set_selected(index, false);
            }
        }
    }

    /// Prune deleted items in-place while preserving selection state on remaining entries.
    pub fn remove_paths(&mut self, removed: &[PathBuf]) {
        if removed.is_empty() || self.items.is_empty() {
            return;
        }
        let mut kept_items = Vec::with_capacity(self.items.len());
        let mut kept_selected = Vec::with_capacity(self.items.len());
        let mut count = 0usize;
        let mut bytes = 0u128;
        let mut changed = false;

        for (index, item) in self.items.drain(..).enumerate() {
            if removed.iter().any(|path| item.path.starts_with(path)) {
                changed = true;
                continue;
            }
            let checked = self.selected.get(index).copied().unwrap_or(false);
            if checked {
                count += 1;
                bytes += u128::from(item.bytes);
            }
            kept_items.push(item);
            kept_selected.push(checked);
        }

        self.items = kept_items;
        self.selected = kept_selected;
        self.selected_count = count;
        self.selected_bytes = bytes;
        if changed {
            self.selection_version = self.selection_version.wrapping_add(1);
        }
    }
}

/// Scan the user's `~/Downloads` folder for installers and large entries (excluding archives).
pub fn scan_default_downloads() -> Vec<DownloadItem> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    scan_downloads_dir(
        &PathBuf::from(home).join("Downloads"),
        DEFAULT_MIN_INSTALLER_BYTES,
        DEFAULT_MIN_LARGE_BYTES,
    )
}

/// Scan `dir` (top level) returning installers >= `min_installer_bytes` and
/// other non-archive files/folders >= `min_large_bytes`, sorted largest first.
pub fn scan_downloads_dir(
    dir: &Path,
    min_installer_bytes: u64,
    min_large_bytes: u64,
) -> Vec<DownloadItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut items = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || space_clean::safety::is_never_touch(&path) {
            continue;
        }
        let is_dir = meta.is_dir();
        if !is_dir && !meta.is_file() {
            continue;
        }
        let Some(kind) = classify_download(&name) else {
            continue;
        };
        let threshold = match kind {
            DownloadKind::Installer => min_installer_bytes,
            DownloadKind::LargeItem => min_large_bytes,
        };
        let bytes = if is_dir {
            bounded_dir_bytes(&path, MAX_DIR_WALK_ENTRIES)
        } else {
            entry_bytes(&meta)
        };
        if bytes < threshold {
            continue;
        }
        let age_secs = meta
            .modified()
            .ok()
            .and_then(|mtime| now.duration_since(mtime).ok())
            .map(|d| d.as_secs());
        items.push(DownloadItem {
            path,
            name,
            bytes,
            is_dir,
            kind,
            age_secs,
        });
    }

    items.sort_by(|a, b| {
        Reverse(a.bytes)
            .cmp(&Reverse(b.bytes))
            .then_with(|| a.name.cmp(&b.name))
    });
    items.truncate(MAX_DOWNLOAD_ITEMS);
    items
}

fn classify_download(name: &str) -> Option<DownloadKind> {
    let lower = name.to_ascii_lowercase();
    for ext in [".dmg", ".pkg", ".mpkg", ".iso", ".ipsw"] {
        if lower.ends_with(ext) {
            return Some(DownloadKind::Installer);
        }
    }
    for ext in [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".txz", ".tzst", ".zip",
        ".tar", ".gz", ".bz2", ".xz", ".zst", ".7z", ".rar",
    ] {
        if lower.ends_with(ext) {
            return None;
        }
    }
    Some(DownloadKind::LargeItem)
}

fn entry_bytes(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let allocated = meta.blocks().saturating_mul(512);
        if allocated > 0 {
            return allocated;
        }
    }
    meta.len()
}

fn bounded_dir_bytes(root: &Path, max_entries: usize) -> u64 {
    let mut total = 0_u64;
    let mut visited = 0_usize;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if visited >= max_entries {
                return total;
            }
            visited += 1;
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    total = total.saturating_add(entry_bytes(&meta));
                }
            }
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::UNIX_EPOCH;

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("space-downloads-{name}-{nanos}"))
    }

    #[test]
    fn scans_installers_and_large_items_excluding_archives_without_auto_selection() {
        let dir = unique_dir("scan");
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join(".hidden.dmg"), vec![b'h'; 200]).unwrap();
        fs::write(dir.join("notes.txt"), vec![b't'; 50]).unwrap();
        fs::write(dir.join("installer.dmg"), vec![b'd'; 200]).unwrap();
        fs::write(dir.join("archive.tar.gz"), vec![b'z'; 150]).unwrap();
        fs::write(dir.join("backup.zip"), vec![b'z'; 20_000]).unwrap();
        fs::write(dir.join("video.mp4"), vec![b'v'; 20_000]).unwrap();

        let pkg = dir.join("Tool.pkg");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("Payload"), vec![b'p'; 300]).unwrap();

        let folder = dir.join("ISO-Unpacked");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("big.bin"), vec![b'b'; 20_000]).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("installer.dmg"), dir.join("alias.dmg")).unwrap();

        let items = scan_downloads_dir(&dir, 100, 16_384);
        let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"installer.dmg"));
        assert!(!names.contains(&"archive.tar.gz"));
        assert!(!names.contains(&"backup.zip"));
        assert!(names.contains(&"video.mp4"));
        assert!(names.contains(&"Tool.pkg"));
        assert!(names.contains(&"ISO-Unpacked"));
        assert!(!names.contains(&".hidden.dmg"));
        assert!(!names.contains(&"notes.txt"));
        assert!(!names.contains(&"alias.dmg"));

        let mut state = DownloadsState::default();
        state.set_items(items);
        assert_eq!(state.selected_count(), 0);
        assert_eq!(state.selected_bytes(), 0);
        assert!(state.has_installers());

        state.select_installers();
        let selected_names: Vec<_> = state.selected_items().into_iter().map(|i| i.name).collect();
        assert_eq!(selected_names.len(), 2);
        assert!(selected_names.contains(&"installer.dmg".to_string()));
        assert!(selected_names.contains(&"Tool.pkg".to_string()));

        // Deleting installer.dmg keeps Tool.pkg selected and drops installer.dmg.
        state.remove_paths(&[dir.join("installer.dmg")]);
        assert_eq!(state.selected_count(), 1);
        assert_eq!(state.selected_items()[0].name, "Tool.pkg");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn selection_version_tracks_changes() {
        let mut state = DownloadsState::default();
        let v0 = state.selection_version();
        state.set_items(vec![DownloadItem {
            path: PathBuf::from("/tmp/a.dmg"),
            name: "a.dmg".into(),
            bytes: 100,
            is_dir: false,
            kind: DownloadKind::Installer,
            age_secs: None,
        }]);
        let v1 = state.selection_version();
        assert_ne!(v1, v0);

        state.set_selected(0, true);
        let v2 = state.selection_version();
        assert_ne!(v2, v1);

        // No-op toggle leaves version unchanged.
        state.set_selected(0, true);
        assert_eq!(state.selection_version(), v2);

        state.clear_selection();
        let v3 = state.selection_version();
        assert_ne!(v3, v2);

        // Clearing an empty selection leaves version unchanged.
        state.clear_selection();
        assert_eq!(state.selection_version(), v3);

        state.remove_paths(&[PathBuf::from("/tmp/a.dmg")]);
        assert_ne!(state.selection_version(), v3);
    }
}
