//! Emit duplicate ID -> retained ID as JSON. Uses the same defaults/tags as the loader.
use anyhow::Result;
use serde::Deserialize;
use space_clean::{enrich_rule_kind, enrich_rule_tags, Rule};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Deserialize)]
struct File {
    rules: Vec<Rule>,
}
fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).unwrap_or("rules".into()));
    let mut files: Vec<_> = walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            matches!(p.extension().and_then(|s| s.to_str()), Some("yaml" | "yml"))
                && !p.file_name().unwrap().to_string_lossy().starts_with('_')
        })
        .collect();
    files.sort();
    let mut seen: HashMap<blake3::Hash, String> = HashMap::new();
    let mut aliases = BTreeMap::new();
    let mut count = 0;
    for path in files {
        let file: File = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
        for mut rule in file.rules {
            count += 1;
            rule.id = rule.id.trim().to_owned();
            rule.name = rule.name.trim().to_owned();
            rule.category = rule.category.trim().to_owned();
            enrich_rule_kind(&mut rule);
            enrich_rule_tags(&mut rule);
            // Order and repetition of paths/patterns do not alter the matching predicate.
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
            match seen.entry(hash) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    aliases.insert(rule.id, entry.get().clone());
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(rule.id);
                }
            }
        }
    }
    eprintln!(
        "rules={count} duplicates={} retained={}",
        aliases.len(),
        seen.len()
    );
    serde_json::to_writer_pretty(std::io::stdout(), &aliases)?;
    Ok(())
}
