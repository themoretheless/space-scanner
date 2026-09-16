use crate::rules::expand_path_template;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// Paths and path prefixes that must never be deleted, even if a rule matches.
pub fn never_touch_patterns() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        for rel in [
            "Desktop",
            "Pictures",
            "Movies",
            "Music",
            "Videos",
            "Photos Library.photoslibrary",
            ".ssh",
            ".gnupg",
            ".aws",
            ".config/gcloud",
            "Library/Keychains",
            "Library/Mail",
            "Library/Messages",
            "Library/Calendars",
            "Library/Mobile Documents",
            "Library/CloudStorage",
            // Windows profile folders (same names under USERPROFILE)
            "OneDrive",
            "AppData/Roaming/Microsoft/Credentials",
            "AppData/Local/Microsoft/Credentials",
        ] {
            out.push(home.join(rel));
        }
    }
    // System trees (prefix-matched). Bare "/" / "C:\\" are handled separately
    // so they do not mark every absolute path as forbidden.
    for p in [
        "/System",
        "/bin",
        "/sbin",
        "/usr",
        "/etc",
        "/dev",
        "/proc",
        "/sys",
        "/boot",
        "/lib",
        "/lib64",
        "/Applications",
        "/Library",
        "/private/var/db",
        "/private/etc",
        "C:\\Windows",
        "C:\\Program Files",
        "C:\\Program Files (x86)",
    ] {
        out.push(PathBuf::from(p));
    }
    // Expanded sensitive templates
    for t in [
        "$HOME/.ssh",
        "$HOME/.gnupg",
        "$HOME/Library/Keychains",
        "%USERPROFILE%\\Desktop",
        "%USERPROFILE%\\.ssh",
        "%APPDATA%\\Microsoft\\Credentials",
        "%LOCALAPPDATA%\\Microsoft\\Credentials",
    ] {
        if let Some(p) = expand_path_template(t) {
            out.push(p);
        }
    }
    out
}

struct BuiltinProtectionIndex {
    paths: Vec<PathBuf>,
    exact: HashSet<PathBuf>,
}

static BUILTIN_PROTECTED_PATHS: OnceLock<BuiltinProtectionIndex> = OnceLock::new();

fn builtin_protected_paths() -> &'static BuiltinProtectionIndex {
    BUILTIN_PROTECTED_PATHS.get_or_init(|| {
        let mut paths: Vec<PathBuf> = never_touch_patterns()
            .into_iter()
            .filter(|path| !is_filesystem_root(path))
            .filter_map(|path| normalize(&path).ok())
            .collect();
        paths.sort();
        paths.dedup();
        let minimal = crate::clean::coalesce_nested_paths(paths, PathBuf::as_path);
        let exact = minimal.iter().cloned().collect();
        BuiltinProtectionIndex {
            paths: minimal,
            exact,
        }
    })
}

fn intersects_sorted_prefixes(candidate: &Path, protected: &BuiltinProtectionIndex) -> bool {
    if candidate
        .ancestors()
        .any(|ancestor| protected.exact.contains(ancestor))
    {
        return true;
    }
    let lower = protected
        .paths
        .partition_point(|path| path.as_path() < candidate);
    lower < protected.paths.len() && protected.paths[lower].starts_with(candidate)
}

/// Return true if `path` is forbidden to delete.
pub fn is_never_touch(path: &Path) -> bool {
    let Ok(canon) = normalize(path) else {
        // If we cannot normalize, be conservative.
        return true;
    };
    is_never_touch_normalized(&canon)
}

/// Fast path for a path already canonicalized under a non-symlink traversal
/// root. Used by scanners to avoid one filesystem canonicalization per entry.
pub(crate) fn is_never_touch_normalized(canon: &Path) -> bool {
    // Exact root-like paths only (do not treat "/" as a prefix of every path).
    if is_filesystem_root(canon) {
        return true;
    }

    if crate::protection::is_protected_normalized(canon) {
        return true;
    }

    if intersects_sorted_prefixes(canon, builtin_protected_paths()) {
        return true;
    }

    // Block deleting the home directory itself
    if let Some(home) = home_dir() {
        if let Ok(h) = normalize(&home) {
            if canon == h {
                return true;
            }
        }
    }

    false
}

/// A candidate root from a rule must not itself be a never-touch path.
pub fn is_safe_rule_root(path: &Path) -> bool {
    let Ok(canon) = normalize(path) else {
        return false;
    };
    if is_never_touch_normalized(&canon) {
        return false;
    }
    // Refuse scanning pure system roots as rule roots
    if is_filesystem_root(&canon) {
        return false;
    }
    true
}

/// Ensure a discovered file stays under its rule root (symlink escape guard).
pub fn stays_under_root(root: &Path, candidate: &Path) -> bool {
    let Ok(r) = normalize(root) else {
        return false;
    };
    let Ok(c) = normalize(candidate) else {
        return false;
    };
    c.starts_with(&r)
}

pub fn normalize(path: &Path) -> std::io::Result<PathBuf> {
    // Prefer canonicalize when path exists; otherwise lexical normalize.
    if path.exists() {
        return path.canonicalize();
    }
    Ok(lexical_normalize(path))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_filesystem_root(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s == "/"
        || s == "C:\\"
        || s == "C:/"
        || path.parent().is_none()
        || (path.components().count() == 1 && path.has_root())
}

fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

/// Age check: true if file is old enough to clean (or age filter disabled).
pub fn is_old_enough(path: &Path, min_age_days: u64) -> bool {
    if min_age_days == 0 {
        return true;
    }
    let Ok(meta) = path.metadata() else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return true; // if unknown, allow under other filters
    };
    let Ok(elapsed) = modified.elapsed() else {
        // future mtime
        return false;
    };
    elapsed.as_secs() >= min_age_days.saturating_mul(24 * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_documents_allowed_but_home_and_desktop_blocked() {
        if let Some(home) = home_dir() {
            assert!(!is_never_touch(&home.join("Documents")));
            assert!(!is_never_touch(&home.join("Documents/file.txt")));
            assert!(!is_never_touch(&home.join("Downloads/archive.dmg")));
            assert!(!is_never_touch(&home.join(
                "Library/Containers/com.apple.mail/Data/Library/Mail Downloads/file.pdf"
            )));
            assert!(is_never_touch(&home.join("Desktop")));
            assert!(is_never_touch(&home.join("Library/Mail")));
            assert!(is_never_touch(&home));
        }
    }

    #[test]
    fn cache_not_blocked() {
        if let Some(home) = home_dir() {
            // User caches are not in never-touch list
            assert!(!is_never_touch(&home.join("Library/Caches/foo")));
        }
    }

    #[test]
    fn sibling_sort_order_cannot_hide_builtin_ancestor() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/a-child")];
        let index = BuiltinProtectionIndex {
            exact: paths.iter().cloned().collect(),
            paths,
        };
        assert!(intersects_sorted_prefixes(Path::new("/a/nested"), &index));
        assert!(!intersects_sorted_prefixes(Path::new("/unrelated"), &index));
    }
}
