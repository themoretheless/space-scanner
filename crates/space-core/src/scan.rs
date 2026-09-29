//! High-throughput parallel disk scanner (work-stealing, same-volume, skiplist).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{unbounded, Receiver, Sender};
use crossbeam_deque::{Injector, Steal, Worker};
use crossbeam_utils::thread::scope;
use unicode_normalization::UnicodeNormalization;

use crate::tree::{Node, NodeId, SpaceTree};

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub current_path: String,
    pub skipped_errors: u64,
}

#[derive(Debug)]
pub enum ScanEvent {
    Progress(ScanProgress),
    Done(Box<SpaceTree>),
    Cancelled,
    Error(String),
}

pub struct ScanHandle {
    pub events: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Entry {
    parent: Option<usize>,
    name: String,
    is_dir: bool,
    synthetic: bool,
    apparent: u64,
    allocated: u64,
    file_count: u64,
    children: Vec<usize>,
}

struct DirTask {
    path: PathBuf,
    id: usize,
}

/// Flat record emitted by workers; directory IDs are assigned before enqueueing.
struct Rec {
    parent: usize,
    id: Option<usize>,
    name: String,
    is_dir: bool,
    synthetic: bool,
    apparent: u64,
    allocated: u64,
    file_count: u64,
}

struct Shared {
    cancel: Arc<AtomicBool>,
    root: PathBuf,
    allowed_devs: HashSet<u64>,
    skip_volumes_prefix: bool,
    next_dir: AtomicUsize,
    files_seen: AtomicU64,
    bytes_seen: AtomicU64,
    skipped_errors: AtomicU64,
    hardlink_candidates: AtomicU64,
    hardlinks_deduped: AtomicU64,
    last_emit: AtomicU64,
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
    tx: Sender<ScanEvent>,
    /// Sharded record buffers — one per worker, merged at end.
    shards: Vec<Mutex<Vec<Rec>>>,
}

pub fn scan_path(root: impl Into<PathBuf>) -> ScanHandle {
    scan_path_with_expect(root, None)
}

pub fn scan_path_with_expect(root: impl Into<PathBuf>, _expected_items: Option<u64>) -> ScanHandle {
    let root = root.into();
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_bg = Arc::clone(&cancel);

    thread::Builder::new()
        .name("space-scan".into())
        .spawn(move || match run_scan(root, tx.clone(), cancel_bg) {
            Ok(()) => {}
            Err(ScanFail::Cancelled) => {
                let _ = tx.send(ScanEvent::Cancelled);
            }
            Err(ScanFail::Error(err)) => {
                let _ = tx.send(ScanEvent::Error(err));
            }
        })
        .expect("spawn scan thread");

    ScanHandle { events: rx, cancel }
}

enum ScanFail {
    Cancelled,
    Error(String),
}

fn run_scan(root: PathBuf, tx: Sender<ScanEvent>, cancel: Arc<AtomicBool>) -> Result<(), ScanFail> {
    if !root.exists() {
        return Err(ScanFail::Error(format!(
            "path does not exist: {}",
            root.display()
        )));
    }

    let root_meta = std::fs::metadata(&root).map_err(|e| ScanFail::Error(e.to_string()))?;
    let root_dev = device_id(&root_meta);
    let mut allowed_devs = HashSet::new();
    allowed_devs.insert(root_dev);
    // macOS: System + Data often share semantics; include Data if present
    if let Ok(m) = std::fs::metadata("/System/Volumes/Data") {
        allowed_devs.insert(device_id(&m));
    }

    // The eight-worker cap outperformed caps of four and sixteen on the
    // warm macOS metadata fixtures; extra workers add contention and overhead.
    let max_threads = if cfg!(target_os = "macos") { 8 } else { 16 };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, max_threads);

    let mut shards = Vec::with_capacity(threads);
    for _ in 0..threads {
        shards.push(Mutex::new(Vec::with_capacity(65_536)));
    }

    let skip_volumes_prefix = {
        let s = root.to_string_lossy();
        !(s.starts_with("/Volumes/") || root == Path::new("/Volumes"))
    };

