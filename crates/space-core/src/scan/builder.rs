//! Single-allocation `SpaceTree` builder with reverse topological size rollup.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::dag::{
    normalize_root_display, resolve_hardlink_dag, DirRec, FileRec, SmallBucket, WorkerShard,
};
use crate::tree::{Node, NodeId, SpaceTree};

pub(crate) struct BuildMeta {
    pub skipped_errors: u64,
    pub skipped_roots: Vec<String>,
    pub small_file_threshold: u64,
}

pub(crate) fn build_tree(
    root: &Path,
    root_mtime: u64,
    dir_count: usize,
    shards: Vec<WorkerShard>,
    meta: BuildMeta,
) -> Result<SpaceTree, String> {
    if dir_count == 0 {
        return Err("missing root directory slot".into());
    }

    let total_dirs: usize = shards.iter().map(|s| s.dirs.len()).sum();
    let total_files: usize = shards.iter().map(|s| s.files.len()).sum();
    debug_assert_eq!(total_dirs + 1, dir_count);

    let mut dirs: Vec<DirRec> = Vec::with_capacity(total_dirs);
    let mut files: Vec<FileRec> = Vec::with_capacity(total_files);
    let mut small_by_dir = vec![SmallBucket::default(); dir_count];

    for mut shard in shards {
        dirs.append(&mut shard.dirs);
        files.append(&mut shard.files);
        for (dir_id, bucket) in shard.small_buckets {
            if let Some(slot) = small_by_dir.get_mut(dir_id) {
                slot.count = slot.count.saturating_add(bucket.count);
                slot.apparent = slot.apparent.saturating_add(bucket.apparent);
                slot.allocated = slot.allocated.saturating_add(bucket.allocated);
            }
        }
    }

    dirs.sort_unstable_by_key(|d| d.id);

    let est_nodes = dir_count
        .saturating_add(files.len())
        .saturating_add(small_by_dir.iter().filter(|b| b.count > 0).count());
    let mut nodes: Vec<Node> = Vec::with_capacity(est_nodes);

    nodes.push(Node {
        name: normalize_root_display(root),
        apparent_size: 0,
        allocated_size: 0,
        file_count: 0,
        is_dir: true,
        synthetic: false,
        mtime_unix: root_mtime,
        children: Vec::new(),
        parent: None,
    });

    for dir in dirs {
        debug_assert_eq!(nodes.len(), dir.id);
        let parent_id = u32::try_from(dir.parent)
            .map(NodeId)
            .map_err(|_| "node id overflow".to_string())?;
        let my_id = u32::try_from(nodes.len())
            .map(NodeId)
            .map_err(|_| "node id overflow".to_string())?;
        nodes.push(Node {
            name: dir.name,
            apparent_size: 0,
            allocated_size: 0,
            file_count: 0,
            is_dir: true,
            synthetic: false,
            mtime_unix: dir.mtime_unix,
            children: Vec::new(),
            parent: Some(parent_id),
        });
        if dir.parent < nodes.len() {
            nodes[dir.parent].children.push(my_id);
        }
    }

    let (hardlink_candidates, hardlinks_deduped) = resolve_hardlink_dag(&mut files, &nodes);

    let threshold = meta.small_file_threshold;
    for file in files {
        if file.allocated < threshold && file.apparent < threshold {
            if let Some(slot) = small_by_dir.get_mut(file.parent) {
                slot.count = slot.count.saturating_add(1);
                slot.apparent = slot.apparent.saturating_add(file.apparent);
                slot.allocated = slot.allocated.saturating_add(file.allocated);
            }
            continue;
        }
        let parent_id = u32::try_from(file.parent)
            .map(NodeId)
            .map_err(|_| "node id overflow".to_string())?;
        let child_id = u32::try_from(nodes.len())
            .map(NodeId)
            .map_err(|_| "node id overflow".to_string())?;
        nodes.push(Node {
            name: file.name,
            apparent_size: file.apparent,
            allocated_size: file.allocated,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            mtime_unix: file.mtime_unix,
            children: Vec::new(),
            parent: Some(parent_id),
        });
        nodes[file.parent].children.push(child_id);
    }

    for (dir_id, bucket) in small_by_dir.into_iter().enumerate() {
        if bucket.count == 0 {
            continue;
        }
        let parent_id = u32::try_from(dir_id)
            .map(NodeId)
            .map_err(|_| "node id overflow".to_string())?;
        let child_id = u32::try_from(nodes.len())
            .map(NodeId)
            .map_err(|_| "node id overflow".to_string())?;
        nodes.push(Node {
            name: format!("· {} small files", bucket.count),
            apparent_size: bucket.apparent,
            allocated_size: bucket.allocated,
            file_count: bucket.count,
            is_dir: false,
            synthetic: true,
            mtime_unix: 0,
            children: Vec::new(),
            parent: Some(parent_id),
        });
        nodes[dir_id].children.push(child_id);
    }

    // Reverse topological rollup: every directory `parent < child_dir` because
    // directories are ordered by dense pre-order/discovery ID `0..dir_count`.
    for i in (0..dir_count).rev() {
        let mut apparent = 0u64;
        let mut allocated = 0u64;
        let mut file_count = 0u64;
        for &child in &nodes[i].children {
            let child_node = &nodes[child.0 as usize];
            apparent = apparent.saturating_add(child_node.apparent_size);
            allocated = allocated.saturating_add(child_node.allocated_size);
            file_count = file_count.saturating_add(child_node.file_count);
        }
        nodes[i].apparent_size = apparent;
        nodes[i].allocated_size = allocated;
        nodes[i].file_count = file_count;
    }

    // Unstable sort children by allocated size desc, name asc.
    for i in 0..dir_count {
        let mut kids = std::mem::take(&mut nodes[i].children);
        kids.sort_unstable_by(|&a, &b| {
            let na = &nodes[a.0 as usize];
            let nb = &nodes[b.0 as usize];
            nb.allocated_size
                .cmp(&na.allocated_size)
                .then_with(|| na.name.cmp(&nb.name))
        });
        nodes[i].children = kids;
    }

    let item_count = nodes.len() as u64;
    let scanned_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut skipped_roots = meta.skipped_roots;
    skipped_roots.sort();

    Ok(SpaceTree {
        root_path: root.display().to_string(),
        nodes,
        root: NodeId(0),
        scanned_at_unix,
        item_count,
        skipped_errors: meta.skipped_errors,
        hardlink_candidates,
        hardlinks_deduped,
        skipped_roots,
    })
}
