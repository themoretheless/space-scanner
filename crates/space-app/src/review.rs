//! Review queue helpers.

use space_core::{NodeId, SizeMode, SpaceTree};

pub fn queue_bytes(tree: &SpaceTree, queue: &[NodeId], mode: SizeMode) -> u64 {
    queue.iter().map(|id| tree.node(*id).size(mode)).sum()
}

pub fn toggle(queue: &mut Vec<NodeId>, id: NodeId) {
    if let Some(i) = queue.iter().position(|x| *x == id) {
        queue.remove(i);
    } else {
        queue.push(id);
    }
}
