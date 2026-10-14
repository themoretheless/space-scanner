mod inspect;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars,
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};
use space_clean::{RuleFilter, RuleManager, ScanOptions};
use space_core::{NodeId, ScanEvent, SpaceTree};
use std::time::Duration;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DiskArgs {
    /// Absolute directory path to scan.
    path: PathBuf,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct JunkArgs {
    /// Categories or rule IDs. Empty means all enabled host rules, respecting preferences.
    #[serde(default)]
    categories: Vec<String>,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageArgs {
    /// ID returned by scan_disk or scan_junk. Only the latest scan of each kind is retained.
    scan_id: u64,
    #[serde(default)]
    offset: usize,
    /// 1..200 items, default 100.
    #[serde(default = "default_limit")]
    limit: usize,
    /// Disk directory node ID; omit for root. Unused for junk pages.
    node_id: Option<u32>,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    #[serde(default)]
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RuleArgs {
    id: String,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LargestArgs {
    scan_id: u64,
    /// files, directories or all (default).
    #[serde(default)]
    kind: EntryKind,
    /// Rank by apparent bytes instead of allocated disk bytes.
    #[serde(default)]
    apparent: bool,
    #[serde(default = "default_limit")]
    limit: usize,
}
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EntryKind {
    Files,
    Directories,
    #[default]
    All,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectArgs {
    path: PathBuf,
    /// Optional completed junk scan for rule evidence. This evidence is a snapshot.
    junk_scan_id: Option<u64>,
}
fn default_limit() -> usize {
    100
}
fn limit(value: usize) -> anyhow::Result<usize> {
    anyhow::ensure!(
        (1..=200).contains(&value),
        "limit must be between 1 and 200"
    );
    Ok(value)
}

#[derive(Default)]
struct State {
    next_id: u64,
    disk: Option<(u64, SpaceTree)>,
    junk: Option<(u64, space_clean::ScanReport)>,
    /// Catalog + prefs reused while the rule sources' fingerprint is unchanged.
    rules: Option<RuleManager>,
}

/// The rule manager for the discovered rule directories. The shared pack
/// loader fingerprints the YAML sources (stat fast path); when it hands back
/// the same `Arc` the cached manager is reused, otherwise prefs and catalog
/// are reloaded.
fn rule_manager(state: &Mutex<State>) -> anyhow::Result<RuleManager> {
    let dirs = space_clean::default_rules_dirs();
    anyhow::ensure!(
        !dirs.is_empty(),
        "no rules directory found; set {}",
        space_clean::RULES_DIR_ENV
    );
    let pack = space_clean::RulePack::load_many_shared(&dirs)?;
    let mut state = state
        .lock()
        .map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
    if let Some(cached) = &state.rules {
        if Arc::ptr_eq(&cached.pack, &pack) {
            return Ok(cached.clone());
        }
    }
    let manager = RuleManager::load_with_dirs(&dirs)?;
    state.rules = Some(manager.clone());
    Ok(manager)
}

/// Pagination window; `offset` past the end is an explicit error so a client
/// never mistakes an empty page for the end of the data.
fn page_window(offset: usize, limit: usize, total: usize) -> anyhow::Result<(usize, Option<usize>)> {
    anyhow::ensure!(
        offset <= total,
        "offset {offset} is past the end (total {total})"
    );
    let end = offset.saturating_add(limit).min(total);
    Ok((end, (end < total).then_some(end)))
}
#[derive(Clone)]
struct Scanner {
    state: Arc<Mutex<State>>,
    gate: Arc<tokio::sync::Semaphore>,
}
impl Default for Scanner {
    fn default() -> Self {
        Self {
            state: Default::default(),
            gate: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}
fn response(result: anyhow::Result<Value>) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
    }
}
impl Scanner {
    /// Run a scan-class operation; only one may run at a time.
    async fn work(
        &self,
        f: impl FnOnce(Arc<Mutex<State>>) -> anyhow::Result<Value> + Send + 'static,
    ) -> CallToolResult {
        let Ok(permit) = self.gate.clone().try_acquire_owned() else {
            return response(Err(anyhow::anyhow!(
                "busy: another scan or catalog operation is running"
            )));
        };
        let state = self.state.clone();
        response(
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                f(state)
            })
            .await
            .unwrap_or_else(|e| Err(e.into())),
        )
    }

    /// Run a read-only catalog/metadata operation off the async runtime
    /// without taking the scan gate, so lookups stay available during scans.
    async fn read(
        &self,
        f: impl FnOnce(Arc<Mutex<State>>) -> anyhow::Result<Value> + Send + 'static,
    ) -> CallToolResult {
        let state = self.state.clone();
        response(
            tokio::task::spawn_blocking(move || f(state))
                .await
                .unwrap_or_else(|e| Err(e.into())),
        )
    }
}
#[tool_router]
impl Scanner {
    #[tool(
        description = "Find the largest individual files/directories across a completed disk scan. Excludes scan root and synthetic small-file aggregates. Directory totals overlap descendants; do not sum them. Paths use normalized display names.",
        annotations(read_only_hint = true)
    )]
    async fn largest_entries(&self, Parameters(args): Parameters<LargestArgs>) -> CallToolResult {
        self.read(move |state| {
            let limit = limit(args.limit)?;
            let state = state.lock().map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
            let (id, tree) = state.disk.as_ref().ok_or_else(|| anyhow::anyhow!("run scan_disk first"))?;
            anyhow::ensure!(*id == args.scan_id, "scan_id expired; run scan_disk again");
            let (dirs, files) = match args.kind {
                EntryKind::Files => (false, true),
                EntryKind::Directories => (true, false),
                EntryKind::All => (true, true),
            };
            let mode = if args.apparent { space_core::SizeMode::Apparent } else { space_core::SizeMode::OnDisk };
            let eligible = tree.nodes.iter().enumerate()
                .filter(|(i, node)| *i != tree.root.0 as usize && !node.synthetic && ((node.is_dir && dirs) || (!node.is_dir && files)))
                .count();
            let items: Vec<_> = space_core::largest_entries(tree, mode, limit, dirs, files).into_iter().map(|id| {
                let node = tree.node(id);
                json!({"node_id":id.0,"display_path":tree.path_string(id),"is_dir":node.is_dir,"apparent_bytes":node.apparent_size,"allocated_bytes":node.allocated_size,"files":node.file_count})
            }).collect();
            Ok(json!({"scan_id":id,"scanned_at_unix":tree.scanned_at_unix,"eligible_entries":eligible,"truncated":eligible>limit,"items":items,"small_files_aggregated":true,"skipped_errors":tree.skipped_errors}))
        }).await
    }

    #[tool(
        description = "Inspect a live absolute path: metadata, dates, safety blocks and project/cache hints. Optionally correlate with a completed junk scan. Returns evidence for human/agent review, never a safe-to-delete verdict. Does not read file contents.",
        annotations(read_only_hint = true)
    )]
    async fn inspect_path(&self, Parameters(args): Parameters<InspectArgs>) -> CallToolResult {
        self.read(move |state| {
            let mut result = inspect::inspect(&args.path)?;
            if let Some(requested) = args.junk_scan_id {
                anyhow::ensure!(!std::fs::symlink_metadata(&args.path)?.file_type().is_symlink(), "junk evidence for symlink targets is not evidence for the link itself");
                let canonical = args.path.canonicalize()?;
                let state = state.lock().map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
                let (id, report) = state.junk.as_ref().ok_or_else(|| anyhow::anyhow!("run scan_junk first"))?;
                anyhow::ensure!(*id == requested, "scan_id expired; run scan_junk again");
                let matches: Vec<_> = report.findings.iter().filter(|f| f.path == canonical || (f.is_dir && canonical.starts_with(&f.path))).collect();
                result["junk_evidence"] = json!({"scan_id":id,"snapshot_only":true,"match_count":matches.len(),"findings":matches.iter().take(20).collect::<Vec<_>>(),"truncated":matches.len()>20,"meaning":"Prior finding includes this path; consult rule_details. No match does not prove this is valuable or disposable."});
            }
            Ok(result)
        }).await
    }

    #[tool(
        description = "Scan a directory's disk usage. Returns totals and a scan_id; use disk_page to inspect children. Does not delete files.",
        annotations(read_only_hint = true)
    )]
    async fn scan_disk(
        &self,
        Parameters(args): Parameters<DiskArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let cancel = context.ct.clone();
        self.work(move |state| {
            anyhow::ensure!(args.path.is_absolute(), "path must be absolute");
            let path = args.path.canonicalize()?;
            anyhow::ensure!(path.is_dir(), "path must be a directory");
            let scan = space_core::scan_path(path);
            loop {
                if cancel.is_cancelled() {
                    // Stops the walker threads; the handle's Drop would too, but
                    // this reports the cancellation instead of a spurious result.
                    scan.cancel();
                    anyhow::bail!("scan cancelled by client");
                }
                match scan.events.recv_timeout(Duration::from_millis(100)) {
                    Ok(ScanEvent::Done(tree)) => {
                        let mut state = state.lock().map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
                        state.next_id += 1;
                        let id = state.next_id;
                        let root = tree.node(tree.root);
                        let result = json!({"scan_id":id,"root":tree.root_path,"root_node_id":tree.root.0,"apparent_bytes":root.apparent_size,"allocated_bytes":root.allocated_size,"files":root.file_count,"skipped_errors":tree.skipped_errors});
                        state.disk = Some((id, *tree));
                        return Ok(result);
                    }
                    Ok(ScanEvent::Error(e)) => anyhow::bail!(e),
                    Ok(ScanEvent::Cancelled) => anyhow::bail!("scan cancelled"),
                    Ok(ScanEvent::Progress(_)) => {}
                    Err(e) if e.is_timeout() => {}
                    Err(_) => anyhow::bail!("scan ended without a result"),
                }
            }
        }).await
    }
    #[tool(
        description = "Read one directory from a completed disk scan. Synthetic nodes aggregate small files and are not filesystem paths.",
        annotations(read_only_hint = true)
    )]
    fn disk_page(&self, Parameters(args): Parameters<PageArgs>) -> CallToolResult {
        response((|| {
            let limit = limit(args.limit)?;
            let state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
            let (id, tree) = state
                .disk
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("run scan_disk first"))?;
            anyhow::ensure!(*id == args.scan_id, "scan_id expired; run scan_disk again");
            let node = tree
                .try_node(args.node_id.map(NodeId).unwrap_or(tree.root))
                .ok_or_else(|| anyhow::anyhow!("unknown node_id"))?;
            anyhow::ensure!(node.is_dir && !node.synthetic, "node is not a directory");
            let (end, next_offset) = page_window(args.offset, limit, node.children.len())?;
            let items: Vec<_> = node.children[args.offset..end]
                .iter()
                .map(|id| {
                    let node = tree.node(*id);
                    json!({"node_id":id.0,"display_path":(!node.synthetic).then(|| tree.path_string(*id)),"name":node.name,"is_dir":node.is_dir,"synthetic":node.synthetic,"apparent_bytes":node.apparent_size,"allocated_bytes":node.allocated_size,"files":node.file_count,"child_count":node.children.len()})
                })
                .collect();
            Ok(
                json!({"scan_id":id,"total":node.children.len(),"items":items,"next_offset":next_offset.map_or(Value::Null, Value::from)}),
            )
        })())
    }
    #[tool(
        description = "Scan enabled junk rules for this host, respecting saved preferences. Does not delete files. Use junk_page for findings.",
        annotations(read_only_hint = true)
    )]
    async fn scan_junk(&self, Parameters(args): Parameters<JunkArgs>) -> CallToolResult {
        self.work(move |state| {
            let manager = rule_manager(&state)?;
            let opts = ScanOptions { prefs_min_age: manager.prefs.min_age.clone(), ..Default::default() };
            let report = space_clean::scan_rules(manager.scan_rules(&args.categories), &opts)?;
            let mut state = state.lock().map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
            state.next_id += 1;
            let id = state.next_id;
            let result = json!({"scan_id":id,"total_bytes":report.total_bytes,"findings":report.findings.len(),"rules_scanned":report.rules_scanned,"rules_matched":report.rules_matched,"skipped_roots":report.skipped_roots.len()});
            state.junk = Some((id, report));
            Ok(result)
        }).await
    }
    #[tool(
        description = "Page through findings from the latest junk scan. Findings are candidates, not authorization to delete files.",
        annotations(read_only_hint = true)
    )]
    fn junk_page(&self, Parameters(args): Parameters<PageArgs>) -> CallToolResult {
        response((|| {
            let limit = limit(args.limit)?;
            anyhow::ensure!(
                args.node_id.is_none(),
                "node_id is only valid for disk_page"
            );
            let state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("scan state unavailable"))?;
            let (id, report) = state
                .junk
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("run scan_junk first"))?;
            anyhow::ensure!(*id == args.scan_id, "scan_id expired; run scan_junk again");
            let (end, next_offset) = page_window(args.offset, limit, report.findings.len())?;
            let items: Vec<_> = report.findings[args.offset..end].iter().collect();
            Ok(
                json!({"scan_id":id,"total":report.findings.len(),"items":items,"next_offset":next_offset.map_or(Value::Null, Value::from)}),
            )
        })())
    }
    #[tool(
        description = "Search host-platform rules by text. Returns up to 200 summaries and total match count. Use rule_details for full conditions.",
        annotations(read_only_hint = true)
    )]
    async fn search_rules(&self, Parameters(args): Parameters<SearchArgs>) -> CallToolResult {
        self.read(move |state| {
            let limit = limit(args.limit)?;
            let manager = rule_manager(&state)?;
            let (total, rows) = manager.list_page(&RuleFilter { query: args.query, host_only:true, ..Default::default() }, limit);
            let items: Vec<_> = rows.into_iter().map(|r| json!({"id":r.id,"name":r.name,"category":r.category,"risk":r.risk,"enabled":r.effectively_enabled})).collect();
            Ok(json!({"total":total,"items":items,"truncated":total>limit}))
        }).await
    }
    #[tool(
        description = "Get full rule conditions and effective enabled/minimum-age preferences by exact ID.",
        annotations(read_only_hint = true)
    )]
    async fn rule_details(&self, Parameters(args): Parameters<RuleArgs>) -> CallToolResult {
        self.read(move |state| {
            let manager = rule_manager(&state)?;
            let rule = manager.pack.get(&args.id).ok_or_else(|| anyhow::anyhow!("unknown rule ID"))?;
            Ok(json!({"rule":rule,"enabled":manager.prefs.is_enabled(rule),"min_age_days":manager.prefs.min_age_for(rule)}))
        }).await
    }
}
#[tool_handler]
impl ServerHandler for Scanner {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Scanner::default()
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn busy_operation_does_not_queue_another_scan() {
        let server = Scanner::default();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let worker = server.clone();
        let task = tokio::spawn(async move {
            worker
                .work(move |_| {
                    let _ = started_tx.send(());
                    finish_rx.recv()?;
                    Ok(json!({"finished": true}))
                })
                .await
        });
        started_rx.await.unwrap();
        let busy = server
            .work(|_| panic!("busy request must not execute"))
            .await;
        assert_eq!(busy.is_error, Some(true));
        finish_tx.send(()).unwrap();
        assert_ne!(task.await.unwrap().is_error, Some(true));
        assert_ne!(server.work(|_| Ok(json!({}))).await.is_error, Some(true));
    }
}
