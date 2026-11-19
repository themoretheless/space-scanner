//! Guided read-only inventory for developer toolchains, SDKs, package caches, and E2E browsers.

use crate::scenario_common::{measure_path, user_home};
use serde::Serialize;
use space_core::format_bytes;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolchainClass {
    SafeCache,
    Review,
    Preserve,
}

impl ToolchainClass {
    fn tag(self) -> &'static str {
        match self {
            Self::SafeCache => "safe-cache",
            Self::Review => "review",
            Self::Preserve => "preserve",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ToolchainEntry {
    pub tool: &'static str,
    pub path: PathBuf,
    pub classification: ToolchainClass,
    pub disk_bytes: u64,
    pub apparent_bytes: u64,
    pub files: u64,
    pub reclaim_hint: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DevToolchainsReport {
    pub total_disk_bytes: u64,
    pub safe_cache_disk_bytes: u64,
    pub review_disk_bytes: u64,
    pub preserve_disk_bytes: u64,
    pub entries: Vec<ToolchainEntry>,
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
        "Developer toolchains & SDKs: {} total ({} safe-cache · {} review · {} preserve)",
        format_bytes(report.total_disk_bytes),
        format_bytes(report.safe_cache_disk_bytes),
        format_bytes(report.review_disk_bytes),
        format_bytes(report.preserve_disk_bytes),
    );
    if report.entries.is_empty() {
        println!("No toolchain or package cache stores found.");
        return Ok(());
    }
    println!("\nInventory (sorted by allocated disk bytes):");
    for entry in &report.entries {
        println!(
            "[{:>10}] {:>10}  [{}]  {}  — {}",
            entry.classification.tag(),
            format_bytes(entry.disk_bytes),
            entry.tool,
            entry.path.display(),
            entry.reclaim_hint
        );
    }
    println!("\nRead-only diagnostic scenario: nothing was modified or deleted.");
    Ok(())
}

pub fn collect_report(home: &Path) -> DevToolchainsReport {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    // Per-toolchain rustup installations (broken out individually so oldest unused versions stand out).
    let toolchains_dir = home.join(".rustup/toolchains");
    if toolchains_dir.is_dir() {
        if let Ok(read) = fs::read_dir(&toolchains_dir) {
            for child in read.flatten() {
                if child.path().is_dir() {
                    push_entry(
                        "rustup toolchain",
                        &child.path(),
                        ToolchainClass::Review,
                        "Inspect `rustup toolchain list`; remove old versions via `rustup toolchain uninstall <name>`",
                        &mut seen,
                        &mut entries,
                    );
                }
            }
        }
    }

    // Fixed home-relative toolchains, SDKs, IDEs, and headless browsers.
    let home_specs: &[(&'static str, &str, ToolchainClass, &'static str)] = &[
        (
            "rustup tmp/downloads",
            ".rustup/tmp",
            ToolchainClass::SafeCache,
            "Regenerable rustup download staging",
        ),
        (
            "rustup downloads",
            ".rustup/downloads",
            ToolchainClass::SafeCache,
            "Regenerable rustup tarball downloads",
        ),
        (
            "cargo registry",
            ".cargo/registry",
            ToolchainClass::Review,
            "Crate tarballs/extracted sources (redownloaded on build)",
        ),
        (
            "cargo git",
            ".cargo/git",
            ToolchainClass::Review,
            "Cargo git dependency db/checkouts",
        ),
        (
            "cargo bin",
            ".cargo/bin",
            ToolchainClass::Preserve,
            "Installed Cargo CLI tools",
        ),
        (
            ".NET SDKs/runtime",
            ".dotnet",
            ToolchainClass::Review,
            "Remove old SDKs with `dotnet sdk check` / `dotnet-core-uninstall`",
        ),
        (
            "NuGet packages",
            ".nuget/packages",
            ToolchainClass::Review,
            "Global NuGet package cache (`dotnet nuget locals global-packages --clear`)",
        ),
        (
            "NuGet HTTP cache",
            ".local/share/NuGet",
            ToolchainClass::SafeCache,
            "`dotnet nuget locals http-cache --clear`",
        ),
        (
            "npm cache",
            ".npm/_cacache",
            ToolchainClass::SafeCache,
            "`npm cache clean --force`",
        ),
        (
            "npm npx cache",
            ".npm/_npx",
            ToolchainClass::SafeCache,
            "Cached ephemeral npx packages",
        ),
        (
            "yarn cache",
            ".yarn",
            ToolchainClass::SafeCache,
            "`yarn cache clean`",
        ),
        (
            "pnpm store",
            ".pnpm-store",
            ToolchainClass::Review,
            "`pnpm store prune`",
        ),
        (
            "pnpm store (XDG)",
            "Library/pnpm/store",
            ToolchainClass::Review,
            "`pnpm store prune`",
        ),
        (
            "bun cache",
            ".bun/install/cache",
            ToolchainClass::SafeCache,
            "Regenerable Bun package cache",
        ),
        (
            "uv cache",
            "Library/Caches/uv",
            ToolchainClass::SafeCache,
            "`uv cache clean`",
        ),
        (
            "uv cache (XDG)",
            ".cache/uv",
            ToolchainClass::SafeCache,
            "`uv cache clean`",
        ),
        (
            "pip cache",
            "Library/Caches/pip",
            ToolchainClass::SafeCache,
            "`pip cache purge`",
        ),
        (
            "pip cache (XDG)",
            ".cache/pip",
            ToolchainClass::SafeCache,
            "`pip cache purge`",
        ),
        (
            "Homebrew cache",
            "Library/Caches/Homebrew",
            ToolchainClass::SafeCache,
            "`brew cleanup --prune=all -s`",
        ),
        (
            "Playwright browsers",
            "Library/Caches/ms-playwright",
            ToolchainClass::Review,
            "`npx playwright uninstall` or remove unused browser versions",
        ),
        (
            "Playwright (XDG)",
            ".cache/ms-playwright",
            ToolchainClass::Review,
            "Headless Playwright browser binaries",
        ),
        (
            "Puppeteer browsers",
            ".cache/puppeteer",
            ToolchainClass::Review,
            "Headless Chrome/Chromium binaries for Puppeteer",
        ),
        (
            "Cypress cache",
            "Library/Caches/Cypress",
            ToolchainClass::Review,
            "`npx cypress cache clear`",
        ),
        (
            "Cypress (XDG)",
            ".cache/Cypress",
            ToolchainClass::Review,
            "`npx cypress cache clear`",
        ),
        (
            "Gradle caches",
            ".gradle/caches",
            ToolchainClass::Review,
            "Gradle transform/dependency caches",
        ),
        (
            "Maven repository",
            ".m2/repository",
            ToolchainClass::Review,
            "Local Maven artifact repository",
        ),
        (
            "JetBrains caches",
            "Library/Caches/JetBrains",
            ToolchainClass::SafeCache,
            "Regenerable IDE index/system caches (close IDEs first)",
        ),
        (
            "JetBrains logs",
            "Library/Logs/JetBrains",
            ToolchainClass::SafeCache,
            "JetBrains IDE logs",
        ),
        (
            "JetBrains Support",
            "Library/Application Support/JetBrains",
            ToolchainClass::Review,
            "Per-version plugins/configs (remove old IDE versions)",
        ),
        (
            "VS Code extensions",
            ".vscode/extensions",
            ToolchainClass::Preserve,
            "Installed VS Code extensions",
        ),
        (
            "VS Code Insiders ext",
            ".vscode-insiders/extensions",
            ToolchainClass::Preserve,
            "Installed VS Code Insiders extensions",
        ),
        (
            "CoreSimulator",
            "Library/Developer/CoreSimulator",
            ToolchainClass::Review,
            "`xcrun simctl delete unavailable`",
        ),
        (
            "Xcode DerivedData",
            "Library/Developer/Xcode/DerivedData",
            ToolchainClass::SafeCache,
            "Regenerable Xcode build intermediates",
        ),
        (
            "Xcode Archives",
            "Library/Developer/Xcode/Archives",
            ToolchainClass::Preserve,
            "Signed app archives (.xcarchive)",
        ),
        (
            "Android SDK/AVDs",
            ".android",
            ToolchainClass::Review,
            "Check `.android/avd` vs `.android/cache`",
        ),
        (
            "Miniconda pkgs",
            "miniconda3/pkgs",
            ToolchainClass::Review,
            "`conda clean --all`",
        ),
        (
            "Miniforge pkgs",
            "miniforge3/pkgs",
            ToolchainClass::Review,
            "`conda clean --all`",
        ),
        (
            "Anaconda pkgs",
            "anaconda3/pkgs",
            ToolchainClass::Review,
            "`conda clean --all`",
        ),
    ];

    for (tool, rel, class, hint) in home_specs {
        push_entry(tool, &home.join(rel), *class, hint, &mut seen, &mut entries);
    }

    // System-prefix Homebrew/Linuxbrew trees (only inspected if present).
    let sys_specs: &[(&'static str, &str, ToolchainClass, &'static str)] = &[
        (
            "Homebrew Cellar",
            "/opt/homebrew/Cellar",
            ToolchainClass::Preserve,
            "`brew list` / `brew autoremove`",
        ),
        (
            "Homebrew Caskroom",
            "/opt/homebrew/Caskroom",
            ToolchainClass::Preserve,
            "`brew list --cask`",
        ),
        (
            "Homebrew tmp",
            "/opt/homebrew/var/homebrew/tmp",
            ToolchainClass::SafeCache,
            "Stale Homebrew download/build staging",
        ),
        (
            "Homebrew var/log",
            "/opt/homebrew/var/log",
            ToolchainClass::SafeCache,
            "Homebrew service log files",
        ),
        (
            "Linuxbrew tmp",
            "/home/linuxbrew/.linuxbrew/var/homebrew/tmp",
            ToolchainClass::SafeCache,
            "Stale Linuxbrew download/build staging",
        ),
    ];
    for (tool, abs, class, hint) in sys_specs {
        push_entry(tool, Path::new(abs), *class, hint, &mut seen, &mut entries);
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
    let mut preserve_disk_bytes = 0u64;
    for entry in &entries {
        total_disk_bytes = total_disk_bytes.saturating_add(entry.disk_bytes);
        match entry.classification {
            ToolchainClass::SafeCache => {
                safe_cache_disk_bytes = safe_cache_disk_bytes.saturating_add(entry.disk_bytes);
            }
            ToolchainClass::Review => {
                review_disk_bytes = review_disk_bytes.saturating_add(entry.disk_bytes);
            }
            ToolchainClass::Preserve => {
                preserve_disk_bytes = preserve_disk_bytes.saturating_add(entry.disk_bytes);
            }
        }
    }

    DevToolchainsReport {
        total_disk_bytes,
        safe_cache_disk_bytes,
        review_disk_bytes,
        preserve_disk_bytes,
        entries,
    }
}

fn push_entry(
    tool: &'static str,
    path: &Path,
    class: ToolchainClass,
    reclaim_hint: &'static str,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<ToolchainEntry>,
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
    entries.push(ToolchainEntry {
        tool,
        path: path.to_path_buf(),
        classification: class,
        disk_bytes: stats.disk_bytes,
        apparent_bytes: stats.apparent_bytes,
        files: stats.files,
        reclaim_hint,
    });
}
