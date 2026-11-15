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

/// Maximum number of skipped directory paths captured for scan reporting.
const MAX_SKIPPED_ROOTS: usize = 32;

/// Files below this allocated size are rolled into one "· small files" node per folder.
const SMALL_FILE_THRESHOLD: u64 = 512 * 1024;

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
}

struct Entry {
    parent: Option<usize>,
    name: String,
    is_dir: bool,
    synthetic: bool,
    apparent: u64,
    allocated: u64,
    file_count: u64,
    mtime_unix: u64,
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
    mtime_unix: u64,
}

struct Shared {
    cancel: Arc<AtomicBool>,
    aborted: AtomicBool,
    fatal_error: Mutex<Option<String>>,
    root: PathBuf,
    options: ScanOptions,
    exclude_set: HashSet<String>,
    allowed_devs: HashSet<u64>,
    skip_volumes_prefix: bool,
    next_dir: AtomicUsize,
    files_seen: AtomicU64,
    bytes_seen: AtomicU64,
    skipped_errors: AtomicU64,
    skipped_roots: Mutex<Vec<String>>,
    hardlink_candidates: AtomicU64,
    hardlinks_deduped: AtomicU64,
    last_emit: AtomicU64,
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
    tx: Sender<ScanEvent>,
    /// Sharded record buffers — one per worker, merged at end.
    shards: Vec<Mutex<Vec<Rec>>>,
}

