use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tree::SpaceTree;

const CACHE_MAGIC: &[u8; 8] = b"SPSCACHE";
/// Bump when `SpaceTree` / `Node` on-disk schema changes.
pub const CACHE_VERSION: u32 = 3;

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
    /// True when root mtime moved ahead of cached snapshot.
    pub stale: bool,
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
        let base = directories::ProjectDirs::from("dev", "space", "space-scanner")
            .ok_or(CacheError::NoDir)?;
        let dir = base.cache_dir().join("trees");
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn key_for(root: &Path) -> String {
        let canonical = root
            .canonicalize()
            .unwrap_or_else(|_| root.to_path_buf());
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

    /// Load cache even if stale; sets `stale` flag when root mtime advanced.
    pub fn load(&self, root: &Path) -> Result<CacheLoad, CacheError> {
        let (meta_path, bin_path) = self.paths(root);
        if !meta_path.exists() || !bin_path.exists() {
            return Err(CacheError::Miss);
        }

        let meta_bytes = fs::read(&meta_path)?;
        let meta: CacheMeta = bincode::deserialize(&meta_bytes)
            .map_err(|e| CacheError::Decode(e.to_string()))?;

        let current_mtime = path_mtime_secs(root).unwrap_or(0);
        let stale = current_mtime > meta.root_mtime_secs.saturating_add(1);

        let mut file = fs::File::open(&bin_path)?;
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
        let mut compressed = Vec::new();
        file.read_to_end(&mut compressed)?;
        let payload = zstd::decode_all(compressed.as_slice())
            .map_err(|e| CacheError::Decode(e.to_string()))?;
        let mut tree: SpaceTree = bincode::deserialize(&payload)
            .map_err(|e| CacheError::Decode(e.to_string()))?;
        tree.normalize_unicode_names();

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

        let meta = CacheMeta {
            root_path: tree.root_path.clone(),
            root_mtime_secs: path_mtime_secs(root).unwrap_or(0),
            scanned_at_unix: tree.scanned_at_unix,
            item_count: tree.item_count,
            total_size: tree.total_size(),
        };

        let meta_bytes =
            bincode::serialize(&meta).map_err(|e| CacheError::Encode(e.to_string()))?;
        let payload =
            bincode::serialize(tree).map_err(|e| CacheError::Encode(e.to_string()))?;
        let compressed =
            zstd::encode_all(payload.as_slice(), 3).map_err(|e| CacheError::Encode(e.to_string()))?;

        let tmp_bin = bin_path.with_extension("bin.tmp");
        {
            let mut f = fs::File::create(&tmp_bin)?;
            f.write_all(CACHE_MAGIC)?;
            f.write_all(&CACHE_VERSION.to_le_bytes())?;
            f.write_all(&compressed)?;
            f.sync_all()?;
        }
        fs::rename(&tmp_bin, &bin_path)?;
        fs::write(&meta_path, meta_bytes)?;

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
        out.sort_by(|a, b| b.scanned_at_unix.cmp(&a.scanned_at_unix));
        Ok(out)
    }
}

fn path_mtime_secs(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

#[allow(dead_code)]
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
