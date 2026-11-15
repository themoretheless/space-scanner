use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use regex_automata::{meta::Regex, util::syntax};
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
    #[serde(default)]
    pub mtime_unix: u64,
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
    /// Bounded sample of directories/subpaths skipped during scan.
    #[serde(default)]
    pub skipped_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub name: String,
    pub size: u64,
    pub nodes: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtGroup {
    pub ext: String,
    pub total_bytes: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct FindQuery {
    pub name_pattern: Option<String>,
    pub use_regex: bool,
    pub exts: Vec<String>,
    pub min_bytes: Option<u64>,
    pub max_bytes: Option<u64>,
    pub dirs_only: bool,
    pub files_only: bool,
    pub min_age_days: Option<u64>,
    /// Visit cap; `0` means unbounded.
    pub max_visit: usize,
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

    pub fn try_node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0 as usize)
    }

    /// Pre-sorted children (no clone).
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.try_node(id)
            .map_or(&[], |node| node.children.as_slice())
    }

    /// Compatibility: returns owned sorted list (already sorted).
    pub fn children_sorted(&self, id: NodeId) -> Vec<NodeId> {
        self.children(id).to_vec()
    }

    pub fn path_to(&self, id: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        let limit = self.nodes.len();
        while let Some(n) = cur {
            if chain.len() >= limit {
                break;
            }
            let Some(node) = self.try_node(n) else {
                break;
            };
            chain.push(n);
            cur = node.parent;
        }
        chain.reverse();
        chain
    }

    pub fn breadcrumb_names(&self, id: NodeId) -> Vec<String> {
        self.path_to(id)
            .into_iter()
            .filter_map(|n| self.try_node(n).map(|node| node.name.clone()))
            .collect()
    }

    pub fn total_size(&self) -> u64 {
        self.try_node(self.root).map_or(0, |n| n.allocated_size)
    }

    pub fn total_size_mode(&self, mode: SizeMode) -> u64 {
        self.try_node(self.root).map_or(0, |n| n.size(mode))
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
        if chain.is_empty() {
            return self.root_path.clone();
        }
        let root = self.root_path.trim_end_matches('/');
        let capacity = root.len()
            + chain
                .iter()
                .skip(1)
                .filter_map(|&node| self.try_node(node).map(|n| n.name.len() + 1))
                .sum::<usize>();
        let mut path = String::with_capacity(capacity.max(1));
        path.push_str(root);
        for &node in chain.iter().skip(1) {
            if let Some(n) = self.try_node(node) {
                path.push('/');
                path.push_str(&n.name);
            }
        }
        if path.is_empty() && self.root_path.starts_with('/') {
            path.push('/');
        }
        path
    }

    /// Validate structural invariants after cache load or mutation.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.nodes.is_empty() {
            return Err("empty node table");
        }
        let len = self.nodes.len();
        let root_idx = self.root.0 as usize;
        if root_idx >= len {
            return Err("root index out of bounds");
        }
        if self.nodes[root_idx].parent.is_some() {
            return Err("root node must not have parent");
        }
        for (idx, node) in self.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                let p_idx = parent.0 as usize;
                if p_idx >= len || p_idx == idx {
                    return Err("invalid parent reference");
                }
            }
            for &child in &node.children {
                if child.0 as usize >= len {
                    return Err("child index out of bounds");
                }
            }
        }
        let mut visited = vec![false; len];
        visited[root_idx] = true;
        let mut stack = vec![self.root];
        while let Some(nid) = stack.pop() {
            for &cid in &self.nodes[nid.0 as usize].children {
                let cidx = cid.0 as usize;
                if visited[cidx] {
                    return Err("cycle or duplicate edge in tree");
                }
                if self.nodes[cidx].parent != Some(nid) {
                    return Err("child parent mismatch");
                }
                visited[cidx] = true;
                stack.push(cid);
            }
        }
        Ok(())
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

    /// Largest descendant directories under `id`.
    pub fn largest_dirs(&self, id: NodeId, mode: SizeMode, limit: usize) -> Vec<NodeId> {
        self.largest_dirs_bounded(id, mode, limit, 50_000)
    }

    pub fn largest_dirs_bounded(
        &self,
        id: NodeId,
        mode: SizeMode,
        limit: usize,
        max_visit: usize,
    ) -> Vec<NodeId> {
        if limit == 0 || max_visit == 0 {
            return Vec::new();
        }
        let mut dirs = Vec::new();
        self.walk_descendants_bounded(id, max_visit, |child, node| {
            if node.is_dir && !node.synthetic {
                let order = dirs.len();
                dirs.push((child, order));
            }
        });
        let compare = |&(a, a_order): &(NodeId, usize), &(b, b_order): &(NodeId, usize)| {
            self.compare_nodes(a, b, mode)
                .then_with(|| a_order.cmp(&b_order))
        };
        if dirs.len() > limit {
            dirs.select_nth_unstable_by(limit, compare);
        }
        dirs.truncate(limit);
        dirs.sort_unstable_by(compare);
        dirs.into_iter().map(|(cid, _)| cid).collect()
    }

    /// Oldest non-synthetic files at or above `min_bytes` (requires known `mtime_unix > 0`).
    pub fn oldest_large_files(
        &self,
        id: NodeId,
        mode: SizeMode,
        min_bytes: u64,
        limit: usize,
    ) -> Vec<NodeId> {
        if limit == 0 {
            return Vec::new();
        }
        let mut files = Vec::new();
        self.walk_descendants_bounded(id, 100_000, |child, node| {
            if !node.is_dir
                && !node.synthetic
                && node.mtime_unix > 0
                && node.size(mode) >= min_bytes
            {
                files.push(child);
            }
        });
        let compare = |&a: &NodeId, &b: &NodeId| {
            let (Some(na), Some(nb)) = (self.try_node(a), self.try_node(b)) else {
                return std::cmp::Ordering::Equal;
            };
            na.mtime_unix
                .cmp(&nb.mtime_unix)
                .then_with(|| nb.size(mode).cmp(&na.size(mode)))
                .then_with(|| na.name.cmp(&nb.name))
        };
        if files.len() > limit {
            files.select_nth_unstable_by(limit, compare);
        }
        files.truncate(limit);
        files.sort_unstable_by(compare);
        files
    }

    /// Groups non-synthetic files with identical `(name, size)` at or above `min_bytes`.
    pub fn duplicate_name_size(
        &self,
        id: NodeId,
        mode: SizeMode,
        min_bytes: u64,
        limit: usize,
    ) -> Vec<DuplicateGroup> {
        if limit == 0 {
            return Vec::new();
        }
        let mut buckets: HashMap<(String, u64), Vec<NodeId>> = HashMap::new();
        self.walk_descendants_bounded(id, 100_000, |child, node| {
            let size = node.size(mode);
            if !node.is_dir && !node.synthetic && size >= min_bytes && size > 0 {
                buckets
                    .entry((node.name.clone(), size))
                    .or_default()
                    .push(child);
            }
        });
        let mut groups: Vec<DuplicateGroup> = buckets
            .into_iter()
            .filter_map(|((name, size), mut nodes)| {
                if nodes.len() < 2 {
                    return None;
                }
                nodes.sort_unstable_by_key(|nid| nid.0);
                Some(DuplicateGroup { name, size, nodes })
            })
            .collect();
        groups.sort_by(|a, b| {
            let a_wasted = (a.nodes.len() as u64 - 1).saturating_mul(a.size);
            let b_wasted = (b.nodes.len() as u64 - 1).saturating_mul(b.size);
            b_wasted
                .cmp(&a_wasted)
                .then_with(|| b.size.cmp(&a.size))
                .then_with(|| a.name.cmp(&b.name))
        });
        groups.truncate(limit);
        groups
    }

    /// Aggregate non-synthetic files under `id` by lowercase extension (`""` for none).
    pub fn group_by_ext(&self, id: NodeId, mode: SizeMode, limit: usize) -> Vec<ExtGroup> {
        if limit == 0 {
            return Vec::new();
        }
        let mut map: HashMap<String, (u64, u64)> = HashMap::new();
        self.walk_descendants_bounded(id, 150_000, |_child, node| {
            if !node.is_dir && !node.synthetic {
                let ext = file_ext_lower(&node.name);
                let entry = map.entry(ext).or_insert((0, 0));
                entry.0 = entry.0.saturating_add(node.size(mode));
                entry.1 = entry.1.saturating_add(1);
            }
        });
        let mut groups: Vec<ExtGroup> = map
            .into_iter()
            .map(|(ext, (total_bytes, file_count))| ExtGroup {
                ext,
                total_bytes,
                file_count,
            })
            .collect();
        groups.sort_by(|a, b| {
            b.total_bytes
                .cmp(&a.total_bytes)
                .then_with(|| b.file_count.cmp(&a.file_count))
                .then_with(|| a.ext.cmp(&b.ext))
        });
        groups.truncate(limit);
        groups
    }

    /// Filter descendants under `id` by name/regex, extension, size range, kind, and age.
    pub fn find_nodes(
        &self,
        id: NodeId,
        mode: SizeMode,
        query: &FindQuery,
        limit: usize,
    ) -> Result<Vec<NodeId>, String> {
        let regex = match (query.use_regex, query.name_pattern.as_deref()) {
            (true, Some(pat)) if !pat.is_empty() => Some(
                Regex::builder()
                    .syntax(syntax::Config::new().case_insensitive(true))
                    .build(pat)
                    .map_err(|e| e.to_string())?,
            ),
            _ => None,
        };
        if limit == 0 {
            return Ok(Vec::new());
        }
        let pat_lower = if !query.use_regex {
            query
                .name_pattern
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase)
        } else {
            None
        };
        let exts: Vec<String> = query
            .exts
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        let cutoff_unix = query.min_age_days.map(|days| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            now.saturating_sub(days.saturating_mul(86_400))
        });
        let max_visit = if query.max_visit == 0 {
            usize::MAX
        } else {
            query.max_visit
        };
        let mut hits = Vec::new();
        self.walk_descendants_bounded(id, max_visit, |child, node| {
            if node.synthetic {
                return;
            }
            if query.dirs_only && !node.is_dir {
                return;
            }
            if query.files_only && node.is_dir {
                return;
            }
            let size = node.size(mode);
            if let Some(min) = query.min_bytes {
                if size < min {
                    return;
                }
            }
            if let Some(max) = query.max_bytes {
                if size > max {
                    return;
                }
            }
            if let Some(cutoff) = cutoff_unix {
                if node.mtime_unix == 0 || node.mtime_unix > cutoff {
                    return;
                }
            }
            if !exts.is_empty() {
                if node.is_dir {
                    return;
                }
                let ext = file_ext_lower(&node.name);
                if !exts.contains(&ext) {
                    return;
                }
            }
            if let Some(re) = &regex {
                if !re.is_match(node.name.as_bytes()) {
                    return;
                }
            } else if let Some(sub) = &pat_lower {
                if !node.name.to_lowercase().contains(sub) {
                    return;
                }
            }
            hits.push(child);
        });
        hits.sort_by(|&a, &b| self.compare_nodes(a, b, mode));
        hits.truncate(limit);
        Ok(hits)
    }

    fn walk_descendants_bounded(
        &self,
        id: NodeId,
        max_visit: usize,
        mut visit: impl FnMut(NodeId, &Node),
    ) {
        let Some(root) = self.try_node(id) else {
            return;
        };
        let mut visited = 0usize;
        let mut stack = vec![root.children.iter()];
        while visited < max_visit {
            let Some(children) = stack.last_mut() else {
                break;
            };
            let Some(&child) = children.next() else {
                stack.pop();
                continue;
            };
            let Some(node) = self.try_node(child) else {
                continue;
            };
            visited += 1;
            visit(child, node);
            if node.is_dir {
                stack.push(node.children.iter());
            }
        }
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
            let Some(node) = self.try_node(child) else {
                continue;
            };
            visited += 1;
            if node.is_dir {
                stack.push(node.children.iter());
            } else {
                files.push((child, files.len()));
            }
        }
        files
    }

    fn compare_nodes(&self, a: NodeId, b: NodeId, mode: SizeMode) -> std::cmp::Ordering {
        let (Some(na), Some(nb)) = (self.try_node(a), self.try_node(b)) else {
            return a.0.cmp(&b.0);
        };
        nb.size(mode)
            .cmp(&na.size(mode))
            .then_with(|| na.name.cmp(&nb.name))
    }

    /// Remove node from parent and subtract sizes up the ancestor chain.
    pub fn remove_node(&mut self, id: NodeId) {
        let Some((parent, apparent, allocated, file_count)) = self
            .try_node(id)
            .map(|n| (n.parent, n.apparent_size, n.allocated_size, n.file_count))
        else {
            return;
        };
        if let Some(pid) = parent {
            if let Some(pnode) = self.try_node_mut(pid) {
                pnode.children.retain(|c| *c != id);
            }
            let limit = self.nodes.len();
            let mut cur = Some(pid);
            let mut steps = 0usize;
            while let Some(cid) = cur {
                if steps >= limit {
                    break;
                }
                steps += 1;
                let Some(n) = self.try_node_mut(cid) else {
                    break;
                };
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

fn file_ext_lower(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

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
    let mut heap =
        std::collections::BinaryHeap::with_capacity(limit.saturating_add(1).min(tree.nodes.len()));
    for (i, node) in tree.nodes.iter().enumerate() {
        if i == tree.root.0 as usize || node.synthetic {
            continue;
        }
        if (node.is_dir && !dirs) || (!node.is_dir && !files) {
            continue;
        }
        heap.push(std::cmp::Reverse((node.size(mode), std::cmp::Reverse(i))));
        if heap.len() > limit {
            heap.pop();
        }
    }
    let mut ranked: Vec<_> = heap.into_iter().map(|std::cmp::Reverse(v)| v).collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    ranked
        .into_iter()
        .map(|(_, std::cmp::Reverse(i))| NodeId(i as u32))
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
            mtime_unix: 0,
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
            skipped_roots: Vec::new(),
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

    #[test]
    fn validate_and_safe_helpers_reject_corruption() {
        let mut tree = mixed_tree();
        assert!(tree.validate().is_ok());
        assert!(tree.try_node_mut(NodeId(99_999)).is_none());
        assert!(tree.children(NodeId(99_999)).is_empty());
        assert!(tree.path_to(NodeId(99_999)).is_empty());
        tree.remove_node(NodeId(99_999));

        let mut bad_cycle = empty_tree();
        let d1 = add_node(&mut bad_cycle, NodeId(0), node("a", Some(NodeId(0)), true));
        bad_cycle.node_mut(d1).children.push(NodeId(0));
        assert!(bad_cycle.validate().is_err());
        assert_eq!(bad_cycle.path_to(d1).len(), bad_cycle.nodes.len().min(2));

        let mut cycle_up = empty_tree();
        let a = add_node(&mut cycle_up, NodeId(0), node("a", Some(NodeId(0)), true));
        let b = add_node(&mut cycle_up, a, node("b", Some(a), true));
        cycle_up.node_mut(a).parent = Some(b);
        assert!(cycle_up.path_to(b).len() <= cycle_up.nodes.len());
        assert!(cycle_up.validate().is_err());

        let mut oob = empty_tree();
        oob.node_mut(NodeId(0)).children.push(NodeId(77));
        assert!(oob.validate().is_err());
    }

    #[test]
    fn query_helpers_cover_dirs_oldest_duplicates_exts_and_find() {
        let mut tree = empty_tree();
        let root = tree.root;
        let mut big_dir = node("builds", Some(root), true);
        big_dir.allocated_size = 500;
        let big_dir_id = add_node(&mut tree, root, big_dir);

        let mut small_dir = node("notes", Some(root), true);
        small_dir.allocated_size = 50;
        let small_dir_id = add_node(&mut tree, root, small_dir);

        let mut old_iso = node("Ubuntu.ISO", Some(big_dir_id), false);
        old_iso.allocated_size = 300;
        old_iso.apparent_size = 300;
        old_iso.mtime_unix = 100;
        let old_id = add_node(&mut tree, big_dir_id, old_iso);

        let mut dup_a = node("archive.tar.gz", Some(big_dir_id), false);
        dup_a.allocated_size = 100;
        dup_a.apparent_size = 100;
        dup_a.mtime_unix = 1_000_000_000;
        let dup_a_id = add_node(&mut tree, big_dir_id, dup_a);

        let mut dup_b = node("archive.tar.gz", Some(small_dir_id), false);
        dup_b.allocated_size = 100;
        dup_b.apparent_size = 100;
        dup_b.mtime_unix = 1_000_000_000;
        let dup_b_id = add_node(&mut tree, small_dir_id, dup_b);

        let mut synth = node("· 4 small files", Some(small_dir_id), false);
        synth.allocated_size = 400;
        synth.synthetic = true;
        add_node(&mut tree, small_dir_id, synth);

        let dirs = tree.largest_dirs(root, SizeMode::OnDisk, 10);
        assert_eq!(dirs, vec![big_dir_id, small_dir_id]);

        let oldest = tree.oldest_large_files(root, SizeMode::OnDisk, 100, 10);
        assert_eq!(oldest, vec![old_id, dup_a_id, dup_b_id]);

        let dups = tree.duplicate_name_size(root, SizeMode::OnDisk, 50, 10);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].name, "archive.tar.gz");
        assert_eq!(dups[0].nodes, vec![dup_a_id, dup_b_id]);

        let exts = tree.group_by_ext(root, SizeMode::OnDisk, 10);
        assert_eq!(
            exts,
            vec![
                ExtGroup {
                    ext: "iso".into(),
                    total_bytes: 300,
                    file_count: 1,
                },
                ExtGroup {
                    ext: "gz".into(),
                    total_bytes: 200,
                    file_count: 2,
                },
            ]
        );

        let found_re = tree
            .find_nodes(
                root,
                SizeMode::OnDisk,
                &FindQuery {
                    name_pattern: Some("^ubuntu\\.".into()),
                    use_regex: true,
                    exts: vec![".ISO".into()],
                    min_bytes: Some(200),
                    files_only: true,
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        assert_eq!(found_re, vec![old_id]);

        assert!(tree
            .find_nodes(
                root,
                SizeMode::OnDisk,
                &FindQuery {
                    name_pattern: Some("[unclosed".into()),
                    use_regex: true,
                    ..Default::default()
                },
                10,
            )
            .is_err());
    }
}
