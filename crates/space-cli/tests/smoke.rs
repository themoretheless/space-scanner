//! End-to-end checks against the real `space` executable with an isolated
//! cache, prefs home and a tiny rule pack.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    root: tempfile::TempDir,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        fs::create_dir_all(data.join("nested/deeper")).unwrap();
        fs::write(data.join("big.log"), vec![b'l'; 900_000]).unwrap();
        fs::write(data.join("nested/medium.bin"), vec![b'b'; 600_000]).unwrap();
        fs::write(data.join("nested/deeper/tiny.tmp"), b"tiny").unwrap();
        fs::create_dir_all(data.join("dir,with comma")).unwrap();
        fs::create_dir_all(root.path().join("rules")).unwrap();
        fs::create_dir_all(root.path().join("home")).unwrap();
        let data_str = data.to_str().unwrap();
        fs::write(
            root.path().join("rules/fixture.yaml"),
            format!(
                r#"rules:
  - id: fx-logs
    name: Fixture logs
    description: smoke fixture
    category: fixture
    platform: any
    paths: ["{data_str}"]
    min_age_days: 0
    include_globs: ["**/*.log"]
    default_enabled: true
  - id: fx-tmp
    name: Fixture temp
    description: smoke fixture
    category: fixture
    platform: any
    paths: ["{data_str}"]
    min_age_days: 0
    include_globs: ["**/*.tmp"]
    default_enabled: false
  - id: fx-report
    name: Fixture report only
    description: smoke fixture
    category: fixture
    platform: any
    risk: report-only
    report_only: true
    paths: ["{data_str}"]
    min_age_days: 0
    include_globs: ["**/*.bin"]
    default_enabled: true
"#
            ),
        )
        .unwrap();
        Self { root, data }
    }

    fn run(&self, args: &[&str]) -> Output {
        let home = self.root.path().join("home");
        Command::new(env!("CARGO_BIN_EXE_space"))
            .args(args)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("SPACE_CACHE_DIR", self.root.path().join("cache"))
            .env("SPACE_RULES_DIR", self.root.path().join("rules"))
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn fails(&self, args: &[&str], code: i32) -> String {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stderr).unwrap()
    }

    fn data(&self) -> &str {
        self.data.to_str().unwrap()
    }
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

#[test]
fn index_commands() {
    let f = Fixture::new();
    let scan = f.ok(&["scan", f.data()]);
    assert!(scan.contains("cache saved"), "{scan}");
    assert!(exists(&f.root.path().join("cache")));

    let ls = f.ok(&["ls", f.data()]);
    assert!(ls.contains("big.log"), "{ls}");
    assert!(ls.contains("nested/"), "{ls}");
    let ls_limited = f.ok(&["ls", f.data(), "--limit", "1"]);
    assert_eq!(ls_limited.lines().count(), 1, "{ls_limited}");

    let du = f.ok(&["du", f.data(), "--apparent"]);
    assert!(du.contains("apparent"), "{du}");
    assert!(du.contains("3 files"), "{du}");
    assert!(f.ok(&["du", f.data()]).contains("on-disk"));

    let largest = f.ok(&["largest", f.data(), "--files", "--limit", "1"]);
    assert_eq!(largest.lines().count(), 1, "{largest}");
    assert!(largest.contains("big.log"), "{largest}");
    let dirs = f.ok(&["largest", f.data(), "--dirs"]);
    assert!(dirs.contains("nested/"), "{dirs}");
    assert!(!dirs.contains("big.log"), "{dirs}");

    let find = f.ok(&["find", f.data(), "--name", "*.bin"]);
    assert!(find.contains("medium.bin"), "{find}");
    assert!(!find.contains("big.log"), "{find}");
    let find_size = f.ok(&["find", f.data(), "--min-size", "800K", "--files-only"]);
    assert!(find_size.contains("big.log"), "{find_size}");
    assert!(!find_size.contains("medium.bin"), "{find_size}");
    let find_re = f.ok(&["find", f.data(), "--name", "^med.*n$", "--regex"]);
    assert!(find_re.contains("medium.bin"), "{find_re}");
    assert!(f.ok(&["find", f.data(), "--name", "nope"]).is_empty());
    f.fails(&["find", f.data(), "--name", "(", "--regex"], 2);
    f.fails(&["find", f.data(), "--dirs-only", "--files-only"], 2);
}

#[test]
fn export_formats() {
    let f = Fixture::new();
    let csv = f.ok(&["export", "csv", f.data()]);
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("path,size_disk,size_apparent,files,is_dir"));
    let root = lines.next().unwrap();
    assert!(root.starts_with(f.data()), "{root}");
    assert!(root.ends_with(",3,true"), "{root}");
    assert!(csv.contains("/dir,with comma\",0"), "{csv}");
    assert!(csv.contains("medium.bin,"), "{csv}");

    let shallow = f.ok(&["export", "csv", f.data(), "--depth", "1"]);
    assert!(!shallow.contains("medium.bin"), "{shallow}");
    assert!(shallow.contains("nested,"), "{shallow}");

    let out = f.root.path().join("tree.json");
    f.ok(&["export", "json", f.data(), "--out", out.to_str().unwrap()]);
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(rows[0]["path"], f.data());
    assert_eq!(rows[0]["is_dir"], true);
    assert_eq!(rows[0]["files"], 3);
    assert!(rows.iter().any(|r| r["path"].as_str().unwrap().ends_with("medium.bin")));
    assert!(rows.iter().all(|r| r["size_disk"].is_u64() && r["size_apparent"].is_u64()));
}

