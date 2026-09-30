//! Repository maintenance without interpreter dependencies.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use space_clean::{enrich_rule_kind, enrich_rule_tags, Rule};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        let p = entry.path();
        if entry.file_type().is_file()
            && matches!(p.extension().and_then(|s| s.to_str()), Some("yaml" | "yml"))
            && !p.file_name().unwrap().to_string_lossy().starts_with('_')
        {
            paths.push(p.to_owned());
        }
    }
    paths.sort();
    Ok(paths)
}
fn read(path: &Path) -> Result<Value> {
    serde_yaml::from_str(&fs::read_to_string(path)?).with_context(|| path.display().to_string())
}
fn write(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_yaml::to_string(value)?)?;
    Ok(())
}
fn duplicates(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut seen = HashMap::new();
    let mut ids = HashSet::new();
    let mut aliases = BTreeMap::new();
    for path in files(root)? {
        let doc = read(&path)?;
        for v in doc["rules"].as_array().context("missing rules array")? {
            let mut rule: Rule = serde_json::from_value(v.clone())?;
            rule.id = rule.id.trim().to_owned();
            ensure!(
                ids.insert(rule.id.clone()),
                "repeated source ID: {}",
                rule.id
            );
            rule.name = rule.name.trim().to_owned();
            rule.category = rule.category.trim().to_owned();
            enrich_rule_kind(&mut rule);
            enrich_rule_tags(&mut rule);
            for list in [
                &mut rule.paths,
                &mut rule.include_globs,
                &mut rule.exclude_globs,
                &mut rule.required_ancestor_globs,
            ] {
                list.sort();
                list.dedup();
            }
            let mut value = serde_json::to_value(&rule)?;
            for key in ["id", "name", "description"] {
                value.as_object_mut().unwrap().remove(key);
            }
            let hash = blake3::hash(&serde_json::to_vec(&value)?);
            if let Some(id) = seen.get(&hash) {
                aliases.insert(rule.id, String::clone(id));
            } else {
                seen.insert(hash, rule.id);
            }
        }
    }
    Ok(aliases)
}
fn deduplicate(root: &Path, apply: bool) -> Result<()> {
    let aliases = duplicates(root)?;
    println!("Duplicate rules: {}", aliases.len());
    if !apply {
        ensure!(aliases.is_empty(), "duplicate rules found");
        return Ok(());
    }
    let mut edits = Vec::new();
    let mut removed = HashSet::new();
    let mut retained = HashSet::new();
    for path in files(root)? {
        let text = fs::read_to_string(&path)?;
        let mut starts = Vec::new();
        let mut offset = 0;
        for raw_line in text.split_inclusive('\n') {
            let line = raw_line.trim_start();
            if line.starts_with("- id: ") || line.starts_with("- name: ") {
                starts.push(offset);
            }
            offset += raw_line.len();
        }
        let mut output = text[..starts.first().copied().unwrap_or(text.len())].to_owned();
        let mut changed = false;
        for (i, start) in starts.iter().enumerate() {
            let block = &text[*start..starts.get(i + 1).copied().unwrap_or(text.len())];
            let scalar = block
                .lines()
                .find_map(|line| {
                    line.trim_start()
                        .trim_start_matches("- ")
                        .strip_prefix("id: ")
                })
                .context("missing ID")?;
            let id: String = serde_yaml::from_str(scalar)?;
            let id = id.trim();
            if aliases.contains_key(id) {
                ensure!(removed.insert(id.to_owned()), "repeated ID");
                changed = true;
            } else {
                retained.insert(id.to_owned());
                output.push_str(block);
            }
        }
        if changed {
            if serde_yaml::from_str::<Value>(&output)?["rules"].is_null() {
                output = output
                    .lines()
                    .map(|l| if l.trim() == "rules:" { "rules: []" } else { l })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n";
            }
            let _: Value = serde_yaml::from_str(&output)?;
            // Validate typed rules too, before any replacement.
            let doc: Value = serde_yaml::from_str(&output)?;
            for rule in doc["rules"].as_array().context("missing rules")? {
                let _: Rule = serde_json::from_value(rule.clone())?;
            }
            edits.push((path, output));
        }
    }
    ensure!(
        removed.len() == aliases.len(),
        "unrecognized YAML layout; no files changed"
    );
    ensure!(
        aliases.values().all(|id| retained.contains(id)),
        "missing canonical rule"
    );
    let alias_path = root.join("_rule-aliases.json");
    let mut previous: BTreeMap<String, String> = if alias_path.exists() {
        serde_json::from_str(&fs::read_to_string(&alias_path)?)?
    } else {
        BTreeMap::new()
    };
    for target in previous.values_mut() {
        if let Some(new) = aliases.get(target) {
            *target = new.clone();
        }
    }
    previous.extend(aliases);
    ensure!(
        previous.values().all(|id| retained.contains(id)),
        "stale alias targets; no files changed"
    );
    for (path, output) in &edits {
        fs::write(path, output)?;
    }
    if !previous.is_empty() {
        fs::write(alias_path, serde_json::to_string_pretty(&previous)? + "\n")?;
    }
    println!("Removed {} rules from {} files", removed.len(), edits.len());
    Ok(())
}
fn classify(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if path.contains('%')
        || path.contains('\\')
        || [
            "c:/",
            "program files",
            "localappdata",
            "appdata",
            "userprofile",
            "windir",
            "programdata",
            "systemdrive",
            "$recycle",
        ]
        .iter()
        .any(|s| lower.contains(s))
    {
        "windows"
    } else if [
        "Library/",
        "/Applications",
        "Photos Library",
        ".savedState",
        "HTTPStorages",
        "Containers/",
        "QuickLook",
        "CoreSimulator",
        "Xcode",
        "Mobile Documents",
        "CloudStorage",
    ]
    .iter()
    .any(|s| path.contains(s))
    {
        "macos"
    } else if [
        "$XDG_",
        "/var/",
        "/usr/",
        "/etc/",
        ".linuxbrew",
        ".var/app",
        "flatpak",
        "/tmp",
    ]
    .iter()
    .any(|s| path.contains(s))
    {
        "linux"
    } else {
        "unix"
    }
}
fn split_rule(rule: &Value) -> Result<Vec<Value>> {
    if rule["platform"].as_str().unwrap_or("any") != "any" {
        return Ok(vec![rule.clone()]);
    }
    let paths = rule["paths"].as_array().context("missing paths")?;
    let mut groups = Vec::new();
    for platform in ["macos", "linux", "windows"] {
        let mut selected = Vec::new();
        for path in paths {
            let class = classify(path.as_str().context("invalid path")?);
            if (class == platform || (class == "unix" && platform != "windows"))
                && !selected.contains(path)
            {
                selected.push(path.clone());
            }
        }
        if !selected.is_empty() || (paths.is_empty() && platform != "windows") {
            groups.push((platform, selected));
        }
    }
    let multiple = groups.len() > 1;
    groups
        .into_iter()
        .map(|(platform, paths)| {
            let mut r = rule.clone();
            if multiple {
                r["id"] = json!(format!(
                    "{}-{platform}",
                    rule["id"].as_str().context("missing id")?
                ));
            }
            r["platform"] = json!(platform);
            r["paths"] = json!(paths);
            Ok(r)
        })
        .collect()
}
fn split(root: &Path) -> Result<()> {
    let mut edits = Vec::new();
    for path in files(root)? {
        let mut doc = read(&path)?;
        let rules = doc["rules"].as_array().context("missing rules")?;
        if !rules
            .iter()
            .any(|r| r["platform"].as_str().unwrap_or("any") == "any")
        {
            continue;
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for rule in rules {
            for mut r in split_rule(rule)? {
                if r["paths"].as_array().unwrap().is_empty() {
                    continue;
                }
                let base = r["id"].as_str().context("missing ID")?.to_owned();
                let mut id = base.clone();
                let mut n = 2;
                while !seen.insert(id.clone()) {
                    id = format!("{base}-{n}");
                    n += 1;
                }
                r["id"] = json!(id);
                out.push(r);
            }
        }
        doc["rules"] = json!(out);
        edits.push((path, doc));
    }
    for (path, doc) in &edits {
        write(path, doc)?;
    }
    println!("Split {} packs", edits.len());
    Ok(())
}
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(64).collect()
}
fn catalog(root: &Path, source: &Path, prefix: &str, target: usize) -> Result<()> {
    ensure!(
        target > 0 && !prefix.is_empty() && slug(prefix) == prefix,
        "invalid prefix or target"
    );
    let mut seen = HashSet::new();
    for path in files(root)? {
        let doc = read(&path)?;
        for r in doc["rules"].as_array().context("missing rules")? {
            seen.insert(r["id"].as_str().context("missing id")?.to_owned());
        }
    }
    let mut rules = Vec::new();
    let mut names = HashSet::new();
    for line in fs::read_to_string(source)?.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        if name.len() < 2
            || name.len() > 64
            || !name.starts_with(|c: char| c.is_ascii_alphabetic())
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+._-".contains(c))
        {
            continue;
        }
        let s = slug(name);
        if !names.insert(s.clone()) {
            continue;
        }
        for (short, platform, paths) in [
            (
                "mac",
                "macos",
                vec![
                    format!("$HOME/Library/Caches/{name}"),
                    format!("$HOME/Library/Logs/{name}"),
                ],
            ),
            (
                "lin",
                "linux",
                vec![
                    format!("$HOME/.cache/{name}"),
                    format!("$XDG_CACHE_HOME/{name}"),
                ],
            ),
            (
                "win",
                "windows",
                vec![
                    format!("%LOCALAPPDATA%/{name}/Cache"),
                    format!("%LOCALAPPDATA%/{name}/Logs"),
                ],
            ),
        ] {
            let id = format!("{prefix}-{short}-{s}");
            if !seen.insert(id.clone()) {
                continue;
            }
            rules.push(json!({"id":id,"name":format!("{prefix} {name} {platform} cache"),"description":format!("Catalog cache/log paths for {name}; configurations are preserved. Review matches before enabling."),"category":"caches","platform":platform,"risk":"low","default_enabled":false,"min_age_days":14,"paths":paths,"delete_mode":"contents","max_depth":6}));
            if rules.len() == target {
                break;
            }
        }
        if rules.len() == target {
            break;
        }
    }
    ensure!(
        rules.len() == target,
        "only {} rules available, need {target}; no files changed",
        rules.len()
    );
    let mut outputs = Vec::new();
    for (i, chunk) in rules.chunks(800).enumerate() {
        let path = root.join(format!("generated/{prefix}-{:02}.yaml", i + 1));
        ensure!(!path.exists(), "output exists: {}", path.display());
        outputs.push((
            path,
            json!({"pack":format!("{prefix}-{}",i+1),"rules":chunk}),
        ));
    }
    for (path, doc) in outputs {
        write(&path, &doc)?;
    }
    println!("Generated {target} rules");
    Ok(())
}
fn features(root: &Path) -> Result<()> {
    let items: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(root.join("docs/features_items.json"))?)?;
    let mut out = String::from("# Space Scanner — feature catalog (~1000)\n\nIDs are stable. Priority: `P0` (top-200 candidates), `P1`, `P2`.\n\nStatus in this file is backlog-only; execution tracking is [ROADMAP-200.md](ROADMAP-200.md).\n\n");
    let mut category = String::new();
    let mut count = 0;
    for item in &items {
        let cat = item["cat"].as_str().context("missing category")?;
        if cat != category {
            if count > 0 {
                out.push('\n');
            }
            out.push_str(&format!("## {cat}\n\n"));
            category = cat.to_owned();
            count += 1;
        }
        out.push_str(&format!(
            "- `{}` [{}] {}\n",
            item["id"].as_str().context("missing id")?,
            item["pri"].as_str().context("missing priority")?,
            item["title"].as_str().context("missing title")?
        ));
    }
    out.push_str(&format!(
        "\n---\n\n**Total: {} features** across {count} categories.\n",
        items.len()
    ));
    fs::write(root.join("docs/FEATURES.md"), out)?;
    Ok(())
}
fn main() -> Result<()> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rules = args
        .windows(2)
        .find(|a| a[0] == "--root")
        .map(|a| PathBuf::from(&a[1]))
        .unwrap_or_else(|| repo.join("rules"));
    match cmd {
        "deduplicate" => deduplicate(&rules, args.iter().any(|a| a == "--apply")),
        "split" => split(&rules),
        "catalog" => {
            ensure!(
                args.len() >= 4,
                "catalog <names.txt> <prefix> <target> [--root rules]"
            );
            catalog(&rules, Path::new(&args[1]), &args[2], args[3].parse()?)
        }
        "features" => features(&repo),
        "target-dir" => {
            let output = std::process::Command::new("cargo")
                .args(["metadata", "--format-version", "1", "--no-deps"])
                .current_dir(&repo)
                .output()?;
            ensure!(output.status.success(), "cargo metadata failed");
            let v: Value = serde_json::from_slice(&output.stdout)?;
            println!(
                "{}",
                v["target_directory"]
                    .as_str()
                    .context("missing target directory")?
            );
            Ok(())
        }
        "protections" => protections(&rules),
        "pypi" => {
            if args.get(1).is_some_and(|arg| arg != "--root") {
                ensure!(
                    args.len() >= 5,
                    "pypi <names.txt> <source.json> <prefix> <target> [--root rules]"
                );
                pypi_batch(
                    &rules,
                    Path::new(&args[1]),
                    Path::new(&args[2]),
                    &args[3],
                    args[4].parse()?,
                )
            } else {
                pypi(&repo, &rules)
            }
        }
        "run-app" => launch(&repo),
        "help" | "--help" => {
            println!("cargo run -p space-tools -- <command>\nCommands: deduplicate [--apply] [--root rules], split [--root rules], catalog <names.txt> <prefix> <target> [--root rules], protections [--root rules], features, target-dir, pypi [--root rules], run-app");
            Ok(())
        }
        _ => bail!("unknown command: {cmd}"),
    }
}
fn protections(root: &Path) -> Result<()> {
    let apps = "1password bitwarden keepassxc lastpass authy microsoft-authenticator google-authenticator proton-pass dashlane nordpass keeper enpass yubico electrum exodus metamask ledger-live trezor-suite phantom solflare sparrow-wallet wasabi-wallet monero-wallet bitcoin-core ethereum chrome chromium firefox safari edge brave vivaldi opera tor-browser arc thunderbird outlook proton-mail mailbird spark-mail airmail mailspring signal telegram whatsapp element discord slack teams zoom skype messages facetime contacts calendars reminders notes notion obsidian evernote onenote bear craft todoist things omnifocus logseq roam vscode cursor zed jetbrains xcode android-studio visual-studio github-desktop sourcetree tower fork gitkraken docker-desktop dropbox onedrive google-drive icloud-drive nextcloud syncthing box-drive photos lightroom photoshop affinity-photo capture-one davinci-resolve final-cut-pro logic-pro ableton-live pro-tools premiere-pro blender";
    ensure!(
        apps.split_whitespace().count() == 100,
        "expected 100 app families"
    );
    for (platform, base, suffixes) in [
        (
            "macos",
            "$HOME/Library/Application Support",
            vec!["User Data", "Profiles", "Databases", "Local Storage"],
        ),
        (
            "windows",
            "%APPDATA%",
            vec!["User Data", "Profiles", "Databases"],
        ),
        (
            "linux",
            "$HOME/.config",
            vec!["User Data", "Profiles", "databases"],
        ),
    ] {
        let mut guards = Vec::new();
        for app in apps.split_whitespace() {
            for suffix in &suffixes {
                guards.push(json!({"id":format!("protect-{platform}-{}-{}",slug(app),slug(suffix)),"name":format!("Protect {app} {suffix}"),"description":"Hard safety boundary for credentials, profiles, settings, or user-created data.","platform":platform,"paths":[format!("{base}/{app}/{suffix}")],"tags":["never-delete",slug(app),slug(suffix)]}));
            }
        }
        write(
            &root.join(format!("protected/_protected-{platform}.yaml")),
            &json!({"pack":format!("protected-{platform}"),"description":"Generated hard deletion guards; do not load as cleanup rules.","protections":guards}),
        )?;
    }
    println!("Generated 1000 protections");
    Ok(())
}
fn pypi(repo: &Path, root: &Path) -> Result<()> {
    pypi_batch(
        root,
        &repo.join("scripts/catalogs/pypi-100k-names.txt"),
        &repo.join("scripts/catalogs/pypi-100k-source.json"),
        "pypi100k",
        100000,
    )
}
fn pypi_batch(
    root: &Path,
    names_path: &Path,
    source_path: &Path,
    prefix: &str,
    target: usize,
) -> Result<()> {
    ensure!(
        target > 0 && !prefix.is_empty() && slug(prefix) == prefix,
        "invalid prefix or target"
    );
    let names = fs::read_to_string(names_path)?;
    let names: Vec<_> = names.lines().collect();
    ensure!(
        names.len() == target.div_ceil(3)
            && names.iter().collect::<HashSet<_>>().len() == names.len(),
        "expected {} unique names",
        target.div_ceil(3)
    );
    let source: Value = serde_json::from_str(&fs::read_to_string(source_path)?)?;
    let date = source["retrieved"]
        .as_str()
        .context("missing source date")?;
    let url = source["source"].as_str().context("missing source URL")?;
    let mut seen = HashSet::new();
    for path in files(root)? {
        ensure!(
            !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("src-{prefix}-")),
            "PyPI packs already exist; refusing to overwrite"
        );
        let doc = read(&path)?;
        for rule in doc["rules"].as_array().context("missing rules")? {
            seen.insert(rule["id"].as_str().context("missing ID")?.to_owned());
        }
    }
    let mut rules = Vec::new();
    for name in names {
        ensure!(
            (2..=64).contains(&name.len())
                && name.starts_with(|c: char| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c)),
            "unsafe package name: {name}"
        );
        for (platform, paths) in [
            ("macos", vec![format!("$HOME/Library/Caches/{name}")]),
            (
                "linux",
                vec![
                    format!("$HOME/.cache/{name}"),
                    format!("$XDG_CACHE_HOME/{name}"),
                ],
            ),
            ("windows", vec![format!("%LOCALAPPDATA%/{name}/Cache")]),
        ] {
            if rules.len() == target {
                break;
            }
            let id = format!("{prefix}-{platform}-{}", slug(name));
            ensure!(seen.insert(id.clone()), "duplicate ID: {id}");
            rules.push(json!({"id":id,"name":format!("PyPI {name} {platform} cache probe"),"description":format!("Inferred cache path for PyPI project {name}; package presence and cache layout are unverified. Report only; preserves all files, configs and environments."),"category":"caches","platform":platform,"kind":"cache","risk":"report-only","report_only":true,"default_enabled":false,"min_age_days":14,"paths":paths,"delete_mode":"contents","max_depth":6,"tags":["pypi","catalog-probe","unverified-path"]}));
        }
    }
    ensure!(rules.len() == target, "expected exactly {target} rules");
    for (i, chunk) in rules.chunks(800).enumerate() {
        let n = i + 1;
        write(
            &root.join(format!("generated/src-{prefix}-{n:03}.yaml")),
            &json!({"pack":format!("src-{prefix}-{n}"),"description":format!("PyPI name snapshot {date}; inferred, disabled report-only cache probes. Source: {url}. Chunk {n}."),"rules":chunk}),
        )?;
    }
    println!("Generated {target} disabled report-only probes");
    Ok(())
}
fn launch(repo: &Path) -> Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "run-app bundles a macOS app; elsewhere use cargo run -p space-app --release"
    );
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "space-app", "--release"])
        .current_dir(repo)
        .status()?;
    ensure!(status.success(), "app build failed");
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(repo)
        .output()?;
    ensure!(output.status.success(), "cargo metadata failed");
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let target = PathBuf::from(
        metadata["target_directory"]
            .as_str()
            .context("missing target directory")?,
    );
    let app = std::env::temp_dir().join("SpaceScanner.app");
    for name in ["SpaceScanner", "space-scanner"] {
        let _ = std::process::Command::new("pkill")
            .args(["-x", name])
            .status();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    fs::create_dir_all(app.join("Contents/MacOS"))?;
    fs::copy(
        target.join("release/space-scanner"),
        app.join("Contents/MacOS/SpaceScanner"),
    )?;
    for entry in walkdir::WalkDir::new(repo.join("rules")) {
        let entry = entry?;
        let dest = app
            .join("Contents/rules")
            .join(entry.path().strip_prefix(repo.join("rules"))?);
        if entry.file_type().is_dir() {
            fs::create_dir_all(dest)?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), dest)?;
        }
    }
    fs::write(
        app.join("Contents/Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Space Scanner</string>
<key>CFBundleDisplayName</key><string>Space Scanner</string>
<key>CFBundleIdentifier</key><string>com.themoretheless.space-scanner</string>
<key>CFBundleVersion</key><string>0.1.0</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleExecutable</key><string>SpaceScanner</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#,
    )?;
    ensure!(
        std::process::Command::new("open")
            .arg(&app)
            .status()?
            .success(),
        "opening app failed"
    );
    println!("Launched {}", app.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "space-tools-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn duplicate_apply_preserves_text_and_migrates_aliases() {
        let f = Fixture::new();
        let prefix = "# retain this comment\npack: fixture\nrules:\n";
        let canonical = "- id: first\n  name: Example\n  category: caches\n  platform: linux\n  paths: ['$HOME/.cache/example']\n";
        let duplicate = canonical.replace("id: first", "id: second");
        fs::write(
            f.0.join("fixture.yaml"),
            format!("{prefix}{canonical}{duplicate}"),
        )
        .unwrap();
        fs::write(f.0.join("_rule-aliases.json"), "{\"older\":\"second\"}").unwrap();
        assert!(deduplicate(&f.0, false).is_err());
        deduplicate(&f.0, true).unwrap();
        assert_eq!(
            fs::read_to_string(f.0.join("fixture.yaml")).unwrap(),
            format!("{prefix}{canonical}")
        );
        let aliases: Value =
            serde_json::from_str(&fs::read_to_string(f.0.join("_rule-aliases.json")).unwrap())
                .unwrap();
        assert_eq!(aliases, json!({"older":"first","second":"first"}));
        assert!(duplicates(&f.0).unwrap().is_empty());
    }
    #[test]
    fn protection_generator_matches_existing_guards() {
        let f = Fixture::new();
        protections(&f.0).unwrap();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules");
        for platform in ["macos", "linux", "windows"] {
            let path = format!("protected/_protected-{platform}.yaml");
            assert_eq!(
                read(&f.0.join(&path)).unwrap(),
                read(&repo.join(&path)).unwrap()
            );
        }
    }
    #[test]
    fn catalog_rejects_short_input_and_overwrite() {
        let f = Fixture::new();
        let source = f.0.join("names.txt");
        fs::write(&source, "valid-name\n../unsafe\nvalid-name\n").unwrap();
        assert!(catalog(&f.0, &source, "test-wave", 4).is_err());
        assert!(!f.0.join("generated").exists());
        catalog(&f.0, &source, "test-wave", 3).unwrap();
        let path = f.0.join("generated/test-wave-01.yaml");
        let original = fs::read(&path).unwrap();
        assert!(catalog(&f.0, &source, "test-wave", 3).is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }
    #[test]
    fn pypi_custom_batch_is_report_only_and_refuses_overwrite() {
        let f = Fixture::new();
        let names = f.0.join("names.txt");
        let source = f.0.join("source.json");
        fs::write(&names, "alpha-example\nbeta-example\n").unwrap();
        fs::write(
            &source,
            r#"{"retrieved":"2026-10-01","source":"https://pypi.org/simple/"}"#,
        )
        .unwrap();
        assert!(pypi_batch(&f.0, &names, &source, "../bad", 5).is_err());
        assert!(pypi_batch(&f.0, &names, &source, "test-pypi", 7).is_err());
        assert!(!f.0.join("generated").exists());
        pypi_batch(&f.0, &names, &source, "test-pypi", 5).unwrap();
        let path = f.0.join("generated/src-test-pypi-001.yaml");
        let original = fs::read(&path).unwrap();
        let doc = read(&path).unwrap();
        let rules = doc["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 5);
        for value in rules {
            let rule: Rule = serde_json::from_value(value.clone()).unwrap();
            assert!(rule.report_only);
            assert!(!rule.default_enabled);
            assert_eq!(rule.risk, space_clean::Risk::ReportOnly);
        }
        assert!(pypi_batch(&f.0, &names, &source, "test-pypi", 5).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn pypi_matches_all_existing_rules_and_refuses_overwrite() {
        let f = Fixture::new();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        pypi(&repo, &f.0).unwrap();
        for n in 1..=125 {
            let path = format!("generated/src-pypi100k-{n:03}.yaml");
            assert_eq!(
                read(&f.0.join(&path)).unwrap(),
                read(&repo.join("rules").join(&path)).unwrap()
            );
        }
        assert!(pypi(&repo, &f.0).is_err());
    }
    #[test]
    fn split_shared_and_specific_paths() {
        let rules = split_rule(&json!({"id":"a","platform":"any","paths":["$HOME/.cache/a","$HOME/Library/Caches/a","%APPDATA%/a"]})).unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0]["paths"].as_array().unwrap().len(), 2);
        assert_eq!(rules[1]["paths"], json!(["$HOME/.cache/a"]));
        assert_eq!(rules[2]["id"], "a-windows");
    }
    #[test]
    fn single_platform_keeps_id() {
        assert_eq!(
            split_rule(&json!({"id":"a","paths":["%APPDATA%/a"]})).unwrap()[0]["id"],
            "a"
        );
    }
}
