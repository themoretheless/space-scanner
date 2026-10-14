//! On-disk cache of scanned trees: `<key>.meta` (header + `CacheMeta`) and
//! `<key>.bin` (header + blake3 checksum + zstd-compressed bincode tree).

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bincode::Options;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::now_unix;
use crate::tree::SpaceTree;

const CACHE_MAGIC: &[u8; 8] = b"SPSCACHE";
const META_MAGIC: &[u8; 8] = b"SPSCMETA";
/// Bump when `SpaceTree` / `Node` on-disk schema changes.
pub const CACHE_VERSION: u32 = 5;
/// Upper bound for one decoded tree; guards against hostile length prefixes.
const MAX_TREE_BYTES: u64 = 8 << 30;
const MAX_META_BYTES: u64 = 1 << 20;
const HEADER_LEN: usize = 8 + 4;
const CHECKSUM_LEN: usize = 32;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode: {0}")]
    Encode(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("stale or missing cache")]
    Miss,
    #[error("cache directory unavailable")]
    NoDir,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMeta {
    pub root_path: String,
    pub root_mtime_secs: u64,
    pub scanned_at_unix: u64,
    pub item_count: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone)]
pub struct CacheLoad {
    pub meta: CacheMeta,
    pub tree: SpaceTree,
    /// True when root mtime moved ahead of cached snapshot (or the root is gone).
    pub stale: bool,
}

#[derive(Debug, Clone)]
pub struct CacheStore {
    dir: PathBuf,
    ttl: Option<Duration>,
    max_bytes: Option<u64>,
}

/// One cached root on disk.
struct Entry {
    meta_path: PathBuf,
    bin_path: PathBuf,
    bytes: u64,
    /// Meta file mtime: refreshed on every load, so it doubles as "last used".
    last_used: SystemTime,
    scanned_at_unix: u64,
}

impl CacheStore {
    pub fn open() -> Result<Self, CacheError> {
        let base = directories::ProjectDirs::from("dev", "space", "space-scanner")
            .ok_or(CacheError::NoDir)?;
        Self::with_dir(base.cache_dir().join("trees"))
    }

    /// Open (and create) a cache in an explicit directory.
    pub fn with_dir(dir: impl Into<PathBuf>) -> Result<Self, CacheError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            ttl: None,
            max_bytes: None,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Entries scanned more than `days` ago are removed by [`CacheStore::prune_expired`].
    /// Zero disables the TTL.
    pub fn with_ttl_days(mut self, days: u64) -> Self {
        self.ttl = (days > 0).then(|| Duration::from_secs(days.saturating_mul(86_400)));
        self
    }

    /// Least-recently-loaded entries are evicted after each save until the
    /// cache fits in `bytes`. Zero disables the budget.
    pub fn with_max_bytes(mut self, bytes: u64) -> Self {
        self.max_bytes = (bytes > 0).then_some(bytes);
        self
    }

    pub fn ttl(&self) -> Option<Duration> {
        self.ttl
    }

