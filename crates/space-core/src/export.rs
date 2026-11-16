//! Serialization helpers for CSV, TSV, JSON, and standalone SVG charts.

use std::fmt::Write as _;

use serde::Serialize;

use crate::format::format_bytes;
use crate::tree::{NodeId, SizeMode, SpaceTree};

const PALETTE: &[&str] = &[
    "#4C8BF5", "#34A853", "#FBBC05", "#EA4335", "#A142F4", "#24C1E0", "#FA7B17", "#E52592",
    "#1E8E3E", "#3949AB",
];

#[derive(Serialize)]
struct ShallowChildJson {
    name: String,
    is_dir: bool,
    synthetic: bool,
    size: u64,
    apparent_size: u64,
    allocated_size: u64,
    file_count: u64,
    mtime_unix: u64,
    path: String,
}

#[derive(Serialize)]
struct ShallowTreeJson<'a> {
    root_path: &'a str,
    dir_path: String,
    size_mode: &'static str,
    total_bytes: u64,
    item_count: u64,
    skipped_errors: u64,
    skipped_roots: &'a [String],
    children: Vec<ShallowChildJson>,
}

/// Format direct children of `dir` as RFC-4180 CSV (`limit == 0` includes all).
pub fn children_csv(tree: &SpaceTree, dir: NodeId, mode: SizeMode, limit: usize) -> String {
    let cap = if limit == 0 { usize::MAX } else { limit };
    let kids: Vec<NodeId> = tree.children(dir).iter().copied().take(cap).collect();
    rows_csv(tree, &kids, mode)
}

/// Format largest files under `dir` as RFC-4180 CSV.
pub fn largest_csv(tree: &SpaceTree, dir: NodeId, mode: SizeMode, limit: usize) -> String {
    let nodes: Vec<NodeId> = tree
        .largest_files_bounded(dir, mode, limit.max(1), 50_000)
        .into_iter()
        .filter(|&id| tree.try_node(id).is_some_and(|n| !n.synthetic))
        .collect();
    rows_csv(tree, &nodes, mode)
}

/// Format an arbitrary set of nodes as RFC-4180 CSV.
pub fn rows_csv(tree: &SpaceTree, nodes: &[NodeId], mode: SizeMode) -> String {
    let mut out =
        String::from("name,kind,size_bytes,apparent_bytes,allocated_bytes,files,mtime_unix,path\n");
    for &id in nodes {
        let Some(node) = tree.try_node(id) else {
            continue;
        };
        let kind = if node.is_dir {
            "dir"
        } else if node.synthetic {
            "bucket"
        } else {
            "file"
        };
        let path = if node.synthetic {
            String::new()
        } else {
            tree.path_string(id)
        };
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{}",
            csv_escape(&node.name),
            kind,
            node.size(mode),
            node.apparent_size,
            node.allocated_size,
            node.file_count,
            node.mtime_unix,
            csv_escape(&path),
        );
    }
    out
}

/// Format selected nodes as tab-separated lines suitable for spreadsheet/clipboard paste.
pub fn nodes_tsv(tree: &SpaceTree, nodes: &[NodeId], mode: SizeMode) -> String {
    let mut out = String::from("size_bytes\tsize_human\tfiles\tkind\tname\tpath\n");
    for &id in nodes {
        let Some(node) = tree.try_node(id) else {
            continue;
        };
        let kind = if node.is_dir { "dir" } else { "file" };
        let path = if node.synthetic {
            String::new()
        } else {
            tree.path_string(id)
        };
        let size = node.size(mode);
        let _ = writeln!(
            out,
            "{size}\t{}\t{}\t{kind}\t{}\t{}",
            format_bytes(size),
            node.file_count,
            tsv_clean(&node.name),
            tsv_clean(&path),
        );
    }
    out
}

