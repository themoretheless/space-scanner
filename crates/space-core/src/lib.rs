//! Fast filesystem tree indexing with on-disk cache.

mod cache;
mod format;
mod scan;
mod tree;

pub use cache::{CacheError, CacheLoad, CacheMeta, CacheStore, CACHE_VERSION};
pub use format::format_bytes;
pub use scan::{
    rescan_subtree, scan_path, scan_path_with_expect, ScanEvent, ScanHandle, ScanProgress,
};
pub use tree::{largest_entries, Node, NodeId, SizeMode, SpaceTree};

/// Seconds since the Unix epoch (0 when the clock is before the epoch).
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
