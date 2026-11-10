//! Test helpers shared by the unit tests of this crate.

use space_core::{Node, NodeId};

/// A tree node built through serde so fields the core adds later (with a
/// serde default) never break the app's tests.
#[allow(clippy::too_many_arguments)]
pub(crate) fn node(
    name: &str,
    apparent_size: u64,
    allocated_size: u64,
    file_count: u64,
    is_dir: bool,
    synthetic: bool,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
) -> Node {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "apparent_size": apparent_size,
        "allocated_size": allocated_size,
        "file_count": file_count,
        "is_dir": is_dir,
        "synthetic": synthetic,
        "children": children.iter().map(|c| c.0).collect::<Vec<_>>(),
        "parent": parent.map(|p| p.0),
        "raw_name": null,
    }))
    .expect("test node")
}
