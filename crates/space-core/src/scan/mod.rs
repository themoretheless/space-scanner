//! High-throughput parallel disk scanner structured as an acyclic pipeline DAG:
//! `mod -> {reader, builder, rescan} -> dag`.

mod builder;
mod dag;
mod reader;
mod rescan;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use crossbeam_utils::thread::scope;
use crossbeam_utils::Backoff;

use crate::tree::{NodeId, SpaceTree};
use dag::{device_id, dir_identity, mtime_secs, DirVisitDag, WorkerShard, SMALL_FILE_THRESHOLD};
use reader::{scan_directory, DirScanCtx, DirTask, ReaderConfig, SharedScanEnv};

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub small_file_threshold: u64,
    pub skip_hidden: bool,
    pub cross_mount: bool,
    pub exclude_names: Vec<String>,
    /// Override worker thread count (`None` uses the host-tuned default).
    pub threads: Option<usize>,
    /// Disable Darwin `getattrlistbulk` fast path and use portable `read_dir`.
    pub force_read_dir: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            small_file_threshold: SMALL_FILE_THRESHOLD,
            skip_hidden: false,
            cross_mount: false,
            exclude_names: Vec::new(),
            threads: None,
            force_read_dir: false,
        }
    }
}

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

    /// Blocks until scanning finishes, discarding intermediate progress updates.
    pub fn wait(self) -> Result<SpaceTree, String> {
        self.wait_with(|_| {})
    }

    /// Blocks until scanning finishes while forwarding progress events to `on_progress`.
    pub fn wait_with(self, mut on_progress: impl FnMut(ScanProgress)) -> Result<SpaceTree, String> {
        loop {
            match self.events.recv() {
                Ok(ScanEvent::Progress(progress)) => on_progress(progress),
                Ok(ScanEvent::Done(tree)) => return Ok(*tree),
                Ok(ScanEvent::Cancelled) => return Err("cancelled".into()),
                Ok(ScanEvent::Error(err)) => return Err(err),
                Err(err) => return Err(err.to_string()),
            }
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn scan_path(root: impl Into<PathBuf>) -> ScanHandle {
    scan_path_with_options(root, ScanOptions::default())
}

pub fn scan_path_with_expect(root: impl Into<PathBuf>, _expected_items: Option<u64>) -> ScanHandle {
    scan_path_with_options(root, ScanOptions::default())
}

pub fn scan_path_with_options(root: impl Into<PathBuf>, options: ScanOptions) -> ScanHandle {
    let root = root.into();
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_bg = Arc::clone(&cancel);
    let tx_bg = tx.clone();

    if let Err(err) =
        thread::Builder::new()
            .name("space-scan".into())
            .spawn(
                move || match run_scan(root, options, tx_bg.clone(), cancel_bg) {
                    Ok(()) => {}
                    Err(ScanFail::Cancelled) => {
                        let _ = tx_bg.send(ScanEvent::Cancelled);
                    }
                    Err(ScanFail::Error(err)) => {
                        let _ = tx_bg.send(ScanEvent::Error(err));
                    }
                },
            )
    {
        let _ = tx.send(ScanEvent::Error(format!(
            "failed to spawn scan thread: {err}"
        )));
    }

    ScanHandle { events: rx, cancel }
}

pub fn rescan_subtree(tree: &SpaceTree, dir_id: NodeId) -> Result<SpaceTree, String> {
    rescan_subtree_with_options(tree, dir_id, ScanOptions::default())
}

pub fn rescan_subtree_with_options(
    tree: &SpaceTree,
    dir_id: NodeId,
    options: ScanOptions,
) -> Result<SpaceTree, String> {
    rescan::rescan_subtree_with(tree, dir_id, |target| {
        scan_path_with_options(target.clone(), options.clone()).wait()
    })
}

enum ScanFail {
    Cancelled,
    Error(String),
}

fn run_scan(
    root: PathBuf,
    options: ScanOptions,
    tx: Sender<ScanEvent>,
    cancel: Arc<AtomicBool>,
) -> Result<(), ScanFail> {
    if !root.exists() {
        return Err(ScanFail::Error(format!(
            "path does not exist: {}",
            root.display()
        )));
    }

    let root_meta = std::fs::metadata(&root).map_err(|e| ScanFail::Error(e.to_string()))?;
    let root_dev = device_id(&root_meta);
    let root_mtime = mtime_secs(&root_meta);
    let mut allowed_devs = HashSet::new();
    allowed_devs.insert(root_dev);
    // macOS: System + Data often share semantics; include Data if present
    if let Ok(m) = std::fs::metadata("/System/Volumes/Data") {
        allowed_devs.insert(device_id(&m));
    }

    // The eight-worker cap outperformed caps of four and sixteen on the
    // warm macOS metadata fixtures; extra workers add contention and overhead.
    let max_threads = if cfg!(target_os = "macos") { 8 } else { 16 };
    let threads = options.threads.map(|n| n.clamp(1, 32)).unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, max_threads)
    });

    let skip_volumes_prefix = !options.cross_mount && {
        let s = root.to_string_lossy();
        !(s.starts_with("/Volumes/") || root == Path::new("/Volumes"))
    };
    let exclude_set: HashSet<String> = options
        .exclude_names
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let small_file_threshold = options.small_file_threshold;
    let config = ReaderConfig {
        root: root.clone(),
        small_file_threshold,
        skip_hidden: options.skip_hidden,
        cross_mount: options.cross_mount,
        exclude_set,
        allowed_devs,
        skip_volumes_prefix,
        force_read_dir: options.force_read_dir,
    };

    let dag = DirVisitDag::new(dir_identity(&root_meta));
    let tx_progress = tx.clone();
    let env = Arc::new(SharedScanEnv {
        cancel,
        aborted: AtomicBool::new(false),
        fatal_error: Mutex::new(None),
        config,
        dag,
        next_dir: AtomicUsize::new(1),
        files_seen: AtomicU64::new(0),
        bytes_seen: AtomicU64::new(0),
        skipped_errors: AtomicU64::new(0),
        skipped_roots: Mutex::new(Vec::new()),
        last_emit: AtomicU64::new(0),
        sleeping: AtomicUsize::new(0),
        pending: AtomicUsize::new(1), // root queued
        workers: Mutex::new(Vec::with_capacity(threads)),
        on_progress: Box::new(move |files_seen, bytes_seen, path, skipped_errors| {
            let _ = tx_progress.send(ScanEvent::Progress(ScanProgress {
                files_seen,
                bytes_seen,
                current_path: path.display().to_string(),
                skipped_errors,
            }));
        }),
    });

    let injector = Arc::new(Injector::<DirTask>::new());
    injector.push(DirTask {
        path: root.clone(),
        id: 0,
    });

    let mut worker_stealers = Vec::with_capacity(threads);
    let mut local_workers = Vec::with_capacity(threads);
    for _ in 0..threads {
        let w = Worker::<DirTask>::new_lifo();
        worker_stealers.push(w.stealer());
        local_workers.push(Some(w));
    }
    let stealers = Arc::new(worker_stealers);

    let scoped_result = scope(|scope| {
        let mut handles = Vec::with_capacity(threads);
        for (wid, slot) in local_workers.iter_mut().enumerate() {
            let worker = slot.take().unwrap();
            let env = Arc::clone(&env);
            let injector = Arc::clone(&injector);
            let stealers = Arc::clone(&stealers);
            handles.push(scope.spawn(move |_| worker_loop(wid, worker, env, injector, stealers)));
        }
        let mut collected = Vec::with_capacity(threads);
        for h in handles {
            collected.push(h.join());
        }
        collected
    });

    let shard_results =
        scoped_result.map_err(|e| ScanFail::Error(format!("scan scope panicked: {e:?}")))?;
    let mut shards = Vec::with_capacity(threads);
    for res in shard_results {
        match res {
            Ok(shard) => shards.push(shard),
            Err(e) => return Err(ScanFail::Error(format!("scan workers panicked: {e:?}"))),
        }
    }

    if env.cancel.load(Ordering::Relaxed) {
        return Err(ScanFail::Cancelled);
    }
    if env.aborted.load(Ordering::Acquire) {
        let msg = env
            .fatal_error
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| format!("scan root became unavailable: {}", root.display()));
        return Err(ScanFail::Error(msg));
    }

    let dir_count = env.next_dir.load(Ordering::Acquire);
    let skipped_errors = env.skipped_errors.load(Ordering::Relaxed);
    let skipped_roots = std::mem::take(&mut *env.skipped_roots.lock().unwrap());
    let tree = builder::build_tree(
        &root,
        root_mtime,
        dir_count,
        shards,
        builder::BuildMeta {
            skipped_errors,
            skipped_roots,
            small_file_threshold,
        },
    )
    .map_err(ScanFail::Error)?;
    let _ = tx.send(ScanEvent::Done(Box::new(tree)));
    Ok(())
}

