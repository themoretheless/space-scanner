//! Guided read-only inventory for heavy caches, custom tool stores, and hidden DB bloat.

use crate::scenario_common::{dev_project_roots, measure_path, user_home};
use serde::Serialize;
use space_core::format_bytes;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeavyClass {
    SafeCache,
    Review,
    PreserveDb,
}

impl HeavyClass {
    fn tag(self) -> &'static str {
        match self {
            Self::SafeCache => "safe-cache",
            Self::Review => "review",
            Self::PreserveDb => "preserve-db",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HeavyEntry {
    pub source: &'static str,
    pub path: PathBuf,
    pub classification: HeavyClass,
    pub disk_bytes: u64,
    pub apparent_bytes: u64,
    pub files: u64,
    pub db_files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct HeavyCachesReport {
    pub min_size_bytes: u64,
    pub total_disk_bytes: u64,
    pub safe_cache_disk_bytes: u64,
    pub review_disk_bytes: u64,
    pub preserve_db_disk_bytes: u64,
    pub entries: Vec<HeavyEntry>,
}

pub fn run(min_size: u64, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let home = user_home()?;
    let report = collect_report(&home, min_size);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Heavy caches & hidden stores (>= {}): {} total ({} safe-cache · {} review · {} preserve-db)",
        format_bytes(report.min_size_bytes),
        format_bytes(report.total_disk_bytes),
        format_bytes(report.safe_cache_disk_bytes),
        format_bytes(report.review_disk_bytes),
        format_bytes(report.preserve_db_disk_bytes),
    );
    if report.entries.is_empty() {
        println!(
            "No directories/files >= {} found in scanned cache roots.",
            format_bytes(min_size)
        );
        return Ok(());
    }
    println!("\nInventory (sorted by allocated disk bytes):");
    for entry in &report.entries {
        let db_note = if entry.db_files.is_empty() {
            String::new()
        } else {
            format!("  [DB files: {}]", entry.db_files.join(", "))
        };
        println!(
            "[{:>11}] {:>10}  ({})  {}{}",
            entry.classification.tag(),
            format_bytes(entry.disk_bytes),
            entry.source,
            entry.path.display(),
            db_note
        );
    }
    println!("\nDirectories marked `preserve-db` contain DuckDB/SQLite/data.db stores — compact or prune via the owning tool, never delete blindly.");
    println!("Read-only diagnostic scenario: nothing was modified or deleted.");
    Ok(())
}

pub fn collect_report(home: &Path, min_size: u64) -> HeavyCachesReport {
    let mut seen_roots = HashSet::new();
    let mut seen_entries = HashSet::new();
    let mut entries = Vec::new();

    let mut roots: Vec<(&'static str, PathBuf, HeavyClass)> = vec![
        ("xdg-cache", home.join(".cache"), HeavyClass::Review),
        ("local-share", home.join(".local/share"), HeavyClass::Review),
        ("tmpdir", std::env::temp_dir(), HeavyClass::SafeCache),
        ("pcloud", home.join(".pcloud"), HeavyClass::Review),
        ("rustup", home.join(".rustup"), HeavyClass::Review),
        ("cargo", home.join(".cargo"), HeavyClass::Review),
        ("nuget", home.join(".nuget"), HeavyClass::Review),
        ("npm", home.join(".npm"), HeavyClass::SafeCache),
        (
            "library-caches",
            home.join("Library/Caches"),
            HeavyClass::SafeCache,
        ),
        (
            "app-support",
            home.join("Library/Application Support"),
            HeavyClass::Review,
        ),
        (
            "homebrew-var",
            PathBuf::from("/opt/homebrew/var/homebrew"),
            HeavyClass::Review,
        ),
    ];

    if let Some(darwin_cache) = space_clean::expand_path_template("$DARWIN_USER_CACHE_DIR") {
        roots.push(("darwin-user-cache", darwin_cache, HeavyClass::SafeCache));
    }

    for (source, root, default_class) in roots {
        if !root.is_dir() {
            continue;
        }
        let root_canon = root.canonicalize().unwrap_or_else(|_| root.clone());
        if !seen_roots.insert(root_canon) {
            continue;
        }
        let Ok(read) = fs::read_dir(&root) else {
            continue;
        };
        for child in read.flatten() {
            let child_path = child.path();
            if is_never_touch_like(&child_path) {
                continue;
            }
            let class = classify_child(source, &child_path, default_class);
            inspect_candidate(
                source,
                &child_path,
                class,
                min_size,
                &mut seen_entries,
                &mut entries,
            );
        }
    }

    for dev_root in dev_project_roots(home) {
        scan_project_hidden_dirs(&dev_root, 0, min_size, &mut seen_entries, &mut entries);
    }

    entries.sort_by(|a, b| {
        b.disk_bytes
            .cmp(&a.disk_bytes)
            .then_with(|| b.apparent_bytes.cmp(&a.apparent_bytes))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut total_disk_bytes = 0u64;
    let mut safe_cache_disk_bytes = 0u64;
    let mut review_disk_bytes = 0u64;
    let mut preserve_db_disk_bytes = 0u64;
    for entry in &entries {
        total_disk_bytes = total_disk_bytes.saturating_add(entry.disk_bytes);
        match entry.classification {
            HeavyClass::SafeCache => {
                safe_cache_disk_bytes = safe_cache_disk_bytes.saturating_add(entry.disk_bytes);
            }
            HeavyClass::Review => {
                review_disk_bytes = review_disk_bytes.saturating_add(entry.disk_bytes);
            }
            HeavyClass::PreserveDb => {
                preserve_db_disk_bytes = preserve_db_disk_bytes.saturating_add(entry.disk_bytes);
            }
        }
    }

    HeavyCachesReport {
        min_size_bytes: min_size,
        total_disk_bytes,
        safe_cache_disk_bytes,
        review_disk_bytes,
        preserve_db_disk_bytes,
        entries,
    }
}

fn is_never_touch_like(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some(
            "SpaceScanner.app"
                | "AddressBook"
                | "CallHistoryDB"
                | "CloudDocs"
                | "FileProvider"
                | "Mail"
                | "Messages"
                | "MobileSync"
                | "HomeKit"
                | "Dock"
                | "Knowledge"
        )
    )
}

fn classify_child(source: &'static str, path: &Path, default_class: HeavyClass) -> HeavyClass {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match (source, name) {
        ("pcloud", "temp")
        | ("rustup", "tmp")
        | ("rustup", "downloads")
        | ("homebrew-var", "tmp") => HeavyClass::SafeCache,
        ("local-share", "NuGet") => HeavyClass::SafeCache,
        _ => default_class,
    }
}

fn scan_project_hidden_dirs(
    dir: &Path,
    depth: usize,
    min_size: u64,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<HeavyEntry>,
) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for child in read.flatten() {
        let Ok(ft) = child.file_type() else {
            continue;
        };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        let name = child.file_name();
        let name_str = name.to_string_lossy();
        if matches!(name_str.as_ref(), ".git" | "node_modules" | "target") {
            continue;
        }
        let path = child.path();
        if name_str == ".cache" || (name_str.starts_with('.') && name_str.contains("-work")) {
            inspect_candidate(
                "project-hidden",
                &path,
                HeavyClass::Review,
                min_size,
                seen,
                entries,
            );
            continue;
        }
        if depth < 2 && !name_str.starts_with('.') {
            scan_project_hidden_dirs(&path, depth + 1, min_size, seen, entries);
        }
    }
}

fn inspect_candidate(
    source: &'static str,
    path: &Path,
    default_class: HeavyClass,
    min_size: u64,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<HeavyEntry>,
) {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canon) {
        return;
    }
    let Some(stats) = measure_path(path) else {
        return;
    };
    if stats.disk_bytes < min_size {
        return;
    }
    let db_files = detect_db_files(path);
    let classification = if db_files.is_empty() {
        default_class
    } else {
        HeavyClass::PreserveDb
    };
    entries.push(HeavyEntry {
        source,
        path: path.to_path_buf(),
        classification,
        disk_bytes: stats.disk_bytes,
        apparent_bytes: stats.apparent_bytes,
        files: stats.files,
        db_files,
    });
}

fn detect_db_files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if dir.is_file() {
            if is_db_file(&dir) {
                if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                    found.push(name.to_string());
                }
            }
            continue;
        }
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let p = entry.path();
            if ft.is_file() && is_db_file(&p) {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    found.push(name.to_string());
                    if found.len() >= 4 {
                        return found;
                    }
                }
            } else if ft.is_dir() && depth < 2 {
                stack.push((p, depth + 1));
            }
        }
    }
    found
}

fn is_db_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name == "data.db" {
        return true;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("duckdb" | "sqlite" | "sqlite3")
    )
}
