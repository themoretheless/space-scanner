use std::borrow::Cow;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::ffi::OsStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SizeMode {
    Apparent,
    #[default]
    OnDisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Display name: NFC-normalized, lossily converted to UTF-8.
    pub name: String,
    /// Logical size (sum of file lengths / subtree).
    pub apparent_size: u64,
    /// Allocated on-disk size (st_blocks*512), hardlink-deduped in scan.
    pub allocated_size: u64,
    /// Descendant file count (dirs count nested files; files are 1).
    pub file_count: u64,
    pub is_dir: bool,
    /// Aggregated placeholder for small files, not a filesystem path.
    pub synthetic: bool,
    /// Children pre-sorted by allocated_size desc at scan time.
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    /// Exact on-disk name bytes, present only when they differ from `name`
    /// (non-UTF-8 or NFD names). Used to rebuild real filesystem paths.
    #[serde(default)]
    pub raw_name: Option<Vec<u8>>,
}

impl Node {
    pub fn size(&self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Apparent => self.apparent_size,
            SizeMode::OnDisk => self.allocated_size,
        }
    }

    /// The real on-disk file name (falls back to the display name).
    pub fn os_name(&self) -> Cow<'_, OsStr> {
        match &self.raw_name {
            Some(raw) => raw_os_name(raw),
            None => Cow::Borrowed(OsStr::new(&self.name)),
        }
    }
}

#[cfg(unix)]
fn raw_os_name(raw: &[u8]) -> Cow<'_, OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(OsStr::from_bytes(raw))
}

