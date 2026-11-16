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
