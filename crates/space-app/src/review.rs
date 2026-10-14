//! Review queue helpers.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use space_core::{NodeId, SizeMode, SpaceTree};

/// Bytes of the queue with nested entries counted once (an entry under a
/// queued ancestor contributes nothing). Ancestor lookups go through a set,
/// so a queue of n items costs O(n · depth) rather than O(n² · depth).
pub fn queue_bytes(tree: &SpaceTree, queue: &[NodeId], mode: SizeMode) -> u64 {
    let members: HashSet<NodeId> = queue.iter().copied().collect();
    queue
        .iter()
        .filter(|&&id| tree.try_node(id).is_some())
        .filter(|&&id| {
            let mut parent = tree.node(id).parent;
            while let Some(ancestor) = parent {
                if members.contains(&ancestor) {
                    return false;
                }
                parent = tree.node(ancestor).parent;
            }
            true
        })
        .fold(0_u64, |total, id| {
            total.saturating_add(tree.node(*id).size(mode))
        })
}

/// Cheap fingerprint of the queue for caches keyed on its contents.
pub fn signature(queue: &[NodeId]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    queue.len().hash(&mut h);
    for id in queue {
        id.0.hash(&mut h);
    }
    h.finish()
}

pub fn toggle(queue: &mut Vec<NodeId>, id: NodeId) {
    if let Some(i) = queue.iter().position(|x| *x == id) {
        queue.remove(i);
    } else {
        queue.push(id);
    }
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
            children: vec![NodeId(1)],
            parent: None,
            raw_name: None,
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
        };
        for queue in [[NodeId(0), NodeId(1)], [NodeId(1), NodeId(0)]] {
            assert_eq!(queue_bytes(&tree, &queue, SizeMode::Apparent), 100);
            assert_eq!(queue_bytes(&tree, &queue, SizeMode::OnDisk), 80);
        }
        assert_eq!(queue_bytes(&tree, &[NodeId(1)], SizeMode::Apparent), 40);
    }

    #[test]
    fn deep_queues_count_each_branch_once_and_skip_unknown_ids() {
        // root → a → b → c (files hang off c); queue every level plus siblings.
        let mut nodes: Vec<Node> = Vec::new();
        let mut parent = None;
        for i in 0..4u32 {
            nodes.push(Node {
                name: format!("d{i}"),
                apparent_size: 100 - i as u64 * 10,
                allocated_size: 100 - i as u64 * 10,
                file_count: 1,
                is_dir: true,
                synthetic: false,
                children: if i < 3 { vec![NodeId(i + 1)] } else { vec![] },
                parent,
                raw_name: None,
            });
            parent = Some(NodeId(i));
        }
        let tree = SpaceTree {
            root_path: "/tmp".into(),
            nodes,
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 4,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        };
        let queue = [NodeId(3), NodeId(2), NodeId(1)];
        assert_eq!(queue_bytes(&tree, &queue, SizeMode::OnDisk), 90);
        let with_root = [NodeId(3), NodeId(0), NodeId(2)];
        assert_eq!(queue_bytes(&tree, &with_root, SizeMode::OnDisk), 100);
        assert_eq!(queue_bytes(&tree, &[NodeId(3), NodeId(42)], SizeMode::OnDisk), 70);
        assert_ne!(signature(&queue), signature(&with_root));
        assert_eq!(signature(&queue), signature(&[NodeId(3), NodeId(2), NodeId(1)]));
    }
}
