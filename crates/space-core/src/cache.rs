use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tree::SpaceTree;

const CACHE_MAGIC: &[u8; 8] = b"SPSCACHE";
/// Bump when `SpaceTree` / `Node` on-disk schema changes.
pub const CACHE_VERSION: u32 = 5;
const DEFAULT_MAX_CACHE_ENTRIES: usize = 32;

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
    #[serde(default)]
    pub last_accessed_unix: u64,
}

#[derive(Debug, Clone)]
pub struct CacheLoad {
    pub meta: CacheMeta,
    pub tree: SpaceTree,
    /// True when root mtime moved ahead of cached snapshot.
    pub stale: bool,
    /// True when the root path is currently unreachable/offline.
    pub offline: bool,
}

pub struct CacheStore {
    dir: PathBuf,
}

impl Clone for CacheStore {
    fn clone(&self) -> Self {
        Self {
            dir: self.dir.clone(),
        }
    }
}

impl CacheStore {
    pub fn open() -> Result<Self, CacheError> {
        let dir = if let Some(override_dir) =
            std::env::var_os("SPACE_CACHE_DIR").filter(|v| !v.is_empty())
        {
            PathBuf::from(override_dir)
        } else {
            let base = directories::ProjectDirs::from("dev", "space", "space-scanner")
                .ok_or(CacheError::NoDir)?;
            base.cache_dir().join("trees")
        };
        Self::with_dir(dir)
    }