    let shared = Arc::new(Shared {
        cancel,
        root: root.clone(),
        allowed_devs,
        skip_volumes_prefix,
        next_dir: AtomicUsize::new(1),
        files_seen: AtomicU64::new(0),
        bytes_seen: AtomicU64::new(0),
        skipped_errors: AtomicU64::new(0),
        hardlink_candidates: AtomicU64::new(0),
        hardlinks_deduped: AtomicU64::new(0),
        last_emit: AtomicU64::new(0),
        seen_inodes: Mutex::new(HashSet::with_capacity(4096)),
        tx: tx.clone(),
        shards,
    });

    let injector = Arc::new(Injector::<DirTask>::new());
    injector.push(DirTask {
        path: root.clone(),
        id: 0,
    });

    let pending = Arc::new(AtomicUsize::new(1)); // root queued

    let mut worker_stealers = Vec::with_capacity(threads);
    let mut local_workers = Vec::with_capacity(threads);
    for _ in 0..threads {
        let w = Worker::<DirTask>::new_fifo();
        worker_stealers.push(w.stealer());
        local_workers.push(Some(w));
    }
    let stealers = Arc::new(worker_stealers);

    let result = scope(|scope| {
        for (wid, slot) in local_workers.iter_mut().enumerate() {
            let worker = slot.take().unwrap();
            let shared = Arc::clone(&shared);
            let injector = Arc::clone(&injector);
            let stealers = Arc::clone(&stealers);
            let pending = Arc::clone(&pending);
            scope.spawn(move |_| {
                worker_loop(wid, worker, shared, injector, stealers, pending);
            });
        }
    });

    if let Err(e) = result {
        return Err(ScanFail::Error(format!("scan workers panicked: {e:?}")));
    }

    if shared.cancel.load(Ordering::Relaxed) {
        return Err(ScanFail::Cancelled);
    }

    // Merge shards
    let mut all = Vec::new();
    for shard in &shared.shards {
        all.append(&mut shard.lock().unwrap());
    }

    let tree = build_tree(&root, all, &shared)?;
    let _ = tx.send(ScanEvent::Done(Box::new(tree)));
    Ok(())
}

/// Files below this allocated size are rolled into one "· small files" node per folder.
const SMALL_FILE_THRESHOLD: u64 = 512 * 1024;

fn worker_loop(
    wid: usize,
    local: Worker<DirTask>,
    shared: Arc<Shared>,
    injector: Arc<Injector<DirTask>>,
    stealers: Arc<Vec<crossbeam_deque::Stealer<DirTask>>>,
    pending: Arc<AtomicUsize>,
) {
    let mut buf: Vec<Rec> = Vec::with_capacity(8192);

    loop {
        if shared.cancel.load(Ordering::Relaxed) {
            flush_buf(wid, &shared, &mut buf);
            return;
        }

        let dir = match find_task(&local, &injector, &stealers) {
            Some(d) => d,
            None => {
                if pending.load(Ordering::Acquire) == 0 {
                    flush_buf(wid, &shared, &mut buf);
                    return;
                }
                std::thread::sleep(Duration::from_micros(50));
                if pending.load(Ordering::Acquire) == 0 {
                    flush_buf(wid, &shared, &mut buf);
                    return;
                }
                continue;
            }
        };

        scan_dir(&dir, &shared, &injector, &pending, &mut buf);
        pending.fetch_sub(1, Ordering::Release);

        if buf.len() >= 8192 {
            flush_buf(wid, &shared, &mut buf);
        }
    }
}

fn find_task(
    local: &Worker<DirTask>,
    injector: &Injector<DirTask>,
    stealers: &[crossbeam_deque::Stealer<DirTask>],
) -> Option<DirTask> {
    local.pop().or_else(|| match injector.steal() {
        Steal::Success(v) => Some(v),
        _ => {
            for s in stealers {
                loop {
                    match s.steal() {
                        Steal::Success(v) => return Some(v),
                        Steal::Empty => break,
                        Steal::Retry => continue,
                    }
                }
            }
            None
        }
    })
}

