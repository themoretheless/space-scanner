//! Text exports of what the interface shows: CSV for spreadsheets, a shallow
//! JSON of one folder, TSV for the clipboard. Pure functions; the app writes
//! the result through a save dialog on a background thread.

use std::path::{Path, PathBuf};

use space_core::{NodeId, SizeMode, SpaceTree};

/// RFC 4180 field: quoted when it holds a comma, quote or line break.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn csv_row(fields: &[&str]) -> String {
    let mut line = fields
        .iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",");
    line.push('\n');
    line
}

fn kind(tree: &SpaceTree, id: NodeId) -> &'static str {
    let n = tree.node(id);
    if n.synthetic {
        "group"
    } else if n.is_dir {
        "folder"
    } else {
        "file"
    }
}

/// `ids` as rows of name, path, kind, size in bytes and file count.
pub fn nodes_csv(tree: &SpaceTree, ids: &[NodeId], mode: SizeMode) -> String {
    let mut out = csv_row(&["name", "path", "kind", "size_bytes", "files"]);
    for &id in ids {
        let n = tree.node(id);
        out.push_str(&csv_row(&[
            &n.name,
            &tree.path_string(id),
            kind(tree, id),
            &n.size(mode).to_string(),
            &n.file_count.to_string(),
        ]));
    }
    out
}

/// The children of `dir`, largest first (F0873).
pub fn children_csv(tree: &SpaceTree, dir: NodeId, mode: SizeMode) -> String {
    let mut ids = tree.children(dir).to_vec();
    ids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
    nodes_csv(tree, &ids, mode)
}

/// `dir` and its direct children as JSON, one level deep (F0875).
pub fn shallow_json(tree: &SpaceTree, dir: NodeId, mode: SizeMode) -> String {
    let n = tree.node(dir);
    let mut ids = tree.children(dir).to_vec();
    ids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
    let children: Vec<serde_json::Value> = ids
        .iter()
        .map(|&c| {
            let k = tree.node(c);
            serde_json::json!({
                "name": k.name,
                "path": tree.path_string(c),
                "kind": kind(tree, c),
                "size_bytes": k.size(mode),
                "files": k.file_count,
            })
        })
        .collect();
    let value = serde_json::json!({
        "path": tree.path_string(dir),
        "name": n.name,
        "size_bytes": n.size(mode),
        "files": n.file_count,
        "size_mode": match mode {
            SizeMode::OnDisk => "on_disk",
            SizeMode::Apparent => "apparent",
        },
        "children": children,
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Tab-separated name, size and path: pastes into any editor or sheet (F0877).
pub fn listing_tsv(tree: &SpaceTree, ids: &[NodeId], mode: SizeMode) -> String {
    let mut out = String::from("name\tsize\tsize_bytes\tpath\n");
    for &id in ids {
        let n = tree.node(id);
        let size = n.size(mode);
        let name = n.name.replace(['\t', '\n', '\r'], " ");
        let path = tree.path_string(id).replace(['\t', '\n', '\r'], " ");
        out.push_str(&format!(
            "{name}\t{}\t{size}\t{path}\n",
            crate::i18n::bytes(size)
        ));
    }
    out
}

/// The review queue: path and size per line (F0603).
pub fn review_csv(items: &[(PathBuf, u64)]) -> String {
    let mut out = csv_row(&["path", "size_bytes"]);
    for (path, size) in items {
        out.push_str(&csv_row(&[&path.display().to_string(), &size.to_string()]));
    }
    out
}

/// Where this platform keeps the Trash the app moves files to.
pub fn trash_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    {
        Some(home.join(".Trash"))
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Some(base.join("Trash/files"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = home;
        None
    }
}

/// Open `dir` in the system file browser.
pub fn open_in_file_browser(dir: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(dir);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        c.arg(dir);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(dir);
        c
    };
    cmd.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_core::Node;

    fn node(name: &str, size: u64, is_dir: bool, parent: Option<NodeId>, children: Vec<NodeId>) -> Node {
        Node {
            name: name.into(),
            apparent_size: size,
            allocated_size: size,
            file_count: if is_dir { 2 } else { 1 },
            is_dir,
            synthetic: false,
            children,
            parent,
            raw_name: None,
        }
    }

    fn sample() -> SpaceTree {
        SpaceTree {
            root_path: "/r".into(),
            nodes: vec![
                node("r", 30, true, None, vec![NodeId(1), NodeId(2)]),
                node("a, \"quoted\"", 10, false, Some(NodeId(0)), vec![]),
                node("big", 20, true, Some(NodeId(0)), vec![]),
            ],
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 3,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        }
    }

    #[test]
    fn csv_quotes_commas_and_quotes_and_sorts_largest_first() {
        let csv = children_csv(&sample(), NodeId(0), SizeMode::OnDisk);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "name,path,kind,size_bytes,files");
        assert_eq!(lines[1], "big,/r/big,folder,20,2");
        assert_eq!(lines[2], "\"a, \"\"quoted\"\"\",\"/r/a, \"\"quoted\"\"\",file,10,1");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn json_is_one_level_deep_with_sizes() {
        let json = shallow_json(&sample(), NodeId(0), SizeMode::Apparent);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["path"], "/r");
        assert_eq!(v["size_bytes"], 30);
        assert_eq!(v["size_mode"], "apparent");
        assert_eq!(v["children"].as_array().unwrap().len(), 2);
        assert_eq!(v["children"][0]["name"], "big");
        assert!(v["children"][0].get("children").is_none(), "shallow");
    }

    #[test]
    fn tsv_and_review_csv_list_every_row() {
        let tsv = listing_tsv(&sample(), &[NodeId(2), NodeId(1)], SizeMode::OnDisk);
        assert_eq!(tsv.lines().count(), 3);
        assert!(tsv.lines().nth(1).unwrap().starts_with("big\t"));
        let csv = review_csv(&[(PathBuf::from("/r/big"), 20)]);
        assert_eq!(csv, "path,size_bytes\n/r/big,20\n");
    }
}
