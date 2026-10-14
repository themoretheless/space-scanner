//! Rule tags: explicit YAML tags + inferred labels for filtering / bulk prefs.

use crate::rules::{Platform, Risk, Rule};
use std::collections::{BTreeSet, HashSet};

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

/// Tags written in YAML, normalized: everything in `tags` that was not inferred.
pub fn explicit_tags(rule: &Rule) -> Vec<String> {
    let inferred: HashSet<&str> = rule.inferred_tags.iter().map(String::as_str).collect();
    rule.tags
        .iter()
        .map(|t| normalize_tag(t))
        .filter(|t| !t.is_empty() && !inferred.contains(t.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Merge explicit + inferred tags (sorted, unique).
pub fn effective_tags(rule: &Rule) -> Vec<String> {
    let mut set: BTreeSet<String> = explicit_tags(rule).into_iter().collect();
    set.extend(infer_tags(rule));
    set.into_iter().collect()
}

/// Lower-case alphanumeric tokens of a text, in order.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Whole-token match: a needle such as `edge` must not fire on `knowledge`,
/// and `amd` must not fire on `amdgpu`. Multi-token needles (`node_modules`,
/// `places.sqlite`, `typed url`) match as a contiguous token sequence. A plain
/// plural (`cookie` / `cookies`) is treated as the same token.
fn contains_tokens(hay: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len())
        .any(|window| window.iter().zip(needle).all(|(h, n)| same_token(h, n)))
}

fn same_token(a: &str, b: &str) -> bool {
    a == b
        || a.strip_suffix('s').is_some_and(|stem| stem == b)
        || b.strip_suffix('s').is_some_and(|stem| stem == a)
}

struct Keyword {
    tokens: Vec<String>,
    /// `tokens[0] + "s"` for single-token needles.
    plural: String,
    /// `tokens[0]` without a trailing `s`, when it has one.
    singular: Option<String>,
    tag: &'static str,
}

const KEYWORDS: &[(&str, &str)] = &[
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
    ("tmp", "temp"),
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

/// Keyword needles tokenized once per process, not once per rule.
fn keyword_table() -> &'static [Keyword] {
    static TABLE: std::sync::OnceLock<Vec<Keyword>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        KEYWORDS
            .iter()
            .map(|(needle, tag)| {
                let tokens = tokens(needle);
                let first = tokens.first().cloned().unwrap_or_default();
                Keyword {
                    plural: format!("{first}s"),
                    singular: first.strip_suffix('s').map(str::to_string),
                    tokens,
                    tag,
                }
            })
            .collect()
    })
}

/// Infer tags from id / category / paths / risk / platform.
pub fn infer_tags(rule: &Rule) -> Vec<String> {
    let mut set = BTreeSet::new();
    let id = rule.id.to_ascii_lowercase();
    let paths = rule.paths.join("\n").to_ascii_lowercase();
    let hay_text = format!(
        "{id}\n{}\n{}\n{}\n{paths}",
        rule.name, rule.category, rule.description
    );
    let hay = tokens(&hay_text);
    let hay_set: HashSet<&str> = hay.iter().map(String::as_str).collect();

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

    if rule.is_report_only() {
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
    for keyword in keyword_table() {
        let hit = match keyword.tokens.as_slice() {
            [single] => {
                hay_set.contains(single.as_str())
                    || hay_set.contains(keyword.plural.as_str())
                    || keyword
                        .singular
                        .as_deref()
                        .is_some_and(|stem| hay_set.contains(stem))
            }
            sequence => contains_tokens(&hay, sequence),
        };
        if hit {
            set.insert(keyword.tag.into());
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
    if paths.contains("node_modules") || hay_text.contains("**/") {
        set.insert("project".into());
    }

    set.into_iter().collect()
}

/// Fill `rule.inferred_tags` and merge them into `rule.tags` (idempotent:
/// re-running on an enriched rule re-infers from the explicit tags only).
pub fn enrich_rule_tags(rule: &mut Rule) {
    let explicit = explicit_tags(rule);
    let inferred = infer_tags(rule);
    let mut effective: BTreeSet<String> = explicit.into_iter().collect();
    effective.extend(inferred.iter().cloned());
    rule.inferred_tags = inferred;
    rule.tags = effective.into_iter().collect();
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
            inferred_tags: vec![],
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
        assert!(tags.iter().any(|t| t == "electron"));
    }

    #[test]
    fn keyword_tags_match_whole_tokens_only() {
        let mut rule = sample();
        rule.id = "knowledge-base-amdgpu-cursorless".into();
        rule.name = "Knowledge".into();
        rule.description = "Hedge fund notes".into();
        rule.paths = vec!["$HOME/.local/share/knowledgebase".into()];
        let tags = infer_tags(&rule);
        assert!(!tags.iter().any(|t| t == "edge"), "{tags:?}");
        assert!(!tags.iter().any(|t| t == "gpu"), "{tags:?}");
        assert!(!tags.iter().any(|t| t == "editor"), "{tags:?}");
        rule.description = "Microsoft Edge profile; AMD shader cache; places.sqlite".into();
        let tags = infer_tags(&rule);
        assert!(tags.iter().any(|t| t == "edge"));
        assert!(tags.iter().any(|t| t == "gpu"));
        assert!(tags.iter().any(|t| t == "history"));
        assert!(tags.iter().any(|t| t == "shader"));
        rule.description = "Cookies".into();
        assert!(infer_tags(&rule).iter().any(|t| t == "cookies"));
    }

    #[test]
    fn enrichment_keeps_inferred_tags_separate_and_is_idempotent() {
        let mut rule = sample();
        rule.tags = vec!["Custom Tag".into(), "discord".into()];
        enrich_rule_tags(&mut rule);
        assert!(rule.tags.iter().any(|t| t == "custom-tag"));
        assert!(rule.tags.iter().any(|t| t == "discord"));
        assert!(rule.inferred_tags.iter().any(|t| t == "discord"));
        assert!(!rule.inferred_tags.iter().any(|t| t == "custom-tag"));
        assert_eq!(explicit_tags(&rule), vec!["custom-tag".to_string()]);
        let once = rule.clone();
        enrich_rule_tags(&mut rule);
        assert_eq!(rule.tags, once.tags);
        assert_eq!(rule.inferred_tags, once.inferred_tags);
        // Changing the rule re-infers instead of keeping a stale inference.
        rule.platform = Platform::Linux;
        enrich_rule_tags(&mut rule);
        assert!(!rule.tags.iter().any(|t| t == "macos"));
        assert!(rule.tags.iter().any(|t| t == "linux"));
        assert_eq!(explicit_tags(&rule), vec!["custom-tag".to_string()]);
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