fn flush_buf(wid: usize, shared: &Shared, buf: &mut Vec<Rec>) {
    if buf.is_empty() {
        return;
    }
    let mut g = shared.shards[wid].lock().unwrap();
    g.append(buf);
}

fn scan_dir(
    task: &DirTask,
    shared: &Shared,
    injector: &Injector<DirTask>,
    pending: &AtomicUsize,
    buf: &mut Vec<Rec>,
) {
    let dir = &task.path;
    // Decide once per directory instead of constructing every child path.
    if shared.skip_volumes_prefix
        && dir.starts_with("/Volumes")
        && !shared.root.starts_with("/Volumes")
    {
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => {
            shared.skipped_errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    let mut progress_files = 0u64;
    let mut progress_bytes = 0u64;
    let mut small_count = 0u64;
    let mut small_apparent = 0u64;
    let mut small_allocated = 0u64;
    for ent in rd {
        if shared.cancel.load(Ordering::Relaxed) {
            return;
        }
        let ent = match ent {
            Ok(e) => e,
            Err(_) => {
                shared.skipped_errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let name = ent.file_name();
        if name.to_str().is_some_and(should_skip_name) {
            continue;
        }

        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => {
                shared.skipped_errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        if ft.is_symlink() {
            // Do not follow symlinks (avoids cycles / foreign trees)
            continue;
        }

        if ft.is_dir() {
            let meta = match ent.metadata() {
                Ok(m) => m,
                Err(_) => {
                    shared.skipped_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            let dev = device_id(&meta);
            if !shared.allowed_devs.contains(&dev) {
                continue;
            }
            let id = shared.next_dir.fetch_add(1, Ordering::Relaxed);
            buf.push(Rec {
                parent: task.id,
                id: Some(id),
                name: display_name(&name),
                is_dir: true,
                synthetic: false,
                apparent: 0,
                allocated: 0,
                file_count: 0,
            });
            pending.fetch_add(1, Ordering::Release);
            injector.push(DirTask {
                path: ent.path(),
                id,
            });
        } else if ft.is_file() {
            let meta = match ent.metadata() {
                Ok(m) => m,
                Err(_) => {
                    shared.skipped_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            let apparent = meta.len();
            let mut allocated = allocated_bytes(&meta);
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() > 1 {
                    shared.hardlink_candidates.fetch_add(1, Ordering::Relaxed);
                    let key = (meta.dev(), meta.ino());
                    let mut g = shared.seen_inodes.lock().unwrap();
                    if !g.insert(key) {
                        shared.hardlinks_deduped.fetch_add(1, Ordering::Relaxed);
                        allocated = 0;
                    }
                }
            }
            if allocated < SMALL_FILE_THRESHOLD && apparent < SMALL_FILE_THRESHOLD {
                small_count += 1;
                small_apparent = small_apparent.saturating_add(apparent);
                small_allocated = small_allocated.saturating_add(allocated);
            } else {
                buf.push(Rec {
                    parent: task.id,
                    id: None,
                    name: display_name(&name),
                    is_dir: false,
                    synthetic: false,
                    apparent,
                    allocated,
                    file_count: 1,
                });
            }
            progress_files += 1;
            progress_bytes += allocated;
            if progress_files == 256 {
                publish_progress(shared, progress_files, progress_bytes, dir);
                progress_files = 0;
                progress_bytes = 0;
            }
        }
        // else: sockets/fifos/devices — ignore
    }
    publish_progress(shared, progress_files, progress_bytes, dir);
    if small_count > 0 {
        buf.push(Rec {
            parent: task.id,
            id: None,
            name: format!("· {small_count} small files"),
            is_dir: false,
            synthetic: true,
            apparent: small_apparent,
            allocated: small_allocated,
            file_count: small_count,
        });
    }
}

fn publish_progress(shared: &Shared, files: u64, bytes: u64, path: &Path) {
    if files == 0 {
        return;
    }
    shared.bytes_seen.fetch_add(bytes, Ordering::Relaxed);
    let n = shared.files_seen.fetch_add(files, Ordering::Relaxed) + files;
    maybe_progress(shared, n, path);
}

fn maybe_progress(shared: &Shared, n: u64, path: &Path) {
    let prev = shared.last_emit.load(Ordering::Relaxed);
    if n.saturating_sub(prev) >= 8_000
        && shared
            .last_emit
            .compare_exchange(prev, n, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let _ = shared.tx.send(ScanEvent::Progress(ScanProgress {
            files_seen: n,
            bytes_seen: shared.bytes_seen.load(Ordering::Relaxed),
            current_path: path.display().to_string(),
            skipped_errors: shared.skipped_errors.load(Ordering::Relaxed),
        }));
    }
}

fn display_name(name: &std::ffi::OsStr) -> String {
    // macOS HFS+/APFS often stores NFD; NFC renders correctly in UI fonts.
    let name = name.to_string_lossy();
    if name.is_ascii() {
        name.into_owned()
    } else {
        name.nfc().collect()
    }
}

fn should_skip_name(name: &str) -> bool {
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

fn build_tree(root: &Path, recs: Vec<Rec>, shared: &Shared) -> Result<SpaceTree, ScanFail> {
    let root_name = root
        .file_name()
        .map(display_name)
        .unwrap_or_else(|| root.display().to_string().nfc().collect());

    let mut entries: Vec<Entry> = Vec::with_capacity(recs.len() + 1);
    entries.push(Entry {
        parent: None,
        name: root_name,
        is_dir: true,
        synthetic: false,
        apparent: 0,
        allocated: 0,
        file_count: 0,
        children: Vec::new(),
    });

    let (mut directories, files): (Vec<_>, Vec<_>) =
        recs.into_iter().partition(|record| record.is_dir);
    // IDs are dense and every parent is assigned before its children. Keeping
    // that order also makes the reverse aggregation pass valid.
    directories.sort_unstable_by_key(|record| record.id.unwrap());
    for record in directories.into_iter().chain(files) {
        let idx = entries.len();
        if let Some(id) = record.id {
            debug_assert_eq!(idx, id);
        }
        entries.push(Entry {
            parent: Some(record.parent),
            name: record.name,
            is_dir: record.is_dir,
            synthetic: record.synthetic,
            apparent: record.apparent,
            allocated: record.allocated,
            file_count: record.file_count,
            children: Vec::new(),
        });
        entries[record.parent].children.push(idx);
    }

    for i in (0..entries.len()).rev() {
        if entries[i].is_dir {
            let mut apparent = 0u64;
            let mut allocated = 0u64;
            let mut files = 0u64;
            for &c in &entries[i].children {
                apparent += entries[c].apparent;
                allocated += entries[c].allocated;
                files += entries[c].file_count;
            }
            entries[i].apparent = apparent;
            entries[i].allocated = allocated;
            entries[i].file_count = files;
        }
    }

    for i in 0..entries.len() {
        if !entries[i].is_dir {
            continue;
        }
        let mut kids = std::mem::take(&mut entries[i].children);
        kids.sort_by(|&a, &b| {
            entries[b]
                .allocated
                .cmp(&entries[a].allocated)
                .then_with(|| entries[a].name.cmp(&entries[b].name))
        });
        entries[i].children = kids;
    }

    let nodes: Vec<Node> = entries
        .into_iter()
        .map(|e| Node {
            name: e.name,
            apparent_size: e.apparent,
            allocated_size: e.allocated,
            file_count: e.file_count,
            is_dir: e.is_dir,
            synthetic: e.synthetic,
            children: e.children.into_iter().map(|c| NodeId(c as u32)).collect(),
            parent: e.parent.map(|p| NodeId(p as u32)),
        })
        .collect();

    let item_count = nodes.len() as u64;
    let scanned_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let tree = SpaceTree {
        root_path: root.display().to_string(),
        nodes,
        root: NodeId(0),
        scanned_at_unix,
        item_count,
        skipped_errors: shared.skipped_errors.load(Ordering::Relaxed),
        hardlink_candidates: shared.hardlink_candidates.load(Ordering::Relaxed),
        hardlinks_deduped: shared.hardlinks_deduped.load(Ordering::Relaxed),
    };
    Ok(tree)
}

fn device_id(meta: &std::fs::Metadata) -> u64 {
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

fn allocated_bytes(meta: &std::fs::Metadata) -> u64 {
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

pub fn rescan_subtree(tree: &SpaceTree, dir_id: NodeId) -> Result<SpaceTree, String> {
    let Some(dir) = tree.try_node(dir_id) else {
        return Err("rescan node does not exist".into());
    };
    if !dir.is_dir {
        return Err("rescan node is not a directory".into());
    }
    let root = PathBuf::from(&tree.root_path);
    if dir_id == tree.root || tree.hardlink_candidates > 0 {
        return scan_blocking(root);
    }

    let subtree_root = PathBuf::from(tree.path_string(dir_id));
    let scanned = scan_blocking(subtree_root)?;
    // A hardlink can have another name outside this subtree. Without scanning
    // that owner, independently deduping this subtree could overcount bytes.
    if scanned.hardlink_candidates > 0 {
        return scan_blocking(root);
    }
    Ok(splice_subtree(tree, dir_id, scanned))
}

fn scan_blocking(root: PathBuf) -> Result<SpaceTree, String> {
    let handle = scan_path(root);
    loop {
        match handle.events.recv() {
            Ok(ScanEvent::Done(t)) => return Ok(*t),
            Ok(ScanEvent::Cancelled) => return Err("cancelled".into()),
            Ok(ScanEvent::Error(e)) => return Err(e),
            Ok(ScanEvent::Progress(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn splice_subtree(tree: &SpaceTree, dir_id: NodeId, scanned: SpaceTree) -> SpaceTree {
    let old = tree.node(dir_id);
    let old_parent = old.parent.expect("non-root subtree has a parent");
    let old_apparent = old.apparent_size;
    let old_allocated = old.allocated_size;
    let old_files = old.file_count;
    let original_name = old.name.clone();

    let mut merged = tree.clone();
    let append_base = merged.nodes.len() as u32;
    let mut mapping = Vec::with_capacity(scanned.nodes.len());
    mapping.push(dir_id);
    for index in 1..scanned.nodes.len() {
        mapping.push(NodeId(append_base + index as u32 - 1));
    }

    let scanned_at_unix = scanned.scanned_at_unix;
    let scanned_errors = scanned.skipped_errors;
    for (index, mut node) in scanned.nodes.into_iter().enumerate() {
        for child in &mut node.children {
            *child = mapping[child.0 as usize];
        }
        if index == 0 {
            node.name.clone_from(&original_name);
            node.parent = Some(old_parent);
            merged.nodes[dir_id.0 as usize] = node;
        } else {
            node.parent = node.parent.map(|parent| mapping[parent.0 as usize]);
            merged.nodes.push(node);
        }
    }

    let replacement = merged.node(dir_id);
    let new_apparent = replacement.apparent_size;
    let new_allocated = replacement.allocated_size;
    let new_files = replacement.file_count;
    let mut current = Some(old_parent);
    while let Some(id) = current {
        let node = merged.node_mut(id);
        node.apparent_size = node
            .apparent_size
            .saturating_sub(old_apparent)
            .saturating_add(new_apparent);
        node.allocated_size = node
            .allocated_size
            .saturating_sub(old_allocated)
            .saturating_add(new_allocated);
        node.file_count = node
            .file_count
            .saturating_sub(old_files)
            .saturating_add(new_files);
        current = node.parent;
    }
    merged.scanned_at_unix = scanned_at_unix;
    merged.skipped_errors = merged.skipped_errors.saturating_add(scanned_errors);
    compact_tree(merged)
}

fn compact_tree(tree: SpaceTree) -> SpaceTree {
    let mut order = Vec::with_capacity(tree.item_count as usize);
    let mut stack = vec![tree.root];
    let mut mapping = vec![None; tree.nodes.len()];
    while let Some(old_id) = stack.pop() {
        if mapping[old_id.0 as usize].is_some() {
            continue;
        }
        let new_id = NodeId(order.len() as u32);
        mapping[old_id.0 as usize] = Some(new_id);
        order.push(old_id);
        for child in tree.node(old_id).children.iter().rev() {
            stack.push(*child);
        }
    }
    let SpaceTree {
        root_path,
        nodes: old_nodes,
        scanned_at_unix,
        skipped_errors,
        hardlink_candidates,
        hardlinks_deduped,
        ..
    } = tree;
    let mut old_nodes: Vec<Option<Node>> = old_nodes.into_iter().map(Some).collect();
    let mut nodes = Vec::with_capacity(order.len());
    for old_id in order {
        let mut node = old_nodes[old_id.0 as usize]
            .take()
            .expect("reachable node appears once");
        for child in &mut node.children {
            *child = mapping[child.0 as usize].expect("reachable child is mapped");
        }
        node.parent = node.parent.and_then(|parent| mapping[parent.0 as usize]);
        nodes.push(node);
    }
    SpaceTree {
        root_path,
        item_count: nodes.len() as u64,
        nodes,
        root: NodeId(0),
        scanned_at_unix,
        skipped_errors,
        hardlink_candidates,
        hardlinks_deduped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn display_names_are_normalized_once_at_ingest() {
        let nfd = std::ffi::OsStr::new("e\u{301}");
        assert_eq!(display_name(nfd), "é");
    }

    #[test]
    fn scans_nested_tree_and_sums_sizes() {
        let dir = std::env::temp_dir().join(format!("space-scanner-ws-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::create_dir_all(dir.join("c")).unwrap();
        fs::File::create(dir.join("a/b/file.bin"))
            .unwrap()
            .write_all(&[0u8; 1024])
            .unwrap();
        fs::File::create(dir.join("c/x.bin"))
            .unwrap()
            .write_all(&[2u8; 2048])
            .unwrap();
        fs::File::create(dir.join("root.txt"))
            .unwrap()
            .write_all(&[1u8; 512])
            .unwrap();

        let handle = scan_path(&dir);
        let tree = loop {
            match handle.events.recv_timeout(Duration::from_secs(30)) {
                Ok(ScanEvent::Done(t)) => break t,
                Ok(ScanEvent::Progress(_)) => continue,
                Ok(ScanEvent::Cancelled) => panic!("cancelled"),
                Ok(ScanEvent::Error(e)) => panic!("{e}"),
                Err(e) => panic!("timeout/recv: {e}"),
            }
        };

        assert!(tree.total_size() >= 1536 + 2048);
        assert!(tree.item_count >= 5);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_counted_once_on_disk() {
        let dir = std::env::temp_dir().join(format!("space-scanner-hl2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        fs::File::create(&a)
            .unwrap()
            .write_all(&[7u8; 4096])
            .unwrap();
        let _ = fs::remove_file(&b);
        std::fs::hard_link(&a, &b).unwrap();

        let handle = scan_path(&dir);
        let tree = loop {
            match handle.events.recv_timeout(Duration::from_secs(30)) {
                Ok(ScanEvent::Done(t)) => break t,
                Ok(ScanEvent::Progress(_)) => continue,
                Ok(ScanEvent::Cancelled) => panic!("cancelled"),
                Ok(ScanEvent::Error(e)) => panic!("{e}"),
                Err(e) => panic!("{e}"),
            }
        };

        assert!(tree.node(tree.root).allocated_size < 4096 * 2);
        assert_eq!(tree.hardlink_candidates, 2);
        assert!(tree.hardlinks_deduped >= 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parallel_scan_preserves_hierarchy_and_exact_totals() {
        let dir = std::env::temp_dir().join(format!("space-scanner-ids-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut apparent = 0;
        let mut allocated = 0;
        for i in 0..64 {
            // Decomposed Unicode names must not create duplicate directories
            // when workers emit parent and child records in different orders.
            let leaf = dir.join(format!("e\u{301}-{i}/nested/empty"));
            fs::create_dir_all(&leaf).unwrap();
            for (name, size) in [("small", 17), ("large", SMALL_FILE_THRESHOLD + 1)] {
                let path = leaf.parent().unwrap().join(name);
                fs::File::create(&path).unwrap().set_len(size).unwrap();
                let meta = fs::metadata(&path).unwrap();
                apparent += meta.len();
                allocated += allocated_bytes(&meta);
            }
        }
        for _ in 0..3 {
            let tree = scan_blocking(dir.clone()).unwrap();
            assert_eq!(tree.node(tree.root).file_count, 128);
            assert_eq!(tree.node(tree.root).apparent_size, apparent);
            assert_eq!(tree.node(tree.root).allocated_size, allocated);
            assert_eq!(tree.nodes.len(), 1 + 64 * 5);
            assert_eq!(tree.children(tree.root).len(), 64);
            for (i, node) in tree.nodes.iter().enumerate() {
                for &child in &node.children {
                    assert_eq!(tree.node(child).parent, Some(NodeId(i as u32)));
                }
                if node.is_dir {
                    assert_eq!(
                        node.file_count,
                        node.children
                            .iter()
                            .map(|&id| tree.node(id).file_count)
                            .sum::<u64>()
                    );
                    assert_eq!(
                        node.apparent_size,
                        node.children
                            .iter()
                            .map(|&id| tree.node(id).apparent_size)
                            .sum::<u64>()
                    );
                }
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlink_cycles_and_excluded_directories() {
        let dir = std::env::temp_dir().join(format!("space-scanner-skip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".Trash")).unwrap();
        fs::write(dir.join(".Trash/ignored"), b"ignored").unwrap();
        fs::write(dir.join("kept"), b"kept").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("cycle")).unwrap();
        let tree = scan_blocking(dir.clone()).unwrap();
        assert_eq!(tree.node(tree.root).file_count, 1);
        assert_eq!(tree.node(tree.root).apparent_size, 4);
        assert_eq!(tree.nodes.len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn incremental_rescan_replaces_only_the_selected_subtree() {
        let dir =
            std::env::temp_dir().join(format!("space-scanner-incremental-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/one"), b"1").unwrap();
        fs::write(dir.join("b/one"), b"1").unwrap();

        let tree = scan_blocking(dir.clone()).unwrap();
        let a = tree
            .children(tree.root)
            .iter()
            .copied()
            .find(|id| tree.node(*id).name == "a")
            .unwrap();
        fs::write(dir.join("a/two"), b"22").unwrap();
        let rescanned = rescan_subtree(&tree, a).unwrap();

        let rescanned_a = rescanned
            .children(rescanned.root)
            .iter()
            .copied()
            .find(|id| rescanned.node(*id).name == "a")
            .unwrap();
        assert_eq!(rescanned.node(rescanned_a).file_count, 2);
        assert_eq!(rescanned.node(rescanned.root).file_count, 3);
        assert_eq!(rescanned.nodes.len() as u64, rescanned.item_count);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn incremental_rescan_falls_back_when_hardlinks_require_global_dedupe() {
        let dir = std::env::temp_dir().join(format!(
            "space-scanner-incremental-hardlink-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/one"), b"1").unwrap();
        fs::write(dir.join("b/link-one"), b"linked").unwrap();
        fs::hard_link(dir.join("b/link-one"), dir.join("b/link-two")).unwrap();

        let tree = scan_blocking(dir.clone()).unwrap();
        assert!(tree.hardlink_candidates > 0);
        let a = tree
            .children(tree.root)
            .iter()
            .copied()
            .find(|id| tree.node(*id).name == "a")
            .unwrap();
        // This outside-subtree change is observed only by the exact full-scan
        // fallback required for global hardlink ownership.
        fs::write(dir.join("b/new"), b"new").unwrap();
        let rescanned = rescan_subtree(&tree, a).unwrap();
        assert_eq!(rescanned.node(rescanned.root).file_count, 4);
        let _ = fs::remove_dir_all(&dir);
    }
}
