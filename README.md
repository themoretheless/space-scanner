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
- CLI: `space scan|ls|du|junk|rules|categories`

## Roadmap

- Full catalog (~1000): [docs/FEATURES.md](docs/FEATURES.md)
- Top 200 execution checklist: [docs/ROADMAP-200.md](docs/ROADMAP-200.md)
- Perf notes: [docs/PERF.md](docs/PERF.md)

## Run

```bash
# Recommended on macOS (wgpu Metal + Dock .app)
./scripts/run-app.sh

# or
cargo run -p space-app --release

cargo run -p space-cli --release -- scan ~/Downloads
cargo run -p space-cli --release -- ls ~/Downloads
cargo run -p space-cli --release -- du ~/Downloads
```

Renderer: **wgpu** (Metal on macOS). Folder navigation and view switches use short zoom/fade transitions.

### Benches

```bash
cargo bench -p space-core --bench scan -- --profile quick
cargo bench -p space-core --bench scan -- --profile normal
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

```bash
cargo run -p space-cli --release -- rules
cargo run -p space-cli --release -- categories
cargo run -p space-cli --release -- junk -c caches,logs
cargo run -p space-cli --release -- junk -c caches --clean --yes   # → Trash
```

In the app: bottom **Junk** or menu `⋯` → Junk cleaner.
