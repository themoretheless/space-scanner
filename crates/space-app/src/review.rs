//! Review queue helpers.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use space_core::{NodeId, SizeMode, SpaceTree};

pub fn signature(queue: &[NodeId]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    queue.len().hash(&mut h);
    for id in queue {
        id.0.hash(&mut h);
    }
    h.finish()
}

fn has_ancestor(tree: &SpaceTree, mut node: NodeId, candidate: NodeId) -> bool {
    while let Some(parent) = tree.try_node(node).and_then(|n| n.parent) {
        if parent == candidate {
            return true;
        }
        node = parent;
    }
    false
}

fn has_queued_ancestor(
    tree: &SpaceTree,
    queue: &[NodeId],
    set: Option<&HashSet<NodeId>>,
    id: NodeId,
) -> bool {
    let mut parent = tree.try_node(id).and_then(|n| n.parent);
    while let Some(ancestor) = parent {
        let covered = match set {
            Some(s) => s.contains(&ancestor),
            None => queue.contains(&ancestor),
        };
        if covered {
            return true;
        }
        parent = tree.try_node(ancestor).and_then(|n| n.parent);
    }
    false
}

/// Returns `true` when any ancestor of `id` is already present in `queued`.
pub fn has_ancestor_in_set(tree: &SpaceTree, queued: &HashSet<NodeId>, id: NodeId) -> bool {
    !queued.is_empty() && has_queued_ancestor(tree, &[], Some(queued), id)
}

pub fn queue_bytes(tree: &SpaceTree, queue: &[NodeId], mode: SizeMode) -> u64 {
    let set = (queue.len() > 8).then(|| queue.iter().copied().collect::<HashSet<_>>());
    queue
        .iter()
        .filter(|&&id| {
            tree.try_node(id).is_some() && !has_queued_ancestor(tree, queue, set.as_ref(), id)
        })
        .fold(0_u64, |total, id| {
            let sz = tree.try_node(*id).map(|n| n.size(mode)).unwrap_or(0);
            total.saturating_add(sz)
        })
}

/// Toggle `id` in `queue`: refuses root/synthetic/ancestor-covered nodes, and when
/// adding a folder removes any descendants already queued under it.
pub fn toggle_in_tree(tree: &SpaceTree, queue: &mut Vec<NodeId>, id: NodeId) {
    let Some(node) = tree.try_node(id) else {
        return;
    };
    if node.synthetic || node.parent.is_none() {
        return;
    }
    if let Some(i) = queue.iter().position(|x| *x == id) {
        queue.remove(i);
        return;
    }
    // If an ancestor is already queued, the item is already covered.
    let set = (queue.len() > 8).then(|| queue.iter().copied().collect::<HashSet<_>>());
    if has_queued_ancestor(tree, queue, set.as_ref(), id) {
        return;
    }
    queue.retain(|&other| !has_ancestor(tree, other, id));
    queue.push(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_core::Node;

    #[test]
    fn selected_parent_includes_child_once_in_either_selection_order() {
        let mut parent = Node {
            name: "root".into(),
            apparent_size: 100,
            allocated_size: 80,
            file_count: 1,
            is_dir: true,
            synthetic: false,
            mtime_unix: 0,
            children: vec![NodeId(1)],
            parent: None,
        };
        let mut child = parent.clone();
        child.name = "file".into();
        child.parent = Some(NodeId(0));
        child.children.clear();
        child.is_dir = false;
        child.apparent_size = 40;
        child.allocated_size = 32;
        parent.children = vec![NodeId(1)];
        let tree = SpaceTree {
            root_path: "/tmp".into(),
            nodes: vec![parent, child],
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 2,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        };
        for queue in [[NodeId(0), NodeId(1)], [NodeId(1), NodeId(0)]] {
            assert_eq!(queue_bytes(&tree, &queue, SizeMode::Apparent), 100);
            assert_eq!(queue_bytes(&tree, &queue, SizeMode::OnDisk), 80);
        }
        assert_eq!(queue_bytes(&tree, &[NodeId(1)], SizeMode::Apparent), 40);
    }

    #[test]
    fn signature_detects_middle_item_change() {
        let a = [NodeId(1), NodeId(2), NodeId(3)];
        let b = [NodeId(1), NodeId(99), NodeId(3)];
        assert_ne!(signature(&a), signature(&b));
        assert_eq!(signature(&a), signature(&[NodeId(1), NodeId(2), NodeId(3)]));
    }

    #[test]
    fn toggle_in_tree_prunes_descendants_without_losing_siblings() {
        let root = Node {
            name: "root".into(),
            apparent_size: 100,
            allocated_size: 100,
            file_count: 3,
            is_dir: true,
            synthetic: false,
            mtime_unix: 0,
            children: vec![NodeId(1), NodeId(3)],
            parent: None,
        };
        let folder = Node {
            name: "folder".into(),
            apparent_size: 70,
            allocated_size: 70,
            file_count: 2,
            is_dir: true,
            synthetic: false,
            mtime_unix: 0,
            children: vec![NodeId(2)],
            parent: Some(NodeId(0)),
        };
        let leaf = Node {
            name: "leaf".into(),
            apparent_size: 70,
            allocated_size: 70,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            mtime_unix: 0,
            children: Vec::new(),
            parent: Some(NodeId(1)),
        };
        let sibling = Node {
            name: "sibling".into(),
            apparent_size: 30,
            allocated_size: 30,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            mtime_unix: 0,
            children: Vec::new(),
            parent: Some(NodeId(0)),
        };
        let tree = SpaceTree {
            root_path: "/tmp".into(),
            nodes: vec![root, folder, leaf, sibling],
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 4,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        };
        let mut queue = vec![NodeId(2), NodeId(3)];
        toggle_in_tree(&tree, &mut queue, NodeId(1));
        assert_eq!(queue, vec![NodeId(3), NodeId(1)]);
        assert_eq!(queue_bytes(&tree, &queue, SizeMode::OnDisk), 100);

        // Clicking leaf while parent folder is queued is a no-op.
        toggle_in_tree(&tree, &mut queue, NodeId(2));
        assert_eq!(queue, vec![NodeId(3), NodeId(1)]);
    }
}
