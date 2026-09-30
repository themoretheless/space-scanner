use anyhow::Result;
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn timestamp(value: std::io::Result<SystemTime>) -> Option<u64> {
    value
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Metadata only: no file contents, subprocesses or recursive filesystem walks.
pub fn inspect(path: &Path) -> Result<Value> {
    anyhow::ensure!(path.is_absolute(), "path must be absolute");
    let meta = fs::symlink_metadata(path)?;
    let canonical = path.canonicalize().ok();
    let symlink = meta.file_type().is_symlink();
    let kind = if symlink {
        "symlink"
    } else if meta.is_dir() {
        "directory"
    } else if meta.is_file() {
        "file"
    } else {
        "special"
    };
    let modified = timestamp(meta.modified());
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let allocated: Option<u64> = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(meta.blocks().saturating_mul(512))
        }
        #[cfg(not(unix))]
        {
            None
        }
    };
    let mut evidence = Vec::new();
    let mut errors = Vec::new();
    // Never inspect through a symlink's target to infer ownership of the link.
    if !symlink {
        let start = canonical.as_deref().unwrap_or(path);
        let start = if meta.is_dir() {
            start
        } else {
            start.parent().unwrap_or(start)
        };
        for ancestor in start.ancestors().take(16) {
            for (marker, ecosystem) in [
                ("Cargo.toml", "rust"),
                ("package.json", "javascript"),
                ("pyproject.toml", "python"),
                ("go.mod", "go"),
                (".git", "git"),
            ] {
                let marker_path = ancestor.join(marker);
                match fs::symlink_metadata(&marker_path) {
                    Ok(_) => evidence.push(json!({"kind":"project_marker", "path":marker_path,"ecosystem":ecosystem,"meaning":"nearby project; ownership and regenerability not verified"})),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                    Err(e) => errors.push(json!({"path":marker_path,"error":e.to_string()})),
                }
            }
            if ancestor.extension().is_some_and(|e| e == "app") {
                evidence.push(json!({"kind":"application_bundle", "path":ancestor}));
            }
        }
    }
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if [
            "target",
            "node_modules",
            "__pycache__",
            ".cache",
            "Caches",
            "DerivedData",
        ]
        .contains(&name.as_ref())
        {
            evidence.push(json!({"kind":"directory_name_hint", "name":name,"meaning":"possible generated/cache data; name alone does not establish deletability"}));
        }
    }
    Ok(json!({
        "path":path,"canonical_path":canonical,"type":kind,
        "symlink_target":if symlink { fs::read_link(path).ok() } else { None },
        "apparent_bytes":meta.len(),"allocated_bytes":allocated,
        "size_scope":if meta.is_dir() {"directory_entry_only; use scan_disk for recursive totals"} else {"entry"},
        "modified_unix":modified,"created_unix":timestamp(meta.created()),
        "age_days":modified.and_then(|t| now.checked_sub(t)).map(|s| s / 86400),
        "readonly":meta.permissions().readonly(),
        "blocked_by_safety":space_clean::safety::is_never_touch(path),
        "protected_by_rule":space_clean::is_protected_by_rule(path),
        "evidence":evidence,"inspection_errors":errors,
        "assessment":"requires_review", "deletion_authorized":false,
        "limitations":["Project markers and directory names are hints, not proof of ownership or regenerability", "No file contents or active-process checks", "Ancestor inspection is limited to 16 levels"]
    }))
}