impl Shared {
    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.aborted.load(Ordering::Acquire)
    }

    fn record_skipped_root(&self, path: &Path) {
        let mut guard = self.skipped_roots.lock().unwrap();
        if guard.len() >= MAX_SKIPPED_ROOTS {
            return;
        }
        let s = path.display().to_string();
        if !guard.iter().any(|existing| existing == &s) {
            guard.push(s);
        }
    }

    fn check_io_error(&self, path: &Path, err: &std::io::Error) {
        if is_fatal_device_error(err) || !self.root.exists() {
            let mut guard = self.fatal_error.lock().unwrap();
            if guard.is_none() {
                *guard = Some(format!("scan interrupted at {}: {err}", path.display()));
            }
            self.aborted.store(true, Ordering::Release);
            return;
        }
        self.skipped_errors.fetch_add(1, Ordering::Relaxed);
        self.record_skipped_root(path);
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

    thread::Builder::new()
        .name("space-scan".into())
        .spawn(
            move || match run_scan(root, options, tx.clone(), cancel_bg) {
                Ok(()) => {}
                Err(ScanFail::Cancelled) => {
                    let _ = tx.send(ScanEvent::Cancelled);
                }
                Err(ScanFail::Error(err)) => {
                    let _ = tx.send(ScanEvent::Error(err));
                }
            },
        )
        .expect("spawn scan thread");

    ScanHandle { events: rx, cancel }
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

    let mut shards = Vec::with_capacity(threads);
    for _ in 0..threads {
        shards.push(Mutex::new(Vec::with_capacity(65_536)));
    }

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

    let shared = Arc::new(Shared {
        cancel,
        aborted: AtomicBool::new(false),
        fatal_error: Mutex::new(None),
        root: root.clone(),
        options,
        exclude_set,
        allowed_devs,
        skip_volumes_prefix,
        next_dir: AtomicUsize::new(1),
        files_seen: AtomicU64::new(0),
        bytes_seen: AtomicU64::new(0),
        skipped_errors: AtomicU64::new(0),
        skipped_roots: Mutex::new(Vec::new()),
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
    if shared.aborted.load(Ordering::Acquire) {
        let msg = shared
            .fatal_error
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| format!("scan root became unavailable: {}", root.display()));
        return Err(ScanFail::Error(msg));
    }

    // Merge shards
    let mut all = Vec::new();
    for shard in &shared.shards {
        all.append(&mut shard.lock().unwrap());
    }

    let tree = build_tree(&root, root_mtime, all, &shared)?;
    let _ = tx.send(ScanEvent::Done(Box::new(tree)));
    Ok(())
}

fn worker_loop(
    wid: usize,
    local: Worker<DirTask>,
    shared: Arc<Shared>,
    injector: Arc<Injector<DirTask>>,
    stealers: Arc<Vec<crossbeam_deque::Stealer<DirTask>>>,
    pending: Arc<AtomicUsize>,
) {
    let mut buf: Vec<Rec> = Vec::with_capacity(8192);
    #[cfg(target_os = "macos")]
    let mut bulk_buf: Vec<u8> = vec![0u8; 64 * 1024];

    loop {
        if shared.stopped() {
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
                if pending.load(Ordering::Acquire) == 0 || shared.stopped() {
                    flush_buf(wid, &shared, &mut buf);
                    return;
                }
                continue;
            }
        };

        scan_dir(
            &dir,
            &shared,
            &injector,
            &pending,
            &mut buf,
            #[cfg(target_os = "macos")]
            &mut bulk_buf,
        );
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
    #[cfg(target_os = "macos")] bulk_buf: &mut Vec<u8>,
) {
    let dir = &task.path;
    // Decide once per directory instead of constructing every child path.
    if shared.skip_volumes_prefix
        && dir.starts_with("/Volumes")
        && !shared.root.starts_with("/Volumes")
    {
        return;
    }

    #[cfg(target_os = "macos")]
    if !shared.options.force_read_dir {
        match scan_dir_bulk(task, shared, injector, pending, buf, bulk_buf) {
            BulkOutcome::Done => return,
            BulkOutcome::Fallback => {}
        }
    }

    scan_dir_read_dir(task, shared, injector, pending, buf);
}

#[derive(Default)]
struct SmallBucket {
    count: u64,
    apparent: u64,
    allocated: u64,
}

struct FileStat {
    apparent: u64,
    allocated: u64,
    mtime_unix: u64,
    link_count: u64,
    dev_ino: Option<(u64, u64)>,
}

fn record_file(
    task: &DirTask,
    shared: &Shared,
    buf: &mut Vec<Rec>,
    name: String,
    mut stat: FileStat,
    small: &mut SmallBucket,
) -> u64 {
    if stat.link_count > 1 {
        if let Some(key) = stat.dev_ino {
            shared.hardlink_candidates.fetch_add(1, Ordering::Relaxed);
            let mut g = shared.seen_inodes.lock().unwrap();
            if !g.insert(key) {
                shared.hardlinks_deduped.fetch_add(1, Ordering::Relaxed);
                stat.allocated = 0;
            }
        }
    }
    let threshold = shared.options.small_file_threshold;
    if stat.allocated < threshold && stat.apparent < threshold {
        small.count += 1;
        small.apparent = small.apparent.saturating_add(stat.apparent);
        small.allocated = small.allocated.saturating_add(stat.allocated);
    } else {
        buf.push(Rec {
            parent: task.id,
            id: None,
            name,
            is_dir: false,
            synthetic: false,
            apparent: stat.apparent,
            allocated: stat.allocated,
            file_count: 1,
            mtime_unix: stat.mtime_unix,
        });
    }
    stat.allocated
}

fn push_small_bucket(task: &DirTask, buf: &mut Vec<Rec>, small: SmallBucket) {
    if small.count > 0 {
        buf.push(Rec {
            parent: task.id,
            id: None,
            name: format!("· {} small files", small.count),
            is_dir: false,
            synthetic: true,
            apparent: small.apparent,
            allocated: small.allocated,
            file_count: small.count,
            mtime_unix: 0,
        });
    }
}

fn should_skip_entry(name: &str, shared: &Shared) -> Option<SkipReason> {
    if should_skip_name(name) {
        return Some(SkipReason::SystemNoise);
    }
    if shared.options.skip_hidden && name.starts_with('.') {
        return Some(SkipReason::UserExcluded);
    }
    if shared.exclude_set.contains(name) {
        return Some(SkipReason::UserExcluded);
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    SystemNoise,
    UserExcluded,
}

fn scan_dir_read_dir(
    task: &DirTask,
    shared: &Shared,
    injector: &Injector<DirTask>,
    pending: &AtomicUsize,
    buf: &mut Vec<Rec>,
) {
    let dir = &task.path;
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(err) => {
            shared.check_io_error(dir, &err);
            return;
        }
    };

    let mut progress_files = 0u64;
    let mut progress_bytes = 0u64;
    let mut small = SmallBucket::default();
    for ent in rd {
        if shared.stopped() {
            return;
        }
        let ent = match ent {
            Ok(e) => e,
            Err(err) => {
                shared.check_io_error(dir, &err);
                continue;
            }
        };

        let raw_name = ent.file_name();
        let display = display_name(&raw_name);
        if let Some(reason) = should_skip_entry(&display, shared) {
            if reason == SkipReason::UserExcluded {
                shared.record_skipped_root(&dir.join(&raw_name));
            }
            continue;
        }

        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(err) => {
                shared.check_io_error(&dir.join(&raw_name), &err);
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
                Err(err) => {
                    shared.check_io_error(&dir.join(&raw_name), &err);
                    continue;
                }
            };
            let dev = device_id(&meta);
            if !shared.options.cross_mount && !shared.allowed_devs.contains(&dev) {
                shared.record_skipped_root(&dir.join(&raw_name));
                continue;
            }
            let id = shared.next_dir.fetch_add(1, Ordering::Relaxed);
            buf.push(Rec {
                parent: task.id,
                id: Some(id),
                name: display,
                is_dir: true,
                synthetic: false,
                apparent: 0,
                allocated: 0,
                file_count: 0,
                mtime_unix: mtime_secs(&meta),
            });
            pending.fetch_add(1, Ordering::Release);
            injector.push(DirTask {
                path: ent.path(),
                id,
            });
        } else if ft.is_file() {
            let meta = match ent.metadata() {
                Ok(m) => m,
                Err(err) => {
                    shared.check_io_error(&dir.join(&raw_name), &err);
                    continue;
                }
            };
            let apparent = meta.len();
            let allocated = allocated_bytes(&meta);
            let mtime = mtime_secs(&meta);
            let (link_count, dev_ino) = unix_link_identity(&meta);
            let counted = record_file(
                task,
                shared,
                buf,
                display,
                FileStat {
                    apparent,
                    allocated,
                    mtime_unix: mtime,
                    link_count,
                    dev_ino,
                },
                &mut small,
            );
            progress_files += 1;
            progress_bytes += counted;
            if progress_files == 256 {
                publish_progress(shared, progress_files, progress_bytes, dir);
                progress_files = 0;
                progress_bytes = 0;
            }
        }
        // else: sockets/fifos/devices — ignore
    }
    publish_progress(shared, progress_files, progress_bytes, dir);
    push_small_bucket(task, buf, small);
}