    pub fn with_dir(dir: PathBuf) -> Result<Self, CacheError> {
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn key_for(root: &Path) -> String {
        let canonical = canonicalize_best_effort(root);
        let hash = blake3::hash(canonical.to_string_lossy().as_bytes());
        hash.to_hex().to_string()
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
        let bytes = fs::read(meta_path).ok()?;
        bincode::deserialize(&bytes).ok()
    }

    /// Load cache even if stale; sets `stale` when root mtime advanced or `offline` when unreachable.
    pub fn load(&self, root: &Path) -> Result<CacheLoad, CacheError> {
        self.load_with_ttl(root, 0)
    }

    /// Load cache enforcing `ttl_days` (`0` = no age limit); auto-deletes corrupt entries.
    pub fn load_with_ttl(&self, root: &Path, ttl_days: u32) -> Result<CacheLoad, CacheError> {
        let (meta_path, bin_path) = self.paths(root);
        if !meta_path.exists() || !bin_path.exists() {
            return Err(CacheError::Miss);
        }

        let mut meta: CacheMeta =
            match fs::read(&meta_path)
                .map_err(CacheError::Io)
                .and_then(|bytes| {
                    bincode::deserialize(&bytes).map_err(|e| CacheError::Decode(e.to_string()))
                }) {
                Ok(meta) => meta,
                Err(err) => {
                    let _ = self.invalidate(root);
                    return Err(err);
                }
            };

        let now = now_unix();
        if ttl_days > 0 && now.saturating_sub(meta.scanned_at_unix) > u64::from(ttl_days) * 86_400 {
            let _ = self.invalidate(root);
            return Err(CacheError::Miss);
        }

        let offline = !root.exists();
        let stale = if offline {
            false
        } else {
            let current_mtime = path_mtime_secs(root).unwrap_or(0);
            current_mtime > meta.root_mtime_secs.saturating_add(1)
        };

        let tree = match read_and_validate_tree(&bin_path) {
            Ok(tree) => tree,
            Err(err) => {
                let _ = self.invalidate(root);
                return Err(err);
            }
        };

        meta.last_accessed_unix = now;
        let _ = write_meta_atomic(&meta_path, &meta);

        Ok(CacheLoad {
            meta,
            tree,
            stale,
            offline,
        })
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

        let now = now_unix();
        let meta = CacheMeta {
            root_path: tree.root_path.clone(),
            root_mtime_secs: path_mtime_secs(root).unwrap_or(0),
            scanned_at_unix: tree.scanned_at_unix,
            item_count: tree.item_count,
            total_size: tree.total_size(),
            last_accessed_unix: now,
        };

        let tmp_bin = bin_path.with_extension("bin.tmp");
        {
            let mut f = fs::File::create(&tmp_bin)?;
            f.write_all(CACHE_MAGIC)?;
            f.write_all(&CACHE_VERSION.to_le_bytes())?;
            {
                let mut encoder = zstd::stream::write::Encoder::new(&mut f, 3)
                    .map_err(|e| CacheError::Encode(e.to_string()))?;
                bincode::serialize_into(&mut encoder, tree)
                    .map_err(|e| CacheError::Encode(e.to_string()))?;
                encoder
                    .finish()
                    .map_err(|e| CacheError::Encode(e.to_string()))?;
            }
            f.sync_all()?;
        }
        fs::rename(&tmp_bin, &bin_path)?;
        write_meta_atomic(&meta_path, &meta)?;
        let _ = self.evict_lru(DEFAULT_MAX_CACHE_ENTRIES);

        Ok(meta)
    }

    pub fn invalidate(&self, root: &Path) -> Result<(), CacheError> {
        let (meta_path, bin_path) = self.paths(root);
        let _ = fs::remove_file(meta_path);
        let _ = fs::remove_file(bin_path);
        Ok(())
    }

    pub fn list_recent(&self) -> Result<Vec<CacheMeta>, CacheError> {
        let mut out = Vec::new();
        if !self.dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(meta) = bincode::deserialize::<CacheMeta>(&bytes) {
                    out.push(meta);
                }
            }
        }
        out.sort_by_key(|meta| std::cmp::Reverse(meta.scanned_at_unix));
        Ok(out)
    }

    pub fn wipe_all(&self) -> Result<usize, CacheError> {
        if !self.dir.exists() {
            return Ok(0);
        }
        let mut removed = 0usize;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("meta") {
                removed += 1;
            }
            let _ = fs::remove_file(path);
        }
        Ok(removed)
    }

    pub fn prune_ttl(&self, ttl_days: u32) -> Result<usize, CacheError> {
        if ttl_days == 0 || !self.dir.exists() {
            return Ok(0);
        }
        let cutoff = now_unix().saturating_sub(u64::from(ttl_days).saturating_mul(86_400));
        let mut removed = 0usize;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }
            let expired = fs::read(&path)
                .ok()
                .and_then(|b| bincode::deserialize::<CacheMeta>(&b).ok())
                .is_none_or(|m| m.scanned_at_unix < cutoff);
            if expired {
                let bin = path.with_extension("bin");
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(bin);
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn evict_lru(&self, max_entries: usize) -> Result<usize, CacheError> {
        if !self.dir.exists() {
            return Ok(0);
        }
        let mut entries: Vec<(u64, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("meta") {
                continue;
            }
            let stamp = fs::read(&path)
                .ok()
                .and_then(|b| bincode::deserialize::<CacheMeta>(&b).ok())
                .map_or(0, |m| m.last_accessed_unix.max(m.scanned_at_unix));
            entries.push((stamp, path));
        }
        if entries.len() <= max_entries {
            return Ok(0);
        }
        entries.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
        let mut evicted = 0usize;
        for (_, meta_path) in entries.into_iter().skip(max_entries) {
            let bin_path = meta_path.with_extension("bin");
            let _ = fs::remove_file(&meta_path);
            let _ = fs::remove_file(bin_path);
            evicted += 1;
        }
        Ok(evicted)
    }
}

fn read_and_validate_tree(bin_path: &Path) -> Result<SpaceTree, CacheError> {
    let mut file = fs::File::open(bin_path)?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)?;
    if &magic != CACHE_MAGIC {
        return Err(CacheError::Decode("bad magic".into()));
    }
    let mut ver = [0u8; 4];
    file.read_exact(&mut ver)?;
    if u32::from_le_bytes(ver) != CACHE_VERSION {
        return Err(CacheError::Decode("version mismatch".into()));
    }
    let decoder =
        zstd::stream::read::Decoder::new(file).map_err(|e| CacheError::Decode(e.to_string()))?;
    let tree: SpaceTree =
        bincode::deserialize_from(decoder).map_err(|e| CacheError::Decode(e.to_string()))?;
    tree.validate()
        .map_err(|msg| CacheError::Decode(msg.into()))?;
    Ok(tree)
}

