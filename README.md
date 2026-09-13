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
- Volumes list, Back/Forward, filter, largest files, Reveal in Finder
- Review queue + **Move to Trash** (right-click to queue)
- Cancelable scan with progress vs previous index
- Incremental **Rescan folder** (rebuilds index for current root path)
- CLI: `space scan|ls|du`

## Run

```bash
cargo run -p space-app --release
# or
cargo run -p space-cli --release -- scan ~/Downloads
cargo run -p space-cli --release -- ls ~/Downloads
cargo run -p space-cli --release -- du ~/Downloads
```

### Benches

```bash
cargo bench -p space-core --bench scan -- --profile quick
cargo bench -p space-core --bench scan -- --profile normal
```

See [docs/PERF.md](docs/PERF.md) for the latest bottleneck report.

### UI tips

- Click sunburst segment to zoom; center / **↑** to go up; **← →** history
- Right-click items to add to Review; **Review and Remove** → Trash
- Toggle **On disk / Apparent** in the top-right
- Container legend chips filter by extension

## Crates

- `space-core` — scan, tree, cache
- `space-app` — egui UI (`space-scanner`)
- `space-cli` — `space` binary
