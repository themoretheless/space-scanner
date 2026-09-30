//! Files directly under a folder, cached per tree snapshot and filter.

use std::sync::{Arc, Weak};

use space_core::{NodeId, SizeMode, SpaceTree};

use crate::views::ViewFilter;

/// Derived rows belong to an immutable tree snapshot and a specific filter.
#[derive(Default)]
pub struct FolderFiles {
    tree: Weak<SpaceTree>,
    key: Option<(NodeId, SizeMode, u64, usize)>,
    rows: Vec<(NodeId, u64)>,
}

impl FolderFiles {
    pub fn get(
        &mut self,
        tree: &Arc<SpaceTree>,
        dir: NodeId,
        mode: SizeMode,
        filter: &ViewFilter,
        limit: usize,
    ) -> &[(NodeId, u64)] {
        let key = (dir, mode, filter.hash64(), limit);
        let snapshot = Arc::downgrade(tree);
        if self.key != Some(key) || !self.tree.ptr_eq(&snapshot) {
            self.rows = folder_files(tree, dir, mode, filter, limit);
            self.tree = snapshot;
            self.key = Some(key);
        }
        &self.rows
    }
}

/// Files directly under `dir` (not recursive), filter-aware, size-desc.
pub fn folder_files(
    tree: &SpaceTree,
    dir: NodeId,
    mode: SizeMode,
    filter: &ViewFilter,
    limit: usize,
) -> Vec<(NodeId, u64)> {
    let mut files: Vec<(NodeId, u64)> = tree
        .children(dir)
        .iter()
        .copied()
        .filter_map(|cid| {
            let n = tree.node(cid);
            if n.is_dir {
                return None;
            }
            let sz = n.size(mode);
            if filter.is_active() && !filter.matches(&n.name, false, sz) {
                return None;
            }
            Some((cid, sz))
        })
        .collect();
    files.sort_by_key(|row| std::cmp::Reverse(row.1));
    files.truncate(limit);
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_core::Node;

    fn fixture(count: usize) -> Arc<SpaceTree> {
        let mut nodes = vec![Node {
            name: "root".into(),
            apparent_size: 0,
            allocated_size: 0,
            file_count: count as u64,
            is_dir: true,
            synthetic: false,
            children: (1..=count).map(|i| NodeId(i as u32)).collect(),
            parent: None,
        }];
        for i in 1..=count {
            nodes.push(Node {
                name: format!("file-{i}.log"),
                apparent_size: (i * 7919 % (count + 1)) as u64,
                allocated_size: (count - i) as u64,
                file_count: 1,
                is_dir: false,
                synthetic: false,
                children: vec![],
                parent: Some(NodeId(0)),
            });
        }
        Arc::new(SpaceTree {
            root_path: "/fixture".into(),
            nodes,
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: count as u64,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        })
    }

    #[test]
    fn cached_files_follow_filter_mode_limit_and_directory() {
        let mut tree = fixture(20);
        let empty = Node {
            name: "empty".into(),
            is_dir: true,
            children: vec![],
            ..tree.nodes[0].clone()
        };
        Arc::make_mut(&mut tree).nodes.push(empty);
        let mut cache = FolderFiles::default();
        for mode in [SizeMode::Apparent, SizeMode::OnDisk] {
            for limit in [0, 1, 10, 100] {
                for query in ["", "file-1", "missing"] {
                    let filter = ViewFilter {
                        query: query.into(),
                        ..Default::default()
                    };
                    for dir in [NodeId(0), NodeId(21)] {
                        let expected = folder_files(&tree, dir, mode, &filter, limit);
                        assert_eq!(cache.get(&tree, dir, mode, &filter, limit), expected);
                        let previous = cache.rows.as_ptr();
                        assert_eq!(cache.get(&tree, dir, mode, &filter, limit), expected);
                        assert_eq!(cache.rows.as_ptr(), previous);
                    }
                }
            }
        }
    }

    #[test]
    fn cached_files_follow_tree_replacement_and_in_place_edits() {
        let mut tree = fixture(10);
        let mut cache = FolderFiles::default();
        let filter = ViewFilter::default();
        let original = cache
            .get(&tree, tree.root, SizeMode::Apparent, &filter, 100)
            .to_vec();
        // Arc::make_mut detaches the cached Weak even when there is no other strong owner.
        Arc::make_mut(&mut tree).nodes[1].apparent_size = 999;
        let edited = cache.get(&tree, tree.root, SizeMode::Apparent, &filter, 100);
        assert_eq!(edited[0], (NodeId(1), 999));
        assert_ne!(edited, original);
        tree = fixture(10);
        assert_eq!(
            cache.get(&tree, tree.root, SizeMode::Apparent, &filter, 100),
            original
        );
    }

    #[test]
    #[ignore = "manual release profile: --release folder_files_profile -- --ignored --nocapture"]
    fn folder_files_profile() {
        use std::hint::black_box;
        use std::time::Instant;
        let tree = fixture(50_000);
        let filter = ViewFilter::default();
        let mut cache = FolderFiles::default();
        let expected = folder_files(&tree, tree.root, SizeMode::Apparent, &filter, 100);
        assert_eq!(
            cache.get(&tree, tree.root, SizeMode::Apparent, &filter, 100),
            expected
        );
        let mut uncached = Vec::new();
        let mut cached = Vec::new();
        for _ in 0..31 {
            let start = Instant::now();
            black_box(folder_files(
                black_box(&tree),
                tree.root,
                SizeMode::Apparent,
                &filter,
                100,
            ));
            uncached.push(start.elapsed());
            let start = Instant::now();
            for _ in 0..1_000 {
                black_box(cache.get(
                    black_box(&tree),
                    tree.root,
                    SizeMode::Apparent,
                    &filter,
                    100,
                ));
            }
            cached.push(start.elapsed() / 1_000);
        }
        uncached.sort();
        cached.sort();
        println!(
            "50,000 files, top 100: uncached {:?}, unchanged cached {:?}",
            uncached[15], cached[15]
        );
    }
}
