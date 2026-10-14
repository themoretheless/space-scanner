//! High-throughput parallel disk scanner (work-stealing, same-volume, skiplist).

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use crossbeam_utils::thread::scope;
use crossbeam_utils::Backoff;
use unicode_normalization::UnicodeNormalization;

use crate::now_unix;
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

/// Handle to a background scan. Dropping it cancels the scan.
pub struct ScanHandle {
    pub events: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Block until the scan finishes, discarding progress events.
    pub fn wait(self) -> Result<SpaceTree, String> {
        self.wait_with(|_| {})
    }

    /// Block until the scan finishes, reporting every progress event.
    pub fn wait_with(self, mut on_progress: impl FnMut(&ScanProgress)) -> Result<SpaceTree, String> {
        loop {
            match self.events.recv() {
                Ok(ScanEvent::Done(tree)) => return Ok(*tree),
                Ok(ScanEvent::Progress(progress)) => on_progress(&progress),
                Ok(ScanEvent::Cancelled) => return Err("cancelled".into()),
                Ok(ScanEvent::Error(err)) => return Err(err),
                Err(_) => return Err("scan ended without a result".into()),
            }
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Default)]
struct ScanOptions {
    /// Device ids the scan may descend into; derived from the root when unset.
    allowed_devs: Option<HashSet<u64>>,
}

struct DirTask {
    path: PathBuf,
    id: u32,
}

/// One hardlinked file as observed by a worker. Ownership (which name keeps
/// the allocated bytes) is decided deterministically after the merge.
#[derive(Clone, Copy)]
struct HardLink {
    dev: u64,
    ino: u64,
    allocated: u64,
}

/// Flat record emitted by workers; directory ids are assigned before enqueueing.
struct Rec {
    parent: u32,
    /// Dense directory id (unused for files).
    id: u32,
    name: String,
    raw_name: Option<Vec<u8>>,
    synthetic: bool,
    apparent: u64,
    allocated: u64,
    file_count: u64,
    links: Vec<HardLink>,
}

#[derive(Default)]
struct Shard {
    dirs: Vec<Rec>,
    files: Vec<Rec>,
}

struct Shared {
    cancel: Arc<AtomicBool>,
    allowed_devs: HashSet<u64>,
    skip_volumes_prefix: bool,
    next_dir: AtomicU32,
    id_overflow: AtomicBool,
    files_seen: AtomicU64,
    bytes_seen: AtomicU64,
    skipped_errors: AtomicU64,
    hardlink_candidates: AtomicU64,
    last_emit: AtomicU64,
    tx: Sender<ScanEvent>,
    /// Sharded record buffers, one per worker, merged at the end.
    shards: Vec<Mutex<Shard>>,
}

pub fn scan_path(root: impl Into<PathBuf>) -> ScanHandle {
    scan_path_with_expect(root, None)
}

pub fn scan_path_with_expect(root: impl Into<PathBuf>, _expected_items: Option<u64>) -> ScanHandle {
    spawn_scan(root.into(), ScanOptions::default())
}

fn spawn_scan(root: PathBuf, options: ScanOptions) -> ScanHandle {
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_bg = Arc::clone(&cancel);

    let spawned = thread::Builder::new().name("space-scan".into()).spawn({
        let tx = tx.clone();
        move || match run_scan(root, options, &tx, cancel_bg) {
            Ok(()) => {}
            Err(ScanFail::Cancelled) => {
                let _ = tx.send(ScanEvent::Cancelled);
            }
            Err(ScanFail::Error(err)) => {
                let _ = tx.send(ScanEvent::Error(err));
            }
        }
    });
    if let Err(err) = spawned {
        let _ = tx.send(ScanEvent::Error(format!("cannot spawn scan thread: {err}")));
    }

    ScanHandle { events: rx, cancel }
}

enum ScanFail {
    Cancelled,
    Error(String),
}

/// Devices a scan rooted at `root_meta` may descend into.
fn allowed_devices(root_meta: &std::fs::Metadata) -> HashSet<u64> {
    let mut devs = HashSet::with_capacity(2);
    devs.insert(device_id(root_meta));
    // macOS: the System and Data volumes present as one tree.
    #[cfg(target_os = "macos")]
    if let Ok(m) = std::fs::metadata("/System/Volumes/Data") {
        devs.insert(device_id(&m));
    }
    devs
}

fn run_scan(
    root: PathBuf,
    options: ScanOptions,
    tx: &Sender<ScanEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<(), ScanFail> {
    let root_meta = std::fs::metadata(&root)
        .map_err(|e| ScanFail::Error(format!("cannot read {}: {e}", root.display())))?;
    if !root_meta.is_dir() {
        return Err(ScanFail::Error(format!(
            "not a directory: {}",
            root.display()
        )));
    }
    // Fail loudly instead of returning an empty tree for an unreadable root.
    std::fs::read_dir(&root)
        .map_err(|e| ScanFail::Error(format!("cannot read directory {}: {e}", root.display())))?;

    let scanned_at_unix = now_unix();
    let allowed_devs = options
        .allowed_devs
        .unwrap_or_else(|| allowed_devices(&root_meta));

    // The eight-worker cap outperformed caps of four and sixteen on the
    // warm macOS metadata fixtures; extra workers add contention and overhead.
    let max_threads = if cfg!(target_os = "macos") { 8 } else { 16 };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, max_threads);

    let shards = (0..threads).map(|_| Mutex::new(Shard::default())).collect();

    // Mounted volumes are separate trees unless the scan starts inside them.
    let skip_volumes_prefix = !root.starts_with("/Volumes");

    let shared = Arc::new(Shared {
        cancel,
        allowed_devs,
        skip_volumes_prefix,
        next_dir: AtomicU32::new(1),
        id_overflow: AtomicBool::new(false),
        files_seen: AtomicU64::new(0),
        bytes_seen: AtomicU64::new(0),
        skipped_errors: AtomicU64::new(0),
        hardlink_candidates: AtomicU64::new(0),
        last_emit: AtomicU64::new(0),
        tx: tx.clone(),
        shards,
    });

    let injector = Injector::<DirTask>::new();
    injector.push(DirTask {
        path: root.clone(),
        id: 0,
    });

    let pending = AtomicU64::new(1); // root queued

    let mut stealers = Vec::with_capacity(threads);
    let mut local_workers = Vec::with_capacity(threads);
    for _ in 0..threads {
        let w = Worker::<DirTask>::new_fifo();
        stealers.push(w.stealer());
        local_workers.push(w);
    }

    let result = scope(|scope| {
        for (wid, worker) in local_workers.into_iter().enumerate() {
            let shared = Arc::clone(&shared);
            let injector = &injector;
            let stealers = &stealers;
            let pending = &pending;
            scope.spawn(move |_| {
                worker_loop(wid, worker, &shared, injector, stealers, pending);
            });
        }
    });

    if let Err(e) = result {
        return Err(ScanFail::Error(format!("scan workers panicked: {e:?}")));
    }

    if shared.cancel.load(Ordering::Relaxed) {
        return Err(ScanFail::Cancelled);
    }
    if shared.id_overflow.load(Ordering::Relaxed) {
        return Err(ScanFail::Error(
            "too many directories for a single tree (u32 id space exhausted)".into(),
        ));
    }

    let tree = build_tree(&root, &shared, scanned_at_unix)?;
    let _ = tx.send(ScanEvent::Done(Box::new(tree)));
    Ok(())
}

/// Files below this allocated size are rolled into one "· small files" node per folder.
const SMALL_FILE_THRESHOLD: u64 = 512 * 1024;

/// Flush local buffers to the shard once this many records accumulate.
const FLUSH_THRESHOLD: usize = 8192;

struct LocalBuf {
    dirs: Vec<Rec>,
    files: Vec<Rec>,
}

impl LocalBuf {
    fn len(&self) -> usize {
        self.dirs.len() + self.files.len()
    }
}

fn worker_loop(
    wid: usize,
    local: Worker<DirTask>,
    shared: &Shared,
    injector: &Injector<DirTask>,
    stealers: &[Stealer<DirTask>],
    pending: &AtomicU64,
) {
    let mut buf = LocalBuf {
        dirs: Vec::with_capacity(1024),
        files: Vec::with_capacity(FLUSH_THRESHOLD),
    };
    let backoff = Backoff::new();

    loop {
        if shared.cancel.load(Ordering::Relaxed) {
            break;
        }

        match find_task(&local, injector, stealers) {
            Some(dir) => {
                backoff.reset();
                scan_dir(&dir, shared, &local, pending, &mut buf);
                pending.fetch_sub(1, Ordering::Release);
                if buf.len() >= FLUSH_THRESHOLD {
                    flush_buf(wid, shared, &mut buf);
                }
            }
            None => {
                if pending.load(Ordering::Acquire) == 0 {
                    break;
                }
                if backoff.is_completed() {
                    // Bounded park: no unpark is needed, the timeout wakes us.
                    thread::park_timeout(Duration::from_micros(100));
                } else {
                    backoff.snooze();
                }
            }
        }
    }
    flush_buf(wid, shared, &mut buf);
}

fn find_task(
    local: &Worker<DirTask>,
    injector: &Injector<DirTask>,
    stealers: &[Stealer<DirTask>],
) -> Option<DirTask> {
    if let Some(task) = local.pop() {
        return Some(task);
    }
    loop {
        match injector.steal_batch_and_pop(local) {
            Steal::Success(task) => return Some(task),
            Steal::Empty => break,
            Steal::Retry => continue,
        }
    }
    for s in stealers {
        loop {
            match s.steal_batch_and_pop(local) {
                Steal::Success(task) => return Some(task),
                Steal::Empty => break,
                Steal::Retry => continue,
            }
        }
    }
    None
}

fn flush_buf(wid: usize, shared: &Shared, buf: &mut LocalBuf) {
    if buf.len() == 0 {
        return;
    }
    let mut shard = shared.shards[wid].lock().unwrap();
    shard.dirs.append(&mut buf.dirs);
    shard.files.append(&mut buf.files);
}

fn scan_dir(
    task: &DirTask,
    shared: &Shared,
    local: &Worker<DirTask>,
    pending: &AtomicU64,
    buf: &mut LocalBuf,
) {
    let dir = &task.path;
    // Decide once per directory instead of constructing every child path.
    if shared.skip_volumes_prefix && dir.starts_with("/Volumes") {
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
    let mut small_links: Vec<HardLink> = Vec::new();
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
            if !shared.allowed_devs.contains(&device_id(&meta)) {
                continue;
            }
            let id = shared.next_dir.fetch_add(1, Ordering::Relaxed);
            if id == u32::MAX {
                shared.id_overflow.store(true, Ordering::Relaxed);
                shared.cancel.store(true, Ordering::Relaxed);
                return;
            }
            let (display, raw_name) = split_name(&name);
            buf.dirs.push(Rec {
                parent: task.id,
                id,
                name: display,
                raw_name,
                synthetic: false,
                apparent: 0,
                allocated: 0,
                file_count: 0,
                links: Vec::new(),
            });
            pending.fetch_add(1, Ordering::Release);
            local.push(DirTask {
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
            let allocated = allocated_bytes(&meta);
            let link = hardlink_of(&meta, allocated);
            if link.is_some() {
                shared.hardlink_candidates.fetch_add(1, Ordering::Relaxed);
            }
            if allocated < SMALL_FILE_THRESHOLD && apparent < SMALL_FILE_THRESHOLD {
                small_count += 1;
                small_apparent = small_apparent.saturating_add(apparent);
                small_allocated = small_allocated.saturating_add(allocated);
                small_links.extend(link);
            } else {
                let (display, raw_name) = split_name(&name);
                buf.files.push(Rec {
                    parent: task.id,
                    id: 0,
                    name: display,
                    raw_name,
                    synthetic: false,
                    apparent,
                    allocated,
                    file_count: 1,
                    links: link.into_iter().collect(),
                });
            }
            progress_files += 1;
            progress_bytes = progress_bytes.saturating_add(allocated);
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
        buf.files.push(Rec {
            parent: task.id,
            id: 0,
            name: format!("· {small_count} small files"),
            raw_name: None,
            synthetic: true,
            apparent: small_apparent,
            allocated: small_allocated,
            file_count: small_count,
            links: small_links,
        });
    }
}

#[cfg(unix)]
fn hardlink_of(meta: &std::fs::Metadata, allocated: u64) -> Option<HardLink> {
    use std::os::unix::fs::MetadataExt;
    (meta.nlink() > 1).then(|| HardLink {
        dev: meta.dev(),
        ino: meta.ino(),
        allocated,
    })
}

#[cfg(not(unix))]
fn hardlink_of(_meta: &std::fs::Metadata, _allocated: u64) -> Option<HardLink> {
    None
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

/// Display name: NFC-normalized, lossily converted to UTF-8.
/// macOS HFS+/APFS often stores NFD; NFC renders correctly in UI fonts.
fn display_name(name: &OsStr) -> String {
    split_name(name).0
}

/// Display name plus the exact on-disk bytes when they differ from it.
fn split_name(name: &OsStr) -> (String, Option<Vec<u8>>) {
    match name.to_str() {
        Some(s) if s.is_ascii() => (s.to_owned(), None),
        Some(s) => {
            let nfc: String = s.nfc().collect();
            if nfc == s {
                (nfc, None)
            } else {
                (nfc, Some(s.as_bytes().to_vec()))
            }
        }
        None => (
            name.to_string_lossy().nfc().collect(),
            Some(os_name_bytes(name)),
        ),
    }
}

#[cfg(unix)]
fn os_name_bytes(name: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_name_bytes(name: &OsStr) -> Vec<u8> {
    name.to_string_lossy().into_owned().into_bytes()
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

fn placeholder_node() -> Node {
    Node {
        name: String::new(),
        apparent_size: 0,
        allocated_size: 0,
        file_count: 0,
        is_dir: true,
        synthetic: false,
        children: Vec::new(),
        parent: None,
        raw_name: None,
    }
}

fn rec_into_node(rec: Rec, is_dir: bool, links: &mut Vec<(HardLink, u32)>, idx: u32) -> Node {
    for link in rec.links {
        links.push((link, idx));
    }
    Node {
        name: rec.name,
        apparent_size: rec.apparent,
        allocated_size: rec.allocated,
        file_count: rec.file_count,
        is_dir,
        synthetic: rec.synthetic,
        children: Vec::new(),
        parent: Some(NodeId(rec.parent)),
        raw_name: rec.raw_name,
    }
}

fn build_tree(root: &Path, shared: &Shared, scanned_at_unix: u64) -> Result<SpaceTree, ScanFail> {
    let root_name = root
        .file_name()
        .map(display_name)
        .unwrap_or_else(|| root.display().to_string().nfc().collect());

    // Directory ids are dense (1..=n_dirs) and every parent is assigned before
    // its children, so parent < child holds for directories; files follow.
    let n_dirs = shared.next_dir.load(Ordering::Relaxed) as usize - 1;
    let n_files: usize = shared
        .shards
        .iter()
        .map(|s| s.lock().unwrap().files.len())
        .sum();
    let total = 1 + n_dirs + n_files;
    u32::try_from(total).map_err(|_| ScanFail::Error("too many entries for a single tree".into()))?;

    let mut nodes: Vec<Node> = Vec::with_capacity(total);
    nodes.push(Node {
        name: root_name,
        ..placeholder_node()
    });
    nodes.resize_with(n_dirs + 1, placeholder_node);

    let mut links: Vec<(HardLink, u32)> = Vec::new();
    let mut placed = 0usize;
    for shard in &shared.shards {
        let Shard { dirs, files } = std::mem::take(&mut *shard.lock().unwrap());
        for rec in dirs {
            let id = rec.id as usize;
            if id == 0 || id > n_dirs {
                return Err(ScanFail::Error(format!("directory id {id} out of range")));
            }
            nodes[id] = rec_into_node(rec, true, &mut links, rec_id(id));
            placed += 1;
        }
        for rec in files {
            let idx = nodes.len();
            if rec.parent as usize > n_dirs {
                return Err(ScanFail::Error("file parent out of range".into()));
            }
            nodes.push(rec_into_node(rec, false, &mut links, rec_id(idx)));
        }
    }
    if placed != n_dirs {
        return Err(ScanFail::Error(format!(
            "directory records lost: placed {placed} of {n_dirs}"
        )));
    }

    for idx in 1..nodes.len() {
        let parent = nodes[idx].parent.map(|p| p.0 as usize).unwrap_or(0);
        if parent >= idx && idx <= n_dirs {
            return Err(ScanFail::Error("directory precedes its parent".into()));
        }
        nodes[parent].children.push(NodeId(rec_id(idx)));
    }

    let hardlinks_deduped = resolve_hardlinks(&mut nodes, links);

    for i in (0..nodes.len()).rev() {
        if !nodes[i].is_dir {
            continue;
        }
        let (mut apparent, mut allocated, mut files) = (0u64, 0u64, 0u64);
        for &c in &nodes[i].children {
            let child = &nodes[c.0 as usize];
            apparent = apparent.saturating_add(child.apparent_size);
            allocated = allocated.saturating_add(child.allocated_size);
            files = files.saturating_add(child.file_count);
        }
        nodes[i].apparent_size = apparent;
        nodes[i].allocated_size = allocated;
        nodes[i].file_count = files;
    }

    for i in 0..nodes.len() {
        if !nodes[i].is_dir {
            continue;
        }
        let mut kids = std::mem::take(&mut nodes[i].children);
        kids.sort_unstable_by(|&a, &b| {
            let (na, nb) = (&nodes[a.0 as usize], &nodes[b.0 as usize]);
            nb.allocated_size
                .cmp(&na.allocated_size)
                .then_with(|| na.name.cmp(&nb.name))
                .then_with(|| a.0.cmp(&b.0))
        });
        nodes[i].children = kids;
    }

    Ok(SpaceTree {
        root_path: root.display().to_string(),
        item_count: nodes.len() as u64,
        nodes,
        root: NodeId(0),
        scanned_at_unix,
        skipped_errors: shared.skipped_errors.load(Ordering::Relaxed),
        hardlink_candidates: shared.hardlink_candidates.load(Ordering::Relaxed),
        hardlinks_deduped,
    })
}

fn rec_id(idx: usize) -> u32 {
    u32::try_from(idx).expect("node count was bounds-checked")
}

/// Hardlink semantics: every name keeps its apparent size; the allocated bytes
/// of one inode are charged to exactly one owner — the member whose path
/// (component-wise display names, then node id) sorts first. The result is
/// independent of worker scheduling.
fn resolve_hardlinks(nodes: &mut [Node], links: Vec<(HardLink, u32)>) -> u64 {
    if links.is_empty() {
        return 0;
    }
    let mut groups: HashMap<(u64, u64), Vec<(u32, u64)>> = HashMap::new();
    for (link, idx) in links {
        groups
            .entry((link.dev, link.ino))
            .or_default()
            .push((idx, link.allocated));
    }
    let mut deduped = 0u64;
    let mut path_cache: HashMap<u32, Vec<String>> = HashMap::new();
    for members in groups.into_values() {
        if members.len() < 2 {
            continue;
        }
        for &(idx, _) in &members {
            path_cache
                .entry(idx)
                .or_insert_with(|| path_components(nodes, idx));
        }
        let owner = (0..members.len())
            .min_by(|&a, &b| {
                let (ia, ib) = (members[a].0, members[b].0);
                path_cache[&ia]
                    .cmp(&path_cache[&ib])
                    .then_with(|| ia.cmp(&ib))
                    .then_with(|| a.cmp(&b))
            })
            .expect("non-empty group");
        for (position, &(idx, allocated)) in members.iter().enumerate() {
            if position == owner {
                continue;
            }
            let node = &mut nodes[idx as usize];
            node.allocated_size = node.allocated_size.saturating_sub(allocated);
            deduped += 1;
        }
        path_cache.clear();
    }
    deduped
}

fn path_components(nodes: &[Node], idx: u32) -> Vec<String> {
    let mut chain = Vec::new();
    let mut cur = Some(idx as usize);
    while let Some(i) = cur {
        chain.push(nodes[i].name.clone());
        cur = nodes[i].parent.map(|p| p.0 as usize);
    }
    chain.reverse();
    chain
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

/// Rescan one directory and splice the result into a copy of `tree`.
///
/// The subtree scan inherits the device filter of the original scan (derived
/// from the tree root), so it never descends into mounts the full scan skipped.
pub fn rescan_subtree(tree: &SpaceTree, dir_id: NodeId) -> Result<SpaceTree, String> {
    let Some(dir) = tree.try_node(dir_id) else {
        return Err("rescan node does not exist".into());
    };
    if !dir.is_dir || dir.synthetic {
        return Err("rescan node is not a directory".into());
    }
    let root = PathBuf::from(&tree.root_path);
    if dir_id == tree.root || tree.hardlink_candidates > 0 {
        return scan_path(root).wait();
    }

    let root_meta = std::fs::metadata(&root)
        .map_err(|e| format!("cannot read scan root {}: {e}", root.display()))?;
    let options = ScanOptions {
        allowed_devs: Some(allowed_devices(&root_meta)),
    };
    let scanned = spawn_scan(tree.path_os(dir_id), options).wait()?;
    // A hardlink can have another name outside this subtree. Without scanning
    // that owner, independently deduping this subtree could overcount bytes.
    if scanned.hardlink_candidates > 0 {
        return scan_path(root).wait();
    }
    Ok(splice_subtree(tree.clone(), dir_id, scanned))
}

fn splice_subtree(mut merged: SpaceTree, dir_id: NodeId, scanned: SpaceTree) -> SpaceTree {
    let old = merged.node(dir_id);
    let old_parent = old.parent.expect("non-root subtree has a parent");
    let old_apparent = old.apparent_size;
    let old_allocated = old.allocated_size;
    let old_files = old.file_count;
    let original_name = old.name.clone();
    let original_raw = old.raw_name.clone();

    let append_base = merged.nodes.len() as u32;
    let mut mapping = Vec::with_capacity(scanned.nodes.len());
    mapping.push(dir_id);
    for index in 1..scanned.nodes.len() {
        mapping.push(NodeId(append_base + index as u32 - 1));
    }

    let scanned_at_unix = scanned.scanned_at_unix;
    let scanned_errors = scanned.skipped_errors;
    merged.nodes.reserve(scanned.nodes.len());
    for (index, mut node) in scanned.nodes.into_iter().enumerate() {
        for child in &mut node.children {
            *child = mapping[child.0 as usize];
        }
        if index == 0 {
            node.name = original_name.clone();
            node.raw_name = original_raw.clone();
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
    use tempfile::TempDir;

    fn scan_blocking(root: impl Into<PathBuf>) -> Result<SpaceTree, String> {
        scan_path(root).wait()
    }

    fn child_named(tree: &SpaceTree, parent: NodeId, name: &str) -> NodeId {
        tree.children(parent)
            .iter()
            .copied()
            .find(|id| tree.node(*id).name == name)
            .unwrap_or_else(|| panic!("no child named {name}"))
    }

    #[test]
    fn display_names_are_normalized_once_at_ingest() {
        let nfd = OsStr::new("e\u{301}");
        assert_eq!(display_name(nfd), "é");
        let (display, raw) = split_name(nfd);
        assert_eq!(display, "é");
        assert_eq!(raw.as_deref(), Some("e\u{301}".as_bytes()));
        assert_eq!(split_name(OsStr::new("é")), ("é".to_owned(), None));
        assert_eq!(split_name(OsStr::new("plain")), ("plain".to_owned(), None));
    }

    #[test]
    fn scans_nested_tree_and_sums_sizes() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
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

        let handle = scan_path(dir);
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
        assert!(tree.validate().is_ok());
        assert!(tree.scanned_at_unix > 0);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_counted_once_on_disk() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        fs::File::create(&a)
            .unwrap()
            .write_all(&[7u8; 4096])
            .unwrap();
        fs::hard_link(&a, &b).unwrap();

        let tree = scan_blocking(dir).unwrap();
        assert!(tree.node(tree.root).allocated_size < 4096 * 2);
        assert_eq!(tree.hardlink_candidates, 2);
        assert_eq!(tree.hardlinks_deduped, 1);
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_owner_is_deterministic_across_runs() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        for name in ["z", "a", "m"] {
            fs::create_dir_all(dir.join(name)).unwrap();
        }
        let size = SMALL_FILE_THRESHOLD + 1;
        let origin = dir.join("z/big");
        fs::File::create(&origin).unwrap().set_len(size).unwrap();
        fs::hard_link(&origin, dir.join("a/big")).unwrap();
        fs::hard_link(&origin, dir.join("m/big")).unwrap();
        let allocated = allocated_bytes(&fs::metadata(&origin).unwrap());

        for _ in 0..8 {
            let tree = scan_blocking(dir).unwrap();
            assert_eq!(tree.hardlink_candidates, 3);
            assert_eq!(tree.hardlinks_deduped, 2);
            assert_eq!(tree.node(tree.root).allocated_size, allocated);
            assert_eq!(tree.node(tree.root).apparent_size, size * 3);
            let a = child_named(&tree, tree.root, "a");
            let owner = child_named(&tree, a, "big");
            assert_eq!(tree.node(owner).allocated_size, allocated, "owner is a/big");
            for other in ["m", "z"] {
                let d = child_named(&tree, tree.root, other);
                assert_eq!(tree.node(child_named(&tree, d, "big")).allocated_size, 0);
            }
        }
    }

    #[test]
    fn parallel_scan_preserves_hierarchy_and_exact_totals() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
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
            let tree = scan_blocking(dir).unwrap();
            assert_eq!(tree.node(tree.root).file_count, 128);
            assert_eq!(tree.node(tree.root).apparent_size, apparent);
            assert_eq!(tree.node(tree.root).allocated_size, allocated);
            assert_eq!(tree.nodes.len(), 1 + 64 * 5);
            assert_eq!(tree.children(tree.root).len(), 64);
            assert!(tree.validate().is_ok());
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
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlink_cycles_and_excluded_directories() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir.join(".Trash")).unwrap();
        fs::write(dir.join(".Trash/ignored"), b"ignored").unwrap();
        fs::write(dir.join("kept"), b"kept").unwrap();
        std::os::unix::fs::symlink(dir, dir.join("cycle")).unwrap();
        let tree = scan_blocking(dir).unwrap();
        assert_eq!(tree.node(tree.root).file_count, 1);
        assert_eq!(tree.node(tree.root).apparent_size, 4);
        assert_eq!(tree.nodes.len(), 2);
    }

    #[test]
    fn incremental_rescan_replaces_only_the_selected_subtree() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/one"), b"1").unwrap();
        fs::write(dir.join("b/one"), b"1").unwrap();

        let tree = scan_blocking(dir).unwrap();
        let a = child_named(&tree, tree.root, "a");
        fs::write(dir.join("a/two"), b"22").unwrap();
        let rescanned = rescan_subtree(&tree, a).unwrap();

        let rescanned_a = child_named(&rescanned, rescanned.root, "a");
        assert_eq!(rescanned.node(rescanned_a).file_count, 2);
        assert_eq!(rescanned.node(rescanned.root).file_count, 3);
        assert_eq!(rescanned.nodes.len() as u64, rescanned.item_count);
        assert!(rescanned.validate().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn incremental_rescan_falls_back_when_hardlinks_require_global_dedupe() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        fs::write(dir.join("a/one"), b"1").unwrap();
        fs::write(dir.join("b/link-one"), b"linked").unwrap();
        fs::hard_link(dir.join("b/link-one"), dir.join("b/link-two")).unwrap();

        let tree = scan_blocking(dir).unwrap();
        assert!(tree.hardlink_candidates > 0);
        let a = child_named(&tree, tree.root, "a");
        // This outside-subtree change is observed only by the exact full-scan
        // fallback required for global hardlink ownership.
        fs::write(dir.join("b/new"), b"new").unwrap();
        let rescanned = rescan_subtree(&tree, a).unwrap();
        assert_eq!(rescanned.node(rescanned.root).file_count, 4);
    }

    #[test]
    fn file_root_and_missing_root_are_errors() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("just-a-file");
        fs::write(&file, b"x").unwrap();
        let err = scan_blocking(&file).unwrap_err();
        assert!(err.contains("not a directory"), "{err}");
        let err = scan_blocking(tmp.path().join("missing")).unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
        assert!(rescan_subtree(
            &scan_blocking(tmp.path()).unwrap(),
            NodeId(99)
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_root_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let locked = tmp.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let result = scan_blocking(&locked);
        let readable = fs::read_dir(&locked).is_ok(); // root bypasses modes
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if readable {
            assert!(result.is_ok());
        } else {
            assert!(result.unwrap_err().contains("cannot read directory"));
        }
    }

    #[test]
    fn cancel_stops_scan_and_drop_cancels() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        for i in 0..200 {
            let d = dir.join(format!("d{i}"));
            fs::create_dir_all(&d).unwrap();
            for j in 0..10 {
                fs::write(d.join(format!("f{j}")), b"x").unwrap();
            }
        }
        let handle = scan_path(dir);
        handle.cancel();
        assert!(handle.is_cancelled());
        let mut terminal = None;
        while let Ok(event) = handle.events.recv_timeout(Duration::from_secs(30)) {
            match event {
                ScanEvent::Progress(_) => continue,
                other => {
                    assert!(terminal.is_none(), "two terminal events");
                    terminal = Some(other);
                }
            }
        }
        assert!(
            matches!(terminal, Some(ScanEvent::Cancelled) | Some(ScanEvent::Done(_))),
            "{terminal:?}"
        );

        let handle = scan_path(dir);
        let flag = Arc::clone(&handle.cancel);
        drop(handle);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_and_nfd_names_round_trip_through_rescan() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let nfd = dir.join("e\u{301}-dir");
        let raw = dir.join(OsString::from_vec(b"raw-\xff\xfe".to_vec()));
        fs::create_dir_all(nfd.join("inner")).unwrap();
        fs::write(nfd.join("inner/one"), b"1").unwrap();
        // APFS (macOS) rejects names that are not valid UTF-8 with EILSEQ;
        // only exercise the raw-bytes path on filesystems that accept them.
        let raw_supported = fs::create_dir_all(&raw).is_ok();
        let big = raw.join(OsString::from_vec(b"big-\xff".to_vec()));
        if raw_supported {
            fs::write(raw.join("one"), b"1").unwrap();
            fs::File::create(&big)
                .unwrap()
                .set_len(SMALL_FILE_THRESHOLD + 1)
                .unwrap();
        }

        let tree = scan_blocking(dir).unwrap();
        let nfd_id = child_named(&tree, tree.root, "é-dir");
        assert_eq!(
            tree.node(nfd_id).raw_name.as_deref(),
            Some("e\u{301}-dir".as_bytes())
        );
        assert_eq!(tree.path_os(nfd_id), nfd);
        if raw_supported {
            let raw_id = child_named(&tree, tree.root, "raw-\u{FFFD}\u{FFFD}");
            assert_eq!(tree.path_os(raw_id), raw);
            let big_id = child_named(&tree, raw_id, "big-\u{FFFD}");
            assert_eq!(tree.path_os(big_id).as_os_str().as_bytes(), big.as_os_str().as_bytes());
            assert!(tree.path_string(big_id).ends_with("big-\u{FFFD}"));
        }

        fs::write(nfd.join("inner/two"), b"2").unwrap();
        let tree = rescan_subtree(&tree, nfd_id).unwrap();
        let nfd_id = child_named(&tree, tree.root, "é-dir");
        assert_eq!(tree.node(nfd_id).file_count, 2);
        assert_eq!(tree.path_os(nfd_id), nfd);
        if !raw_supported {
            assert_eq!(tree.node(tree.root).file_count, 2);
            assert!(tree.validate().is_ok());
            return;
        }
        fs::write(raw.join("two"), b"2").unwrap();
        let raw_id = child_named(&tree, tree.root, "raw-\u{FFFD}\u{FFFD}");
        assert_eq!(tree.node(raw_id).file_count, 2);
        let tree = rescan_subtree(&tree, raw_id).unwrap();
        let raw_id = child_named(&tree, tree.root, "raw-\u{FFFD}\u{FFFD}");
        assert_eq!(tree.node(raw_id).file_count, 3);
        assert_eq!(tree.path_os(raw_id), raw);
        assert_eq!(tree.node(tree.root).file_count, 5);
        assert!(tree.validate().is_ok());
    }

    #[test]
    fn child_order_is_size_desc_then_name_then_id() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        for (name, size) in [("b", 3), ("a", 3), ("c", 1)] {
            fs::File::create(dir.join(name))
                .unwrap()
                .set_len(SMALL_FILE_THRESHOLD + size)
                .unwrap();
        }
        let tree = scan_blocking(dir).unwrap();
        let names: Vec<_> = tree
            .children(tree.root)
            .iter()
            .map(|id| tree.node(*id).name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }
}
