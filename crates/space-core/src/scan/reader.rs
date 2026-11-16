//! Directory reader abstraction (`DirReader`) unifying Darwin `getattrlistbulk` and portable `read_dir`.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread::Thread;

use crossbeam_deque::Worker;

use super::dag::{
    allocated_bytes, device_id, dir_identity, display_name, is_fatal_device_error, mtime_secs,
    should_skip_entry, unix_link_identity, DirRec, DirVisitDag, FileRec, SkipReason, SmallBucket,
    WorkerShard, MAX_SKIPPED_ROOTS,
};

pub(crate) struct DirTask {
    pub path: PathBuf,
    pub id: usize,
}

pub(crate) struct ReaderConfig {
    pub root: PathBuf,
    pub small_file_threshold: u64,
    pub skip_hidden: bool,
    pub cross_mount: bool,
    pub exclude_set: HashSet<String>,
    pub allowed_devs: HashSet<u64>,
    pub skip_volumes_prefix: bool,
    pub force_read_dir: bool,
}

pub(crate) type ProgressCallback = Box<dyn Fn(u64, u64, &Path, u64) + Send + Sync>;

pub(crate) struct SharedScanEnv {
    pub cancel: std::sync::Arc<AtomicBool>,
    pub aborted: AtomicBool,
    pub fatal_error: Mutex<Option<String>>,
    pub config: ReaderConfig,
    pub dag: DirVisitDag,
    pub next_dir: AtomicUsize,
    pub files_seen: AtomicU64,
    pub bytes_seen: AtomicU64,
    pub skipped_errors: AtomicU64,
    pub skipped_roots: Mutex<Vec<String>>,
    pub last_emit: AtomicU64,
    pub sleeping: AtomicUsize,
    pub pending: AtomicUsize,
    pub workers: Mutex<Vec<Thread>>,
    pub on_progress: ProgressCallback,
}

impl SharedScanEnv {
    pub(crate) fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.aborted.load(Ordering::Acquire)
    }

    pub(crate) fn wake_sleepers(&self) {
        if self.sleeping.load(Ordering::Acquire) > 0 {
            self.unpark_all();
        }
    }

    pub(crate) fn unpark_all(&self) {
        let threads = self.workers.lock().unwrap();
        for t in threads.iter() {
            t.unpark();
        }
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
        if is_fatal_device_error(err) || !self.config.root.exists() {
            let mut guard = self.fatal_error.lock().unwrap();
            if guard.is_none() {
                *guard = Some(format!("scan interrupted at {}: {err}", path.display()));
            }
            self.aborted.store(true, Ordering::Release);
            self.unpark_all();
            return;
        }
        self.skipped_errors.fetch_add(1, Ordering::Relaxed);
        self.record_skipped_root(path);
    }

    fn publish_progress(&self, files: u64, bytes: u64, path: &Path) {
        if files == 0 {
            return;
        }
        self.bytes_seen.fetch_add(bytes, Ordering::Relaxed);
        let n = self.files_seen.fetch_add(files, Ordering::Relaxed) + files;
        let prev = self.last_emit.load(Ordering::Relaxed);
        if n.saturating_sub(prev) >= 8_000
            && self
                .last_emit
                .compare_exchange(prev, n, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            (self.on_progress)(
                n,
                self.bytes_seen.load(Ordering::Relaxed),
                path,
                self.skipped_errors.load(Ordering::Relaxed),
            );
        }
    }
}

pub(crate) struct FileStat {
    pub apparent: u64,
    pub allocated: u64,
    pub mtime_unix: u64,
    pub link_count: u64,
    pub dev_ino: Option<(u64, u64)>,
}

pub(crate) struct DirScanCtx<'a> {
    pub env: &'a SharedScanEnv,
    pub local: &'a Worker<DirTask>,
    pub shard: &'a mut WorkerShard,
}

#[derive(Default)]
struct DirBatch {
    small: SmallBucket,
    files: u64,
    bytes: u64,
}

