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
}

impl Node {
    pub fn size(&self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Apparent => self.apparent_size,
            SizeMode::OnDisk => self.allocated_size,
        }
    }
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

    /// Compatibility: returns owned sorted list (already sorted).
    pub fn children_sorted(&self, id: NodeId) -> Vec<NodeId> {
        self.children(id).to_vec()
    }

    pub fn path_to(&self, id: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::new();
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

    /// Normalize names to NFC (macOS often stores NFD; UI fonts expect NFC).
    pub fn normalize_unicode_names(&mut self) {
        use unicode_normalization::UnicodeNormalization;
        self.root_path = self.root_path.nfc().collect();
        for node in &mut self.nodes {
            let nfc: String = node.name.nfc().collect();
            if nfc != node.name {
                node.name = nfc;
            }
        }
    }

    pub fn path_string(&self, id: NodeId) -> String {
        let chain = self.path_to(id);
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

    fn compare_nodes(&self, a: NodeId, b: NodeId, mode: SizeMode) -> std::cmp::Ordering {
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

    pub fn sort_children_by(&mut self, mode: SizeMode) {
        // Re-sort all dirs by chosen mode (after scan we sort by allocated).
        for i in 0..self.nodes.len() {
            if !self.nodes[i].is_dir {
                continue;
            }
            let mut kids = std::mem::take(&mut self.nodes[i].children);
            kids.sort_by(|&a, &b| self.compare_nodes(a, b, mode));
            self.nodes[i].children = kids;
        }
    }
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
        tree.root_path = "/".into();
        assert_eq!(tree.path_string(directory), "/résumé");
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
}
