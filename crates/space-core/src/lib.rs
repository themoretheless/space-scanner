//! Fast filesystem tree indexing with on-disk cache.

mod cache;
mod format;
mod scan;
mod tree;

pub use cache::{CacheError, CacheLoad, CacheMeta, CacheStore, CACHE_VERSION};
pub use format::format_bytes;
pub use scan::{
    is_volume_unavailable, rescan_subtree, scan_path, scan_path_with_expect, scan_path_with_options,
    should_descend, ScanEvent, ScanHandle, ScanOptions, ScanProgress, DEFAULT_SMALL_FILE_THRESHOLD,
};
pub use tree::{
    duplicate_groups, extension_totals, largest_entries, oldest_large_files, Node, NodeId,
    SizeMode, SpaceTree,
};

/// Seconds since the Unix epoch (0 when the clock is before the epoch).
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
