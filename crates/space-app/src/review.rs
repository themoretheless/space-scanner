//! Review queue helpers.

use space_core::{NodeId, SizeMode, SpaceTree};

pub fn queue_bytes(tree: &SpaceTree, queue: &[NodeId], mode: SizeMode) -> u64 {
    queue
        .iter()
        .filter(|&&id| {
            let mut parent = tree.node(id).parent;
            while let Some(ancestor) = parent {
                if queue.contains(&ancestor) {
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
}