#[cfg(target_os = "macos")]
enum BulkOutcome {
    Done,
    Fallback,
}

#[cfg(target_os = "macos")]
struct DirFd(libc::c_int);

#[cfg(target_os = "macos")]
impl Drop for DirFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe {
                libc::close(self.0);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn scan_dir_bulk(
    task: &DirTask,
    shared: &Shared,
    injector: &Injector<DirTask>,
    pending: &AtomicUsize,
    buf: &mut Vec<Rec>,
    bulk_buf: &mut Vec<u8>,
) -> BulkOutcome {
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;

    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const SF_FIRMLINK: u32 = 0x0080_0000;
    const VREG: u32 = 1;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;

    let dir = &task.path;
    let Ok(cpath) = CString::new(dir.as_os_str().as_bytes()) else {
        return BulkOutcome::Fallback;
    };

    let fd = unsafe {
        libc::open(
            cpath.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        shared.check_io_error(dir, &err);
        return BulkOutcome::Done;
    }
    let fd = DirFd(fd);

    let mut attr_list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME
            | libc::ATTR_CMN_FLAGS
            | libc::ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: libc::ATTR_DIR_MOUNTSTATUS,
        fileattr: libc::ATTR_FILE_LINKCOUNT
            | libc::ATTR_FILE_ALLOCSIZE
            | libc::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };

    let mut progress_files = 0u64;
    let mut progress_bytes = 0u64;
    let mut small = SmallBucket::default();
    let mut any_entries = false;

    loop {
        if shared.stopped() {
            return BulkOutcome::Done;
        }
        let count = unsafe {
            libc::getattrlistbulk(
                fd.0,
                (&raw mut attr_list).cast::<libc::c_void>(),
                bulk_buf.as_mut_ptr().cast::<libc::c_void>(),
                bulk_buf.len(),
                0,
            )
        };
        if count < 0 {
            let err = std::io::Error::last_os_error();
            let raw = err.raw_os_error();
            if !any_entries
                && matches!(
                    raw,
                    Some(libc::ENOTSUP) | Some(libc::EINVAL) | Some(libc::ERANGE)
                )
            {
                return BulkOutcome::Fallback;
            }
            shared.check_io_error(dir, &err);
            break;
        }
        if count == 0 {
            break;
        }
        any_entries = true;

        let mut offset = 0usize;
        for _ in 0..count {
            if shared.stopped() {
                return BulkOutcome::Done;
            }
            if offset + 4 > bulk_buf.len() {
                break;
            }
            let entry_len = read_u32(bulk_buf, offset) as usize;
            if entry_len < 4 || offset + entry_len > bulk_buf.len() {
                break;
            }
            let entry = &bulk_buf[offset..offset + entry_len];
            offset += entry_len;

            let mut cur = 4usize;
            // ATTR_CMN_RETURNED_ATTRS (attribute_set_t: 5 x u32 = 20 bytes)
            if cur + 20 > entry.len() {
                shared.skipped_errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let ret_common = read_u32(entry, cur);
            let _ret_vol = read_u32(entry, cur + 4);
            let ret_dir = read_u32(entry, cur + 8);
            let ret_file = read_u32(entry, cur + 12);
            cur += 20;

            if (ret_common & ATTR_CMN_ERROR) != 0 {
                if cur + 4 > entry.len() {
                    continue;
                }
                let entry_err = read_u32(entry, cur);
                cur += 4;
                if entry_err != 0 {
                    let io_err = std::io::Error::from_raw_os_error(entry_err as i32);
                    shared.check_io_error(dir, &io_err);
                    continue;
                }
            }

            let mut raw_name_bytes: &[u8] = &[];
            if (ret_common & libc::ATTR_CMN_NAME) != 0 {
                if cur + 8 > entry.len() {
                    continue;
                }
                let data_offset = read_i32(entry, cur);
                let attr_len = read_u32(entry, cur + 4) as usize;
                let str_start = (cur as isize + data_offset as isize) as usize;
                let str_end = str_start.saturating_add(attr_len).min(entry.len());
                if str_start < str_end {
                    let raw = &entry[str_start..str_end];
                    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                    raw_name_bytes = &raw[..end];
                }
                cur += 8;
            }
            if raw_name_bytes.is_empty() || raw_name_bytes == b"." || raw_name_bytes == b".." {
                continue;
            }

            let mut dev_id = 0u64;
            if (ret_common & libc::ATTR_CMN_DEVID) != 0 {
                if cur + 4 > entry.len() {
                    continue;
                }
                dev_id = read_i32(entry, cur) as u64;
                cur += 4;
            }

            let mut obj_type = 0u32;
            if (ret_common & libc::ATTR_CMN_OBJTYPE) != 0 {
                if cur + 4 > entry.len() {
                    continue;
                }
                obj_type = read_u32(entry, cur);
                cur += 4;
            }

            let mut mtime_unix = 0u64;
            if (ret_common & libc::ATTR_CMN_MODTIME) != 0 {
                // Darwin 64-bit timespec: tv_sec (i64) + tv_nsec (i64)
                let ts_size = std::mem::size_of::<libc::timespec>();
                if cur + ts_size > entry.len() {
                    continue;
                }
                let sec = read_i64(entry, cur);
                mtime_unix = sec.max(0) as u64;
                cur += ts_size;
            }

            let mut flags = 0u32;
            if (ret_common & libc::ATTR_CMN_FLAGS) != 0 {
                if cur + 4 > entry.len() {
                    continue;
                }
                flags = read_u32(entry, cur);
                cur += 4;
            }

            let mut file_id = 0u64;
            if (ret_common & libc::ATTR_CMN_FILEID) != 0 {
                if cur + 8 > entry.len() {
                    continue;
                }
                file_id = read_u64(entry, cur);
                cur += 8;
            }

            let mut mount_status = 0u32;
            if (ret_dir & libc::ATTR_DIR_MOUNTSTATUS) != 0 {
                if cur + 4 > entry.len() {
                    continue;
                }
                mount_status = read_u32(entry, cur);
                cur += 4;
            }

            let mut link_count = 1u64;
            if (ret_file & libc::ATTR_FILE_LINKCOUNT) != 0 {
                if cur + 4 > entry.len() {
                    continue;
                }
                link_count = u64::from(read_u32(entry, cur));
                cur += 4;
            }

            let mut alloc_size = 0u64;
            if (ret_file & libc::ATTR_FILE_ALLOCSIZE) != 0 {
                if cur + 8 > entry.len() {
                    continue;
                }
                alloc_size = read_i64(entry, cur).max(0) as u64;
                cur += 8;
            }

            let mut data_len = 0u64;
            if (ret_file & libc::ATTR_FILE_DATALENGTH) != 0 {
                if cur + 8 > entry.len() {
                    continue;
                }
                data_len = read_i64(entry, cur).max(0) as u64;
            }

            let raw_os = OsStr::from_bytes(raw_name_bytes);
            let display = display_name(raw_os);
            if let Some(reason) = should_skip_entry(&display, shared) {
                if reason == SkipReason::UserExcluded {
                    shared.record_skipped_root(&dir.join(raw_os));
                }
                continue;
            }

            if obj_type == VLNK {
                continue;
            }

            if obj_type == VDIR {
                let is_mntpoint = (mount_status & libc::DIR_MNTSTATUS_MNTPOINT) != 0;
                let is_firmlink = (flags & SF_FIRMLINK) != 0;
                let child_path = dir.join(raw_os);
                if is_mntpoint || is_firmlink {
                    match std::fs::metadata(&child_path) {
                        Ok(meta) => {
                            dev_id = device_id(&meta);
                            mtime_unix = mtime_secs(&meta);
                        }
                        Err(err) => {
                            shared.check_io_error(&child_path, &err);
                            continue;
                        }
                    }
                }
                if !shared.options.cross_mount && !shared.allowed_devs.contains(&dev_id) {
                    shared.record_skipped_root(&child_path);
                    continue;
                }
                let id = shared.next_dir.fetch_add(1, Ordering::Relaxed);
                buf.push(Rec {
                    parent: task.id,
                    id: Some(id),
                    name: display,
                    is_dir: true,
                    synthetic: false,
                    apparent: 0,
                    allocated: 0,
                    file_count: 0,
                    mtime_unix,
                });
                pending.fetch_add(1, Ordering::Release);
                injector.push(DirTask {
                    path: child_path,
                    id,
                });
            } else if obj_type == VREG {
                let counted = record_file(
                    task,
                    shared,
                    buf,
                    display,
                    FileStat {
                        apparent: data_len,
                        allocated: alloc_size,
                        mtime_unix,
                        link_count,
                        dev_ino: Some((dev_id, file_id)),
                    },
                    &mut small,
                );
                progress_files += 1;
                progress_bytes += counted;
                if progress_files == 256 {
                    publish_progress(shared, progress_files, progress_bytes, dir);
                    progress_files = 0;
                    progress_bytes = 0;
                }
            }
        }
    }

    publish_progress(shared, progress_files, progress_bytes, dir);
    push_small_bucket(task, buf, small);
    BulkOutcome::Done
}

#[cfg(target_os = "macos")]
fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap())
}