/// Shallow JSON summary for `dir` and its top `limit` children.
pub fn json_shallow(tree: &SpaceTree, dir: NodeId, mode: SizeMode, limit: usize) -> String {
    let cap = if limit == 0 { usize::MAX } else { limit };
    let total_bytes = tree.try_node(dir).map_or(0, |n| n.size(mode));
    let children = tree
        .children(dir)
        .iter()
        .copied()
        .take(cap)
        .filter_map(|cid| {
            let n = tree.try_node(cid)?;
            Some(ShallowChildJson {
                name: n.name.clone(),
                is_dir: n.is_dir,
                synthetic: n.synthetic,
                size: n.size(mode),
                apparent_size: n.apparent_size,
                allocated_size: n.allocated_size,
                file_count: n.file_count,
                mtime_unix: n.mtime_unix,
                path: if n.synthetic {
                    String::new()
                } else {
                    tree.path_string(cid)
                },
            })
        })
        .collect();
    let doc = ShallowTreeJson {
        root_path: &tree.root_path,
        dir_path: tree.path_string(dir),
        size_mode: match mode {
            SizeMode::Apparent => "apparent",
            SizeMode::OnDisk => "on-disk",
        },
        total_bytes,
        item_count: tree.item_count,
        skipped_errors: tree.skipped_errors,
        skipped_roots: &tree.skipped_roots,
        children,
    };
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// Full serialized tree as pretty-printed JSON.
pub fn json_full(tree: &SpaceTree) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(tree)
}

/// Render a standalone SVG sunburst diagram for `dir`.
pub fn sunburst_svg(
    tree: &SpaceTree,
    dir: NodeId,
    mode: SizeMode,
    depth: usize,
    size_px: u32,
) -> String {
    let rings = depth.clamp(1, 4);
    let side = size_px.max(240) as f64;
    let cx = side / 2.0;
    let cy = side / 2.0;
    let inner_r = side * 0.14;
    let outer_r = side * 0.46;
    let ring_w = (outer_r - inner_r) / rings as f64;

    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{side:.0}" height="{side:.0}" viewBox="0 0 {side:.0} {side:.0}">"##,
    );
    let _ = writeln!(
        out,
        r##"  <rect width="100%" height="100%" fill="#11151C"/>"##,
    );

    if let Some(root_node) = tree.try_node(dir) {
        let total = root_node.size(mode).max(1) as f64;
        emit_svg_ring(
            tree,
            dir,
            mode,
            0,
            rings,
            0.0,
            std::f64::consts::TAU,
            inner_r,
            ring_w,
            cx,
            cy,
            total,
            0,
            &mut out,
        );
        let _ = writeln!(
            out,
            r##"  <circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="#1A212D" stroke="#2D3748" stroke-width="1.5"/>"##,
            inner_r - 2.0
        );
        let _ = writeln!(
            out,
            r##"  <text x="{cx:.1}" y="{:.1}" text-anchor="middle" fill="#F1F5F9" font-family="system-ui, -apple-system, sans-serif" font-size="15" font-weight="600">{}</text>"##,
            cy - 4.0,
            xml_escape(&truncate_label(&root_node.name, 16)),
        );
        let _ = writeln!(
            out,
            r##"  <text x="{cx:.1}" y="{:.1}" text-anchor="middle" fill="#94A3B8" font-family="system-ui, -apple-system, sans-serif" font-size="12">{}</text>"##,
            cy + 15.0,
            xml_escape(&format_bytes(root_node.size(mode))),
        );
    }

    out.push_str("</svg>\n");
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_svg_ring(
    tree: &SpaceTree,
    parent: NodeId,
    mode: SizeMode,
    level: usize,
    max_levels: usize,
    a0: f64,
    a1: f64,
    inner_r: f64,
    ring_w: f64,
    cx: f64,
    cy: f64,
    parent_total: f64,
    color_base: usize,
    out: &mut String,
) {
    if level >= max_levels {
        return;
    }
    let span = a1 - a0;
    if span < 0.015 || parent_total <= 0.0 {
        return;
    }
    let r0 = inner_r + level as f64 * ring_w + 1.5;
    let r1 = inner_r + (level + 1) as f64 * ring_w - 1.5;

    let kids = tree.children(parent);
    let mut cursor = a0;
    for (idx, &cid) in kids.iter().take(28).enumerate() {
        let Some(node) = tree.try_node(cid) else {
            continue;
        };
        let sz = node.size(mode) as f64;
        if sz <= 0.0 {
            continue;
        }
        let wedge = (sz / parent_total) * span;
        let next = (cursor + wedge).min(a1);
        if next - cursor >= 0.01 {
            let color_idx = if level == 0 { idx } else { color_base };
            let fill = PALETTE[color_idx % PALETTE.len()];
            let d = annular_sector_path(cx, cy, r0, r1, cursor, next);
            let title = format!("{} — {}", node.name, format_bytes(node.size(mode)));
            let _ = writeln!(
                out,
                r##"  <path d="{d}" fill="{fill}" fill-opacity="{:.2}" stroke="#11151C" stroke-width="1"><title>{}</title></path>"##,
                if level == 0 { 0.92 } else { 0.72 },
                xml_escape(&title),
            );
            if node.is_dir {
                emit_svg_ring(
                    tree,
                    cid,
                    mode,
                    level + 1,
                    max_levels,
                    cursor,
                    next,
                    inner_r,
                    ring_w,
                    cx,
                    cy,
                    sz,
                    color_idx,
                    out,
                );
            }
        }
        cursor = next;
    }
}

