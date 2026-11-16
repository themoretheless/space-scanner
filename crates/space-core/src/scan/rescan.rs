//! Incremental subtree rescanning and tree compaction.

use std::path::PathBuf;

use super::dag::MAX_SKIPPED_ROOTS;
use crate::tree::{Node, NodeId, SpaceTree};

pub(crate) fn rescan_subtree_with<F>(
    tree: &SpaceTree,
    dir_id: NodeId,
    run_scan: F,
) -> Result<SpaceTree, String>
where
    F: Fn(&PathBuf) -> Result<SpaceTree, String>,
{
    let Some(dir) = tree.try_node(dir_id) else {
        return Err("rescan node does not exist".into());
    };
    if !dir.is_dir {
        return Err("rescan node is not a directory".into());
    }
    let root = PathBuf::from(&tree.root_path);
    if dir_id == tree.root || tree.hardlink_candidates > 0 {
        return run_scan(&root);
    }

    let subtree_str = tree.path_string(dir_id);
    let subtree_root = PathBuf::from(&subtree_str);
    let scanned = run_scan(&subtree_root)?;
    // A hardlink can have another name outside this subtree. Without scanning
    // that owner, independently deduping this subtree could overcount bytes.
    if scanned.hardlink_candidates > 0 {
        return run_scan(&root);
    }
    Ok(splice_subtree(tree, dir_id, &subtree_str, scanned))
}

pub(crate) fn splice_subtree(
    tree: &SpaceTree,
    dir_id: NodeId,
    subtree_prefix: &str,
    scanned: SpaceTree,
) -> SpaceTree {
    let old = tree.node(dir_id);
    let old_parent = old.parent.expect("non-root subtree has a parent");
    let old_apparent = old.apparent_size;
    let old_allocated = old.allocated_size;
    let old_files = old.file_count;
    let original_name = old.name.clone();

    let mut merged = tree.clone();
    let append_base = merged.nodes.len() as u32;
    let mut mapping = Vec::with_capacity(scanned.nodes.len());
    mapping.push(dir_id);
    for index in 1..scanned.nodes.len() {
        mapping.push(NodeId(append_base + index as u32 - 1));
    }

    let scanned_at_unix = scanned.scanned_at_unix;
    let scanned_errors = scanned.skipped_errors;
    let scanned_skipped_roots = scanned.skipped_roots;
    for (index, mut node) in scanned.nodes.into_iter().enumerate() {
        for child in &mut node.children {
            *child = mapping[child.0 as usize];
        }
        if index == 0 {
            node.name.clone_from(&original_name);
            node.parent = Some(old_parent);
            merged.nodes[dir_id.0 as usize] = node;
        } else {
            node.parent = node.parent.map(|parent| mapping[parent.0 as usize]);
            merged.nodes.push(node);
        }
    }

    let replacement = merged.node(dir_id);
    let new_apparent = replacement.apparent_size;
    let new_allocated = replacement.allocated_size;
    let new_files = replacement.file_count;
    let mut current = Some(old_parent);
    while let Some(id) = current {
        let node = merged.node_mut(id);
        node.apparent_size = node
            .apparent_size
            .saturating_sub(old_apparent)
            .saturating_add(new_apparent);
        node.allocated_size = node
            .allocated_size
            .saturating_sub(old_allocated)
            .saturating_add(new_allocated);
        node.file_count = node
            .file_count
            .saturating_sub(old_files)
            .saturating_add(new_files);
        current = node.parent;
    }
    merged.scanned_at_unix = scanned_at_unix;
    merged.skipped_errors = merged.skipped_errors.saturating_add(scanned_errors);
    let prefix_slash = format!("{}/", subtree_prefix.trim_end_matches('/'));
    merged
        .skipped_roots
        .retain(|p| p != subtree_prefix && !p.starts_with(&prefix_slash));
    for p in scanned_skipped_roots {
        if merged.skipped_roots.len() >= MAX_SKIPPED_ROOTS {
            break;
        }
        if !merged.skipped_roots.iter().any(|existing| existing == &p) {
            merged.skipped_roots.push(p);
        }
    }
    compact_tree(merged)
}

fn compact_tree(tree: SpaceTree) -> SpaceTree {
    let mut order = Vec::with_capacity(tree.item_count as usize);
    let mut stack = vec![tree.root];
    let mut mapping = vec![None; tree.nodes.len()];
    while let Some(old_id) = stack.pop() {
        if mapping[old_id.0 as usize].is_some() {
            continue;
        }
        let new_id = NodeId(order.len() as u32);
        mapping[old_id.0 as usize] = Some(new_id);
        order.push(old_id);
        for child in tree.node(old_id).children.iter().rev() {
            stack.push(*child);
        }
    }
    let SpaceTree {
        root_path,
        nodes: old_nodes,
        scanned_at_unix,
        skipped_errors,
        hardlink_candidates,
        hardlinks_deduped,
        skipped_roots,
        ..
    } = tree;
    let mut old_nodes: Vec<Option<Node>> = old_nodes.into_iter().map(Some).collect();
    let mut nodes = Vec::with_capacity(order.len());
    for old_id in order {
        let mut node = old_nodes[old_id.0 as usize]
            .take()
            .expect("reachable node appears once");
        for child in &mut node.children {
            *child = mapping[child.0 as usize].expect("reachable child is mapped");
        }
        node.parent = node.parent.and_then(|parent| mapping[parent.0 as usize]);
        nodes.push(node);
    }
    SpaceTree {
        root_path,
        item_count: nodes.len() as u64,
        nodes,
        root: NodeId(0),
        scanned_at_unix,
        skipped_errors,
        hardlink_candidates,
        hardlinks_deduped,
        skipped_roots,
    }
}