#[cfg(target_os = "macos")]
fn read_i32(buf: &[u8], off: usize) -> i32 {
    i32::from_ne_bytes(buf[off..off + 4].try_into().unwrap())
}

#[cfg(target_os = "macos")]
fn read_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_ne_bytes(buf[off..off + 8].try_into().unwrap())
}

#[cfg(target_os = "macos")]
fn read_i64(buf: &[u8], off: usize) -> i64 {
    i64::from_ne_bytes(buf[off..off + 8].try_into().unwrap())
}

fn is_fatal_device_error(err: &std::io::Error) -> bool {
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

fn unix_link_identity(meta: &std::fs::Metadata) -> (u64, Option<(u64, u64)>) {
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

fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime().max(0) as u64
    }
    #[cfg(not(unix))]
    {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
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

fn build_tree(
    root: &Path,
    root_mtime: u64,
    recs: Vec<Rec>,
    shared: &Shared,
) -> Result<SpaceTree, ScanFail> {
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
        mtime_unix: root_mtime,
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
            mtime_unix: record.mtime_unix,
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
                apparent = apparent.saturating_add(entries[c].apparent);
                allocated = allocated.saturating_add(entries[c].allocated);
                files = files.saturating_add(entries[c].file_count);
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
            mtime_unix: e.mtime_unix,
            children: e.children.into_iter().map(|c| NodeId(c as u32)).collect(),
            parent: e.parent.map(|p| NodeId(p as u32)),
        })
        .collect();

    let item_count = nodes.len() as u64;
    let scanned_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut skipped_roots = shared.skipped_roots.lock().unwrap().clone();
    skipped_roots.sort();
    let tree = SpaceTree {
        root_path: root.display().to_string(),
        nodes,
        root: NodeId(0),
        scanned_at_unix,
        item_count,
        skipped_errors: shared.skipped_errors.load(Ordering::Relaxed),
        hardlink_candidates: shared.hardlink_candidates.load(Ordering::Relaxed),
        hardlinks_deduped: shared.hardlinks_deduped.load(Ordering::Relaxed),
        skipped_roots,
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
    rescan_subtree_with_options(tree, dir_id, ScanOptions::default())
}

pub fn rescan_subtree_with_options(
    tree: &SpaceTree,
    dir_id: NodeId,
    options: ScanOptions,
) -> Result<SpaceTree, String> {
    let Some(dir) = tree.try_node(dir_id) else {
        return Err("rescan node does not exist".into());
    };
    if !dir.is_dir {
        return Err("rescan node is not a directory".into());
    }
    let root = PathBuf::from(&tree.root_path);
    if dir_id == tree.root || tree.hardlink_candidates > 0 {
        return scan_blocking_with_options(root, options);
    }

    let subtree_str = tree.path_string(dir_id);
    let subtree_root = PathBuf::from(&subtree_str);
    let scanned = scan_blocking_with_options(subtree_root, options.clone())?;
    // A hardlink can have another name outside this subtree. Without scanning
    // that owner, independently deduping this subtree could overcount bytes.
    if scanned.hardlink_candidates > 0 {
        return scan_blocking_with_options(root, options);
    }
    Ok(splice_subtree(tree, dir_id, &subtree_str, scanned))
}

#[cfg(test)]
fn scan_blocking(root: PathBuf) -> Result<SpaceTree, String> {
    scan_blocking_with_options(root, ScanOptions::default())
}

fn scan_blocking_with_options(root: PathBuf, options: ScanOptions) -> Result<SpaceTree, String> {
    let handle = scan_path_with_options(root, options);
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

fn splice_subtree(
    tree: &SpaceTree,
    dir_id: NodeId,
    subtree_prefix: &str,
    scanned: SpaceTree,
) -> SpaceTree {
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
    let scanned_skipped_roots = scanned.skipped_roots;
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
    let prefix_slash = format!("{}/", subtree_prefix.trim_end_matches('/'));
    merged
        .skipped_roots
        .retain(|p| p != subtree_prefix && !p.starts_with(&prefix_slash));
    for p in scanned_skipped_roots {
        if merged.skipped_roots.len() >= MAX_SKIPPED_ROOTS {
            break;
        }
        if !merged.skipped_roots.iter().any(|existing| existing == &p) {
            merged.skipped_roots.push(p);
        }
    }
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
        skipped_roots,
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
        skipped_roots,
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
            bulk.total_size_mode(crate::SizeMode::Apparent),
            fallback.total_size_mode(crate::SizeMode::Apparent)
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
