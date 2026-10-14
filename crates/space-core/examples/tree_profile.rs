//! Repeat in-memory tree queries without filesystem or fixture creation costs.
//! Run with `cargo run -p space-core --release --example tree_profile`.
use space_core::{Node, NodeId, SizeMode, SpaceTree};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let tree = wide_tree(24_999);
    for mode in [SizeMode::Apparent, SizeMode::OnDisk] {
        let expected = reference_largest(&tree, mode, 50);
        assert_eq!(tree.largest_files(tree.root, mode, 50), expected);
        measure(&format!("largest_50_{mode:?}"), 100, || {
            black_box(tree.largest_files(black_box(tree.root), mode, 50));
        });
    }

    let deep = deep_tree(128);
    let leaf = NodeId((deep.nodes.len() - 1) as u32);
    assert_eq!(deep.path_string(leaf).split('/').count(), 130);
    measure("path_depth_128", 1_000, || {
        black_box(deep.path_string(black_box(leaf)));
    });
}

fn measure(label: &str, iterations: usize, mut work: impl FnMut()) {
    let mut times = Vec::with_capacity(11);
    for sample in 0..12 {
        let start = Instant::now();
        for _ in 0..iterations {
            work();
        }
        if sample > 0 {
            times.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{label}: median_us={:.3} min_us={:.3} max_us={:.3}",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    );
}

fn reference_largest(tree: &SpaceTree, mode: SizeMode, limit: usize) -> Vec<NodeId> {
    let mut files = tree.children(tree.root).to_vec();
    files.sort_by(|&a, &b| {
        tree.node(b)
            .size(mode)
            .cmp(&tree.node(a).size(mode))
            .then_with(|| tree.node(a).name.cmp(&tree.node(b).name))
    });
    files.truncate(limit);
    files
}

fn wide_tree(count: u32) -> SpaceTree {
    let mut tree = empty_tree();
    for index in 0..count {
        let id = NodeId(index + 1);
        tree.nodes.push(Node {
            name: format!("file-{:03}", index % 257),
            apparent_size: u64::from(index.wrapping_mul(2_654_435_761) % 1_024),
            allocated_size: u64::from(index.wrapping_mul(1_664_525) % 1_024) * 4_096,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            children: Vec::new(),
            parent: Some(tree.root),
            raw_name: None,
        });
        tree.nodes[0].children.push(id);
    }
    tree.item_count = tree.nodes.len() as u64;
    tree
}

fn deep_tree(depth: u32) -> SpaceTree {
    let mut tree = empty_tree();
    for index in 1..=depth {
        let mut node = directory();
        node.parent = Some(NodeId(index - 1));
        tree.nodes.push(node);
        tree.nodes[index as usize - 1].children.push(NodeId(index));
    }
    tree.item_count = tree.nodes.len() as u64;
    tree
}

fn empty_tree() -> SpaceTree {
    SpaceTree {
        root_path: "/root".into(),
        nodes: vec![directory()],
        root: NodeId(0),
        scanned_at_unix: 0,
        item_count: 1,
        skipped_errors: 0,
        hardlink_candidates: 0,
        hardlinks_deduped: 0,
    }
}

fn directory() -> Node {
    Node {
        name: "directory".into(),
        apparent_size: 0,
        allocated_size: 0,
        file_count: 0,
        is_dir: true,
        synthetic: false,
        children: Vec::new(),
        parent: None,
        raw_name: None,
    }
}