fn write_meta_atomic(meta_path: &Path, meta: &CacheMeta) -> Result<(), CacheError> {
    let bytes = bincode::serialize(meta).map_err(|e| CacheError::Encode(e.to_string()))?;
    let tmp = meta_path.with_extension("meta.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, meta_path)?;
    Ok(())
}

fn canonicalize_best_effort(path: &Path) -> PathBuf {
    if let Ok(c) = path.canonicalize() {
        return c;
    }
    let mut missing = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        if let Ok(canon) = p.canonicalize() {
            let mut out = canon;
            for part in missing.into_iter().rev() {
                out.push(part);
            }
            return out;
        }
        if let Some(name) = p.file_name() {
            missing.push(name.to_os_string());
        } else {
            break;
        }
        cur = p.parent();
    }
    path.to_path_buf()
}

fn path_mtime_secs(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Node, NodeId};

    fn sample_tree(root: &Path, scanned_at_unix: u64) -> SpaceTree {
        SpaceTree {
            root_path: root.to_string_lossy().into_owned(),
            nodes: vec![Node {
                name: "root".into(),
                apparent_size: 7,
                allocated_size: 8,
                file_count: 1,
                is_dir: true,
                synthetic: false,
                mtime_unix: 100,
                children: Vec::new(),
                parent: None,
            }],
            root: NodeId(0),
            scanned_at_unix,
            item_count: 1,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        }
    }

    fn temp_base(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "space-cache-{name}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn streaming_cache_round_trip_preserves_tree() {
        let base = temp_base("stream");
        let root = base.join("root");
        let cache_dir = base.join("cache");
        fs::create_dir_all(&root).unwrap();
        let store = CacheStore::with_dir(cache_dir).unwrap();
        let tree = sample_tree(&root, 123);

        store.save(&tree).unwrap();
        let loaded = store.load(&root).unwrap();
        assert_eq!(loaded.tree.root_path, tree.root_path);
        assert_eq!(loaded.tree.total_size(), 8);
        assert_eq!(loaded.tree.item_count, 1);
        assert_eq!(loaded.meta.scanned_at_unix, 123);
        assert!(!loaded.offline);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn corrupt_entry_is_auto_deleted_and_offline_root_flags_offline() {
        let base = temp_base("corrupt");
        let root = base.join("root");
        let cache_dir = base.join("cache");
        fs::create_dir_all(&root).unwrap();
        let store = CacheStore::with_dir(cache_dir).unwrap();
        store.save(&sample_tree(&root, now_unix())).unwrap();

        let (meta_path, bin_path) = store.paths(&root);
        fs::write(&bin_path, b"NOT_VALID_CACHE").unwrap();
        assert!(matches!(store.load(&root), Err(CacheError::Decode(_))));
        assert!(!meta_path.exists());
        assert!(!bin_path.exists());

        store.save(&sample_tree(&root, now_unix())).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let loaded = store.load(&root).unwrap();
        assert!(loaded.offline);
        assert!(!loaded.stale);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn ttl_eviction_lru_and_wipe_all_work() {
        let base = temp_base("evict");
        let cache_dir = base.join("cache");
        let store = CacheStore::with_dir(cache_dir).unwrap();

        let r1 = base.join("r1");
        let r2 = base.join("r2");
        let r3 = base.join("r3");
        for r in [&r1, &r2, &r3] {
            fs::create_dir_all(r).unwrap();
        }
        store.save(&sample_tree(&r1, 1)).unwrap();
        store.save(&sample_tree(&r2, now_unix())).unwrap();
        store.save(&sample_tree(&r3, now_unix())).unwrap();

        assert!(matches!(store.load_with_ttl(&r1, 1), Err(CacheError::Miss)));
        assert_eq!(store.list_recent().unwrap().len(), 2);

        // Save r1 again as newest, then keep max 2 via LRU
        store.save(&sample_tree(&r1, now_unix() + 10)).unwrap();
        assert_eq!(store.evict_lru(2).unwrap(), 1);
        assert_eq!(store.list_recent().unwrap().len(), 2);

        assert_eq!(store.wipe_all().unwrap(), 2);
        assert!(store.list_recent().unwrap().is_empty());
        fs::remove_dir_all(base).unwrap();
    }
}