fn annular_sector_path(cx: f64, cy: f64, r0: f64, r1: f64, a0: f64, a1: f64) -> String {
    let mut sweep = a1 - a0;
    if sweep >= std::f64::consts::TAU - 1e-4 {
        sweep = std::f64::consts::TAU - 1e-4;
    }
    let end = a0 + sweep;
    let sa0 = a0 - std::f64::consts::FRAC_PI_2;
    let sa1 = end - std::f64::consts::FRAC_PI_2;
    let large = u8::from(sweep > std::f64::consts::PI);

    let (x0_out, y0_out) = (cx + r1 * sa0.cos(), cy + r1 * sa0.sin());
    let (x1_out, y1_out) = (cx + r1 * sa1.cos(), cy + r1 * sa1.sin());
    let (x1_in, y1_in) = (cx + r0 * sa1.cos(), cy + r0 * sa1.sin());
    let (x0_in, y0_in) = (cx + r0 * sa0.cos(), cy + r0 * sa0.sin());

    format!(
        "M {x0_out:.2} {y0_out:.2} A {r1:.2} {r1:.2} 0 {large} 1 {x1_out:.2} {y1_out:.2} L {x1_in:.2} {y1_in:.2} A {r0:.2} {r0:.2} 0 {large} 0 {x0_in:.2} {y0_in:.2} Z"
    )
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn tsv_clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let taken: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{taken}…")
    } else {
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Node;

    fn sample_tree() -> SpaceTree {
        let root = NodeId(0);
        let dir = NodeId(1);
        let file = NodeId(2);
        SpaceTree {
            root_path: "/tmp/demo".into(),
            nodes: vec![
                Node {
                    name: "demo".into(),
                    apparent_size: 2048,
                    allocated_size: 4096,
                    file_count: 1,
                    is_dir: true,
                    synthetic: false,
                    mtime_unix: 10,
                    children: vec![dir],
                    parent: None,
                },
                Node {
                    name: "dir \"quoted\"".into(),
                    apparent_size: 2048,
                    allocated_size: 4096,
                    file_count: 1,
                    is_dir: true,
                    synthetic: false,
                    mtime_unix: 20,
                    children: vec![file],
                    parent: Some(root),
                },
                Node {
                    name: "big,file.bin".into(),
                    apparent_size: 2048,
                    allocated_size: 4096,
                    file_count: 1,
                    is_dir: false,
                    synthetic: false,
                    mtime_unix: 30,
                    children: Vec::new(),
                    parent: Some(dir),
                },
            ],
            root,
            scanned_at_unix: 99,
            item_count: 3,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        }
    }

    #[test]
    fn csv_tsv_json_and_svg_exports_work() {
        let tree = sample_tree();
        let csv = children_csv(&tree, tree.root, SizeMode::OnDisk, 0);
        assert!(csv.contains("\"dir \"\"quoted\"\"\",dir,4096,2048,4096,1,20"));

        let top_csv = largest_csv(&tree, tree.root, SizeMode::Apparent, 5);
        assert!(top_csv.contains("\"big,file.bin\",file,2048"));

        let tsv = nodes_tsv(&tree, &[NodeId(2)], SizeMode::OnDisk);
        assert!(tsv.contains("4096\t"));
        assert!(tsv.contains("big,file.bin"));

        let shallow: serde_json::Value =
            serde_json::from_str(&json_shallow(&tree, tree.root, SizeMode::OnDisk, 10)).unwrap();
        assert_eq!(shallow["total_bytes"], 4096);
        assert_eq!(shallow["children"][0]["name"], "dir \"quoted\"");

        let full: SpaceTree = serde_json::from_str(&json_full(&tree).unwrap()).unwrap();
        assert_eq!(full.item_count, 3);

        let svg = sunburst_svg(&tree, tree.root, SizeMode::OnDisk, 3, 600);
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("<path d=\"M "));
        assert!(svg.contains("</svg>"));
    }
}