impl DirScanCtx<'_> {
    #[inline]
    fn should_skip(&self, dir: &Path, raw_name: &OsStr, display: &str) -> bool {
        if let Some(reason) = should_skip_entry(
            display,
            self.env.config.skip_hidden,
            &self.env.config.exclude_set,
        ) {
            if reason == SkipReason::UserExcluded {
                self.env.record_skipped_root(&dir.join(raw_name));
            }
            return true;
        }
        false
    }

    fn emit_dir(
        &mut self,
        parent_id: usize,
        child_path: PathBuf,
        name: String,
        dev: u64,
        ino: Option<u64>,
        mtime_unix: u64,
    ) {
        if !self.env.config.cross_mount && !self.env.config.allowed_devs.contains(&dev) {
            self.env.record_skipped_root(&child_path);
            return;
        }
        if let Some(inode) = ino {
            if !self.env.dag.visit(dev, inode) {
                self.env.record_skipped_root(&child_path);
                return;
            }
        }
        let id = self.env.next_dir.fetch_add(1, Ordering::Relaxed);
        self.shard.dirs.push(DirRec {
            id,
            parent: parent_id,
            name,
            mtime_unix,
        });
        self.env.pending.fetch_add(1, Ordering::Release);
        self.local.push(DirTask {
            path: child_path,
            id,
        });
        self.env.wake_sleepers();
    }

    fn emit_file(
        &mut self,
        parent_id: usize,
        dir: &Path,
        name: String,
        stat: FileStat,
        batch: &mut DirBatch,
    ) {
        let is_hardlink_candidate = stat.link_count > 1 && stat.dev_ino.is_some();
        let threshold = self.env.config.small_file_threshold;
        if !is_hardlink_candidate && stat.allocated < threshold && stat.apparent < threshold {
            batch.small.count += 1;
            batch.small.apparent = batch.small.apparent.saturating_add(stat.apparent);
            batch.small.allocated = batch.small.allocated.saturating_add(stat.allocated);
        } else {
            self.shard.files.push(FileRec {
                parent: parent_id,
                name,
                apparent: stat.apparent,
                allocated: stat.allocated,
                mtime_unix: stat.mtime_unix,
                dev_ino: if is_hardlink_candidate {
                    stat.dev_ino
                } else {
                    None
                },
            });
        }
        batch.files += 1;
        batch.bytes += stat.allocated;
        if batch.files == 256 {
            self.env.publish_progress(batch.files, batch.bytes, dir);
            batch.files = 0;
            batch.bytes = 0;
        }
    }

    fn finish_dir(&mut self, dir: &Path, parent_id: usize, batch: DirBatch) {
        self.env.publish_progress(batch.files, batch.bytes, dir);
        if batch.small.count > 0 {
            self.shard.small_buckets.push((parent_id, batch.small));
        }
    }
}

pub(crate) trait DirReader {
    /// Returns `true` if handled (or fatal/skipped), `false` if caller should try fallback.
    fn read_dir(&mut self, task: &DirTask, ctx: &mut DirScanCtx<'_>) -> bool;
}

pub(crate) fn scan_directory(
    task: &DirTask,
    ctx: &mut DirScanCtx<'_>,
    #[cfg(target_os = "macos")] bulk_buf: &mut Vec<u8>,
) {
    let dir = &task.path;
    if ctx.env.config.skip_volumes_prefix
        && dir.starts_with("/Volumes")
        && !ctx.env.config.root.starts_with("/Volumes")
    {
        return;
    }

    #[cfg(target_os = "macos")]
    if !ctx.env.config.force_read_dir && DarwinBulkReader::new(bulk_buf).read_dir(task, ctx) {
        return;
    }

    PortableReadDir.read_dir(task, ctx);
}

struct PortableReadDir;

impl DirReader for PortableReadDir {
    fn read_dir(&mut self, task: &DirTask, ctx: &mut DirScanCtx<'_>) -> bool {
        let dir = &task.path;
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(err) => {
                ctx.env.check_io_error(dir, &err);
                return true;
            }
        };

        let mut batch = DirBatch::default();
        for ent in rd {
            if ctx.env.stopped() {
                return true;
            }
            let ent = match ent {
                Ok(e) => e,
                Err(err) => {
                    ctx.env.check_io_error(dir, &err);
                    continue;
                }
            };

            let raw_name = ent.file_name();
            let display = display_name(&raw_name);
            if ctx.should_skip(dir, &raw_name, &display) {
                continue;
            }

            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(err) => {
                    ctx.env.check_io_error(&dir.join(&raw_name), &err);
                    continue;
                }
            };
            if ft.is_symlink() {
                continue;
            }