#[test]
fn cache_commands() {
    let f = Fixture::new();
    assert!(f.ok(&["cache", "ls"]).contains("0 cached index(es)"));
    f.ok(&["scan", f.data()]);
    let ls = f.ok(&["cache", "ls"]);
    assert!(ls.contains("1 cached index(es)"), "{ls}");
    assert!(ls.contains(f.data()), "{ls}");
    assert!(f.ok(&["cache", "prune", "--ttl-days", "30"]).contains("pruned 0"));
    f.fails(&["cache", "prune", "--ttl-days", "0"], 2);
    f.ok(&["cache", "rm", f.data()]);
    f.fails(&["cache", "rm", f.data()], 1);
    f.ok(&["scan", f.data()]);
    let wipe = f.ok(&["cache", "wipe"]);
    assert!(wipe.contains("wiped 2 file(s)"), "{wipe}");
    assert!(f.ok(&["cache", "ls"]).contains("0 cached index(es)"));
}

#[test]
fn rules_and_junk() {
    let f = Fixture::new();
    let list = f.ok(&["rules", "list", "--all"]);
    assert!(list.starts_with("3 matches"), "{list}");
    assert!(list.contains("fx-logs") && list.contains("fx-tmp"), "{list}");
    let enabled = f.ok(&["rules", "list", "--enabled"]);
    assert!(enabled.starts_with("2 matches"), "{enabled}");
    assert!(!enabled.contains("fx-tmp"), "{enabled}");
    let one = f.ok(&["rules", "list", "--all", "--limit", "1"]);
    assert!(one.starts_with("3 matches"), "{one}");
    assert_eq!(one.lines().count(), 2, "{one}");
    f.fails(&["rules", "list", "--enabled", "--disabled"], 2);
    f.fails(&["rules", "list", "--kind", "bogus"], 2);
    assert!(f.ok(&["rules", "status"]).contains("total\t3"));
    assert!(f.ok(&["categories", "--all"]).contains("fixture\t2/3"));

    let err = f.fails(&["rules", "enable", "does-not-exist"], 1);
    assert!(err.contains("does-not-exist"), "{err}");
    let err = f.fails(&["rules", "reset"], 2);
    assert!(err.contains("--all"), "{err}");
    f.fails(&["rules", "enable"], 2);

    // Enabling one real id alongside an unknown one applies the real one.
    let out = f.run(&["rules", "enable", "fx-tmp", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("enabled 1 rule(s)"));
    assert!(f.ok(&["rules", "list", "--enabled"]).contains("fx-tmp"));
    f.ok(&["rules", "reset", "fx-tmp"]);
    assert!(!f.ok(&["rules", "list", "--enabled"]).contains("fx-tmp"));

    let junk = f.ok(&["junk"]);
    assert!(junk.contains("1 findings"), "{junk}");
    assert!(junk.contains("[clean]") && junk.contains("big.log"), "{junk}");
    // Report-only rules stay out unless prefs opt in; default-off rules stay out.
    assert!(!junk.contains("medium.bin") && !junk.contains("tiny.tmp"), "{junk}");
    let all = f.ok(&["junk", "--all"]);
    assert!(all.contains("2 findings") && all.contains("tiny.tmp"), "{all}");
    let limited = f.ok(&["junk", "--all", "--limit", "1"]);
    assert!(limited.contains("… 1 more"), "{limited}");
    let json: serde_json::Value = serde_json::from_str(&f.ok(&["junk", "--json"])).unwrap();
    assert_eq!(json["findings"].as_array().unwrap().len(), 1);
    assert_eq!(json["findings"][0]["rule_id"], "fx-logs");
    let dry = f.run(&["junk", "--clean"]);
    assert_eq!(dry.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&dry.stderr).contains("Dry-run"));
    for name in ["big.log", "nested/medium.bin", "nested/deeper/tiny.tmp"] {
        assert!(exists(&f.data.join(name)), "{name} must survive a dry run");
    }
}

#[test]
fn usage_and_failure_exit_codes() {
    let f = Fixture::new();
    let missing = f.root.path().join("missing");
    f.fails(&["du", missing.to_str().unwrap()], 2);
    f.fails(&["nonsense"], 2);
    f.fails(&["--rules", missing.to_str().unwrap(), "rules", "status"], 2);
    let help = f.ok(&["--help"]);
    for cmd in ["largest", "find", "export", "cache", "junk"] {
        assert!(help.contains(cmd), "{help}");
    }
}