fn worker_loop(
    wid: usize,
    local: Worker<DirTask>,
    env: Arc<SharedScanEnv>,
    injector: Arc<Injector<DirTask>>,
    stealers: Arc<Vec<Stealer<DirTask>>>,
) -> WorkerShard {
    env.workers.lock().unwrap().push(thread::current());

    let mut shard = WorkerShard {
        dirs: Vec::with_capacity(1024),
        files: Vec::with_capacity(4096),
        small_buckets: Vec::with_capacity(512),
    };
    #[cfg(target_os = "macos")]
    let mut bulk_buf: Vec<u8> = vec![0u8; 64 * 1024];

    while let Some(task) = next_task_or_wait(wid, &local, &injector, &stealers, &env) {
        let mut ctx = DirScanCtx {
            env: &env,
            local: &local,
            shard: &mut shard,
        };
        scan_directory(
            &task,
            &mut ctx,
            #[cfg(target_os = "macos")]
            &mut bulk_buf,
        );
        if env.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            env.unpark_all();
        }
    }

    shard
}

fn next_task_or_wait(
    wid: usize,
    local: &Worker<DirTask>,
    injector: &Injector<DirTask>,
    stealers: &[Stealer<DirTask>],
    env: &SharedScanEnv,
) -> Option<DirTask> {
    let backoff = Backoff::new();
    loop {
        if env.stopped() {
            return None;
        }
        if let Some(task) = find_task(wid, local, injector, stealers) {
            return Some(task);
        }
        if env.pending.load(Ordering::Acquire) == 0 {
            return None;
        }
        if !backoff.is_completed() {
            backoff.snooze();
            continue;
        }

        env.sleeping.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = find_task(wid, local, injector, stealers) {
            env.sleeping.fetch_sub(1, Ordering::Release);
            return Some(task);
        }
        if env.pending.load(Ordering::SeqCst) == 0 || env.stopped() {
            env.sleeping.fetch_sub(1, Ordering::Release);
            return None;
        }
        thread::park_timeout(Duration::from_micros(100));
        env.sleeping.fetch_sub(1, Ordering::Release);
        backoff.reset();
    }
}

