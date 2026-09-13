//! Review queue + trash helpers.

use std::path::Path;

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

pub fn trash_paths(paths: &[impl AsRef<Path>]) -> Result<usize, String> {
    let mut ok = 0usize;
    for p in paths {
        trash::delete(p).map_err(|e| e.to_string())?;
        ok += 1;
    }
    Ok(ok)
}
