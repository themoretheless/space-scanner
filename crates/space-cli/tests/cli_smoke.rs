use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("space-cli-{name}-{}-{nanos}", std::process::id()))
}

#[test]
fn cli_smoke_covers_version_scan_queries_export_cache_and_rules() {
    let base = temp_dir("smoke");
    let root = base.join("fixture");
    let cache = base.join("cache");
    let nested = root.join("sub");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("huge_archive.iso"), vec![1u8; 700 * 1024]).unwrap();
    fs::write(root.join("readme.txt"), b"hello space scanner").unwrap();

    let bin = env!("CARGO_BIN_EXE_space");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .env("SPACE_CACHE_DIR", &cache)
            .output()
            .expect("spawn space cli")
    };

    let ver = run(&["--version"]);
    assert!(
        ver.status.success(),
        "{:?}",
        String::from_utf8_lossy(&ver.stderr)
    );
    let ver_str = String::from_utf8_lossy(&ver.stdout);
    assert!(ver_str.contains(env!("CARGO_PKG_VERSION")));

    let root_s = root.to_str().unwrap();

    let scan = run(&["scan", root_s]);
    assert!(
        scan.status.success(),
        "{:?}",
        String::from_utf8_lossy(&scan.stderr)
    );
    assert!(String::from_utf8_lossy(&scan.stdout).contains("cache saved"));

    let du = run(&["du", root_s]);
    assert!(du.status.success());
    assert!(String::from_utf8_lossy(&du.stdout).contains("on-disk"));

    let ls = run(&["ls", root_s, "--limit", "5"]);
    assert!(ls.status.success());
    assert!(String::from_utf8_lossy(&ls.stdout).contains("sub/"));

    let find = run(&[
        "find",
        root_s,
        "--regex",
        "-q",
        "^huge_.*\\.iso$",
        "--ext",
        "iso",
        "--min-size",
        "1024",
    ]);
    assert!(
        find.status.success(),
        "{:?}",
        String::from_utf8_lossy(&find.stderr)
    );
    assert!(String::from_utf8_lossy(&find.stdout).contains("huge_archive.iso"));

    let largest = run(&["largest", root_s, "--limit", "5"]);
    assert!(largest.status.success());
    assert!(String::from_utf8_lossy(&largest.stdout).contains("huge_archive.iso"));

    let oldest = run(&["oldest", root_s, "--min-size", "1K", "--limit", "5"]);
    assert!(
        oldest.status.success(),
        "{:?}",
        String::from_utf8_lossy(&oldest.stderr)
    );
    assert!(String::from_utf8_lossy(&oldest.stdout).contains("huge_archive.iso"));

    let dupes = run(&["dupes", root_s, "--min-size", "1K", "--limit", "5"]);
    assert!(dupes.status.success());

    let exts = run(&["exts", root_s, "--limit", "5"]);
    assert!(exts.status.success());
    assert!(String::from_utf8_lossy(&exts.stdout).contains("iso"));

    let threaded = run(&["--threads", "2", "du", root_s]);
    assert!(threaded.status.success());

    let export_json = run(&["export", root_s, "--format", "json"]);
    assert!(export_json.status.success());
    let json_txt = String::from_utf8_lossy(&export_json.stdout);
    assert!(json_txt.contains("\"total_bytes\"") && json_txt.contains("\"children\""));

    let export_csv = run(&["export", root_s, "--format", "largest-csv"]);
    assert!(export_csv.status.success());
    assert!(String::from_utf8_lossy(&export_csv.stdout).contains("huge_archive.iso"));

    let svg_path = base.join("chart.svg");
    let export_svg = run(&[
        "export",
        root_s,
        "--format",
        "svg",
        "-o",
        svg_path.to_str().unwrap(),
    ]);
    assert!(export_svg.status.success());
    let svg = fs::read_to_string(&svg_path).unwrap();
    assert!(svg.starts_with("<svg ") && svg.contains("</svg>"));

    let cache_ls = run(&["cache", "ls"]);
    assert!(cache_ls.status.success());
    assert!(String::from_utf8_lossy(&cache_ls.stdout).contains("items"));

    let cache_dir = run(&["cache", "dir"]);
    assert!(cache_dir.status.success());

    let cache_clear = run(&["cache", "clear", "--all"]);
    assert!(cache_clear.status.success());
    assert!(String::from_utf8_lossy(&cache_clear.stdout).contains("Removed"));

    let rules_status = run(&["rules", "status"]);
    assert!(rules_status.status.success());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn cli_rm_permanent_supports_dry_run_readonly_trees_symlinks_and_safety() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let base = temp_dir("rm-perm");
    let victim = base.join("victim");
    let sub = victim.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let ro_file = sub.join("readonly.bin");
    fs::write(&ro_file, vec![9_u8; 4096]).unwrap();
    let outside = base.join("outside.txt");
    fs::write(&outside, b"must-survive").unwrap();

    #[cfg(unix)]
    let top_link = {
        std::os::unix::fs::symlink(&outside, sub.join("link_out")).unwrap();
        let top = base.join("top_link");
        std::os::unix::fs::symlink(&outside, &top).unwrap();
        fs::set_permissions(&ro_file, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();
        top
    };

    let bin = env!("CARGO_BIN_EXE_space");
    let victim_s = victim.to_str().unwrap();

    // 1. Dry-run without --yes leaves everything intact.
    let dry = Command::new(bin)
        .args(["rm", victim_s, "--permanent"])
        .output()
        .expect("run space rm dry-run");
    assert!(dry.status.success());
    let dry_out = format!(
        "{}{}",
        String::from_utf8_lossy(&dry.stdout),
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(
        dry_out.contains("Dry-run"),
        "unexpected dry-run output: {dry_out}"
    );
    assert!(victim.exists());

    // 2. Execute permanent deletion with --yes; removes readonly tree & symlinks safely.
    let mut exec_args = vec!["rm", victim_s];
    #[cfg(unix)]
    let top_s = top_link.to_str().unwrap().to_owned();
    #[cfg(unix)]
    exec_args.push(&top_s);
    exec_args.extend(["--permanent", "--yes"]);

    let exec = Command::new(bin)
        .args(&exec_args)
        .output()
        .expect("run space rm --permanent --yes");
    assert!(
        exec.status.success(),
        "rm failed: stdout={} stderr={}",
        String::from_utf8_lossy(&exec.stdout),
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(!victim.exists());
    #[cfg(unix)]
    assert!(fs::symlink_metadata(&top_link).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"must-survive");

    // 3. Refuses protected root path.
    let refused = Command::new(bin)
        .args(["rm", "/", "--permanent", "--yes"])
        .output()
        .expect("run space rm /");
    assert!(!refused.status.success());
    let refused_err = String::from_utf8_lossy(&refused.stderr);
    assert!(
        refused_err.contains("Protected"),
        "expected Protected in stderr, got: {refused_err}"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn cli_scenarios_inventory_ai_vm_and_heavy_caches_read_only() {
    let base = temp_dir("scenarios");
    let home = base.join("home");
    let tmp = base.join("tmp");
    let darwin_cache = base.join("darwin-c");
    fs::create_dir_all(home.join(".codex/tmp")).unwrap();
    fs::create_dir_all(home.join(".codex/worktrees/w1")).unwrap();
    fs::create_dir_all(home.join("Documents/Sources/demo/.claude/target-shared")).unwrap();
    fs::create_dir_all(home.join("Documents/Sources/demo/.claude/worktrees/agent-1")).unwrap();
    fs::create_dir_all(home.join(".colima/_lima/colima")).unwrap();
    fs::create_dir_all(home.join(".local/share/rag-mcp")).unwrap();
    fs::create_dir_all(home.join(".cache/custom-tool")).unwrap();
    fs::create_dir_all(
        home.join("Library/Application Support/Google/Chrome Dev/OptGuideOnDeviceModel"),
    )
    .unwrap();
    fs::create_dir_all(
        home.join(
            "Library/Application Support/Google/Chrome Dev/Default/Service Worker/CacheStorage",
        ),
    )
    .unwrap();
    fs::create_dir_all(home.join("Library/Application Support/Postman/Partitions/main/Cache"))
        .unwrap();
    fs::create_dir_all(home.join(".rustup/toolchains/1.85.0")).unwrap();
    fs::create_dir_all(home.join("Library/Caches/ms-playwright/chromium")).unwrap();
    fs::create_dir_all(&tmp).unwrap();
    fs::create_dir_all(&darwin_cache).unwrap();

    fs::write(home.join(".codex/tmp/scratch.bin"), vec![1u8; 4096]).unwrap();
    fs::write(home.join(".codex/worktrees/w1/f.bin"), vec![2u8; 4096]).unwrap();
    fs::write(
        home.join("Documents/Sources/demo/.claude/target-shared/out.rlib"),
        vec![3u8; 4096],
    )
    .unwrap();
    fs::write(
        home.join("Documents/Sources/demo/.claude/worktrees/agent-1/src.rs"),
        vec![4u8; 4096],
    )
    .unwrap();
    fs::write(home.join(".colima/_lima/colima/diffdisk"), vec![5u8; 8192]).unwrap();
    fs::write(
        home.join(".local/share/rag-mcp/rag.duckdb"),
        vec![6u8; 8192],
    )
    .unwrap();
    fs::write(home.join(".cache/custom-tool/blob.bin"), vec![7u8; 8192]).unwrap();
    fs::write(
        home.join(
            "Library/Application Support/Google/Chrome Dev/OptGuideOnDeviceModel/weights.bin",
        ),
        vec![8u8; 4096],
    )
    .unwrap();
    fs::write(
        home.join(
            "Library/Application Support/Google/Chrome Dev/Default/Service Worker/CacheStorage/chunk.bin",
        ),
        vec![9u8; 4096],
    )
    .unwrap();
    fs::write(
        home.join("Library/Application Support/Postman/Partitions/main/Cache/cache.bin"),
        vec![10u8; 4096],
    )
    .unwrap();
    fs::write(
        home.join(".rustup/toolchains/1.85.0/bin.dat"),
        vec![11u8; 4096],
    )
    .unwrap();
    fs::write(
        home.join("Library/Caches/ms-playwright/chromium/bin.dat"),
        vec![12u8; 4096],
    )
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_space");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("TMPDIR", &tmp)
            .env("DARWIN_USER_CACHE_DIR", &darwin_cache)
            .output()
            .expect("spawn space scenario")
    };

    let path_ends_with = |entry: &serde_json::Value, suffix: &str| {
        entry["path"]
            .as_str()
            .is_some_and(|p| p.replace('\\', "/").ends_with(suffix))
    };

    let ai = run(&["scenario", "ai-bloat", "--json"]);
    assert!(
        ai.status.success(),
        "{}",
        String::from_utf8_lossy(&ai.stderr)
    );
    let ai_json: serde_json::Value = serde_json::from_slice(&ai.stdout).unwrap();
    let ai_entries = ai_json["entries"].as_array().unwrap();
    assert!(ai_entries
        .iter()
        .any(|e| path_ends_with(e, ".codex/tmp") && e["classification"] == "safe-clean"));
    assert!(ai_entries.iter().any(|e| {
        path_ends_with(e, ".claude/target-shared") && e["classification"] == "safe-clean"
    }));
    assert!(ai_entries
        .iter()
        .any(|e| path_ends_with(e, ".claude/worktrees") && e["classification"] == "review"));

    let vm = run(&["scenario", "vm-disks", "--json"]);
    assert!(
        vm.status.success(),
        "{}",
        String::from_utf8_lossy(&vm.stderr)
    );
    let vm_json: serde_json::Value = serde_json::from_slice(&vm.stdout).unwrap();
    let vm_entries = vm_json["entries"].as_array().unwrap();
    assert!(vm_entries
        .iter()
        .any(|e| { e["kind"] == "Colima VMs" && e["disk_bytes"].as_u64().unwrap_or(0) > 0 }));

    let heavy = run(&["scenario", "heavy-caches", "--min-size", "1K", "--json"]);
    assert!(
        heavy.status.success(),
        "{}",
        String::from_utf8_lossy(&heavy.stderr)
    );
    let heavy_json: serde_json::Value = serde_json::from_slice(&heavy.stdout).unwrap();
    let heavy_entries = heavy_json["entries"].as_array().unwrap();
    assert!(heavy_entries.iter().any(|e| {
        path_ends_with(e, ".local/share/rag-mcp") && e["classification"] == "preserve-db"
    }));
    assert!(heavy_entries
        .iter()
        .any(|e| path_ends_with(e, ".cache/custom-tool") && e["classification"] == "review"));

    let browsers = run(&["scenario", "browsers-electron", "--json"]);
    assert!(
        browsers.status.success(),
        "{}",
        String::from_utf8_lossy(&browsers.stderr)
    );
    let b_json: serde_json::Value = serde_json::from_slice(&browsers.stdout).unwrap();
    let b_entries = b_json["entries"].as_array().unwrap();
    assert!(b_entries.iter().any(|e| {
        path_ends_with(e, "OptGuideOnDeviceModel") && e["classification"] == "review"
    }));
    assert!(b_entries.iter().any(|e| {
        path_ends_with(e, "Partitions/main/Cache") && e["classification"] == "safe-cache"
    }));

    let toolchains = run(&["scenario", "dev-toolchains", "--json"]);
    assert!(
        toolchains.status.success(),
        "{}",
        String::from_utf8_lossy(&toolchains.stderr)
    );
    let t_json: serde_json::Value = serde_json::from_slice(&toolchains.stdout).unwrap();
    let t_entries = t_json["entries"].as_array().unwrap();
    assert!(t_entries
        .iter()
        .any(|e| { e["tool"] == "rustup toolchain" && e["classification"] == "review" }));
    assert!(t_entries
        .iter()
        .any(|e| { e["tool"] == "Playwright browsers" && e["classification"] == "review" }));

    // Read-only guarantee: fixture files remain intact.
    assert!(home.join(".codex/tmp/scratch.bin").exists());
    assert!(home.join(".local/share/rag-mcp/rag.duckdb").exists());
    assert!(home
        .join("Library/Application Support/Google/Chrome Dev/OptGuideOnDeviceModel/weights.bin")
        .exists());
    let _ = fs::remove_dir_all(&base);
}
