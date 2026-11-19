//! Guided read-only inventory for browser caches, AI models, and Electron/CEF app caches.

use crate::scenario_common::{measure_path, user_home};
use serde::Serialize;
use space_core::format_bytes;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserClass {
    SafeCache,
    Review,
    PreserveProfile,
}

impl BrowserClass {
    fn tag(self) -> &'static str {
        match self {
            Self::SafeCache => "safe-cache",
            Self::Review => "review",
            Self::PreserveProfile => "preserve-profile",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BrowserEntry {
    pub app: String,
    pub kind: &'static str,
    pub path: PathBuf,
    pub classification: BrowserClass,
    pub disk_bytes: u64,
    pub apparent_bytes: u64,
    pub files: u64,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct BrowsersElectronReport {
    pub total_disk_bytes: u64,
    pub safe_cache_disk_bytes: u64,
    pub review_disk_bytes: u64,
    pub preserve_profile_disk_bytes: u64,
    pub entries: Vec<BrowserEntry>,
}

const MIN_ENTRY_BYTES: u64 = 1024;

pub fn run(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let home = user_home()?;
    let report = collect_report(&home);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "Browsers & Electron/CEF apps: {} total ({} safe-cache · {} review · {} preserve-profile)",
        format_bytes(report.total_disk_bytes),
        format_bytes(report.safe_cache_disk_bytes),
        format_bytes(report.review_disk_bytes),
        format_bytes(report.preserve_profile_disk_bytes),
    );
    if report.entries.is_empty() {
        println!("No browser or Electron/CEF caches found.");
        return Ok(());
    }
    println!("\nInventory (sorted by allocated disk bytes):");
    for entry in &report.entries {
        println!(
            "[{:>16}] {:>10}  [{}]  {}  — {}",
            entry.classification.tag(),
            format_bytes(entry.disk_bytes),
            entry.app,
            entry.path.display(),
            entry.note
        );
    }
    println!("\nRead-only diagnostic scenario: nothing was modified or deleted.");
    Ok(())
}

pub fn collect_report(home: &Path) -> BrowsersElectronReport {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    // 1. Browser profile / root trees across macOS, Linux, and Windows layouts.
    let chromium_roots: &[(&str, &str)] = &[
        ("Chrome", "Library/Application Support/Google/Chrome"),
        (
            "Chrome Beta",
            "Library/Application Support/Google/Chrome Beta",
        ),
        (
            "Chrome Dev",
            "Library/Application Support/Google/Chrome Dev",
        ),
        (
            "Chrome Canary",
            "Library/Application Support/Google/Chrome Canary",
        ),
        ("Chromium", "Library/Application Support/Chromium"),
        ("Edge", "Library/Application Support/Microsoft Edge"),
        (
            "Edge Beta",
            "Library/Application Support/Microsoft Edge Beta",
        ),
        ("Edge Dev", "Library/Application Support/Microsoft Edge Dev"),
        (
            "Brave",
            "Library/Application Support/BraveSoftware/Brave-Browser",
        ),
        ("Arc", "Library/Application Support/Arc/User Data"),
        ("Vivaldi", "Library/Application Support/Vivaldi"),
        ("Chrome", ".config/google-chrome"),
        ("Chrome Beta", ".config/google-chrome-beta"),
        ("Chrome Dev", ".config/google-chrome-unstable"),
        ("Chromium", ".config/chromium"),
        ("Edge", ".config/microsoft-edge"),
        ("Brave", ".config/BraveSoftware/Brave-Browser"),
        ("Vivaldi", ".config/vivaldi"),
        ("Chrome", "AppData/Local/Google/Chrome/User Data"),
        ("Chrome Dev", "AppData/Local/Google/Chrome Dev/User Data"),
        ("Chrome Canary", "AppData/Local/Google/Chrome SxS/User Data"),
        ("Edge", "AppData/Local/Microsoft/Edge/User Data"),
        (
            "Brave",
            "AppData/Local/BraveSoftware/Brave-Browser/User Data",
        ),
    ];

    for (app, rel) in chromium_roots {
        let root = home.join(rel);
        if !root.is_dir() {
            continue;
        }
        inspect_chromium_root(app, &root, &mut seen, &mut entries);
        scan_bak_siblings(app, &root, &mut seen, &mut entries);
    }

    // 2. OS Caches & updaters.
    let fixed_caches: &[(&str, &str, &'static str, BrowserClass, &str)] = &[
        (
            "Google Caches",
            "Library/Caches/Google",
            "browser-cache",
            BrowserClass::SafeCache,
            "Chrome/Updater HTTP & code caches",
        ),
        (
            "GoogleUpdater",
            "Library/Application Support/Google/GoogleUpdater",
            "updater",
            BrowserClass::SafeCache,
            "GoogleUpdater staging & CRX cache",
        ),
        (
            "EdgeUpdater",
            "Library/Application Support/Microsoft/EdgeUpdater",
            "updater",
            BrowserClass::SafeCache,
            "EdgeUpdater staging & CRX cache",
        ),
        (
            "Edge Caches",
            "Library/Caches/Microsoft Edge",
            "browser-cache",
            BrowserClass::SafeCache,
            "Edge HTTP & code caches",
        ),
        (
            "Edge Caches",
            "Library/Caches/com.microsoft.edgemac",
            "browser-cache",
            BrowserClass::SafeCache,
            "Edge macOS URL cache",
        ),
        (
            "Brave Caches",
            "Library/Caches/BraveSoftware/Brave-Browser",
            "browser-cache",
            BrowserClass::SafeCache,
            "Brave HTTP & code caches",
        ),
        (
            "Arc Caches",
            "Library/Caches/company.thebrowser.Browser",
            "browser-cache",
            BrowserClass::SafeCache,
            "Arc HTTP caches",
        ),
        (
            "Vivaldi Caches",
            "Library/Caches/Vivaldi",
            "browser-cache",
            BrowserClass::SafeCache,
            "Vivaldi HTTP caches",
        ),
        (
            "Firefox Caches",
            "Library/Caches/Firefox",
            "browser-cache",
            BrowserClass::SafeCache,
            "Firefox cache2",
        ),
        (
            "Firefox Caches",
            ".cache/mozilla/firefox",
            "browser-cache",
            BrowserClass::SafeCache,
            "Firefox Linux cache2",
        ),
        (
            "Chrome Caches",
            ".cache/google-chrome",
            "browser-cache",
            BrowserClass::SafeCache,
            "Chrome Linux disk cache",
        ),
        (
            "Chromium Caches",
            ".cache/chromium",
            "browser-cache",
            BrowserClass::SafeCache,
            "Chromium Linux disk cache",
        ),
        (
            "Safari Caches",
            "Library/Caches/com.apple.Safari",
            "browser-cache",
            BrowserClass::SafeCache,
            "Safari WebKit cache",
        ),
    ];
    for (app, rel, kind, class, note) in fixed_caches {
        push_if_sized(
            app,
            kind,
            &home.join(rel),
            *class,
            note,
            &mut seen,
            &mut entries,
        );
    }

    // 3. Electron / CEF desktop app cache subtrees under App Support / .config / AppData.
    let electron_parents = [
        home.join("Library/Application Support"),
        home.join(".config"),
        home.join("AppData/Roaming"),
        home.join("AppData/Local"),
    ];
    for parent in electron_parents {
        if !parent.is_dir() {
            continue;
        }
        let Ok(read) = fs::read_dir(&parent) else {
            continue;
        };
        for child in read.flatten() {
            let Ok(ft) = child.file_type() else {
                continue;
            };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let app_dir = child.path();
            let app_name = child.file_name().to_string_lossy().to_string();
            if is_browser_parent(&app_name) {
                continue;
            }
            for (sub, note) in [
                ("Cache", "Electron/CEF HTTP cache"),
                ("Code Cache", "Electron/CEF JS/Wasm bytecode cache"),
                ("GPUCache", "Electron/CEF GPU shader cache"),
                ("DawnGraphiteCache", "Dawn Graphite GPU cache"),
                ("DawnWebGPUCache", "Dawn WebGPU cache"),
                ("GraphiteDawnCache", "Graphite Dawn GPU cache"),
                ("ShaderCache", "Shader cache"),
                ("CachedData", "V8 code cache"),
                ("snapshots", "Workspace snapshots"),
                ("logs", "App logs"),
            ] {
                push_if_sized(
                    &app_name,
                    "electron-cache",
                    &app_dir.join(sub),
                    BrowserClass::SafeCache,
                    note,
                    &mut seen,
                    &mut entries,
                );
            }
            let parts = app_dir.join("Partitions");
            if parts.is_dir() {
                if let Ok(p_read) = fs::read_dir(&parts) {
                    for part in p_read.flatten() {
                        let p_dir = part.path();
                        if !p_dir.is_dir() {
                            continue;
                        }
                        for (sub, note) in [
                            ("Cache", "Electron partition HTTP cache"),
                            ("Code Cache", "Electron partition JS cache"),
                            ("GPUCache", "Electron partition GPU cache"),
                        ] {
                            push_if_sized(
                                &app_name,
                                "electron-partition-cache",
                                &p_dir.join(sub),
                                BrowserClass::SafeCache,
                                note,
                                &mut seen,
                                &mut entries,
                            );
                        }
                    }
                }
            }
        }
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
    let mut preserve_profile_disk_bytes = 0u64;
    for entry in &entries {
        total_disk_bytes = total_disk_bytes.saturating_add(entry.disk_bytes);
        match entry.classification {
            BrowserClass::SafeCache => {
                safe_cache_disk_bytes = safe_cache_disk_bytes.saturating_add(entry.disk_bytes);
            }
            BrowserClass::Review => {
                review_disk_bytes = review_disk_bytes.saturating_add(entry.disk_bytes);
            }
            BrowserClass::PreserveProfile => {
                preserve_profile_disk_bytes =
                    preserve_profile_disk_bytes.saturating_add(entry.disk_bytes);
            }
        }
    }

    BrowsersElectronReport {
        total_disk_bytes,
        safe_cache_disk_bytes,
        review_disk_bytes,
        preserve_profile_disk_bytes,
        entries,
    }
}

fn is_browser_parent(name: &str) -> bool {
    matches!(
        name,
        "Google"
            | "Chromium"
            | "BraveSoftware"
            | "Microsoft Edge"
            | "Microsoft Edge Beta"
            | "Microsoft Edge Dev"
            | "Arc"
            | "Vivaldi"
            | "Firefox"
            | "Mozilla"
    )
}

fn inspect_chromium_root(
    app: &str,
    root: &Path,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<BrowserEntry>,
) {
    let top_targets: &[(&str, &'static str, BrowserClass, &str)] = &[
        (
            "OptGuideOnDeviceModel",
            "browser-ai-model",
            BrowserClass::Review,
            "On-device Chrome/Edge AI model (re-downloaded on demand)",
        ),
        (
            "OptGuideOnDeviceClassifierModel",
            "browser-ai-model",
            BrowserClass::Review,
            "On-device AI classifier model",
        ),
        (
            "optimization_guide_model_store",
            "browser-ai-model",
            BrowserClass::Review,
            "Optimization Guide model store",
        ),
        (
            "Local Traces",
            "browser-traces",
            BrowserClass::SafeCache,
            "Chromium Local Traces",
        ),
        (
            "DeferredBrowserMetrics",
            "browser-traces",
            BrowserClass::SafeCache,
            "Deferred browser telemetry metrics",
        ),
        (
            "BrowserMetrics",
            "browser-traces",
            BrowserClass::SafeCache,
            "Browser metrics staging",
        ),
        (
            "Crashpad/completed",
            "browser-crashpad",
            BrowserClass::SafeCache,
            "Completed crash minidumps",
        ),
        (
            "Crashpad/new",
            "browser-crashpad",
            BrowserClass::SafeCache,
            "Pending crash dumps",
        ),
        (
            "GrShaderCache",
            "browser-shader",
            BrowserClass::SafeCache,
            "GPU shader cache",
        ),
        (
            "ShaderCache",
            "browser-shader",
            BrowserClass::SafeCache,
            "Legacy shader cache",
        ),
        (
            "GraphiteDawnCache",
            "browser-shader",
            BrowserClass::SafeCache,
            "Dawn GPU cache",
        ),
        (
            "component_crx_cache",
            "browser-crx",
            BrowserClass::SafeCache,
            "Component update CRX blobs",
        ),
        (
            "extensions_crx_cache",
            "browser-crx",
            BrowserClass::SafeCache,
            "Extension CRX blobs",
        ),
    ];
    for (sub, kind, class, note) in top_targets {
        push_if_sized(app, kind, &root.join(sub), *class, note, seen, entries);
    }

    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    for child in read.flatten() {
        let name = child.file_name().to_string_lossy().to_string();
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }
        let prof = child.path();
        if !prof.is_dir() {
            continue;
        }
        for (sub, kind, class, note) in [
            (
                "Cache",
                "browser-cache",
                BrowserClass::SafeCache,
                "Profile disk cache",
            ),
            (
                "Code Cache",
                "browser-cache",
                BrowserClass::SafeCache,
                "Profile JS/Wasm bytecode cache",
            ),
            (
                "GPUCache",
                "browser-cache",
                BrowserClass::SafeCache,
                "Profile GPU cache",
            ),
            (
                "DawnGraphiteCache",
                "browser-cache",
                BrowserClass::SafeCache,
                "Profile Dawn Graphite cache",
            ),
            (
                "DawnWebGPUCache",
                "browser-cache",
                BrowserClass::SafeCache,
                "Profile Dawn WebGPU cache",
            ),
            (
                "Service Worker/CacheStorage",
                "service-worker",
                BrowserClass::SafeCache,
                "Service Worker CacheStorage",
            ),
            (
                "Service Worker/ScriptCache",
                "service-worker",
                BrowserClass::SafeCache,
                "Service Worker ScriptCache",
            ),
            (
                "Shared Dictionary",
                "browser-cache",
                BrowserClass::SafeCache,
                "Compression Shared Dictionary cache",
            ),
            (
                "Local Storage",
                "browser-profile",
                BrowserClass::PreserveProfile,
                "Site Local Storage (preserved)",
            ),
            (
                "IndexedDB",
                "browser-profile",
                BrowserClass::PreserveProfile,
                "Site IndexedDB stores (preserved)",
            ),
            (
                "WebStorage",
                "browser-profile",
                BrowserClass::PreserveProfile,
                "Site WebStorage data (preserved)",
            ),
        ] {
            push_if_sized(app, kind, &prof.join(sub), class, note, seen, entries);
        }
    }
}

fn scan_bak_siblings(
    app: &str,
    root: &Path,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<BrowserEntry>,
) {
    let Some(parent) = root.parent() else {
        return;
    };
    let Some(base_name) = root.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let prefix = format!("{base_name}.bak");
    let Ok(read) = fs::read_dir(parent) else {
        return;
    };
    for child in read.flatten() {
        let name = child.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) {
            push_if_sized(
                app,
                "profile-backup",
                &child.path(),
                BrowserClass::Review,
                "Old browser profile .bak snapshot — verify profile works before removing",
                seen,
                entries,
            );
        }
    }
}

fn push_if_sized(
    app: &str,
    kind: &'static str,
    path: &Path,
    class: BrowserClass,
    note: &str,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<BrowserEntry>,
) {
    let Ok(canon) = path.canonicalize() else {
        return;
    };
    if !seen.insert(canon) {
        return;
    }
    let Some(stats) = measure_path(path) else {
        return;
    };
    if stats.disk_bytes < MIN_ENTRY_BYTES {
        return;
    }
    entries.push(BrowserEntry {
        app: app.to_string(),
        kind,
        path: path.to_path_buf(),
        classification: class,
        disk_bytes: stats.disk_bytes,
        apparent_bytes: stats.apparent_bytes,
        files: stats.files,
        note: note.to_string(),
    });
}
