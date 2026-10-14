# Space Scanner MCP

Build and run the stdio server:

```sh
cargo build -p space-mcp --release
./target/release/space-mcp
```

The process reads MCP JSON-RPC from stdin and writes protocol messages to stdout.
Use an MCP client to launch it; it does not open the desktop application or listen
on a network port. The implementation uses the official Rust MCP SDK (`rmcp`).

Example configuration for clients that accept `mcpServers` JSON (replace the path):

```json
{
  "mcpServers": {
    "space-scanner": {
      "command": "/absolute/path/to/space-scanner/target/release/space-mcp"
    }
  }
}
```

## Tools

| Tool | Arguments | Result |
|---|---|---|
| `scan_disk` | Absolute directory `path` | `scan_id`, root node ID, file count, apparent/allocated bytes and skipped errors |
| `disk_page` | `scan_id`, optional `node_id`, `offset`, `limit` | Directory children, total and `next_offset` |
| `scan_junk` | Optional `categories` (category names or rule IDs) | `scan_id`, finding/byte counts and skipped-root count |
| `junk_page` | `scan_id`, `offset`, `limit` | Findings, total and `next_offset` |
| `search_rules` | Optional `query`, `limit` | Host-platform rule summaries, match count and truncation flag |
| `rule_details` | Exact rule `id` | Full rule and effective enabled/minimum-age settings |

Page limits are 1–200 (default 100). Call page tools with the returned
`next_offset` until it is null (the key is always present). An `offset` past
`total` is an error that names the total; `offset == total` returns an empty
page. For disk navigation pass a directory's `node_id`.
Synthetic nodes aggregate small files; they are not paths and cannot be opened.

Only the most recent disk scan and the most recent junk scan are retained in
memory for the lifetime of the connection. Old IDs return an error after the next
scan of the same kind. IDs must not be reused across server sessions. Failed scans
do not replace the last successful result.

Junk scans use the existing host rules, saved preferences, exclusions and safety
checks. Rule discovery follows the CLI's order: `SPACE_RULES_DIR`, a `rules`
directory beside the executable (or inside the `.app` bundle), then the source
checkout fallback; the working directory is never consulted. The catalog and
preferences are loaded once and reused while the rule files' fingerprint is
unchanged; editing a rule file is picked up on the next call. Catalog caches may
be written by the shared loader. No tool deletes files or changes rule
preferences.

One scan (`scan_disk`, `scan_junk`) runs at a time; a concurrent scan request
receives `busy`. Page reads, `largest_entries`, `inspect_path`, `search_rules`
and `rule_details` do not take the scan slot and remain available during scans.
Scans finish as one tool call; set the client's tool timeout appropriately for
large disks. A `notifications/cancelled` for a running `scan_disk` stops the
filesystem walk within about 100 ms and releases the slot; the request then
fails with `scan cancelled by client` and the previous scan result is kept.
There is no cleanup tool in this version.

## Verification

```sh
cargo test -p space-mcp --test stdio
```

This launches a real stdio process with isolated files/rules and checks handshake,
tool discovery, file totals, junk findings, pagination, invalid arguments, expired
scan IDs and preservation of fixture files.

## Finding large entries and assessing junk

Two additional tools support investigation:

- `largest_entries(scan_id, kind, apparent, limit)` ranks the entire retained disk
  tree, with `kind` set to `files`, `directories` or `all`. Ranking defaults to
  allocated bytes; `apparent: true` uses logical bytes. The root and synthetic
  small-file aggregates are excluded. Results include display paths, sizes and
  file counts. Directory totals overlap their descendants and must not be summed.
- `inspect_path(path, junk_scan_id?)` reads live entry metadata, timestamps,
  permissions and existing safety/protection checks. It collects nearby project
  markers and application-bundle/cache-name hints without reading file contents.
  An optional junk scan ID attaches up to 20 prior findings that include the path.
  Use `rule_details` to inspect the referenced rule's exact conditions.

Suggested workflow: `scan_disk` → `largest_entries` → `inspect_path`; then
`scan_junk` → `inspect_path` with its scan ID → `rule_details` for supporting rules.
Every inspection returns `requires_review` and `deletion_authorized: false`.
A cache-like name or nearby project marker does not prove that data is disposable,
regenerable or owned by a particular application. Absence of a junk finding is
inconclusive. Rule evidence comes from a snapshot and is not a fresh age/exclusion
check. The tool does not determine whether an application is currently using data.

Directory metadata sizes describe the directory entry itself, not recursive
contents; use disk scan totals for those. Symlink metadata describes the link;
project markers are looked up along the link's own (lexical) parent path, never
along its target's. Junk correlation for symlinks is
rejected to avoid confusing target evidence with the link. Display paths from disk
scans use Unicode-normalized names; on case/normalization-sensitive filesystems,
verify a live path with `inspect_path` before relying on it. Individual small files
are aggregated by the core scanner and cannot appear in the largest-entry list.
