use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

fn kahn_topological_sort(
    nodes: &BTreeSet<String>,
    edges: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let mut in_degree: BTreeMap<String, usize> = nodes.iter().map(|n| (n.clone(), 0)).collect();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (from, deps) in edges {
        for dep in deps {
            if from == dep {
                return Err(format!("self-edge detected on `{from}`"));
            }
            if !nodes.contains(dep) {
                return Err(format!("unknown dependency `{dep}` referenced by `{from}`"));
            }
            *in_degree.entry(from.clone()).or_default() += 1;
            dependents
                .entry(dep.clone())
                .or_default()
                .push(from.clone());
        }
    }

    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(node, _)| node.clone())
        .collect();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(node) = queue.pop_front() {
        order.push(node.clone());
        if let Some(children) = dependents.get(&node) {
            for child in children {
                if let Some(deg) = in_degree.get_mut(child) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
    }

    if order.len() != nodes.len() {
        let cyclic: Vec<&str> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg > 0)
            .map(|(n, _)| n.as_str())
            .collect();
        return Err(format!("cycle detected involving: {}", cyclic.join(", ")));
    }
    Ok(order)
}

fn parse_workspace_crates(
    crates_dir: &Path,
) -> (BTreeSet<String>, BTreeMap<String, BTreeSet<String>>) {
    let mut raw_manifests: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(crates_dir).expect("read crates directory") {
        let entry = entry.expect("crates dir entry");
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path).expect("read Cargo.toml");
        let mut in_package = false;
        let mut pkg_name = None;
        for raw_line in text.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.starts_with('[') && line.ends_with(']') {
                in_package = line == "[package]";
                continue;
            }
            if in_package {
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == "name" {
                        pkg_name = Some(v.trim().trim_matches('"').to_string());
                        break;
                    }
                }
            }
        }
        let name = pkg_name.expect("package.name in Cargo.toml");
        raw_manifests.push((name, text));
    }

    let nodes: BTreeSet<String> = raw_manifests.iter().map(|(n, _)| n.clone()).collect();
    let mut edges: BTreeMap<String, BTreeSet<String>> =
        nodes.iter().map(|n| (n.clone(), BTreeSet::new())).collect();

    for (pkg, text) in &raw_manifests {
        let mut in_deps = false;
        for raw_line in text.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];
                in_deps = section == "dependencies"
                    || section == "dev-dependencies"
                    || section == "build-dependencies"
                    || section.ends_with(".dependencies")
                    || section.ends_with(".dev-dependencies")
                    || section.ends_with(".build-dependencies");
                continue;
            }
            if in_deps {
                if let Some((lhs, rhs)) = line.split_once('=') {
                    let dep_key = lhs.split('.').next().unwrap_or("").trim();
                    if nodes.contains(dep_key) && dep_key != pkg {
                        edges.get_mut(pkg).unwrap().insert(dep_key.to_string());
                    }
                    if let Some(pkg_idx) = rhs.find("package") {
                        let tail = &rhs[pkg_idx + "package".len()..];
                        if let Some((_, val)) = tail.split_once('=') {
                            let explicit = val
                                .trim()
                                .trim_start_matches('"')
                                .split('"')
                                .next()
                                .unwrap_or("");
                            if nodes.contains(explicit) && explicit != pkg {
                                edges.get_mut(pkg).unwrap().insert(explicit.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    (nodes, edges)
}

#[test]
fn workspace_crate_dependency_graph_is_a_strict_dag() {
    let core_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = core_dir.parent().expect("crates parent directory");
    let (nodes, edges) = parse_workspace_crates(crates_dir);

    for expected in [
        "space-core",
        "space-clean",
        "space-app",
        "space-cli",
        "space-mcp",
        "space-tools",
    ] {
        assert!(nodes.contains(expected), "missing crate `{expected}`");
    }

    assert!(
        edges["space-core"].is_empty(),
        "space-core must remain a zero-workspace-dependency leaf crate: {:?}",
        edges["space-core"]
    );
    assert!(
        edges["space-clean"].is_empty(),
        "space-clean must remain a zero-workspace-dependency leaf crate: {:?}",
        edges["space-clean"]
    );
    for forbidden in ["space-cli", "space-mcp", "space-tools"] {
        assert!(
            !edges["space-app"].contains(forbidden),
            "space-app must not depend on `{forbidden}`"
        );
    }

    let order = kahn_topological_sort(&nodes, &edges).expect("workspace crates form a DAG");
    assert_eq!(order.len(), nodes.len());
}

fn collect_rs_modules(src_dir: &Path) -> BTreeMap<String, PathBuf> {
    let mut map = BTreeMap::new();
    for entry in fs::read_dir(src_dir).expect("read src dir") {
        let entry = entry.expect("src entry");
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            map.insert(stem, path);
        } else if path.is_dir() {
            let parent = path.file_name().unwrap().to_string_lossy().to_string();
            for sub in fs::read_dir(&path).expect("read submodule dir") {
                let sub = sub.expect("sub entry").path();
                if sub.extension().and_then(|e| e.to_str()) == Some("rs") {
                    let stem = sub.file_stem().unwrap().to_string_lossy().to_string();
                    if stem == "mod" {
                        map.insert(parent.clone(), sub);
                    } else {
                        map.insert(format!("{parent}::{stem}"), sub);
                    }
                }
            }
        }
    }
    map
}

fn strip_tests_and_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut skipping_test_attr = false;
    let mut skip_brace_depth: usize = 0;
    let mut in_test_block = false;
    let mut in_block_comment = false;

    for raw_line in source.lines() {
        let trimmed = raw_line.trim();
        if !in_block_comment && !in_test_block && trimmed == "#[cfg(test)]" {
            skipping_test_attr = true;
            continue;
        }
        if skipping_test_attr && !in_test_block {
            if let Some(pos) = raw_line.find('{') {
                in_test_block = true;
                skip_brace_depth = 1;
                for ch in raw_line[pos + 1..].chars() {
                    match ch {
                        '{' => skip_brace_depth += 1,
                        '}' => {
                            skip_brace_depth -= 1;
                            if skip_brace_depth == 0 {
                                in_test_block = false;
                                skipping_test_attr = false;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }
        if in_test_block {
            for ch in raw_line.chars() {
                match ch {
                    '{' => skip_brace_depth += 1,
                    '}' => {
                        skip_brace_depth -= 1;
                        if skip_brace_depth == 0 {
                            in_test_block = false;
                            skipping_test_attr = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            continue;
        }

        // Strip `//` and `/* ... */` comments on kept non-test lines.
        let mut chars = raw_line.chars().peekable();
        let mut in_string = false;
        let mut escaped = false;
        while let Some(c) = chars.next() {
            if in_block_comment {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
                continue;
            }
            if in_string {
                out.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                continue;
            }
            if c == '"' {
                in_string = true;
                out.push(c);
                continue;
            }
            if c == '/' && chars.peek() == Some(&'/') {
                break;
            }
            if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                in_block_comment = true;
                continue;
            }
            out.push(c);
        }
        out.push('\n');
    }
    out
}

fn read_ident(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn collect_brace_group_roots(group: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut current = String::new();
    for ch in group.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                if let Some(pair) = parse_item_prefix(current.trim()) {
                    out.push(pair);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if let Some(pair) = parse_item_prefix(current.trim()) {
        out.push(pair);
    }
    out
}

fn parse_item_prefix(item: &str) -> Option<(String, Option<String>)> {
    let trimmed = item.trim();
    if trimmed.is_empty() || trimmed == "self" {
        return None;
    }
    let (first, rest) = read_ident(trimmed);
    if first.is_empty() {
        return None;
    }
    let rest = rest.trim_start();
    let second = if let Some(after_colons) = rest.strip_prefix("::") {
        let (sub, _) = read_ident(after_colons.trim_start());
        (!sub.is_empty()).then(|| sub.to_string())
    } else {
        None
    };
    Some((first.to_string(), second))
}

fn extract_module_edges(
    mod_name: &str,
    code: &str,
    all_modules: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();

    for line in code.lines() {
        let t = line.trim();
        let after_vis = t
            .strip_prefix("pub ")
            .or_else(|| t.strip_prefix("pub(crate) "))
            .or_else(|| t.strip_prefix("pub(super) "))
            .unwrap_or(t);
        if let Some(rest) = after_vis.strip_prefix("mod ") {
            let (child, _) = read_ident(rest.trim());
            if !child.is_empty() {
                if mod_name == "lib" && all_modules.contains(child) {
                    deps.insert(child.to_string());
                } else {
                    let submod = format!("{mod_name}::{child}");
                    if all_modules.contains(&submod) {
                        deps.insert(submod);
                    }
                }
            }
        }
    }

    let mut rest = code;
    while let Some(idx) = rest.find("crate::") {
        rest = &rest[idx + "crate::".len()..];
        let trimmed = rest.trim_start();
        if let Some(after_brace) = trimmed.strip_prefix('{') {
            if let Some(end) = find_matching_brace(after_brace) {
                for (first, second) in collect_brace_group_roots(&after_brace[..end]) {
                    record_crate_target(
                        mod_name,
                        &first,
                        second.as_deref(),
                        all_modules,
                        &mut deps,
                    );
                }
            }
        } else {
            let (first, tail) = read_ident(trimmed);
            let tail = tail.trim_start();
            let second = tail.strip_prefix("::").and_then(|after| {
                let (sub, _) = read_ident(after.trim_start());
                (!sub.is_empty()).then(|| sub.to_string())
            });
            if !first.is_empty() {
                record_crate_target(mod_name, first, second.as_deref(), all_modules, &mut deps);
            }
        }
    }

    if let Some((parent, _)) = mod_name.rsplit_once("::") {
        let mut super_rest = code;
        while let Some(idx) = super_rest.find("super::") {
            super_rest = &super_rest[idx + "super::".len()..];
            let trimmed = super_rest.trim_start();
            if let Some(after_brace) = trimmed.strip_prefix('{') {
                if let Some(end) = find_matching_brace(after_brace) {
                    for (first, _) in collect_brace_group_roots(&after_brace[..end]) {
                        record_super_target(mod_name, parent, &first, all_modules, &mut deps);
                    }
                }
            } else {
                let (first, _) = read_ident(trimmed);
                if !first.is_empty() {
                    record_super_target(mod_name, parent, first, all_modules, &mut deps);
                }
            }
        }
    }

    deps
}

fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 1_usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn record_crate_target(
    self_mod: &str,
    first: &str,
    second: Option<&str>,
    all_modules: &BTreeSet<String>,
    deps: &mut BTreeSet<String>,
) {
    if let Some(sub) = second {
        let scoped = format!("{first}::{sub}");
        if all_modules.contains(&scoped) && scoped != self_mod {
            deps.insert(scoped);
            return;
        }
    }
    if all_modules.contains(first) && first != self_mod {
        deps.insert(first.to_string());
    }
}

fn record_super_target(
    self_mod: &str,
    parent: &str,
    first: &str,
    all_modules: &BTreeSet<String>,
    deps: &mut BTreeSet<String>,
) {
    let sibling = format!("{parent}::{first}");
    if all_modules.contains(&sibling) {
        if sibling != self_mod {
            deps.insert(sibling);
        }
    } else if all_modules.contains(parent) && parent != self_mod {
        deps.insert(parent.to_string());
    }
}

#[test]
fn space_core_internal_modules_form_a_strict_dag() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = collect_rs_modules(&src_dir);
    let nodes: BTreeSet<String> = files.keys().cloned().collect();

    for expected in [
        "lib",
        "cache",
        "export",
        "format",
        "tree",
        "scan",
        "scan::dag",
        "scan::builder",
        "scan::reader",
        "scan::rescan",
    ] {
        assert!(nodes.contains(expected), "missing module `{expected}`");
    }

    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (mod_name, path) in &files {
        let src = fs::read_to_string(path).expect("read module source");
        let clean = strip_tests_and_comments(&src);
        edges.insert(
            mod_name.clone(),
            extract_module_edges(mod_name, &clean, &nodes),
        );
    }

    assert!(
        edges["format"].is_empty(),
        "format must be a dependency-free leaf module: {:?}",
        edges["format"]
    );
    assert!(
        edges["tree"].is_empty(),
        "tree must be a dependency-free leaf module: {:?}",
        edges["tree"]
    );
    assert_eq!(
        edges["scan::dag"],
        BTreeSet::from(["tree".to_string()]),
        "scan::dag must depend only on tree"
    );

    for sibling in ["scan::reader", "scan::builder", "scan::rescan"] {
        assert!(
            !edges[sibling].contains("scan"),
            "`{sibling}` must not depend back on parent `scan`: {:?}",
            edges[sibling]
        );
        for other in ["scan::reader", "scan::builder", "scan::rescan"] {
            assert!(
                !edges[sibling].contains(other),
                "`{sibling}` must not depend on sibling `{other}`: {:?}",
                edges[sibling]
            );
        }
    }

    let order = kahn_topological_sort(&nodes, &edges).expect("space-core modules form a DAG");
    assert_eq!(order.len(), nodes.len());
}
