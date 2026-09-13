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
        let names = self.breadcrumb_names(id);
        if names.is_empty() {
            return self.root_path.clone();
        }
        let mut parts: Vec<&str> = Vec::new();
        parts.push(self.root_path.trim_end_matches('/'));
        for name in names.iter().skip(1) {
            parts.push(name.as_str());
        }
        parts.join("/")
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
        let mut files = Vec::new();
        let mut visited = 0usize;
        self.collect_files_bounded(id, &mut files, &mut visited, max_visit);
        files.sort_by(|a, b| {
            self.node(*b)
                .size(mode)
                .cmp(&self.node(*a).size(mode))
                .then_with(|| self.node(*a).name.cmp(&self.node(*b).name))
        });
        files.truncate(limit);
        files
    }

    fn collect_files_bounded(
        &self,
        id: NodeId,
        out: &mut Vec<NodeId>,
        visited: &mut usize,
        max_visit: usize,
    ) {
        if *visited >= max_visit {
            return;
        }
        *visited += 1;
        let node = self.node(id);
        if !node.is_dir {
            out.push(id);
            return;
        }
        for &c in &node.children {
            if *visited >= max_visit {
                break;
            }
            self.collect_files_bounded(c, out, visited, max_visit);
        }
    }

    /// Remove node from parent and subtract sizes up the ancestor chain.
    pub fn remove_node(&mut self, id: NodeId) {
        let (parent, apparent, allocated, file_count) = {
            let n = self.node(id);
            (
                n.parent,
                n.apparent_size,
                n.allocated_size,
                n.file_count,
            )
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
            let mut kids = self.nodes[i].children.clone();
            kids.sort_by(|a, b| {
                self.nodes[b.0 as usize]
                    .size(mode)
                    .cmp(&self.nodes[a.0 as usize].size(mode))
                    .then_with(|| {
                        self.nodes[a.0 as usize]
                            .name
                            .cmp(&self.nodes[b.0 as usize].name)
                    })
            });
            self.nodes[i].children = kids;
        }
    }
}