            if ft.is_dir() {
                let meta = match ent.metadata() {
                    Ok(m) => m,
                    Err(err) => {
                        ctx.env.check_io_error(&dir.join(&raw_name), &err);
                        continue;
                    }
                };
                let dev = device_id(&meta);
                let ino = dir_identity(&meta).map(|(_, i)| i);
                ctx.emit_dir(task.id, ent.path(), display, dev, ino, mtime_secs(&meta));
            } else if ft.is_file() {
                let meta = match ent.metadata() {
                    Ok(m) => m,
                    Err(err) => {
                        ctx.env.check_io_error(&dir.join(&raw_name), &err);
                        continue;
                    }
                };
                let (link_count, dev_ino) = unix_link_identity(&meta);
                ctx.emit_file(
                    task.id,
                    dir,
                    display,
                    FileStat {
                        apparent: meta.len(),
                        allocated: allocated_bytes(&meta),
                        mtime_unix: mtime_secs(&meta),
                        link_count,
                        dev_ino,
                    },
                    &mut batch,
                );
            }
        }
        ctx.finish_dir(dir, task.id, batch);
        true
    }
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
struct DarwinBulkReader<'b> {
    buf: &'b mut Vec<u8>,
}

#[cfg(target_os = "macos")]
impl<'b> DarwinBulkReader<'b> {
    fn new(buf: &'b mut Vec<u8>) -> Self {
        Self { buf }
    }
}

#[cfg(target_os = "macos")]
impl DirReader for DarwinBulkReader<'_> {
    fn read_dir(&mut self, task: &DirTask, ctx: &mut DirScanCtx<'_>) -> bool {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        const ATTR_CMN_ERROR: u32 = 0x2000_0000;
        const SF_FIRMLINK: u32 = 0x0080_0000;
        const VREG: u32 = 1;
        const VDIR: u32 = 2;
        const VLNK: u32 = 5;

        let dir = &task.path;
        let Ok(cpath) = CString::new(dir.as_os_str().as_bytes()) else {
            return false;
        };

        let fd = unsafe {
            libc::open(
                cpath.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            ctx.env.check_io_error(dir, &err);
            return true;
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

        let mut batch = DirBatch::default();
        let mut any_entries = false;

        loop {
            if ctx.env.stopped() {
                return true;
            }
            let count = unsafe {
                libc::getattrlistbulk(
                    fd.0,
                    (&raw mut attr_list).cast::<libc::c_void>(),
                    self.buf.as_mut_ptr().cast::<libc::c_void>(),
                    self.buf.len(),
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
                    return false;
                }
                ctx.env.check_io_error(dir, &err);
                break;
            }
            if count == 0 {
                break;
            }
            any_entries = true;

            let mut offset = 0usize;
            for _ in 0..count {
                if ctx.env.stopped() {
                    return true;
                }
                if offset + 4 > self.buf.len() {
                    break;
                }
                let entry_len = read_u32(self.buf, offset) as usize;
                if entry_len < 4 || offset + entry_len > self.buf.len() {
                    break;
                }
                let entry = &self.buf[offset..offset + entry_len];
                offset += entry_len;

                let mut cur = 4usize;
                if cur + 20 > entry.len() {
                    ctx.env.skipped_errors.fetch_add(1, Ordering::Relaxed);
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
                        ctx.env.check_io_error(dir, &io_err);
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
                    let ts_size = std::mem::size_of::<libc::timespec>();
                    if cur + ts_size > entry.len() {
                        continue;
                    }
                    mtime_unix = read_i64(entry, cur).max(0) as u64;
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

                let mut file_id = None;
                if (ret_common & libc::ATTR_CMN_FILEID) != 0 {
                    if cur + 8 > entry.len() {
                        continue;
                    }
                    file_id = Some(read_u64(entry, cur));
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
                if ctx.should_skip(dir, raw_os, &display) || obj_type == VLNK {
                    continue;
                }

                if obj_type == VDIR {
                    let is_mntpoint = (mount_status & libc::DIR_MNTSTATUS_MNTPOINT) != 0;
                    let is_firmlink = (flags & SF_FIRMLINK) != 0;
                    let child_path = dir.join(raw_os);
                    let mut ino = file_id;
                    if is_mntpoint || is_firmlink {
                        match std::fs::metadata(&child_path) {
                            Ok(meta) => {
                                dev_id = device_id(&meta);
                                ino = dir_identity(&meta).map(|(_, i)| i);
                                mtime_unix = mtime_secs(&meta);
                            }
                            Err(err) => {
                                ctx.env.check_io_error(&child_path, &err);
                                continue;
                            }
                        }
                    }
                    ctx.emit_dir(task.id, child_path, display, dev_id, ino, mtime_unix);
                } else if obj_type == VREG {
                    ctx.emit_file(
                        task.id,
                        dir,
                        display,
                        FileStat {
                            apparent: data_len,
                            allocated: alloc_size,
                            mtime_unix,
                            link_count,
                            dev_ino: file_id.map(|fid| (dev_id, fid)),
                        },
                        &mut batch,
                    );
                }
            }
        }

        ctx.finish_dir(dir, task.id, batch);
        true
    }
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
