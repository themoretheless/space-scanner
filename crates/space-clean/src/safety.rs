use crate::rules::expand_path_template;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

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
    // Expanded sensitive templates. The Windows system roots come from the
    // environment so a system installed on another drive letter is covered.
    for t in [
        "$HOME/.ssh",
        "$HOME/.gnupg",
        "$HOME/Library/Keychains",
        "%USERPROFILE%\\Desktop",
        "%USERPROFILE%\\.ssh",
        "%APPDATA%\\Microsoft\\Credentials",
        "%LOCALAPPDATA%\\Microsoft\\Credentials",
        "%SystemRoot%",
        "%WINDIR%",
        "%ProgramFiles%",
        "%ProgramFiles(x86)%",
    ] {
        if let Some(p) = expand_path_template(t) {
            out.push(p);
        }
    }
    out
}

/// Case-folded form of a path for prefix comparisons on filesystems that
/// ignore case (macOS, Windows). Identity elsewhere.
pub(crate) fn fold_case(path: &Path) -> PathBuf {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// Strip the Windows verbatim prefix (`\\?\C:\x` → `C:\x`, `\\?\UNC\srv\s` →
/// `\\srv\s`) that `canonicalize` adds, so prefixes compare with plain paths.
pub(crate) fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

struct BuiltinProtectionIndex {
    paths: Vec<PathBuf>,
    exact: HashSet<PathBuf>,
}

static NORMALIZED_HOME: OnceLock<Option<PathBuf>> = OnceLock::new();

static BUILTIN_PROTECTED_PATHS: OnceLock<BuiltinProtectionIndex> = OnceLock::new();

fn builtin_protected_paths() -> &'static BuiltinProtectionIndex {
    BUILTIN_PROTECTED_PATHS.get_or_init(|| {
        let mut paths: Vec<PathBuf> = never_touch_patterns()
            .into_iter()
            .filter(|path| !is_filesystem_root(path))
            .filter_map(|path| normalize(&path).ok())
            .map(|path| fold_case(&path))
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

    let folded = fold_case(canon);

    if crate::protection::is_protected_normalized(&folded) {
        return true;
    }

    if intersects_sorted_prefixes(&folded, builtin_protected_paths()) {
        return true;
    }

    // Like the built-in protected paths, the home boundary is fixed for this
    // process. Avoid resolving the same home directory for every scanned entry.
    if NORMALIZED_HOME
        .get_or_init(|| {
            home_dir()
                .and_then(|home| normalize(&home).ok())
                .map(|home| fold_case(&home))
        })
        .as_deref()
        == Some(folded.as_path())
    {
        return true;
    }

    false
}

/// A candidate root from a rule must not itself be a never-touch path.
pub fn is_safe_rule_root(path: &Path) -> bool {
    let Ok(canon) = normalize(path) else {
        return false;
    };
    is_safe_rule_root_normalized(&canon)
}

/// [`is_safe_rule_root`] for a root that was already canonicalized.
pub(crate) fn is_safe_rule_root_normalized(canon: &Path) -> bool {
    if is_never_touch_normalized(canon) {
        return false;
    }
    // Refuse scanning pure system roots as rule roots
    if is_filesystem_root(canon) {
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
    fold_case(&c).starts_with(fold_case(&r))
}

/// Canonicalize a rule root with one filesystem call: `Ok(None)` when it does
/// not exist, the verbatim prefix stripped on Windows.
pub(crate) fn canonical_root(path: &Path) -> std::io::Result<Option<PathBuf>> {
    match path.canonicalize() {
        Ok(canonical) => Ok(Some(if cfg!(windows) {
            strip_verbatim_prefix(&canonical)
        } else {
            canonical
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn normalize(path: &Path) -> std::io::Result<PathBuf> {
    // Prefer canonicalize when path exists; otherwise lexical normalize.
    if path.exists() {
        let canonical = path.canonicalize()?;
        return Ok(if cfg!(windows) {
            strip_verbatim_prefix(&canonical)
        } else {
            canonical
        });
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

/// Upper bound on entries visited when looking for the newest change inside a
/// directory; a cache with more entries uses the newest time seen so far.
pub(crate) const DIR_AGE_WALK_LIMIT: usize = 50_000;

/// Newest modification time of a directory or anything inside it (bounded
/// walk, symlinks are not followed and count with their own timestamp).
pub(crate) fn newest_mtime(path: &Path, own: Option<SystemTime>) -> Option<SystemTime> {
    let mut newest = own;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .skip(1)
        .take(DIR_AGE_WALK_LIMIT)
    {
        if let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
            newest = Some(newest.map_or(modified, |n| n.max(modified)));
        }
    }
    newest
}

/// Age check against a known modification time: true if old enough (or the
/// filter is disabled). Unknown mtimes pass so other filters decide; future
/// timestamps never pass.
pub(crate) fn mtime_is_old_enough(modified: Option<SystemTime>, min_age_days: u64) -> bool {
    if min_age_days == 0 {
        return true;
    }
    let Some(modified) = modified else {
        return true;
    };
    let Ok(elapsed) = modified.elapsed() else {
        return false;
    };
    elapsed.as_secs() >= min_age_days.saturating_mul(24 * 3600)
}

/// Age check: true if the entry is old enough to clean (or age filter disabled).
/// A directory is as young as the newest entry inside it. Symlinks are judged
/// by the link itself, never by their target.
pub fn is_old_enough(path: &Path, min_age_days: u64) -> bool {
    if min_age_days == 0 {
        return true;
    }
    let Ok(meta) = path.symlink_metadata() else {
        return false;
    };
    let own = meta.modified().ok();
    let modified = if meta.is_dir() {
        newest_mtime(path, own)
    } else {
        own
    };
    mtime_is_old_enough(modified, min_age_days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    #[test]
    fn verbatim_prefixes_are_stripped() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\Windows")),
            PathBuf::from(r"C:\Windows")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new("/usr/share")),
            PathBuf::from("/usr/share")
        );
    }

    #[test]
    fn case_folding_follows_the_host_filesystem() {
        let folded = fold_case(Path::new("/Users/Me/Library"));
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            assert_eq!(folded, PathBuf::from("/users/me/library"));
        } else {
            assert_eq!(folded, PathBuf::from("/Users/Me/Library"));
        }
    }

    #[test]
    fn directory_age_is_its_newest_entry_and_symlinks_use_the_link() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cache");
        fs::create_dir_all(dir.join("nested")).unwrap();
        let old_file = dir.join("nested/old.bin");
        fs::write(&old_file, b"old").unwrap();
        let ancient = SystemTime::now() - std::time::Duration::from_secs(30 * 24 * 3600);
        let backdate = |path: &Path| {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(ancient))
                .unwrap();
        };
        backdate(&old_file);
        // Directories: backdate via File on unix is not possible for dirs, so
        // use filetime-free approach: touch the dir mtimes by renaming entries.
        let dir_file = fs::File::open(&dir).unwrap();
        let _ = dir_file.set_times(fs::FileTimes::new().set_modified(ancient));
        let _ = fs::File::open(dir.join("nested"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(ancient));
        assert!(is_old_enough(&old_file, 7));
        assert!(is_old_enough(&dir, 7), "nothing recent inside");
        // A fresh file deep inside makes the whole directory young.
        fs::write(dir.join("nested/fresh.bin"), b"fresh").unwrap();
        let _ = fs::File::open(dir.join("nested"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(ancient));
        let _ = dir_file.set_times(fs::FileTimes::new().set_modified(ancient));
        assert!(!is_old_enough(&dir, 7));
        assert!(is_old_enough(&dir, 0));
        #[cfg(unix)]
        {
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&old_file, &link).unwrap();
            // The link itself was created just now: not old enough even
            // though its target is a month old.
            assert!(!is_old_enough(&link, 7));
        }
        assert!(!is_old_enough(&root.path().join("missing"), 7));
    }
}