#[cfg(not(unix))]
fn raw_os_name(raw: &[u8]) -> Cow<'_, OsStr> {
    Cow::Owned(String::from_utf8_lossy(raw).into_owned().into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceTree {
    pub root_path: String,
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub scanned_at_unix: u64,
    pub item_count: u64,
    pub skipped_errors: u64,
    /// Files encountered with link count > 1. Used to preserve exact
    /// allocated-size accounting during incremental subtree rescans.
    pub hardlink_candidates: u64,
    pub hardlinks_deduped: u64,
}

impl SpaceTree {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0 as usize]
    }

    pub fn try_node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)
    }

    /// Pre-sorted children (no clone).
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }

    pub fn path_to(&self, id: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::with_capacity(16);
        let mut cur = Some(id);
        while let Some(n) = cur {
            chain.push(n);
            cur = self.node(n).parent;
        }
        chain.reverse();
        chain
    }

    pub fn breadcrumb_names(&self, id: NodeId) -> Vec<String> {
        self.path_to(id)
            .into_iter()
            .map(|n| self.node(n).name.clone())
            .collect()
    }

    pub fn total_size(&self) -> u64 {
        self.node(self.root).allocated_size
    }

    pub fn total_size_mode(&self, mode: SizeMode) -> u64 {
        self.node(self.root).size(mode)
    }

    /// Display path (lossy, NFC). Use [`SpaceTree::path_os`] for filesystem access.
    pub fn path_string(&self, id: NodeId) -> String {
        let chain = self.path_to(id);
        if chain.iter().any(|&n| self.node(n).raw_name.is_some()) {
            return self.path_os(id).to_string_lossy().into_owned();
        }
        let root = self.root_path.trim_end_matches('/');
        let capacity = root.len()
            + chain
                .iter()
                .skip(1)
                .map(|&node| self.node(node).name.len() + 1)
                .sum::<usize>();
        let mut path = String::with_capacity(capacity);
        path.push_str(root);
        for &node in chain.iter().skip(1) {
            path.push('/');
            path.push_str(&self.node(node).name);
        }
        path
    }

    /// Real on-disk path built from raw name bytes where they differ from
    /// the display name. Synthetic nodes have no path of their own.
    pub fn path_os(&self, id: NodeId) -> PathBuf {
        let chain = self.path_to(id);
        let mut path = PathBuf::from(self.root_path.trim_end_matches('/'));
        if path.as_os_str().is_empty() {
            path.push("/");
        }
        for &node in chain.iter().skip(1) {
            path.push(self.node(node).os_name());
        }
        path
    }

    /// Largest files under `id`, bounded walk (avoids UI freezes on huge trees).
    pub fn largest_files(&self, id: NodeId, mode: SizeMode, limit: usize) -> Vec<NodeId> {
        self.largest_files_bounded(id, mode, limit, 25_000)
    }

    pub fn largest_files_bounded(
        &self,
        id: NodeId,
        mode: SizeMode,
        limit: usize,
        max_visit: usize,
    ) -> Vec<NodeId> {
        if limit == 0 || max_visit == 0 {
            return Vec::new();
        }
        let mut files = self.collect_files_bounded(id, max_visit);
        // Include discovery order so selection preserves the previous stable
        // ordering of files with identical sizes and names, even at the cutoff.
        let compare = |&(a, a_order): &(NodeId, usize), &(b, b_order): &(NodeId, usize)| {
            self.compare_nodes(a, b, mode)
                .then_with(|| a_order.cmp(&b_order))
        };
        if files.len() > limit {
            files.select_nth_unstable_by(limit, compare);
        }
        files.truncate(limit);
        files.sort_unstable_by(compare);
        files.into_iter().map(|(id, _)| id).collect()
    }

    fn collect_files_bounded(&self, id: NodeId, max_visit: usize) -> Vec<(NodeId, usize)> {
        let mut files = Vec::new();
        let mut visited = 0;
        // Retain only one child iterator per level. Deep trees cannot exhaust
        // the call stack, and wide directories do not enqueue unvisited nodes.
        let mut stack = vec![std::slice::from_ref(&id).iter()];
        while visited < max_visit {
            let Some(children) = stack.last_mut() else {
                break;
            };
            let Some(&child) = children.next() else {
                stack.pop();
                continue;
            };
            visited += 1;
            let node = self.node(child);
            if node.is_dir {
                stack.push(node.children.iter());
            } else {
                files.push((child, files.len()));
            }
        }
        files
    }

    fn compare_nodes(&self, a: NodeId, b: NodeId, mode: SizeMode) -> Ordering {
        self.node(b)
            .size(mode)
            .cmp(&self.node(a).size(mode))
            .then_with(|| self.node(a).name.cmp(&self.node(b).name))
    }

    /// Remove node from parent and subtract sizes up the ancestor chain.
    pub fn remove_node(&mut self, id: NodeId) {
        let (parent, apparent, allocated, file_count) = {
            let n = self.node(id);
            (n.parent, n.apparent_size, n.allocated_size, n.file_count)
        };
        if let Some(pid) = parent {
            self.node_mut(pid).children.retain(|c| *c != id);
            let mut cur = Some(pid);
            while let Some(cid) = cur {
                let n = self.node_mut(cid);
                n.apparent_size = n.apparent_size.saturating_sub(apparent);
                n.allocated_size = n.allocated_size.saturating_sub(allocated);
                n.file_count = n.file_count.saturating_sub(file_count);
                cur = n.parent;
            }
        }
    }

    /// Re-sort every directory's children: size desc, then name, then id.
    pub fn sort_children_by(&mut self, mode: SizeMode) {
        for i in 0..self.nodes.len() {
            if !self.nodes[i].is_dir {
                continue;
            }
            let mut kids = std::mem::take(&mut self.nodes[i].children);
            kids.sort_unstable_by(|&a, &b| {
                self.compare_nodes(a, b, mode)
                    .then_with(|| a.0.cmp(&b.0))
            });
            self.nodes[i].children = kids;
        }
    }

    /// Checks structural invariants: dense ids, root at 0 without a parent,
    /// every child pointing back at its parent, `item_count` matching.
    pub fn validate(&self) -> Result<(), String> {
        let len = self.nodes.len();
        if len == 0 {
            return Err("tree has no nodes".into());
        }
        if self.root != NodeId(0) {
            return Err(format!("root id {} is not 0", self.root.0));
        }
        if self.item_count != len as u64 {
            return Err(format!("item_count {} != nodes {len}", self.item_count));
        }
        if self.nodes[0].parent.is_some() {
            return Err("root has a parent".into());
        }
        for (i, node) in self.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                let p = parent.0 as usize;
                if p >= len {
                    return Err(format!("node {i} parent {p} out of range"));
                }
                if !self.nodes[p].children.contains(&NodeId(i as u32)) {
                    return Err(format!("node {i} missing from parent {p}"));
                }
            } else if i != 0 {
                return Err(format!("node {i} has no parent"));
            }
            for child in &node.children {
                let c = child.0 as usize;
                if c >= len {
                    return Err(format!("node {i} child {c} out of range"));
                }
                if self.nodes[c].parent != Some(NodeId(i as u32)) {
                    return Err(format!("child {c} does not point back at {i}"));
                }
            }
        }
        Ok(())
    }
}

