//! Extra reports in the contents pane beside Largest files/folders: oldest
//! large files, likely duplicates and size by file type. Each is computed off
//! the UI thread and cached per (tree generation, focus, size mode, kind).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use space_core::{NodeId, SizeMode, SpaceTree};

/// Which report the contents pane shows instead of the largest list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportKind {
    Oldest,
    Duplicates,
    Types,
}

/// Rows a report lists at most.
pub const REPORT_LIMIT: usize = 200;
/// Files smaller than this are not considered duplicates worth listing.
pub const DUPLICATE_MIN_SIZE: u64 = 1024 * 1024;
/// Minimum-size presets for the Oldest report, in MiB.
pub const OLDEST_PRESETS_MB: [u64; 4] = [10, 100, 1024, 10 * 1024];

const MIB: u64 = 1024 * 1024;

/// Everything a report's rows depend on. Whole-scan reports (Oldest,
/// Duplicates) use the tree root as their focus so navigation reuses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportKey {
    pub tree_gen: u64,
    pub focus: NodeId,
    pub mode: SizeMode,
    pub kind: ReportKind,
    /// Minimum file size (Oldest only; 0 otherwise).
    pub min_size: u64,
}

impl ReportKey {
    pub fn new(tree: &SpaceTree, tree_gen: u64, focus: NodeId, mode: SizeMode, kind: ReportKind, oldest_min_mb: u64) -> Self {
        let (focus, min_size) = match kind {
            ReportKind::Types => (focus, 0),
            ReportKind::Oldest => (tree.root, oldest_min_mb.saturating_mul(MIB)),
            ReportKind::Duplicates => (tree.root, 0),
        };
        Self { tree_gen, focus, mode, kind, min_size }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportRequest {
    generation: u64,
    pub key: ReportKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub name: String,
    pub members: Vec<NodeId>,
    /// Size of one copy.
    pub size: u64,
    /// Bytes the extra copies take.
    pub wasted: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportData {
    /// (file, size, mtime)
    Oldest(Vec<(NodeId, u64, u32)>),
    Duplicates(Vec<DuplicateGroup>),
    /// (extension, bytes, files)
    Types(Vec<(String, u64, u64)>),
}

/// Kept results; a few keys are enough to flip between reports instantly.
const CACHE_KEYS: usize = 8;

#[derive(Default)]
pub struct Reports {
    generation: u64,
    pending: Option<ReportRequest>,
    cache: Vec<(ReportKey, Arc<ReportData>)>,
}

impl Reports {
    /// Drop every result and forget the running query (tree changed).
    pub fn invalidate(&mut self) {
        self.pending = None;
        self.cache.clear();
    }

    /// A request when `key` is neither cached nor already running.
    pub fn begin(&mut self, key: ReportKey) -> Option<ReportRequest> {
        if self.cache.iter().any(|(k, _)| *k == key) || self.pending.is_some_and(|p| p.key == key) {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let request = ReportRequest { generation: self.generation, key };
        self.pending = Some(request);
        Some(request)
    }

    pub fn complete(&mut self, request: ReportRequest, data: ReportData) {
        if self.pending != Some(request) {
            return;
        }
        self.pending = None;
        // Results of an older tree are useless once a newer one is cached.
        self.cache.retain(|(k, _)| k.tree_gen == request.key.tree_gen);
        if self.cache.len() >= CACHE_KEYS {
            self.cache.clear();
        }
        self.cache.push((request.key, Arc::new(data)));
    }

    pub fn failed(&mut self, request: ReportRequest) {
        if self.pending == Some(request) {
            self.pending = None;
        }
    }

    pub fn get(&self, key: &ReportKey) -> Option<Arc<ReportData>> {
        self.cache.iter().find(|(k, _)| k == key).map(|(_, d)| d.clone())
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }
}

/// Run one report (on a worker thread).
pub fn compute(tree: &SpaceTree, key: ReportKey) -> ReportData {
    match key.kind {
        ReportKind::Oldest => ReportData::Oldest(
            space_core::oldest_large_files(tree, key.mode, key.min_size, REPORT_LIMIT)
                .into_iter()
                .map(|id| {
                    let n = tree.node(id);
                    (id, n.size(key.mode), n.mtime)
                })
                .collect(),
        ),
        ReportKind::Duplicates => ReportData::Duplicates(
            space_core::duplicate_groups(tree, DUPLICATE_MIN_SIZE)
                .into_iter()
                .take(REPORT_LIMIT)
                .map(|members| {
                    let size = members
                        .iter()
                        .map(|&id| {
                            let n = tree.node(id);
                            n.allocated_size.max(n.apparent_size)
                        })
                        .max()
                        .unwrap_or(0);
                    DuplicateGroup {
                        name: tree.node(members[0]).name.clone(),
                        wasted: size.saturating_mul(members.len() as u64 - 1),
                        size,
                        members,
                    }
                })
                .collect(),
        ),
        ReportKind::Types => {
            let mut rows = space_core::extension_totals(tree, key.focus, key.mode);
            rows.truncate(REPORT_LIMIT);
            ReportData::Types(rows)
        }
    }
}

/// Every copy but the newest (by modification time, then the higher node id).
pub fn extras(tree: &SpaceTree, group: &DuplicateGroup) -> Vec<NodeId> {
    let Some(&newest) = group
        .members
        .iter()
        .max_by_key(|&&id| (tree.node(id).mtime, id.0))
    else {
        return Vec::new();
    };
    group.members.iter().copied().filter(|&id| id != newest).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::node;

    fn tree() -> SpaceTree {
        let mut nodes = vec![
            node("r", 0, 0, 0, true, false, None, vec![NodeId(1), NodeId(2)]),
            node("a", 0, 0, 0, true, false, Some(NodeId(0)), vec![NodeId(3), NodeId(5)]),
            node("b", 0, 0, 0, true, false, Some(NodeId(0)), vec![NodeId(4)]),
            node("Movie.mkv", 5 * MIB, 5 * MIB, 1, false, false, Some(NodeId(1)), vec![]),
            node("movie.mkv", 5 * MIB, 5 * MIB, 1, false, false, Some(NodeId(2)), vec![]),
            node("notes.txt", 2 * MIB, 2 * MIB, 1, false, false, Some(NodeId(1)), vec![]),
        ];
        nodes[3].mtime = 100;
        nodes[4].mtime = 200;
        nodes[5].mtime = 50;
        SpaceTree {
            root_path: "/r".into(),
            nodes,
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 6,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        }
    }

    fn key(gen: u64, kind: ReportKind) -> ReportKey {
        ReportKey::new(&tree(), gen, NodeId(1), SizeMode::OnDisk, kind, 1)
    }

    #[test]
    fn whole_scan_reports_ignore_the_focus_and_types_keep_it() {
        let t = tree();
        let a = ReportKey::new(&t, 1, NodeId(1), SizeMode::OnDisk, ReportKind::Duplicates, 100);
        let b = ReportKey::new(&t, 1, NodeId(2), SizeMode::OnDisk, ReportKind::Duplicates, 100);
        assert_eq!(a, b);
        let c = ReportKey::new(&t, 1, NodeId(1), SizeMode::OnDisk, ReportKind::Types, 100);
        let d = ReportKey::new(&t, 1, NodeId(2), SizeMode::OnDisk, ReportKind::Types, 100);
        assert_ne!(c, d);
        assert_eq!(key(1, ReportKind::Oldest).min_size, MIB);
    }

    #[test]
    fn cached_keys_start_no_query_and_stale_generations_are_dropped() {
        let mut r = Reports::default();
        let k = key(1, ReportKind::Types);
        let req = r.begin(k).unwrap();
        assert_eq!(r.begin(k), None, "running query is not restarted");
        r.complete(req, ReportData::Types(vec![]));
        assert!(!r.is_loading());
        assert_eq!(r.begin(k), None, "cached key is reused");
        assert!(r.get(&k).is_some());

        // A newer tree generation: the old answer arrives late and is ignored.
        let old = r.begin(key(2, ReportKind::Oldest)).unwrap();
        r.invalidate();
        let new = r.begin(key(3, ReportKind::Oldest)).unwrap();
        r.complete(old, ReportData::Oldest(vec![(NodeId(9), 1, 1)]));
        assert!(r.get(&key(2, ReportKind::Oldest)).is_none());
        assert!(r.is_loading());
        r.complete(new, ReportData::Oldest(vec![]));
        assert!(r.get(&key(3, ReportKind::Oldest)).is_some());
        assert!(r.get(&k).is_none(), "results of older trees are evicted");
    }

    #[test]
    fn failed_requests_can_be_retried() {
        let mut r = Reports::default();
        let k = key(1, ReportKind::Duplicates);
        let a = r.begin(k).unwrap();
        r.failed(a);
        assert!(!r.is_loading());
        let b = r.begin(k).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn reports_compute_and_extras_keep_the_newest_copy() {
        let t = tree();
        let ReportData::Duplicates(groups) = compute(&t, key(1, ReportKind::Duplicates)) else {
            panic!("duplicates")
        };
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 2);
        assert_eq!(groups[0].wasted, 5 * MIB);
        assert_eq!(extras(&t, &groups[0]), vec![NodeId(3)]);

        let ReportData::Oldest(rows) = compute(&t, key(1, ReportKind::Oldest)) else {
            panic!("oldest")
        };
        assert_eq!(rows.first().map(|r| r.0), Some(NodeId(5)));

        let ReportData::Types(rows) = compute(&t, key(1, ReportKind::Types)) else {
            panic!("types")
        };
        assert_eq!(rows[0], ("mkv".to_string(), 5 * MIB, 1));
        assert_eq!(rows[1].0, "txt");
    }
}
