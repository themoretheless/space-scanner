//! End-to-end JSON-RPC checks against the actual server executable.
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct Server {
    child: Child,
    input: Option<ChildStdin>,
    replies: mpsc::Receiver<Value>,
    sequence: u64,
    root: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Server {
    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.sequence += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.sequence,"method":method,"params":params}));
        loop {
            let v = self
                .replies
                .recv_timeout(Duration::from_secs(30))
                .expect("MCP reply timeout");
            if v["id"] == self.sequence {
                return v;
            }
        }
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        let reply = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert!(reply.get("error").is_none(), "{reply}");
        assert_ne!(reply["result"]["isError"], true, "{reply}");
        reply["result"]["structuredContent"].clone()
    }
    fn error(&mut self, name: &str, args: Value) {
        let reply = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert!(
            reply.get("error").is_some() || reply["result"]["isError"] == true,
            "{reply}"
        );
    }
}
#[test]
fn stdio_protocol() {
    let root = std::env::temp_dir().join(format!(
        "space-mcp-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let data = root.join("data");
    fs::create_dir_all(root.join("rules")).unwrap();
    for name in ["one", "two", "three"] {
        fs::create_dir_all(data.join(name)).unwrap();
        fs::write(data.join(name).join("test.log"), b"fixture").unwrap();
    }
    fs::write(data.join("keep.tmp"), b"keep").unwrap();
    fs::write(root.join("rules/fixture.yaml"),json!({"rules":[{"id":"mcp-fixture","name":"MCP Fixture","description":"stdio integration fixture","category":"mcp-fixture","platform":"any","paths":[data],"min_age_days":0,"include_globs":["**/*.log"],"default_enabled":true}]}).to_string()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_space-mcp"))
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let input = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let (tx, replies) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str(&line) else {
                break;
            };
            if tx.send(value).is_err() {
                break;
            }
        }
    });
    let mut s = Server {
        child,
        input,
        replies,
        sequence: 0,
        root,
    };
    let hello = s.request("initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"space-test","version":"1"}}));
    assert!(hello.get("result").is_some(), "{hello}");
    s.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let tools = s.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    let mut names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "disk_page",
            "inspect_path",
            "junk_page",
            "largest_entries",
            "rule_details",
            "scan_disk",
            "scan_junk",
            "search_rules"
        ]
    );
    assert!(tools
        .iter()
        .all(|t| t["annotations"]["readOnlyHint"] == true));
    s.error("scan_disk", json!({"path":"relative"}));
    s.error("scan_disk", json!({"path":data.join("absent")}));
    let disk = s.call("scan_disk", json!({"path":data}));
    assert_eq!(disk["files"], 4);
    assert_eq!(disk["apparent_bytes"], 25);
    let page = s.call("disk_page", json!({"scan_id":disk["scan_id"],"limit":1}));
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    let second = s.call(
        "disk_page",
        json!({"scan_id":disk["scan_id"],"offset":1,"limit":1}),
    );
    assert_ne!(second["items"][0]["node_id"], page["items"][0]["node_id"]);
    s.error("disk_page", json!({"scan_id":disk["scan_id"],"limit":201}));
    s.error(
        "disk_page",
        json!({"scan_id":disk["scan_id"],"node_id":4294967295u64}),
    );
    s.call("scan_disk", json!({"path":data}));
    s.error("disk_page", json!({"scan_id":disk["scan_id"]}));
    assert_eq!(
        s.call("search_rules", json!({"query":"MCP fixture","limit":10}))["total"],
        1
    );
    let rule = s.call("rule_details", json!({"id":"mcp-fixture"}));
    assert_eq!(rule["enabled"], true);
    assert_eq!(rule["min_age_days"], 0);
    s.error("rule_details", json!({"id":"absent"}));
    let junk = s.call("scan_junk", json!({"categories":["mcp-fixture"]}));
    assert_eq!(junk["findings"], 3);
    assert_eq!(junk["total_bytes"], 21);
    let findings = s.call("junk_page", json!({"scan_id":junk["scan_id"],"limit":2}));
    assert_eq!(findings["items"].as_array().unwrap().len(), 2);
    assert_eq!(findings["next_offset"], 2);
    assert_eq!(
        s.call("junk_page", json!({"scan_id":junk["scan_id"],"offset":2}))["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    s.error("junk_page", json!({"scan_id":junk["scan_id"],"node_id":1}));
    assert!(data.join("keep.tmp").exists());
    for name in ["one", "two", "three"] {
        assert!(data.join(name).join("test.log").exists());
    }
    let detail = s.call(
        "inspect_path",
        json!({"path":data.join("one/test.log"),"junk_scan_id":junk["scan_id"]}),
    );
    assert_eq!(detail["type"], "file");
    assert_eq!(detail["apparent_bytes"], 7);
    assert_eq!(detail["junk_evidence"]["match_count"], 1);
    assert_eq!(detail["deletion_authorized"], false);
    s.error("inspect_path", json!({"path":data.join("missing")}));
    s.error("inspect_path", json!({"path":"relative"}));
    #[cfg(unix)]
    assert_eq!(
        s.call("inspect_path", json!({"path":"/"}))["blocked_by_safety"],
        true
    );
    fs::write(data.join("Cargo.toml"), "[package]").unwrap();
    fs::create_dir(data.join("target")).unwrap();
    fs::write(data.join("target/large.bin"), vec![b'x'; 600000]).unwrap();
    fs::write(data.join("larger.bin"), vec![b'x'; 900000]).unwrap();
    let cache = s.call(
        "inspect_path",
        json!({"path":data.join("target/large.bin")}),
    );
    for kind in ["project_marker", "directory_name_hint"] {
        assert!(
            cache["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == kind),
            "{cache}"
        );
    }
    assert_eq!(cache["assessment"], "requires_review");
    assert!(
        s.call("inspect_path", json!({"path":data.join("target")}))["size_scope"]
            .as_str()
            .unwrap()
            .starts_with("directory_entry_only")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(data.join("larger.bin"), data.join("link")).unwrap();
        assert_eq!(
            s.call("inspect_path", json!({"path":data.join("link")}))["type"],
            "symlink"
        );
        s.error(
            "inspect_path",
            json!({"path":data.join("link"),"junk_scan_id":junk["scan_id"]}),
        );
    }
    let scan = s.call("scan_disk", json!({"path":data}));
    let ranked = s.call(
        "largest_entries",
        json!({"scan_id":scan["scan_id"],"kind":"files","apparent":true,"limit":2}),
    );
    assert_eq!(ranked["items"][0]["apparent_bytes"], 900000);
    assert_eq!(ranked["items"][1]["apparent_bytes"], 600000);
    assert!(ranked["items"][0]["display_path"]
        .as_str()
        .unwrap()
        .ends_with("/larger.bin"));
    let dirs = s.call(
        "largest_entries",
        json!({"scan_id":scan["scan_id"],"kind":"directories","apparent":true,"limit":1}),
    );
    assert!(dirs["items"][0]["display_path"]
        .as_str()
        .unwrap()
        .ends_with("/target"));
    s.error("largest_entries", json!({"scan_id":disk["scan_id"]}));
    s.error(
        "largest_entries",
        json!({"scan_id":scan["scan_id"],"kind":"invalid"}),
    );
    s.error(
        "largest_entries",
        json!({"scan_id":scan["scan_id"],"limit":0}),
    );
    s.input.take();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = s.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "server did not exit on EOF"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
