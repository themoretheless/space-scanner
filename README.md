# Space Scanner

Fast native disk-space explorer (Duc-style index + Scanner/WinDirStat/DaisyDisk UX).

## Views

| View | Idea |
|------|------|
| **Sunburst** | Classic Scanner map — recursive rings, arc ∝ size (default) |
| **Container** | Nested treemap — area ∝ size, color by extension |
| **Radial** | Focus center; children as orbital nodes |
| **Cascade** | Icicle bands by depth |

## Features

- Parallel scan: work-stealing across all cores, stay on the same volume, skip Spotlight/Trash/other mounts
- Small files (&lt;512KB) rolled into one node per folder (faster index, less RAM)
- Hardlink dedup, **On disk / Apparent** sizes
- Pre-sorted tree + `Arc` UI (no per-frame clone)
- Zstd-compressed on-disk cache (v3) with **stale** badge
- Volumes list, Back/Forward, filter (debounce, dirs/files, size presets), largest files, Reveal in Finder
- Hover tooltips + sidebar highlight/scroll; path + % of parent
- Keyboard nav (arrows, Enter, Esc, ⌘1–4 views, ⌘⇧C copy path) — press **?**
- Review queue + **Move to Trash** (right-click to queue)
- **Junk cleaner** (ppduster rule packs): caches / logs / temp → Trash (`space junk`)
- Local Time Machine snapshots list + delete old
- Cancelable scan with progress vs previous index
- Incremental **Rescan folder** (rebuilds index for current root path)
- Persisted settings (`~/Library/Application Support/…/settings.json`)
- CLI: `space scan|ls|du|largest|find|export|cache|junk|rules|categories` (exit codes: 0 ok, 1 failure, 2 usage, 3 partial clean)

## Scenario: disk full, Docker volumes need review

Run `cargo run -p space-cli -- scenario docker-volumes` (or
`space scenario docker-volumes` with the CLI installed). Use `--json` for the
complete machine-readable volume inventory. This read-only scenario queries the
current Docker context and lists sizes, labels, drivers and container reference
counts, including references from stopped containers.

Volumes without references are candidates for **review**, not automatic deletion.
Check ownership using `docker volume inspect VOLUME`, inspect or back up contents
through a read-only mount, and recheck references before explicitly removing any
selected volume. Named database volumes and anonymous volumes can both contain
valuable data. The scenario never runs prune or removes Docker.raw. Docker's
reclaimable bytes do not promise immediate recovery of the same host disk space.

## Roadmap

- Full catalog (~1000): [docs/FEATURES.md](docs/FEATURES.md)
- Top 200 execution checklist: [docs/ROADMAP-200.md](docs/ROADMAP-200.md)
- Perf notes: [docs/PERF.md](docs/PERF.md)

## Run

```bash
# Recommended on macOS (wgpu Metal + Dock .app)
cargo run -p space-tools -- run-app

# or
cargo run -p space-app --release

cargo run -p space-cli --release -- scan ~/Downloads
cargo run -p space-cli --release -- ls ~/Downloads
cargo run -p space-cli --release -- du ~/Downloads
cargo run -p space-cli --release -- largest ~/Downloads --files --limit 20
cargo run -p space-cli --release -- find ~/Downloads --name '*.dmg' --min-size 100M
cargo run -p space-cli --release -- export csv ~/Downloads --depth 2 --out downloads.csv
cargo run -p space-cli --release -- cache ls          # also: cache rm <path> | wipe | prune --ttl-days N
```

Renderer: **wgpu** (Metal on macOS). Folder navigation and view switches use short zoom/fade transitions.

### Benches

```bash
cargo bench -p space-core --features bench --bench scan -- --profile quick
cargo bench -p space-core --features bench --bench scan -- --profile normal
```

### UI tips

- Click sunburst segment to zoom; center / **↑** to go up; **← →** history
- Hover a wedge for tooltip + sidebar sync
- Right-click items to add to Review; **Review and Remove** → Trash
- Toggle **On disk / Apparent** in the top-right
- Container legend chips filter by extension
- Filter: All/Dirs/Files, min-size presets, **Clear**, badge count

## Crates

- `space-core` — scan, tree, cache (`CACHE_VERSION` bump on schema change)
- `space-clean` — junk cleaner (ppduster rules/scan/clean/safety) + `rules/*.yaml`
- `space-app` — egui UI (`space-scanner`)
- `space-cli` — `space` binary

### Junk cleaner (from ppduster)

Rules live in [`rules/`](rules/) — **1,533,179** total across `categories/`, `platforms/`, and `generated/`, including 600,000 disabled, report-only PyPI cache probes. See [`rules/README.md`](rules/README.md).

**Rule manager** (prefs overlay, does not edit YAML):
- App: **Rules** button → search / toggle rules & categories / Enable all / Reset
- Prefs file: `~/Library/Application Support/com.themoretheless.SpaceScanner/rule-prefs.json` (macOS)
- Junk scan respects effective on/off from prefs; only host-matching platforms run

```bash
cargo run -p space-cli --release -- rules status
cargo run -p space-cli --release -- rules list --all -q win-
cargo run -p space-cli --release -- rules list --all -q linux-
cargo run -p space-cli --release -- junk -c caches,logs      # --json, --limit N, --all, --clean --yes
```

Global flags: `--rules <dir>` (or `SPACE_RULES_DIR`) picks the rule pack,
`--cache-dir <dir>` (or `SPACE_CACHE_DIR`) the index cache. Exit codes:
`0` success, `1` failure, `2` usage error, `3` partial clean (`junk --clean`
with errors). Files under 512 KiB are aggregated by the indexer and do not
appear in `largest` / `find`.
```

**`any` is not used.** Cross-OS apps are split into `macos` / `linux` / `windows` rules (paths filtered per OS). Re-split: `cargo run -p space-tools -- split`.

Expand from a local package-name list: `cargo run -p space-tools -- catalog names.txt catalog-wave 15000`.
OS seed packs are maintained directly in `rules/platforms/`.

In the app: **Junk** (clean) · **Rules** (manager).

### MCP server

`cargo build -p space-mcp --release` builds `target/release/space-mcp`, a stdio MCP
server for disk/junk scans and rule inspection. It supports paginated results and
uses the same scanner and preferences as the app. See [MCP setup and tools](docs/MCP.md).
