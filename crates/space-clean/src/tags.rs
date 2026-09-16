//! Rule tags: explicit YAML tags + inferred labels for filtering / bulk prefs.

use crate::rules::{Platform, Risk, Rule};
use std::collections::BTreeSet;

/// Normalize a tag to kebab-case lowercase.
pub fn normalize_tag(raw: &str) -> String {
    let s = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Merge explicit + inferred tags (sorted, unique).
pub fn effective_tags(rule: &Rule) -> Vec<String> {
    let mut set = BTreeSet::new();
    for t in &rule.tags {
        let n = normalize_tag(t);
        if !n.is_empty() {
            set.insert(n);
        }
    }
    for t in infer_tags(rule) {
        set.insert(t);
    }
    set.into_iter().collect()
}

/// Infer tags from id / category / paths / risk / platform.
pub fn infer_tags(rule: &Rule) -> Vec<String> {
    let mut set = BTreeSet::new();
    let id = rule.id.to_ascii_lowercase();
    let name = rule.name.to_ascii_lowercase();
    let cat = rule.category.to_ascii_lowercase();
    let desc = rule.description.to_ascii_lowercase();
    let paths = rule.paths.join("\n").to_ascii_lowercase();
    let hay = format!("{id}\n{name}\n{cat}\n{desc}\n{paths}");

    // Category as tag
    let cat_tag = normalize_tag(&rule.category);
    if !cat_tag.is_empty() {
        set.insert(cat_tag);
    }

    // Platform
    match rule.platform {
        Platform::Macos => {
            set.insert("macos".into());
        }
        Platform::Linux => {
            set.insert("linux".into());
        }
        Platform::Windows => {
            set.insert("windows".into());
        }
        Platform::Any => {
            set.insert("cross-platform".into());
        }
    }

    if rule.report_only || rule.risk == Risk::ReportOnly {
        set.insert("report".into());
    }
    match rule.risk {
        Risk::Low => set.insert("risk-low".into()),
        Risk::Medium => set.insert("risk-medium".into()),
        Risk::High => set.insert("risk-high".into()),
        Risk::ReportOnly => set.insert("risk-report".into()),
    };
    set.insert(format!(
        "danger-{}",
        rule.risk.danger_label().replace(' ', "-")
    ));
    set.insert(format!("kind-{}", rule.kind.as_str()));
    set.insert(rule.kind.as_str().into());

    if rule.default_enabled {
        set.insert("default-on".into());
    } else {
        set.insert("default-off".into());
    }

    if id.starts_with("x10k-") || id.starts_with("gen-") || id.starts_with("gen-pad") {
        set.insert("generated".into());
    } else {
        set.insert("curated".into());
    }

    // Family / stack tags from keywords
    let keywords: &[(&str, &str)] = &[
        ("browser", "browser"),
        ("chrome", "chrome"),
        ("chromium", "chrome"),
        ("firefox", "firefox"),
        ("safari", "safari"),
        ("edge", "edge"),
        ("brave", "brave"),
        ("opera", "opera"),
        ("electron", "electron"),
        ("discord", "discord"),
        ("slack", "slack"),
        ("spotify", "spotify"),
        ("steam", "steam"),
        ("docker", "docker"),
        ("podman", "containers"),
        ("npm", "npm"),
        ("yarn", "yarn"),
        ("pnpm", "pnpm"),
        ("pip", "python"),
        ("python", "python"),
        ("cargo", "rust"),
        ("rustup", "rust"),
        ("gradle", "java"),
        ("maven", "java"),
        ("nuget", "dotnet"),
        ("node_modules", "nodejs"),
        ("vscode", "editor"),
        ("cursor", "editor"),
        ("jetbrains", "ide"),
        ("xcode", "xcode"),
        ("android", "android"),
        ("huggingface", "ml"),
        ("torch", "ml"),
        ("ollama", "ml"),
        ("tensorflow", "ml"),
        ("game", "gaming"),
        ("shader", "shader"),
        ("temp", "temp"),
        ("tmpdir", "temp"),
        ("log", "logs"),
        ("cache", "cache"),
        ("trash", "trash"),
        ("homebrew", "homebrew"),
        ("apt", "distro"),
        ("pacman", "distro"),
        ("dnf", "distro"),
        ("flatpak", "flatpak"),
        ("snap", "snap"),
        ("winget", "winget"),
        ("chocolatey", "chocolatey"),
        ("telegram", "messenger"),
        ("whatsapp", "messenger"),
        ("zoom", "meetings"),
        ("teams", "meetings"),
        ("adobe", "adobe"),
        ("nvidia", "gpu"),
        ("amd", "gpu"),
        ("project", "project"),
        ("deriveddata", "xcode"),
        ("cocoapods", "ios"),
        ("cookie", "cookies"),
        ("cookies", "cookies"),
        ("places.sqlite", "history"),
        ("browsing-history", "history"),
        ("typed url", "history"),
    ];

    for (needle, tag) in keywords {
        if hay.contains(needle) {
            set.insert((*tag).into());
        }
    }

    // Path-shape tags
    if paths.contains("library/") {
        set.insert("library".into());
    }
    if paths.contains("%appdata%") || paths.contains("%localappdata%") {
        set.insert("appdata".into());
    }
    if paths.contains(".config/") || paths.contains("$xdg_") {
        set.insert("xdg".into());
    }
    if paths.contains("node_modules") || hay.contains("**/") {
        set.insert("project".into());
    }

    set.into_iter().collect()
}

/// Ensure `rule.tags` contains effective tags (mutates in place after load).
pub fn enrich_rule_tags(rule: &mut Rule) {
    rule.tags = effective_tags(rule);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{DeleteMode, JunkKind, Platform, Risk};

    fn sample() -> Rule {
        Rule {
            id: "app-discord-cache-macos".into(),
            name: "Discord cache".into(),
            description: "Electron Discord".into(),
            category: "app-cache".into(),
            platform: Platform::Macos,
            risk: Risk::Low,
            kind: JunkKind::Cache,
            default_enabled: false,
            min_age_days: 7,
            paths: vec!["$HOME/Library/Application Support/discord/Cache".into()],
            include_globs: vec![],
            exclude_globs: vec![],
            required_ancestor_globs: vec![],
            match_directories: false,
            max_depth: 6,
            delete_mode: DeleteMode::Contents,
            report_only: false,
            tags: vec![],
        }
    }

    #[test]
    fn infers_browserish_app_tags() {
        let tags = infer_tags(&sample());
        assert!(tags.iter().any(|t| t == "macos"));
        assert!(tags.iter().any(|t| t == "discord"));
        assert!(tags.iter().any(|t| t == "app-cache"));
        assert!(tags.iter().any(|t| t == "curated"));
        assert!(tags.iter().any(|t| t == "kind-cache"));
        assert!(tags.iter().any(|t| t == "danger-safe"));
    }

    #[test]
    fn infers_logs_kind_from_category() {
        let mut rule = sample();
        rule.category = "logs".into();
        rule.kind = JunkKind::Other;
        crate::rules::enrich_rule_kind(&mut rule);
        assert_eq!(rule.kind, JunkKind::Logs);
    }
}