    pub fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
    }

    fn key_for(root: &Path) -> String {
        blake3::hash(&os_path_bytes(canonical_or_partial(root).as_os_str()))
            .to_hex()
            .to_string()
    }

    fn paths(&self, root: &Path) -> (PathBuf, PathBuf) {
        let key = Self::key_for(root);
        (
            self.dir.join(format!("{key}.meta")),
            self.dir.join(format!("{key}.bin")),
        )
    }

    pub fn peek_meta(&self, root: &Path) -> Option<CacheMeta> {
        let (meta_path, _) = self.paths(root);
        read_meta(&meta_path).ok()
    }

    /// Load cache even if stale; sets `stale` flag when root mtime advanced.
    /// A version mismatch deletes the outdated files and reports a miss.
    pub fn load(&self, root: &Path) -> Result<CacheLoad, CacheError> {
        let (meta_path, bin_path) = self.paths(root);
        if !meta_path.exists() || !bin_path.exists() {
            return Err(CacheError::Miss);
        }

        let meta = match read_meta(&meta_path) {
            Ok(meta) => meta,
            Err(CacheError::Miss) => {
                self.invalidate(root)?;
                return Err(CacheError::Miss);
            }
            Err(err) => return Err(err),
        };

        let stale = match path_mtime_secs(root) {
            Some(current) => current > meta.root_mtime_secs.saturating_add(1),
            None => true,
        };

        let tree = match read_tree(&bin_path) {
            Ok(tree) => tree,
            Err(CacheError::Miss) => {
                self.invalidate(root)?;
                return Err(CacheError::Miss);
            }
            Err(err) => return Err(err),
        };
        if tree.item_count != meta.item_count {
            return Err(CacheError::Decode(format!(
                "meta item_count {} != tree {}",
                meta.item_count, tree.item_count
            )));
        }
        touch(&meta_path);

        Ok(CacheLoad { meta, tree, stale })
    }

    pub fn load_if_fresh(&self, root: &Path) -> Result<(CacheMeta, SpaceTree), CacheError> {
        let loaded = self.load(root)?;
        if loaded.stale {
            return Err(CacheError::Miss);
        }
        Ok((loaded.meta, loaded.tree))
    }

    pub fn save(&self, tree: &SpaceTree) -> Result<CacheMeta, CacheError> {
        let root = Path::new(&tree.root_path);
        let (meta_path, bin_path) = self.paths(root);

        // `scanned_at_unix` is taken at scan start. If the root changed after
        // that, record an older mtime so the next load reports the snapshot
        // as stale instead of trusting a tree that may already be outdated.
        let root_mtime_secs = match path_mtime_secs(root) {
            Some(mtime) if mtime > tree.scanned_at_unix => tree.scanned_at_unix.saturating_sub(2),
            Some(mtime) => mtime,
            None => 0,
        };
        let meta = CacheMeta {
            root_path: tree.root_path.clone(),
            root_mtime_secs,
            scanned_at_unix: tree.scanned_at_unix,
            item_count: tree.item_count,
            total_size: tree.total_size(),
        };

        write_tree(&bin_path, tree)?;
        if let Err(err) = write_meta(&meta_path, &meta) {
            let _ = fs::remove_file(&bin_path);
            return Err(err);
        }
        if self.max_bytes.is_some() {
            let _ = self.evict_to_budget_keeping(Some(&meta_path));
        }
        Ok(meta)
    }

    pub fn invalidate(&self, root: &Path) -> Result<(), CacheError> {
        let (meta_path, bin_path) = self.paths(root);
        remove_if_exists(&meta_path)?;
        remove_if_exists(&bin_path)?;
        Ok(())
    }

    /// Cached roots, most recently scanned first.
    pub fn list_recent(&self) -> Result<Vec<CacheMeta>, CacheError> {
        let mut out = Vec::new();
        if !self.dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }
            if let Ok(meta) = read_meta(&path) {
                out.push(meta);
            }
        }
        out.sort_by_key(|meta| std::cmp::Reverse(meta.scanned_at_unix));
        Ok(out)
    }

    /// Delete every cache file (including leftover temporaries).
    /// Returns `(files removed, bytes freed)`.
    pub fn wipe_all(&self) -> io::Result<(usize, u64)> {
        let mut removed = (0usize, 0u64);
        if !self.dir.exists() {
            return Ok(removed);
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("meta" | "bin" | "tmp")) {
                continue;
            }
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if remove_if_exists(&path)? {
                removed.0 += 1;
                removed.1 += bytes;
            }
        }
        Ok(removed)
    }

    /// Remove entries older than the configured TTL.
    /// Returns `(entries removed, bytes freed)`; `(0, 0)` without a TTL.
    pub fn prune_expired(&self) -> io::Result<(usize, u64)> {
        let Some(ttl) = self.ttl else {
            return Ok((0, 0));
        };
        let cutoff = now_unix().saturating_sub(ttl.as_secs());
        let mut removed = (0usize, 0u64);
        for entry in self.entries()? {
            if entry.scanned_at_unix < cutoff {
                removed.0 += 1;
                removed.1 += remove_entry(&entry)?;
            }
        }
        Ok(removed)
    }

    /// Evict least-recently-loaded entries until the cache fits the budget.
    /// Returns `(entries removed, bytes freed)`; `(0, 0)` without a budget.
    pub fn evict_to_budget(&self) -> io::Result<(usize, u64)> {
        self.evict_to_budget_keeping(None)
    }

    /// Like [`CacheStore::evict_to_budget`], but never removes the entry whose
    /// meta file is `keep`: a save must not evict the tree it just wrote when
    /// several entries share the same mtime second.
    fn evict_to_budget_keeping(&self, keep: Option<&Path>) -> io::Result<(usize, u64)> {
        let Some(budget) = self.max_bytes else {
            return Ok((0, 0));
        };
        let mut entries = self.entries()?;
        let mut total: u64 = entries.iter().map(|e| e.bytes).sum();
        let mut removed = (0usize, 0u64);
        if total <= budget {
            return Ok(removed);
        }
        // Oldest first; ties (same mtime second) broken by scan time, then by
        // path so the order is deterministic across runs and platforms.
        entries.sort_by(|a, b| {
            (a.last_used, a.scanned_at_unix, &a.meta_path).cmp(&(
                b.last_used,
                b.scanned_at_unix,
                &b.meta_path,
            ))
        });
        for entry in entries {
            if total <= budget {
                break;
            }
            if keep.is_some_and(|k| k == entry.meta_path) {
                continue;
            }
            let freed = remove_entry(&entry)?;
            total = total.saturating_sub(entry.bytes);
            removed.0 += 1;
            removed.1 += freed;
        }
        Ok(removed)
    }

    /// Total bytes used by cache files.
    pub fn total_bytes(&self) -> io::Result<u64> {
        Ok(self.entries()?.iter().map(|e| e.bytes).sum())
    }

    fn entries(&self) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        if !self.dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let meta_path = entry.path();
            if meta_path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }
            let Ok(meta) = read_meta(&meta_path) else {
                continue;
            };
            let bin_path = meta_path.with_extension("bin");
            let meta_fs = entry.metadata()?;
            let bin_len = fs::metadata(&bin_path).map(|m| m.len()).unwrap_or(0);
            out.push(Entry {
                last_used: meta_fs.modified().unwrap_or(UNIX_EPOCH),
                bytes: meta_fs.len() + bin_len,
                meta_path,
                bin_path,
                scanned_at_unix: meta.scanned_at_unix,
            });
        }
        Ok(out)
    }
}