fn find_task(
    wid: usize,
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
    let count = stealers.len();
    for offset in 1..count {
        let target = (wid + offset) % count;
        loop {
            match stealers[target].steal_batch_and_pop(local) {
                Steal::Success(task) => return Some(task),
                Steal::Empty => break,
                Steal::Retry => continue,
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::dag::{allocated_bytes, display_name, DirVisitDag, SMALL_FILE_THRESHOLD};
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::time::Duration;

    fn scan_blocking(root: PathBuf) -> Result<SpaceTree, String> {
        scan_path(root).wait()
    }

    fn scan_blocking_with_options(
        root: PathBuf,
        options: ScanOptions,
    ) -> Result<SpaceTree, String> {
        scan_path_with_options(root, options).wait()
    }

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

    #[cfg(unix)]
    #[test]
    fn hardlink_dag_ownership_is_deterministic_across_parallel_workers() {
        let dir = std::env::temp_dir().join(format!("space-scanner-hl-dag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("alpha")).unwrap();
        fs::create_dir_all(dir.join("mid")).unwrap();
        fs::create_dir_all(dir.join("zeta")).unwrap();

        let payload = vec![5u8; (SMALL_FILE_THRESHOLD as usize) + 4096];
        let alpha = dir.join("alpha/link.bin");
        fs::write(&alpha, &payload).unwrap();
        fs::hard_link(&alpha, dir.join("mid/link.bin")).unwrap();
        fs::hard_link(&alpha, dir.join("zeta/link.bin")).unwrap();

        let link_sizes = |threads: usize| {
            let tree = scan_blocking_with_options(
                dir.clone(),
                ScanOptions {
                    threads: Some(threads),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(tree.hardlink_candidates, 3);
            assert_eq!(tree.hardlinks_deduped, 2);
            let mut out = Vec::new();
            for folder in ["alpha", "mid", "zeta"] {
                let d_id = tree
                    .children(tree.root)
                    .iter()
                    .copied()
                    .find(|id| tree.node(*id).name == folder)
                    .unwrap();
                let f_id = tree
                    .children(d_id)
                    .iter()
                    .copied()
                    .find(|id| tree.node(*id).name == "link.bin")
                    .unwrap();
                out.push((folder, tree.node(f_id).allocated_size));
            }
            out
        };

        let baseline = link_sizes(1);
        assert!(baseline[0].1 > 0);
        assert_eq!(baseline[1].1, 0);
        assert_eq!(baseline[2].1, 0);
        for _ in 0..4 {
            assert_eq!(link_sizes(8), baseline);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dir_visit_dag_rejects_revisited_directory_inodes() {
        let dag = DirVisitDag::new(Some((1, 100)));
        assert!(!dag.visit(1, 100));
        assert!(dag.visit(1, 101));
        assert!(!dag.visit(1, 101));
        assert!(dag.visit(2, 100));
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

    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_and_read_dir_produce_identical_trees() {
        let dir =
            std::env::temp_dir().join(format!("space-scanner-bulk-parity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("nested/sub")).unwrap();
        fs::create_dir_all(dir.join("empty")).unwrap();
        fs::write(dir.join("nested/small.txt"), b"hello").unwrap();
        fs::write(dir.join("nested/large.bin"), vec![7u8; 600 * 1024]).unwrap();
        fs::write(dir.join(".hidden"), vec![9u8; 600 * 1024]).unwrap();
        fs::hard_link(
            dir.join("nested/large.bin"),
            dir.join("nested/sub/large-link.bin"),
        )
        .unwrap();
        std::os::unix::fs::symlink(dir.join("nested"), dir.join("sym-dir")).unwrap();

        let bulk = scan_blocking_with_options(
            dir.clone(),
            ScanOptions {
                force_read_dir: false,
                ..Default::default()
            },
        )
        .unwrap();
        let fallback = scan_blocking_with_options(
            dir.clone(),
            ScanOptions {
                force_read_dir: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(bulk.item_count, fallback.item_count);
        assert_eq!(bulk.total_size(), fallback.total_size());
        assert_eq!(
            bulk.total_size_mode(crate::tree::SizeMode::Apparent),
            fallback.total_size_mode(crate::tree::SizeMode::Apparent)
        );
        assert_eq!(bulk.hardlink_candidates, fallback.hardlink_candidates);
        assert_eq!(bulk.hardlinks_deduped, fallback.hardlinks_deduped);

        let snapshot = |t: &SpaceTree| {
            let mut rows: Vec<_> = (0..t.nodes.len())
                .map(|i| {
                    let id = NodeId(i as u32);
                    let n = t.node(id);
                    let path = t.path_string(id);
                    let alloc = if path.contains("large") {
                        None
                    } else {
                        Some(n.allocated_size)
                    };
                    (
                        path,
                        n.name.clone(),
                        n.is_dir,
                        n.synthetic,
                        n.apparent_size,
                        alloc,
                        n.file_count,
                        n.mtime_unix,
                    )
                })
                .collect();
            rows.sort();
            rows
        };
        assert_eq!(snapshot(&bulk), snapshot(&fallback));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_options_exclude_names_and_skip_hidden_record_skipped_roots() {
        let dir = std::env::temp_dir().join(format!("space-scanner-opts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("node_modules/pkg/big.js"), vec![1u8; 600 * 1024]).unwrap();
        fs::write(dir.join(".secret"), vec![2u8; 600 * 1024]).unwrap();
        fs::write(dir.join("src/main.rs"), vec![3u8; 600 * 1024]).unwrap();

        let opts = ScanOptions {
            skip_hidden: true,
            exclude_names: vec!["node_modules".into()],
            ..Default::default()
        };
        let tree = scan_blocking_with_options(dir.clone(), opts).unwrap();
        assert_eq!(tree.node(tree.root).file_count, 1);
        assert!(tree
            .skipped_roots
            .iter()
            .any(|p| p.ends_with("node_modules")));
        assert!(tree.skipped_roots.iter().any(|p| p.ends_with(".secret")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_root_returns_error_event() {
        let missing =
            std::env::temp_dir().join(format!("space-scanner-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&missing);
        let err = scan_blocking(missing).unwrap_err();
        assert!(err.contains("does not exist"));
    }
}
