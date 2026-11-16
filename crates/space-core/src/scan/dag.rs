//! Filesystem DAG primitives: directory cycle guard and deterministic hardlink deduplication.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Mutex;
use unicode_normalization::UnicodeNormalization;

use crate::tree::Node;

/// Maximum number of skipped directory paths captured for scan reporting.
pub(crate) const MAX_SKIPPED_ROOTS: usize = 32;

/// Files below this allocated size are rolled into one "· small files" node per folder.
pub(crate) const SMALL_FILE_THRESHOLD: u64 = 512 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    SystemNoise,
    UserExcluded,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SmallBucket {
    pub count: u64,
    pub apparent: u64,
    pub allocated: u64,
}

pub(crate) struct DirRec {
    pub id: usize,
    pub parent: usize,
    pub name: String,
    pub mtime_unix: u64,
}

pub(crate) struct FileRec {
    pub parent: usize,
    pub name: String,
    pub apparent: u64,
    pub allocated: u64,
    pub mtime_unix: u64,
    pub dev_ino: Option<(u64, u64)>,
}

#[derive(Default)]
pub(crate) struct WorkerShard {
    pub dirs: Vec<DirRec>,
    pub files: Vec<FileRec>,
    pub small_buckets: Vec<(usize, SmallBucket)>,
}

/// Sharded `(dev, ino)` visited-set preventing traversal cycles on firmlinks or bind mounts.
pub(crate) struct DirVisitDag {
    shards: [Mutex<HashSet<(u64, u64)>>; 16],
}

impl DirVisitDag {
    pub(crate) fn new(root_identity: Option<(u64, u64)>) -> Self {
        let dag = Self {
            shards: std::array::from_fn(|_| Mutex::new(HashSet::new())),
        };
        if let Some((dev, ino)) = root_identity {
            dag.visit(dev, ino);
        }
        dag
    }

    /// Returns `true` if `(dev, ino)` has not been visited yet.
    pub(crate) fn visit(&self, dev: u64, ino: u64) -> bool {
        let idx = ((dev.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ ino) & 15) as usize;
        let mut guard = self.shards[idx].lock().unwrap();
        guard.insert((dev, ino))
    }
}

/// Resolves hardlink ownership deterministically by canonical relative path so
/// parallel scans always assign `allocated` bytes to the same link edge.
pub(crate) fn resolve_hardlink_dag(files: &mut [FileRec], dir_nodes: &[Node]) -> (u64, u64) {
    let mut path_cache: HashMap<usize, String> = HashMap::new();
    path_cache.insert(0, String::new());

    let mut keyed: Vec<(String, usize)> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.dev_ino.is_some())
        .map(|(idx, f)| {
            let dir_rel = cached_dir_rel_path(f.parent, dir_nodes, &mut path_cache);
            let full = if dir_rel.is_empty() {
                f.name.clone()
            } else {
                format!("{dir_rel}/{}", f.name)
            };
            (full, idx)
        })
        .collect();

    let candidates = keyed.len() as u64;
    if candidates <= 1 {
        return (candidates, 0);
    }

    keyed.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut seen = HashSet::with_capacity(keyed.len());
    let mut deduped = 0u64;
    for (_, idx) in keyed {
        let Some(key) = files[idx].dev_ino else {
            continue;
        };
        if !seen.insert(key) {
            files[idx].allocated = 0;
            deduped += 1;
        }
    }
    (candidates, deduped)
}

fn cached_dir_rel_path(
    dir_id: usize,
    dir_nodes: &[Node],
    cache: &mut HashMap<usize, String>,
) -> String {
    if let Some(cached) = cache.get(&dir_id) {
        return cached.clone();
    }
    let mut parts = Vec::new();
    let mut cur = dir_id;
    while cur > 0 {
        let Some(node) = dir_nodes.get(cur) else {
            break;
        };
        parts.push(node.name.as_str());
        cur = node.parent.map_or(0, |p| p.0 as usize);
    }
    parts.reverse();
    let joined = parts.join("/");
    cache.insert(dir_id, joined.clone());
    joined
}

pub(crate) fn display_name(name: &OsStr) -> String {
    // macOS HFS+/APFS often stores NFD; NFC renders correctly in UI fonts.
    let name = name.to_string_lossy();
    if name.is_ascii() {
        name.into_owned()
    } else {
        name.nfc().collect()
    }
}

pub(crate) fn should_skip_name(name: &str) -> bool {
    matches!(
        name,
        ".Spotlight-V100"
            | ".fseventsd"
            | ".TemporaryItems"
            | ".Trashes"
            | ".Trash"
            | ".DocumentRevisions-V100"
            | ".MobileBackups"
            | ".PKInstallSandboxManager"
            | ".OverProvisioning"
            | ".overprovisioning"
            | "Backups.backupdb"
            | ".vol"
            | ".hotfiles.btree"
            | "lost+found"
            | "$RECYCLE.BIN"
            | "System Volume Information"
            | ".AppleDouble"
            | ".AppleDB"
            | ".AppleDesktop"
            | "Network Trash Folder"
            | "Temporary Items"
    )
}

pub(crate) fn should_skip_entry(
    name: &str,
    skip_hidden: bool,
    exclude_set: &HashSet<String>,
) -> Option<SkipReason> {
    if should_skip_name(name) {
        return Some(SkipReason::SystemNoise);
    }
    if skip_hidden && name.starts_with('.') {
        return Some(SkipReason::UserExcluded);
    }
    if exclude_set.contains(name) {
        return Some(SkipReason::UserExcluded);
    }
    None
}

pub(crate) fn is_fatal_device_error(err: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(err.raw_os_error(), Some(libc::ENXIO) | Some(libc::EIO))
    }
    #[cfg(not(unix))]
    {
        let _ = err;
        false
    }
}

pub(crate) fn device_id(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

pub(crate) fn dir_identity(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

pub(crate) fn unix_link_identity(meta: &std::fs::Metadata) -> (u64, Option<(u64, u64)>) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (meta.nlink(), Some((meta.dev(), meta.ino())))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        (1, None)
    }
}

pub(crate) fn allocated_bytes(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        meta.len()
    }
}

pub(crate) fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime().max(0) as u64
    }
    #[cfg(not(unix))]
    {
        use std::time::UNIX_EPOCH;
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    }
}

pub(crate) fn normalize_root_display(root: &Path) -> String {
    root.file_name()
        .map(display_name)
        .unwrap_or_else(|| root.display().to_string().nfc().collect())
}