fn remove_entry(entry: &Entry) -> io::Result<u64> {
    let mut freed = 0;
    for path in [&entry.meta_path, &entry.bin_path] {
        let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if remove_if_exists(path)? {
            freed += len;
        }
    }
    Ok(freed)
}

fn remove_if_exists(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn touch(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

#[cfg(unix)]
/// Canonicalize `root`, or, when it no longer exists, canonicalize its
/// deepest existing ancestor and re-append the missing tail. This keeps the
/// cache key stable for a root that vanished after it was scanned, even on
/// platforms where the temp directory itself is a symlink (macOS `/var`).
fn canonical_or_partial(root: &Path) -> PathBuf {
    if let Ok(canonical) = root.canonicalize() {
        return canonical;
    }
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = root.to_path_buf();
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_os_string());
        }
        cur = parent.to_path_buf();
        if let Ok(canonical) = cur.canonicalize() {
            let mut out = canonical;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
    }
    root.to_path_buf()
}

fn os_path_bytes(path: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_path_bytes(path: &std::ffi::OsStr) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// Removes a temporary file unless `disarm` was called (after a rename).
struct TmpGuard(Option<PathBuf>);

impl TmpGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TmpGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn bincode_options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .allow_trailing_bytes()
        .with_limit(MAX_TREE_BYTES)
}