/// Largest entries across the whole tree, excluding the root and synthetic
/// small-file aggregates. Ties keep the lower node id first. Directory totals
/// overlap their descendants.
pub fn largest_entries(
    tree: &SpaceTree,
    mode: SizeMode,
    limit: usize,
    dirs: bool,
    files: bool,
) -> Vec<NodeId> {
    if limit == 0 || !(dirs || files) {
        return Vec::new();
    }
    let mut heap = BinaryHeap::with_capacity(limit.saturating_add(1).min(tree.nodes.len()));
    for (i, node) in tree.nodes.iter().enumerate() {
        if i == tree.root.0 as usize || node.synthetic {
            continue;
        }
        if (node.is_dir && !dirs) || (!node.is_dir && !files) {
            continue;
        }
        heap.push(Reverse((node.size(mode), Reverse(i))));
        if heap.len() > limit {
            heap.pop();
        }
    }
    let mut ranked: Vec<_> = heap.into_iter().map(|Reverse(v)| v).collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    ranked
        .into_iter()
        .map(|(_, Reverse(i))| NodeId(i as u32))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: Option<NodeId>, is_dir: bool) -> Node {
        Node {
            name: name.into(),
            apparent_size: 0,
            allocated_size: 0,
            file_count: u64::from(!is_dir),
            is_dir,
            synthetic: false,
            children: Vec::new(),
            parent,
            raw_name: None,
        }
    }

    fn empty_tree() -> SpaceTree {
        SpaceTree {
            root_path: "/tmp/root///".into(),
            nodes: vec![node("root", None, true)],
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 1,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        }
    }

    fn add_node(tree: &mut SpaceTree, parent: NodeId, node: Node) -> NodeId {
        let id = NodeId(tree.nodes.len() as u32);
        tree.nodes.push(node);
        tree.node_mut(parent).children.push(id);
        tree.item_count += 1;
        id
    }

    fn mixed_tree() -> SpaceTree {
        let mut tree = empty_tree();
        let root = tree.root;
        for directory in 0..7 {
            let dir = add_node(
                &mut tree,
                root,
                node(&format!("dir-{directory}"), Some(root), true),
            );
            for index in 0..37 {
                let mut file = node(&format!("file-{}", index % 4), Some(dir), false);
                file.apparent_size = index % 7;
                file.allocated_size = (index * 3 + 1) % 5;
                file.synthetic = index % 11 == 0;
                add_node(&mut tree, dir, file);
            }
            tree.node_mut(dir).children.reverse();
        }
        tree.node_mut(root).children.reverse();
        tree
    }

    // Original bounded depth-first traversal and stable full-sort behavior.
    fn reference_largest(
        tree: &SpaceTree,
        root: NodeId,
        mode: SizeMode,
        limit: usize,
        max_visit: usize,
    ) -> Vec<NodeId> {
        fn visit(tree: &SpaceTree, id: NodeId, remaining: &mut usize, files: &mut Vec<NodeId>) {
            if *remaining == 0 {
                return;
            }
            *remaining -= 1;
            let node = tree.node(id);
            if node.is_dir {
                for &child in &node.children {
                    visit(tree, child, remaining, files);
                }
            } else {
                files.push(id);
            }
        }
        let mut files = Vec::new();
        let mut remaining = max_visit;
        visit(tree, root, &mut remaining, &mut files);
        files.sort_by(|&a, &b| {
            tree.node(b)
                .size(mode)
                .cmp(&tree.node(a).size(mode))
                .then_with(|| tree.node(a).name.cmp(&tree.node(b).name))
        });
        files.truncate(limit);
        files
    }

    #[test]
    fn largest_files_preserves_visit_budget_modes_and_stable_ties() {
        let tree = mixed_tree();
        let directory = tree.children(tree.root)[0];
        let file = tree.children(directory)[0];
        for start in [tree.root, directory, file] {
            for mode in [SizeMode::Apparent, SizeMode::OnDisk] {
                for limit in [0, 1, 2, 15, 50, 260, usize::MAX] {
                    for max_visit in [0, 1, 2, 17, 64, 200, 1_000, usize::MAX] {
                        assert_eq!(
                            tree.largest_files_bounded(start, mode, limit, max_visit),
                            reference_largest(&tree, start, mode, limit, max_visit),
                            "start={start:?}, mode={mode:?}, limit={limit}, budget={max_visit}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn largest_files_walks_deep_trees_without_recursion() {
        let mut tree = empty_tree();
        let mut parent = tree.root;
        for _ in 1..30_000 {
            parent = add_node(&mut tree, parent, node("dir", Some(parent), true));
        }
        tree.node_mut(parent).is_dir = false;
        assert!(tree
            .largest_files(tree.root, SizeMode::OnDisk, 1)
            .is_empty());
        assert!(tree
            .largest_files_bounded(tree.root, SizeMode::OnDisk, 1, 29_999)
            .is_empty());
        assert_eq!(
            tree.largest_files_bounded(tree.root, SizeMode::OnDisk, 1, 30_000),
            vec![parent],
        );
    }

    #[test]
    fn path_string_preserves_root_unicode_and_synthetic_names() {
        let mut tree = empty_tree();
        let root = tree.root;
        let directory = add_node(&mut tree, root, node("résumé", Some(root), true));
        let mut file = node("· 3 small files", Some(directory), false);
        file.synthetic = true;
        let file = add_node(&mut tree, directory, file);
        assert_eq!(tree.path_string(root), "/tmp/root");
        assert_eq!(tree.path_string(directory), "/tmp/root/résumé");
        assert_eq!(tree.path_string(file), "/tmp/root/résumé/· 3 small files");
        assert_eq!(tree.path_os(directory), PathBuf::from("/tmp/root/résumé"));
        tree.root_path = "/".into();
        assert_eq!(tree.path_string(directory), "/résumé");
        assert_eq!(tree.path_os(directory), PathBuf::from("/résumé"));
    }

    #[cfg(unix)]
    #[test]
    fn path_os_uses_raw_bytes_when_present() {
        use std::os::unix::ffi::OsStrExt;
        let mut tree = empty_tree();
        let root = tree.root;
        let mut nfd = node("é", Some(root), true);
        nfd.raw_name = Some("e\u{301}".as_bytes().to_vec());
        let nfd = add_node(&mut tree, root, nfd);
        let mut bad = node("\u{FFFD}", Some(nfd), false);
        bad.raw_name = Some(vec![0xff]);
        let bad = add_node(&mut tree, nfd, bad);
        assert_eq!(
            tree.path_os(bad).as_os_str().as_bytes(),
            b"/tmp/root/e\xcc\x81/\xff"
        );
        assert_eq!(tree.path_string(nfd), "/tmp/root/e\u{301}");
        assert_eq!(tree.path_string(bad), "/tmp/root/e\u{301}/\u{FFFD}");
    }

    #[test]
    fn child_sort_preserves_ties_in_each_size_mode() {
        let mut tree = mixed_tree();
        for mode in [SizeMode::Apparent, SizeMode::OnDisk, SizeMode::Apparent] {
            let expected: Vec<_> = tree
                .nodes
                .iter()
                .map(|node| {
                    let mut children = node.children.clone();
                    children.sort_by(|&a, &b| {
                        tree.node(b)
                            .size(mode)
                            .cmp(&tree.node(a).size(mode))
                            .then_with(|| tree.node(a).name.cmp(&tree.node(b).name))
                            .then_with(|| a.0.cmp(&b.0))
                    });
                    children
                })
                .collect();
            tree.sort_children_by(mode);
            for (node, expected) in tree.nodes.iter().zip(expected) {
                assert_eq!(node.children, expected);
            }
        }
    }

    #[test]
    fn remove_node_updates_counts_up_the_chain() {
        let mut tree = empty_tree();
        let root = tree.root;
        let dir = add_node(&mut tree, root, node("dir", Some(root), true));
        let mut file = node("file", Some(dir), false);
        file.apparent_size = 10;
        file.allocated_size = 12;
        let file = add_node(&mut tree, dir, file);
        let mut other = node("other", Some(root), false);
        other.apparent_size = 5;
        other.allocated_size = 8;
        add_node(&mut tree, root, other);
        for id in [dir, root] {
            let n = tree.node_mut(id);
            n.apparent_size = 10;
            n.allocated_size = 12;
            n.file_count = 1;
        }
        let r = tree.node_mut(root);
        r.apparent_size += 5;
        r.allocated_size += 8;
        r.file_count += 1;
        assert!(tree.validate().is_ok());

        tree.remove_node(file);
        assert_eq!(tree.children(dir), &[]);
        assert_eq!(tree.node(dir).file_count, 0);
        assert_eq!(tree.node(dir).allocated_size, 0);
        assert_eq!(tree.node(root).file_count, 1);
        assert_eq!(tree.node(root).apparent_size, 5);
        assert_eq!(tree.node(root).allocated_size, 8);
        // Removing the root is a no-op.
        tree.remove_node(root);
        assert_eq!(tree.node(root).file_count, 1);
    }

    #[test]
    fn largest_entries_skips_root_and_synthetic_and_filters_kinds() {
        let tree = mixed_tree();
        let all = largest_entries(&tree, SizeMode::OnDisk, usize::MAX, true, true);
        assert_eq!(
            all.len(),
            tree.nodes
                .iter()
                .enumerate()
                .filter(|(i, n)| *i != 0 && !n.synthetic)
                .count()
        );
        for w in all.windows(2) {
            let (a, b) = (tree.node(w[0]), tree.node(w[1]));
            assert!(a.allocated_size >= b.allocated_size);
            if a.allocated_size == b.allocated_size {
                assert!(w[0].0 < w[1].0);
            }
        }
        let top = largest_entries(&tree, SizeMode::OnDisk, 5, true, true);
        assert_eq!(top, all[..5].to_vec());
        assert!(largest_entries(&tree, SizeMode::Apparent, 3, true, false)
            .iter()
            .all(|id| tree.node(*id).is_dir));
        assert!(largest_entries(&tree, SizeMode::Apparent, 3, false, true)
            .iter()
            .all(|id| !tree.node(*id).is_dir));
        assert!(largest_entries(&tree, SizeMode::Apparent, 3, false, false).is_empty());
        assert!(largest_entries(&tree, SizeMode::Apparent, 0, true, true).is_empty());
    }

    #[test]
    fn validate_detects_broken_links() {
        let mut tree = empty_tree();
        let root = tree.root;
        let dir = add_node(&mut tree, root, node("dir", Some(root), true));
        assert!(tree.validate().is_ok());
        tree.node_mut(dir).parent = None;
        assert!(tree.validate().is_err());
        tree.node_mut(dir).parent = Some(NodeId(9));
        assert!(tree.validate().is_err());
        tree.node_mut(dir).parent = Some(root);
        tree.node_mut(root).children.push(NodeId(42));
        assert!(tree.validate().is_err());
        tree.node_mut(root).children.pop();
        tree.item_count = 5;
        assert!(tree.validate().is_err());
    }
}
