//! Guided read-only inventory for AI coding agents and desktop AI assistants.

use crate::scenario_common::{dev_project_roots, measure_path, user_home};
use serde::Serialize;
use space_core::format_bytes;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    SafeClean,
    Review,
    Preserve,
}

impl Classification {
    fn tag(self) -> &'static str {
        match self {
            Self::SafeClean => "safe-clean",
            Self::Review => "review",
            Self::Preserve => "preserve",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AiEntry {
    pub label: String,
    pub path: PathBuf,
    pub classification: Classification,
    pub disk_bytes: u64,
    pub apparent_bytes: u64,
    pub files: u64,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct AiBloatReport {
    pub total_disk_bytes: u64,
    pub safe_clean_disk_bytes: u64,
    pub review_disk_bytes: u64,
    pub preserve_disk_bytes: u64,
    pub entries: Vec<AiEntry>,
}

struct CandidateSpec {
    label: &'static str,
    rel: &'static str,
    classification: Classification,
    note: &'static str,
}

const HOME_CANDIDATES: &[CandidateSpec] = &[
    CandidateSpec {
        label: "Codex temp",
        rel: ".codex/tmp",
        classification: Classification::SafeClean,
        note: "Scratch temp files; auth and sessions preserved",
    },
    CandidateSpec {
        label: "Codex .tmp",
        rel: ".codex/.tmp",
        classification: Classification::SafeClean,
        note: "Scratch temp files",
    },
    CandidateSpec {
        label: "Codex cache",
        rel: ".codex/cache",
        classification: Classification::SafeClean,
        note: "Regenerable Codex cache",
    },
    CandidateSpec {
        label: "Codex runtimes cache",
        rel: ".cache/codex-runtimes",
        classification: Classification::SafeClean,
        note: "Downloaded Codex runtime binaries",
    },
    CandidateSpec {
        label: "Codex worktrees",
        rel: ".codex/worktrees",
        classification: Classification::Review,
        note: "Detached git worktrees; verify no unmerged edits remain",
    },
    CandidateSpec {
        label: "Codex archived sessions",
        rel: ".codex/archived_sessions",
        classification: Classification::Review,
        note: "Archived Codex conversation logs",
    },
    CandidateSpec {
        label: "Codex visualizations",
        rel: ".codex/visualizations",
        classification: Classification::Review,
        note: "Generated visualization outputs",
    },
    CandidateSpec {
        label: "Codex artifacts",
        rel: ".codex/artifacts",
        classification: Classification::Review,
        note: "Agent artifact snapshots",
    },
    CandidateSpec {
        label: "Codex generated images",
        rel: ".codex/generated_images",
        classification: Classification::Review,
        note: "Generated image files",
    },
    CandidateSpec {
        label: "Codex computer-use",
        rel: ".codex/computer-use",
        classification: Classification::Review,
        note: "Computer-use scratch data",
    },
    CandidateSpec {
        label: "Codex live sessions",
        rel: ".codex/sessions",
        classification: Classification::Preserve,
        note: "Active session transcripts (protected)",
    },
    CandidateSpec {
        label: "Codex Desktop Sparkle/Caches",
        rel: "Library/Caches/com.openai.codex",
        classification: Classification::SafeClean,
        note: "App HTTP and Sparkle updater cache",
    },
    CandidateSpec {
        label: "Codex Desktop Caches",
        rel: "Library/Caches/Codex",
        classification: Classification::SafeClean,
        note: "Electron HTTP/Code cache",
    },
    CandidateSpec {
        label: "Codex Desktop Support Cache",
        rel: "Library/Application Support/Codex/Cache",
        classification: Classification::SafeClean,
        note: "Electron disk cache",
    },
    CandidateSpec {
        label: "Codex Desktop Code Cache",
        rel: "Library/Application Support/Codex/Code Cache",
        classification: Classification::SafeClean,
        note: "Electron JS/Wasm bytecode cache",
    },
    CandidateSpec {
        label: "Codex Desktop GPUCache",
        rel: "Library/Application Support/Codex/GPUCache",
        classification: Classification::SafeClean,
        note: "Electron GPU shader cache",
    },
    CandidateSpec {
        label: "Codex Desktop artifact-sessions",
        rel: "Library/Application Support/Codex/artifact-sessions",
        classification: Classification::Review,
        note: "Desktop artifact sessions",
    },
    CandidateSpec {
        label: "Claude CLI cache",
        rel: ".claude/cache",
        classification: Classification::SafeClean,
        note: "Claude CLI cache",
    },
    CandidateSpec {
        label: "Claude CLI paste-cache",
        rel: ".claude/paste-cache",
        classification: Classification::SafeClean,
        note: "Claude CLI paste buffer cache",
    },
    CandidateSpec {
        label: "Claude CLI shell-snapshots",
        rel: ".claude/shell-snapshots",
        classification: Classification::SafeClean,
        note: "Shell environment snapshots",
    },
    CandidateSpec {
        label: "Claude CLI projects",
        rel: ".claude/projects",
        classification: Classification::Preserve,
        note: "Project transcripts and history (protected)",
    },
    CandidateSpec {
        label: "Claude MCP NodeJS logs",
        rel: "Library/Caches/claude-cli-nodejs",
        classification: Classification::SafeClean,
        note: "MCP log cache",
    },
    CandidateSpec {
        label: "Claude Desktop Cache",
        rel: "Library/Application Support/Claude/Cache",
        classification: Classification::SafeClean,
        note: "Electron HTTP cache",
    },
    CandidateSpec {
        label: "Claude Desktop Code Cache",
        rel: "Library/Application Support/Claude/Code Cache",
        classification: Classification::SafeClean,
        note: "Electron JS/Wasm cache",
    },
    CandidateSpec {
        label: "Claude Desktop VM bundles",
        rel: "Library/Application Support/Claude/vm_bundles",
        classification: Classification::Review,
        note: "Claude local VM bundle images (re-downloaded if deleted)",
    },
    CandidateSpec {
        label: "Claude Desktop Code VM",
        rel: "Library/Application Support/Claude/claude-code-vm",
        classification: Classification::Review,
        note: "Claude Code VM image",
    },
    CandidateSpec {
        label: "Claude Desktop Partitions",
        rel: "Library/Application Support/Claude/Partitions",
        classification: Classification::Review,
        note: "Partition storage (cache subdirs are safe-clean)",
    },
    CandidateSpec {
        label: "Grok CLI worktrees",
        rel: ".grok/worktrees",
        classification: Classification::Review,
        note: "Grok git worktrees; verify unmerged work first",
    },
    CandidateSpec {
        label: "Grok CLI downloads",
        rel: ".grok/downloads",
        classification: Classification::SafeClean,
        note: "Grok CLI downloads cache",
    },
    CandidateSpec {
        label: "Grok CLI sessions",
        rel: ".grok/sessions",
        classification: Classification::Preserve,
        note: "Active Grok sessions (protected)",
    },
    CandidateSpec {
        label: "Qoder logs",
        rel: ".qoder/logs",
        classification: Classification::SafeClean,
        note: "Qoder CLI logs",
    },
    CandidateSpec {
        label: "Qoder tmp",
        rel: ".qoder/tmp",
        classification: Classification::SafeClean,
        note: "Qoder temporary files",
    },
    CandidateSpec {
        label: "Qoder projects",
        rel: ".qoder/projects",
        classification: Classification::Preserve,
        note: "Qoder session transcripts",
    },
    CandidateSpec {
        label: "Qoder Desktop Support",
        rel: "Library/Application Support/com.qoder.app.stable",
        classification: Classification::Review,
        note: "Contains main.sqlite alongside Cache/logs/updates",
    },
    CandidateSpec {
        label: "Cursor Cache",
        rel: "Library/Application Support/Cursor/Cache",
        classification: Classification::SafeClean,
        note: "Electron HTTP cache",
    },
    CandidateSpec {
        label: "Cursor CachedData",
        rel: "Library/Application Support/Cursor/CachedData",
        classification: Classification::SafeClean,
        note: "V8 code cache",
    },
    CandidateSpec {
        label: "Cursor snapshots",
        rel: "Library/Application Support/Cursor/snapshots",
        classification: Classification::SafeClean,
        note: "Workspace recovery snapshots",
    },
    CandidateSpec {
        label: "Cursor User data",
        rel: "Library/Application Support/Cursor/User",
        classification: Classification::Preserve,
        note: "Settings, extensions state, workspaceStorage",
    },
    CandidateSpec {
        label: "Antigravity Desktop Cache",
        rel: "Library/Application Support/Antigravity/Cache",
        classification: Classification::SafeClean,
        note: "Electron HTTP cache",
    },
    CandidateSpec {
        label: "Antigravity Desktop Code Cache",
        rel: "Library/Application Support/Antigravity/Code Cache",
        classification: Classification::SafeClean,
        note: "Electron code cache",
    },
    CandidateSpec {
        label: "Antigravity ShipIt/OS cache",
        rel: "Library/Caches/com.google.antigravity",
        classification: Classification::SafeClean,
        note: "macOS app cache",
    },
    CandidateSpec {
        label: "Antigravity brain & state",
        rel: ".gemini/antigravity",
        classification: Classification::Preserve,
        note: "Conversations, artifacts, credentials (protected)",
    },
    CandidateSpec {
        label: "Kimi Desktop Cache",
        rel: "Library/Application Support/kimi-desktop/Cache",
        classification: Classification::SafeClean,
        note: "Electron cache",
    },
    CandidateSpec {
        label: "Kimi Desktop Code Cache",
        rel: "Library/Application Support/kimi-desktop/Code Cache",
        classification: Classification::SafeClean,
        note: "Electron code cache",
    },
    CandidateSpec {
        label: "Kimi Desktop ShipIt",
        rel: "Library/Caches/com.moonshot.kimichat.ShipIt",
        classification: Classification::SafeClean,
        note: "Squirrel updater staging",
    },
    CandidateSpec {
        label: "Kimi Desktop updater",
        rel: "Library/Caches/kimi-desktop-updater",
        classification: Classification::SafeClean,
        note: "Updater download cache",
    },
    CandidateSpec {
        label: "Kimi Desktop daimon-share",
        rel: "Library/Application Support/kimi-desktop/daimon-share",
        classification: Classification::Review,
        note: "Bundled runtime/data share",
    },
    CandidateSpec {
        label: "Kimi Code CLI",
        rel: ".kimi-code",
        classification: Classification::Review,
        note: "Kimi Code CLI local state",
    },
    CandidateSpec {
        label: "Perplexity PythonMLX",
        rel: "Library/Application Support/ai.perplexity.macv3/PythonMLX",
        classification: Classification::Review,
        note: "Local Python/MLX runtime cache (re-downloaded on demand)",
    },
    CandidateSpec {
        label: "Ollama local store",
        rel: ".ollama",
        classification: Classification::Review,
        note: "Local Ollama weights and manifests",
    },
];

pub fn run(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let home = user_home()?;
    let report = collect_report(&home);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "AI agents & assistants: {} total ({} safe-clean · {} review · {} preserve)",
        format_bytes(report.total_disk_bytes),
        format_bytes(report.safe_clean_disk_bytes),
        format_bytes(report.review_disk_bytes),
        format_bytes(report.preserve_disk_bytes),
    );
    if report.entries.is_empty() {
        println!("No AI agent bloat directories found.");
        return Ok(());
    }
    println!("\nInventory (sorted by allocated disk bytes):");
    for entry in &report.entries {
        let sparse = if entry.apparent_bytes > entry.disk_bytes.saturating_mul(2)
            && entry.apparent_bytes > 64 * 1024 * 1024
        {
            format!(" (apparent {})", format_bytes(entry.apparent_bytes))
        } else {
            String::new()
        };
        println!(
            "[{:>10}] {:>10}{}  {}  — {}",
            entry.classification.tag(),
            format_bytes(entry.disk_bytes),
            sparse,
            entry.path.display(),
            entry.note
        );
    }
    println!("\nRead-only diagnostic scenario: nothing was modified or deleted.");
    println!("Use `space junk -c ai-agents` (dry-run) or Quick Clean to review/trash safe and review items.");
    Ok(())
}

pub fn collect_report(home: &Path) -> AiBloatReport {
    let mut seen_paths = HashSet::new();
    let mut entries = Vec::new();

    for spec in HOME_CANDIDATES {
        let path = home.join(spec.rel);
        record_entry(
            &path,
            spec.label.to_string(),
            spec.classification,
            spec.note.to_string(),
            &mut seen_paths,
            &mut entries,
        );
    }

    for root in dev_project_roots(home) {
        scan_repo_claude_dirs(&root, 0, &mut seen_paths, &mut entries);
    }

    entries.sort_by(|a, b| {
        b.disk_bytes
            .cmp(&a.disk_bytes)
            .then_with(|| b.apparent_bytes.cmp(&a.apparent_bytes))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut total_disk_bytes = 0u64;
    let mut safe_clean_disk_bytes = 0u64;
    let mut review_disk_bytes = 0u64;
    let mut preserve_disk_bytes = 0u64;
    for entry in &entries {
        total_disk_bytes = total_disk_bytes.saturating_add(entry.disk_bytes);
        match entry.classification {
            Classification::SafeClean => {
                safe_clean_disk_bytes = safe_clean_disk_bytes.saturating_add(entry.disk_bytes);
            }
            Classification::Review => {
                review_disk_bytes = review_disk_bytes.saturating_add(entry.disk_bytes);
            }
            Classification::Preserve => {
                preserve_disk_bytes = preserve_disk_bytes.saturating_add(entry.disk_bytes);
            }
        }
    }

    AiBloatReport {
        total_disk_bytes,
        safe_clean_disk_bytes,
        review_disk_bytes,
        preserve_disk_bytes,
        entries,
    }
}

fn scan_repo_claude_dirs(
    dir: &Path,
    depth: usize,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<AiEntry>,
) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(name_str.as_ref(), ".git" | "node_modules" | "target") {
            continue;
        }
        let path = entry.path();
        if name_str == ".claude" {
            collect_claude_subtrees(&path, seen, entries);
            continue;
        }
        if depth < 2 && !name_str.starts_with('.') {
            scan_repo_claude_dirs(&path, depth + 1, seen, entries);
        }
    }
}

fn collect_claude_subtrees(
    claude_dir: &Path,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<AiEntry>,
) {
    let Ok(read) = fs::read_dir(claude_dir) else {
        return;
    };
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let path = entry.path();
        if name_str == "target" || name_str.starts_with("target-") || name_str == "cache" {
            record_entry(
                &path,
                "Project .claude shared build target/cache".to_string(),
                Classification::SafeClean,
                "Regenerable build output inside repo .claude/".to_string(),
                seen,
                entries,
            );
        } else if name_str == "worktrees" {
            record_entry(
                &path,
                "Project .claude worktrees".to_string(),
                Classification::Review,
                "Agent git worktrees; confirm branches are merged before deleting".to_string(),
                seen,
                entries,
            );
        }
    }
}

fn record_entry(
    path: &Path,
    label: String,
    classification: Classification,
    note: String,
    seen: &mut HashSet<PathBuf>,
    entries: &mut Vec<AiEntry>,
) {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canon) {
        return;
    }
    let Some(stats) = measure_path(path) else {
        return;
    };
    if stats.disk_bytes == 0 && stats.apparent_bytes == 0 {
        return;
    }
    entries.push(AiEntry {
        label,
        path: path.to_path_buf(),
        classification,
        disk_bytes: stats.disk_bytes,
        apparent_bytes: stats.apparent_bytes,
        files: stats.files,
        note,
    });
}