/// Reads magic + version; `Miss` on a version mismatch, `Decode` on bad magic.
fn check_header(bytes: &[u8], magic: &[u8; 8]) -> Result<(), CacheError> {
    if bytes.len() < HEADER_LEN {
        return Err(CacheError::Decode("truncated header".into()));
    }
    if &bytes[..8] != magic {
        return Err(CacheError::Decode("bad magic".into()));
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
    if version != CACHE_VERSION {
        return Err(CacheError::Miss);
    }
    Ok(())
}

fn read_meta(path: &Path) -> Result<CacheMeta, CacheError> {
    let len = fs::metadata(path)?.len();
    if len > MAX_META_BYTES {
        return Err(CacheError::Decode("meta file too large".into()));
    }
    let bytes = fs::read(path)?;
    check_header(&bytes, META_MAGIC)?;
    bincode_options()
        .deserialize(&bytes[HEADER_LEN..])
        .map_err(|e| CacheError::Decode(e.to_string()))
}

fn write_meta(path: &Path, meta: &CacheMeta) -> Result<(), CacheError> {
    let payload = bincode_options()
        .serialize(meta)
        .map_err(|e| CacheError::Encode(e.to_string()))?;
    let tmp = path.with_extension("meta.tmp");
    let mut guard = TmpGuard(Some(tmp.clone()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(META_MAGIC)?;
        f.write_all(&CACHE_VERSION.to_le_bytes())?;
        f.write_all(&payload)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    guard.disarm();
    Ok(())
}

struct HashWriter<W: Write> {
    inner: W,
    hasher: blake3::Hasher,
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn write_tree(path: &Path, tree: &SpaceTree) -> Result<(), CacheError> {
    use std::io::{Seek, SeekFrom};

    let tmp = path.with_extension("bin.tmp");
    let mut guard = TmpGuard(Some(tmp.clone()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(CACHE_MAGIC)?;
        f.write_all(&CACHE_VERSION.to_le_bytes())?;
        f.write_all(&[0u8; CHECKSUM_LEN])?;
        let checksum = {
            let mut hashed = HashWriter {
                inner: io::BufWriter::new(&mut f),
                hasher: blake3::Hasher::new(),
            };
            let mut encoder = zstd::stream::write::Encoder::new(&mut hashed, 3)
                .map_err(|e| CacheError::Encode(e.to_string()))?;
            bincode_options()
                .serialize_into(&mut encoder, tree)
                .map_err(|e| CacheError::Encode(e.to_string()))?;
            encoder
                .finish()
                .map_err(|e| CacheError::Encode(e.to_string()))?;
            hashed.flush()?;
            hashed.hasher.finalize()
        };
        f.seek(SeekFrom::Start(HEADER_LEN as u64))?;
        f.write_all(checksum.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    guard.disarm();
    Ok(())
}

fn read_tree(path: &Path) -> Result<SpaceTree, CacheError> {
    let mut file = fs::File::open(path)?;
    let mut header = [0u8; HEADER_LEN + CHECKSUM_LEN];
    file.read_exact(&mut header)
        .map_err(|_| CacheError::Decode("truncated header".into()))?;
    check_header(&header[..HEADER_LEN], CACHE_MAGIC)?;
    let expected = blake3::Hash::from_bytes(
        header[HEADER_LEN..].try_into().expect("checksum length"),
    );

    let mut payload = Vec::new();
    file.read_to_end(&mut payload)?;
    if blake3::hash(&payload) != expected {
        return Err(CacheError::Decode("checksum mismatch".into()));
    }

    let decoder = zstd::stream::read::Decoder::new(payload.as_slice())
        .map_err(|e| CacheError::Decode(e.to_string()))?;
    let tree: SpaceTree = bincode_options()
        .deserialize_from(decoder)
        .map_err(|e| CacheError::Decode(e.to_string()))?;
    tree.validate().map_err(CacheError::Decode)?;
    Ok(tree)
}

fn path_mtime_secs(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Node, NodeId};
    use tempfile::TempDir;

    fn node(name: &str, parent: Option<NodeId>, is_dir: bool) -> Node {
        Node {
            name: name.into(),
            apparent_size: 7,
            allocated_size: 8,
            file_count: 1,
            is_dir,
            synthetic: false,
            children: Vec::new(),
            parent,
            raw_name: None,
        }
    }

    fn tree_for(root: &Path, scanned_at: u64) -> SpaceTree {
        let mut file = node("e\u{301}", Some(NodeId(0)), false);
        file.raw_name = Some(b"e\xcc\x81".to_vec());
        let mut root_node = node("root", None, true);
        root_node.children.push(NodeId(1));
        SpaceTree {
            root_path: root.to_string_lossy().into_owned(),
            nodes: vec![root_node, file],
            root: NodeId(0),
            scanned_at_unix: scanned_at,
            item_count: 2,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        }
    }

    struct Fixture {
        _tmp: TempDir,
        root: PathBuf,
        store: CacheStore,
    }

    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        let store = CacheStore::with_dir(tmp.path().join("cache")).unwrap();
        Fixture {
            _tmp: tmp,
            root,
            store,
        }
    }

    fn saved(scanned_at: u64) -> Fixture {
        let fx = fixture();
        fx.store.save(&tree_for(&fx.root, scanned_at)).unwrap();
        fx
    }

    #[test]
    fn streaming_cache_round_trip_preserves_tree() {
        let fx = saved(now_unix());
        let loaded = fx.store.load(&fx.root).unwrap();
        assert_eq!(loaded.tree.root_path, fx.root.to_string_lossy());
        assert_eq!(loaded.tree.total_size(), 8);
        assert_eq!(loaded.tree.item_count, 2);
        assert_eq!(loaded.tree.node(NodeId(1)).raw_name.as_deref(), Some(&b"e\xcc\x81"[..]));
        assert!(!loaded.stale);
        assert!(fx.store.load_if_fresh(&fx.root).is_ok());
        assert_eq!(fx.store.peek_meta(&fx.root).unwrap().item_count, 2);
        assert!(fs::read_dir(fx.store.dir())
            .unwrap()
            .all(|e| e.unwrap().path().extension().unwrap() != "tmp"));
    }

    #[test]
    fn missing_meta_or_bin_is_a_miss() {
        let fx = saved(now_unix());
        let (meta_path, bin_path) = fx.store.paths(&fx.root);
        fs::remove_file(&meta_path).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Miss)));
        fx.store.save(&tree_for(&fx.root, 1)).unwrap();
        fs::remove_file(&bin_path).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Miss)));
        assert!(matches!(
            fx.store.load(&fx.root.join("never")),
            Err(CacheError::Miss)
        ));
    }

    #[test]
    fn corrupt_and_truncated_bin_are_decode_errors() {
        let fx = saved(now_unix());
        let (_, bin_path) = fx.store.paths(&fx.root);
        let original = fs::read(&bin_path).unwrap();

        let mut flipped = original.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0xff;
        fs::write(&bin_path, &flipped).unwrap();
        match fx.store.load(&fx.root) {
            Err(CacheError::Decode(msg)) => assert!(msg.contains("checksum"), "{msg}"),
            other => panic!("{other:?}"),
        }

        fs::write(&bin_path, &original[..original.len() / 2]).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Decode(_))));
        fs::write(&bin_path, &original[..5]).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Decode(_))));

        let mut bad_magic = original.clone();
        bad_magic[0] = b'X';
        fs::write(&bin_path, &bad_magic).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Decode(_))));
        // Corrupt files are kept for inspection; only version mismatches are purged.
        assert!(bin_path.exists());
    }

    #[test]
    fn invalid_tree_structure_is_rejected() {
        let fx = fixture();
        let mut tree = tree_for(&fx.root, 1);
        tree.nodes[1].parent = Some(NodeId(7));
        fx.store.save(&tree).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Decode(_))));
    }

    #[test]
    fn version_mismatch_is_a_miss_and_deletes_files() {
        for which in ["meta", "bin"] {
            let fx = saved(now_unix());
            let (meta_path, bin_path) = fx.store.paths(&fx.root);
            let path = if which == "meta" { &meta_path } else { &bin_path };
            let mut bytes = fs::read(path).unwrap();
            bytes[8..12].copy_from_slice(&(CACHE_VERSION + 1).to_le_bytes());
            fs::write(path, bytes).unwrap();
            assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Miss)));
            assert!(!meta_path.exists(), "{which}");
            assert!(!bin_path.exists(), "{which}");
            assert!(fx.store.peek_meta(&fx.root).is_none());
        }
    }

    #[test]
    fn stale_when_root_changes_or_disappears() {
        let fx = saved(now_unix());
        let future = SystemTime::now() + Duration::from_secs(120);
        fs::File::open(&fx.root).unwrap().set_modified(future).unwrap();
        assert!(fx.store.load(&fx.root).unwrap().stale);
        assert!(matches!(
            fx.store.load_if_fresh(&fx.root),
            Err(CacheError::Miss)
        ));

        // Root modified after the scan started: saved as stale-on-next-load.
        let fx = fixture();
        fx.store.save(&tree_for(&fx.root, now_unix() - 600)).unwrap();
        assert!(fx.store.load(&fx.root).unwrap().stale);

        // A root that vanished is stale too (key stays the un-canonicalized path).
        let fx = saved(now_unix());
        fs::remove_dir_all(&fx.root).unwrap();
        let loaded = fx.store.load(&fx.root).unwrap();
        assert!(loaded.stale);
    }

    /// macOS temp dirs live under `/var`, a symlink to `/private/var`: once
    /// the root is gone it can't be canonicalized, so the key must fall back
    /// to the deepest existing ancestor instead of changing (and missing).
    #[cfg(unix)]
    #[test]
    fn vanished_root_behind_symlinked_parent_keeps_its_key() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(real.join("root")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let root = link.join("root");
        let store = CacheStore::with_dir(tmp.path().join("cache")).unwrap();
        store.save(&tree_for(&root, now_unix())).unwrap();
        assert_eq!(
            canonical_or_partial(&root),
            real.canonicalize().unwrap().join("root")
        );
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            canonical_or_partial(&root),
            real.canonicalize().unwrap().join("root")
        );
        let loaded = store.load(&root).unwrap();
        assert!(loaded.stale);
    }

    #[test]
    fn list_recent_orders_by_scan_time_desc() {
        let fx = fixture();
        for (name, at) in [("a", 10), ("b", 30), ("c", 20)] {
            let root = fx.root.join(name);
            fs::create_dir_all(&root).unwrap();
            fx.store.save(&tree_for(&root, at)).unwrap();
        }
        fs::write(fx.store.dir().join("junk.meta"), b"nope").unwrap();
        let recent = fx.store.list_recent().unwrap();
        let times: Vec<_> = recent.iter().map(|m| m.scanned_at_unix).collect();
        assert_eq!(times, [30, 20, 10]);
        assert!(recent[0].root_path.ends_with('b'));
    }

    #[test]
    fn invalidate_and_wipe_all_remove_files() {
        let fx = saved(now_unix());
        fx.store.invalidate(&fx.root).unwrap();
        assert!(matches!(fx.store.load(&fx.root), Err(CacheError::Miss)));
        fx.store.invalidate(&fx.root).unwrap();

        for name in ["x", "y"] {
            let root = fx.root.join(name);
            fs::create_dir_all(&root).unwrap();
            fx.store.save(&tree_for(&root, 1)).unwrap();
        }
        fs::write(fx.store.dir().join("left.bin.tmp"), b"tmp").unwrap();
        fs::write(fx.store.dir().join("keep.txt"), b"keep").unwrap();
        let (files, bytes) = fx.store.wipe_all().unwrap();
        assert_eq!(files, 5);
        assert!(bytes > 0);
        assert!(fx.store.dir().join("keep.txt").exists());
        assert!(fx.store.list_recent().unwrap().is_empty());
        assert_eq!(fx.store.wipe_all().unwrap(), (0, 0));
    }

    #[test]
    fn ttl_prunes_only_expired_entries() {
        let fx = fixture();
        let store = fx.store.clone().with_ttl_days(7);
        assert_eq!(store.ttl(), Some(Duration::from_secs(7 * 86_400)));
        let now = now_unix();
        for (name, at) in [("old", now - 10 * 86_400), ("new", now - 86_400)] {
            let root = fx.root.join(name);
            fs::create_dir_all(&root).unwrap();
            store.save(&tree_for(&root, at)).unwrap();
        }
        assert_eq!(fx.store.prune_expired().unwrap(), (0, 0));
        let (entries, bytes) = store.prune_expired().unwrap();
        assert_eq!(entries, 1);
        assert!(bytes > 0);
        let recent = store.list_recent().unwrap();
        assert_eq!(recent.len(), 1);
        assert!(recent[0].root_path.ends_with("new"));
        assert!(fx.store.clone().with_ttl_days(0).ttl().is_none());
    }

    #[test]
    fn lru_eviction_keeps_recently_loaded_entries() {
        let fx = fixture();
        let roots: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                let root = fx.root.join(name);
                fs::create_dir_all(&root).unwrap();
                fx.store.save(&tree_for(&root, 1)).unwrap();
                root
            })
            .collect();
        let per_entry = fx.store.total_bytes().unwrap() / 3;
        // Age "a" and "b"; "c" keeps its fresh save time. Loading "a" then
        // makes it the most recently used although it was saved first.
        let old = SystemTime::now() - Duration::from_secs(3_600);
        for root in &roots[..2] {
            let (meta_path, _) = fx.store.paths(root);
            fs::File::open(&meta_path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        fx.store.load(&roots[0]).unwrap();

        let store = fx.store.clone().with_max_bytes(per_entry * 2 + 1);
        assert_eq!(store.max_bytes(), Some(per_entry * 2 + 1));
        let (entries, _) = store.evict_to_budget().unwrap();
        assert_eq!(entries, 1);
        assert!(store.load(&roots[0]).is_ok(), "recently loaded survives");
        assert!(store.load(&roots[2]).is_ok(), "newest save survives");
        assert!(matches!(store.load(&roots[1]), Err(CacheError::Miss)));

        // Saving under a budget evicts automatically.
        let d = fx.root.join("d");
        fs::create_dir_all(&d).unwrap();
        store.save(&tree_for(&d, 1)).unwrap();
        assert_eq!(store.list_recent().unwrap().len(), 2);
        assert!(store.load(&d).is_ok());
        assert_eq!(fx.store.evict_to_budget().unwrap(), (0, 0));
    }
}
