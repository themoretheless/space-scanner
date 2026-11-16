# Space Scanner — Unified Plan

Single authoritative planning document consolidating the UX interaction specification (`docs/UX.md`), Top-200 (`P0`) execution waves (`docs/ROADMAP-200.md`), and the complete 1,125-item feature backlog (`docs/FEATURES.md` / `docs/features_items.json`), plus forward performance work from [PERF.md](PERF.md).

Companion reference docs:
- Performance benchmarks and measurements: [PERF.md](PERF.md)
- MCP server setup and tools: [MCP.md](MCP.md)
- Cleanup rules catalog and safety boundaries: [rules/README.md](../rules/README.md)

## Contents

- [1. Product & UX Plan](#1-product--ux-plan)
- [2. Top-200 Execution Roadmap (P0)](#2-top-200-execution-roadmap-p0)
- [3. Feature Backlog (F0001–F1125)](#3-feature-backlog-f0001f1125)

---

## 1. Product & UX Plan

The primary job is to answer **"what is taking up space, and what can I safely clear?"** alongside **"where did my disk space go folder by folder?"** Screens follow user jobs rather than the rendering engine.

### 1.1 User journey

1. **Quick clean**: present safe caches/logs, manual Downloads review items (installers, archives, large unused downloads), items needing review, and local Time Machine snapshots in a single uncluttered list with checkboxes.
2. **Folders (Disk usage)**: scan a chosen target (Home, a specific folder, or a disk) or open a saved result. Saved results are explicitly historical: opening one is immediate, and updating it is an explicit rescan.
3. **Explore & Select**: show the current folder, its children ranked by size, and a visual map of the same children. Keep location and search controls in a workspace sub-bar separate from global navigation. A single click selects an item and reveals its details; a second explicit action or double-click opens a folder. Selection does not imply removal.
4. **Basket / Review**: collect explicitly checked cleanup items, Downloads items, and manually queued files/folders in one persistent bottom Basket bar and review drawer. Switching between `Quick clean` and `Folders` preserves checked items without starting extra scans or preselecting risky items.
5. **Move to Trash**: confirm with an explicit list of paths, item count, and checked bytes. Report actual successes and failures. Checked bytes are not a prediction of newly available disk capacity.
6. **Update & Snapshots**: refresh the affected folder, cleanup list, and disk capacity; remind the user that Finder's Trash must be emptied to release space. Local Time Machine snapshots stay behind their own permanent-removal confirmation.

### 1.2 Screen responsibilities

- **Quick clean (`Mode::Clean`)**: default streamlined workspace for freeing space. Groups safe caches (`JunkGroup::Safe`), large/installer/archive Downloads (`downloads.rs`), optional review-needed junk (`JunkGroup::Review`), and local snapshots summary. Report-only findings cannot enter the removal selection. A rule classification is guidance, not a guarantee of safety; scanning and selecting remain separate actions.
- **Folders / Disk usage (`Mode::Explore`)**: size-ranked contents list paired with the visual map (Sunburst, Container treemap, Radial, Cascade). When no tree is loaded (or when picking a target), offers a streamlined scan target chooser (Home folder, specific folder, volumes, and individually actionable saved results). The inspector appears only when an item is selected: its primary action for folders is Open folder, Show in Finder verifies contents, and Add to Trash basket is a secondary choice. Avoid duplicating largest-child lists between inspector and contents.
- **Basket bar & review drawer**: shown at the bottom whenever at least one item is checked. Deduplicates overlapping paths before computing totals or trashing, lets users inspect and uncheck individual paths or clear all, and opens the Trash confirmation modal.
- **Secondary dialogs & `⋯` menu**: keeps Scan targets, Time Machine Snapshots, Rules manager, Settings, Appearance (Dark/Light skins), Keyboard shortcuts (`?`), and Language (`EN`/`RU`) accessible without crowding primary screens.

### 1.3 Responsive layout and input contract

- **Global navigation (`app_bar`)**: one `52 pt` row (`metrics::APP_BAR_H = 52.0`) with brand, the two primary mode tabs (`Quick clean` | `Folders`), and a `⋯` More menu (plus a compact status note on wide windows). Location, breadcrumbs, and search use a separate `48 pt` `sub_bar` in `Folders`. Scan progress uses its own bar.
- **Window width classes (`Density` in `crates/space-app/src/app/ui/mod.rs`)**:
  - **Wide (`>= 1180 pt`)**: size-ranked contents (`400 pt`) beside the map, and details in a right inspector panel only when an item is selected.
  - **Medium (`900–1180 pt`)**: contents (`320 pt`) beside the map, with details rendered as a drawer over the map.
  - **Narrow (`< 900 pt`)**: one workspace pane at a time (`List` | `Map` toggle in `sub_bar`), with details in a scrollable drawer.
- **Visual & control discipline**: Map style and size accounting (`On disk` / `Apparent`) sit in a compact toolbar above the map outside its interactive canvas. Segmented choices have no outer frame; only the active choice has a fill. Paths truncate cleanly with full-path tooltips; category counts sit on their own line when needed to avoid colliding with source labels. Shortcut lists scroll inside short windows so Close stays reachable. Nothing in the UI renders below `12 pt` (`type_scale::CAPTION = 12.0`, `TITLE = 18.0`, `DISPLAY = 22.0`), with `5 pt` corner radius (`metrics::RADIUS_CONTROL`) and `30 pt` control height (`metrics::CONTROL_H`). Shared widgets live in `widgets.rs`; tokens live in `theme::type_scale` and `theme::metrics`.
- **Shared selection semantics**: Across map wedges/tiles, contents list rows, and largest-file results: single click selects, double-click opens, and `⌘`-click toggles the Trash basket/review queue. Hover previews never overwrite the persistent selection or steal keyboard focus. Breadcrumbs shorten from the left; search reports its scope and match count and offers Clear.

### 1.4 States and disk-accounting language

- Explicitly distinguish a fresh scan, a saved historical result, a partial/cancelled scan, and inaccessible content. Errors remain visible with a recovery action.
- Scan progress is indeterminate unless measured against a prior index; never present that estimate as an exact percentage.
- For folder scans, disk usage outside the folder is expected—never attribute the difference from volume usage to snapshots. For whole-volume scans, unindexed space can reflect permissions, snapshots, or filesystem accounting; state these as possibilities.

### 1.5 Current implementation & remaining refinements

- **Implemented UI architecture (`crates/space-app/src/`)**: `app.rs` owns state, background workers, and shortcuts; `app/ui/{mod,shell,cleanup,usage,scan,dialogs,start_skins}.rs` draws the streamlined two-mode layout (`Quick clean` + `Folders`), persistent Basket bar and drawer, target chooser, Time Machine snapshot modal, rule manager, and EN/RU localization (`i18n.rs`, `i18n_ru.rs`, painter-drawn vector icons in `icons.rs`, safe Download discovery in `downloads.rs`, and rule findings in `junk_panel.rs`).
- **Completed UX & performance refinements**:
  1. Richer scan coverage and outcome presentation (clear summary of inaccessible/skipped roots after folder and volume scans).
  2. Accessible keyboard and screen-reader selection controls for custom-painted map shapes and list rows (`A11y` items below).
  3. macOS bulk metadata reader (`getattrlistbulk` candidate identified in [PERF.md](PERF.md)) with separate correctness and cold-disk IO measurements.

---

## 2. Top-200 Execution Roadmap (P0)

Status: `TODO` | `DONE`. Wave: `0–9` per release plan.

**Acceptance rule (all 200 items):** `Done when: User-visible or CLI: <Feature> works in release build.`

**Overall progress:** **200 / 200 DONE** (0 remaining).

### 2.1 Wave summary

| Wave | DONE | TODO | Total |
|------|-----:|-----:|------:|
| Wave 0 | 17 | 0 | 17 |
| Wave 1 | 32 | 0 | 32 |
| Wave 2 | 8 | 0 | 8 |
| Wave 3 | 8 | 0 | 8 |
| Wave 4 | 24 | 0 | 24 |
| Wave 5 | 32 | 0 | 32 |
| Wave 6 | 16 | 0 | 16 |
| Wave 7 | 8 | 0 | 8 |
| Wave 8 | 23 | 0 | 23 |
| Wave 9 | 32 | 0 | 32 |
| **Total** | **200** | **0** | **200** |

### 2.2 Top-200 (`P0`) checklist

| ID | Wave | Category | Status | Feature |
|----|-----:|----------|--------|---------|
| `F0001` | 4 | Scan | DONE | parallel work-stealing walk |
| `F0002` | 4 | Scan | DONE | same-volume device filter |
| `F0003` | 4 | Scan | DONE | skip Spotlight |
| `F0004` | 4 | Scan | DONE | skip Trash |
| `F0005` | 4 | Scan | DONE | cancelable scan |
| `F0006` | 4 | Scan | DONE | progress files/bytes/path |
| `F0007` | 4 | Scan | DONE | hardlink inode dedup |
| `F0008` | 4 | Scan | DONE | apparent vs on-disk size |
| `F0111` | 4 | Cache | DONE | zstd+bincode cache |
| `F0112` | 4 | Cache | DONE | magic+version header |
| `F0113` | 4 | Cache | DONE | stale-by-mtime badge |
| `F0114` | 4 | Cache | DONE | async load/save |
| `F0115` | 4 | Cache | DONE | list recent |
| `F0116` | 4 | Cache | DONE | wipe one/all |
| `F0117` | 4 | Cache | DONE | TTL days |
| `F0118` | 4 | Cache | DONE | LRU eviction |
| `F0181` | 4 | Tree | DONE | NodeId stable |
| `F0182` | 4 | Tree | DONE | parent links |
| `F0183` | 4 | Tree | DONE | pre-sorted children |
| `F0184` | 4 | Tree | DONE | Arc sharing to UI |
| `F0185` | 4 | Tree | DONE | file_count rollup |
| `F0186` | 4 | Tree | DONE | allocated_size |
| `F0187` | 4 | Tree | DONE | apparent_size |
| `F0188` | 4 | Tree | DONE | path_string helper |
| `F0241` | 5 | Sunburst | DONE | 2-ring default |
| `F0242` | 5 | Sunburst | DONE | depth slider 1-4 |
| `F0243` | 5 | Sunburst | DONE | mesh geom cache |
| `F0244` | 5 | Sunburst | DONE | quantized size key |
| `F0245` | 5 | Sunburst | DONE | smooth tessellation |
| `F0246` | 5 | Sunburst | DONE | hover wedge stroke |
| `F0247` | 5 | Sunburst | DONE | tooltip on hover |
| `F0248` | 5 | Sunburst | DONE | center click up |
| `F0291` | 5 | Treemap | DONE | squarified layout |
| `F0292` | 5 | Treemap | DONE | strip layout option |
| `F0293` | 5 | Treemap | DONE | nest depth slider |
| `F0294` | 5 | Treemap | DONE | ext legend chips |
| `F0295` | 5 | Treemap | DONE | legend filter toggle |
| `F0296` | 5 | Treemap | DONE | hover highlight |
| `F0297` | 5 | Treemap | DONE | tile labels |
| `F0298` | 5 | Treemap | DONE | min px cull |
| `F0336` | 5 | Radial | DONE | focus center bubble |
| `F0337` | 5 | Radial | DONE | orbital children |
| `F0338` | 5 | Radial | DONE | size-proportional radius |
| `F0339` | 5 | Radial | DONE | max nodes LOD |
| `F0340` | 5 | Radial | DONE | orbit rings decor |
| `F0341` | 5 | Radial | DONE | hover hand cursor |
| `F0342` | 5 | Radial | DONE | double-click zoom |
| `F0343` | 5 | Radial | DONE | center double-click up |
| `F0371` | 5 | Cascade | DONE | icicle bands |
| `F0372` | 5 | Cascade | DONE | depth rows |
| `F0373` | 5 | Cascade | DONE | width-proportional size |
| `F0374` | 5 | Cascade | DONE | hover tip |
| `F0375` | 5 | Cascade | DONE | click select |
| `F0376` | 5 | Cascade | DONE | double-click zoom |
| `F0377` | 5 | Cascade | DONE | max depth control |
| `F0378` | 5 | Cascade | DONE | row height scale |
| `F0406` | 1 | Sidebar | DONE | volumes list |
| `F0407` | 1 | Sidebar | DONE | refresh volumes |
| `F0408` | 1 | Sidebar | DONE | filter box |
| `F0409` | 1 | Sidebar | DONE | children list capped |
| `F0410` | 1 | Sidebar | DONE | largest files list |
| `F0411` | 1 | Sidebar | DONE | hover sync highlight |
| `F0412` | 1 | Sidebar | DONE | scroll-to hover |
| `F0413` | 1 | Sidebar | DONE | info strip |
| `F0451` | 1 | Navigation | DONE | back history |
| `F0452` | 1 | Navigation | DONE | forward history |
| `F0453` | 1 | Navigation | DONE | up to parent |
| `F0454` | 1 | Navigation | DONE | breadcrumb bar |
| `F0455` | 1 | Navigation | DONE | click crumb |
| `F0456` | 1 | Navigation | DONE | copy path |
| `F0457` | 1 | Navigation | DONE | keyboard arrows |
| `F0458` | 1 | Navigation | DONE | Enter open |
| `F0501` | 2 | Filter | DONE | substring query |
| `F0502` | 2 | Filter | DONE | case-insensitive |
| `F0503` | 2 | Filter | DONE | debounce 150ms |
| `F0504` | 2 | Filter | DONE | regex toggle |
| `F0505` | 2 | Filter | DONE | ext chip multi |
| `F0506` | 2 | Filter | DONE | min size |
| `F0507` | 2 | Filter | DONE | max size |
| `F0508` | 2 | Filter | DONE | only dirs |
| `F0556` | 1 | HoverSelect | DONE | hover id action |
| `F0557` | 1 | HoverSelect | DONE | tooltip name/size/files |
| `F0558` | 1 | HoverSelect | DONE | sidebar row map ancestor |
| `F0559` | 1 | HoverSelect | DONE | hover underline |
| `F0560` | 1 | HoverSelect | DONE | sticky select on click |
| `F0561` | 1 | HoverSelect | DONE | hover≠select |
| `F0562` | 1 | HoverSelect | DONE | multi-select shift |
| `F0563` | 1 | HoverSelect | DONE | range select |
| `F0596` | 3 | ReviewTrash | DONE | right-click toggle review |
| `F0597` | 3 | ReviewTrash | DONE | review count badge |
| `F0598` | 3 | ReviewTrash | DONE | queue bytes sum |
| `F0599` | 3 | ReviewTrash | DONE | confirm delete modal |
| `F0600` | 3 | ReviewTrash | DONE | trash crate move |
| `F0601` | 3 | ReviewTrash | DONE | reveal in Trash |
| `F0602` | 3 | ReviewTrash | DONE | clear queue |
| `F0603` | 3 | ReviewTrash | DONE | export queue list |
| `F0646` | 6 | LargestDup | DONE | largest files async |
| `F0647` | 6 | LargestDup | DONE | largest folders |
| `F0648` | 6 | LargestDup | DONE | oldest large files |
| `F0649` | 6 | LargestDup | DONE | name+size duplicates |
| `F0650` | 6 | LargestDup | DONE | group by ext totals |
| `F0651` | 6 | LargestDup | DONE | export CSV top |
| `F0652` | 6 | LargestDup | DONE | limit N setting |
| `F0653` | 6 | LargestDup | DONE | bounded walk 8k |
| `F0691` | 8 | Volumes | DONE | fast list root and Volumes |
| `F0692` | 8 | Volumes | DONE | full sysinfo refresh |
| `F0693` | 8 | Volumes | DONE | dedupe APFS root+Data |
| `F0694` | 8 | Volumes | DONE | prefer Data mount |
| `F0695` | 8 | Volumes | DONE | skip Preboot/VM |
| `F0696` | 8 | Volumes | DONE | Home shortcut row |
| `F0697` | 8 | Volumes | DONE | used/total label |
| `F0698` | 8 | Volumes | DONE | refresh button |
| `F0731` | 7 | CLI | DONE | space scan |
| `F0732` | 7 | CLI | DONE | space ls |
| `F0733` | 7 | CLI | DONE | space du |
| `F0734` | 7 | CLI | DONE | space find |
| `F0735` | 7 | CLI | DONE | space largest |
| `F0736` | 7 | CLI | DONE | space export json |
| `F0737` | 7 | CLI | DONE | space export csv |
| `F0738` | 7 | CLI | DONE | space cache ls |
| `F0776` | 1 | Shortcuts | DONE | ? shortcuts sheet |
| `F0777` | 1 | Shortcuts | DONE | Cmd+O open |
| `F0778` | 1 | Shortcuts | DONE | Cmd+R rescan |
| `F0779` | 1 | Shortcuts | DONE | Cmd+, settings |
| `F0780` | 1 | Shortcuts | DONE | Cmd+F focus filter |
| `F0781` | 1 | Shortcuts | DONE | Cmd+L focus path |
| `F0782` | 1 | Shortcuts | DONE | Space Quick Look |
| `F0783` | 1 | Shortcuts | DONE | Delete review |
| `F0816` | 9 | A11y | DONE | VoiceOver labels |
| `F0817` | 9 | A11y | DONE | high contrast option |
| `F0818` | 9 | A11y | DONE | focus rings |
| `F0819` | 9 | A11y | DONE | keyboard-only |
| `F0820` | 9 | A11y | DONE | reduce motion |
| `F0821` | 9 | A11y | DONE | min contrast text |
| `F0822` | 9 | A11y | DONE | tooltip delay |
| `F0823` | 9 | A11y | DONE | hit target 24px |
| `F0846` | 9 | i18n | DONE | EN strings |
| `F0847` | 9 | i18n | DONE | RU strings |
| `F0848` | 9 | i18n | DONE | string table |
| `F0849` | 9 | i18n | DONE | plural rules |
| `F0850` | 9 | i18n | DONE | locale sizes Si/IEC |
| `F0851` | 9 | i18n | DONE | date format locale |
| `F0852` | 9 | i18n | DONE | RTL ready stubs |
| `F0853` | 9 | i18n | DONE | language setting |
| `F0871` | 6 | Export | DONE | PNG screenshot view |
| `F0872` | 6 | Export | DONE | SVG sunburst export |
| `F0873` | 6 | Export | DONE | CSV children |
| `F0874` | 6 | Export | DONE | CSV largest |
| `F0875` | 6 | Export | DONE | JSON tree shallow |
| `F0876` | 6 | Export | DONE | JSON tree full |
| `F0877` | 6 | Export | DONE | clipboard TSV |
| `F0878` | 6 | Export | DONE | share sheet macOS |
| `F0906` | 0 | Settings | DONE | settings window |
| `F0907` | 0 | Settings | DONE | persist via json |
| `F0908` | 0 | Settings | DONE | size mode default |
| `F0909` | 0 | Settings | DONE | view default |
| `F0910` | 0 | Settings | DONE | small-file threshold |
| `F0911` | 0 | Settings | DONE | scan threads |
| `F0912` | 0 | Settings | DONE | cache TTL |
| `F0913` | 0 | Settings | DONE | exclude list editor |
| `F0946` | 0 | Perf | DONE | geom cache sunburst |
| `F0947` | 0 | Perf | DONE | geom cache treemap |
| `F0948` | 0 | Perf | DONE | no to_vec idle kids |
| `F0949` | 0 | Perf | DONE | sidebar row cache |
| `F0950` | 0 | Perf | DONE | quantize geom key |
| `F0951` | 0 | Perf | DONE | repaint 100/250ms |
| `F0952` | 0 | Perf | DONE | Arial font light |
| `F0953` | 0 | Perf | DONE | mesh Arc share |
| `F0991` | 9 | Robustness | DONE | panic hook /tmp log |
| `F0992` | 9 | Robustness | DONE | try_node guards |
| `F0993` | 9 | Robustness | DONE | channel try_recv |
| `F0994` | 9 | Robustness | DONE | busy flags |
| `F0995` | 9 | Robustness | DONE | confirm destructive |
| `F0996` | 9 | Robustness | DONE | offline volume |
| `F0997` | 9 | Robustness | DONE | ejected mid-scan |
| `F0998` | 9 | Robustness | DONE | corrupt cache recover |
| `F1031` | 8 | macOS | DONE | Reveal in Finder |
| `F1032` | 8 | macOS | DONE | Quick Look |
| `F1033` | 8 | macOS | DONE | open Terminal here |
| `F1034` | 8 | macOS | DONE | Dock name Space Scanner |
| `F1035` | 8 | macOS | DONE | .app bundle script |
| `F1036` | 8 | macOS | DONE | Info.plist |
| `F1037` | 8 | macOS | DONE | Retina capable |
| `F1038` | 8 | macOS | DONE | APFS Data prefer |
| `F1071` | 9 | Testing | DONE | unit tree path |
| `F1072` | 9 | Testing | DONE | unit filter match |
| `F1073` | 9 | Testing | DONE | unit volume dedupe |
| `F1074` | 9 | Testing | DONE | scan fixture tmp |
| `F1075` | 9 | Testing | DONE | bench quick profile |
| `F1076` | 9 | Testing | DONE | bench normal |
| `F1077` | 9 | Testing | DONE | cli smoke |
| `F1078` | 9 | Testing | DONE | ui smoke manual |
| `F1101` | 0 | Packaging | DONE | cargo run -p space-tools -- run-app |
| `F1102` | 8 | Packaging | DONE | dist SpaceScanner.app |
| `F1103` | 8 | Packaging | DONE | cargo release profile |
| `F1104` | 8 | Packaging | DONE | README run section |
| `F1105` | 8 | Packaging | DONE | LICENSE MIT |
| `F1106` | 8 | Packaging | DONE | gitignore target |
| `F1107` | 8 | Packaging | DONE | ci build stub |
| `F1108` | 8 | Packaging | DONE | version in about |

---

## 3. Feature Backlog (F0001–F1125)

IDs are stable across all 25 categories. `P0` entries carry their wave (`W0–W9`) and current status (`DONE` / `TODO`) from Section 2; contiguous `<Category> enhancement NNN` placeholder slots are grouped per `(Category, Priority)` range without omitting any IDs.

### 3.1 Scan

- `F0001` [P0 · W4 · DONE] parallel work-stealing walk
- `F0002` [P0 · W4 · DONE] same-volume device filter
- `F0003` [P0 · W4 · DONE] skip Spotlight
- `F0004` [P0 · W4 · DONE] skip Trash
- `F0005` [P0 · W4 · DONE] cancelable scan
- `F0006` [P0 · W4 · DONE] progress files/bytes/path
- `F0007` [P0 · W4 · DONE] hardlink inode dedup
- `F0008` [P0 · W4 · DONE] apparent vs on-disk size
- `F0009` [P1] small-file rollup
- `F0010` [P1] thread count setting
- `F0011` [P1] scan pause/resume
- `F0012` [P1] profile Quick/Full/Deep
- `F0013` [P1] exclude path prefixes
- `F0014` [P1] exclude globs
- `F0015` [P1] cloud placeholder skip
- `F0016` [P1] no-follow symlinks
- `F0017` [P1] permission-denied nodes
- `F0018` [P1] ETA from prior index
- `F0019` [P1] expected items from cache
- `F0020` [P1] NFC name normalize
- `F0021` [P2] skip .git option
- `F0022` [P2] skip node_modules option
- `F0023` [P2] FSEvents-triggered rescan
- `F0024` [P2] items/sec meter
- `F0025` [P2] MB/s meter
- `F0026` [P2] checkpoint resume
- `F0027` [P2] depth-limited scan
- `F0028` [P2] Home/Downloads shortcuts
- `F0029` [P2] APFS clone note
- `F0030` [P2] thermal throttle
- `F0031`–`F0110` [P2] Reserved Scan enhancement slots (001–080, 80 items)

### 3.2 Cache

- `F0111` [P0 · W4 · DONE] zstd+bincode cache
- `F0112` [P0 · W4 · DONE] magic+version header
- `F0113` [P0 · W4 · DONE] stale-by-mtime badge
- `F0114` [P0 · W4 · DONE] async load/save
- `F0115` [P0 · W4 · DONE] list recent
- `F0116` [P0 · W4 · DONE] wipe one/all
- `F0117` [P0 · W4 · DONE] TTL days
- `F0118` [P0 · W4 · DONE] LRU eviction
- `F0119` [P1] atomic temp+rename
- `F0120` [P1] cache dir open
- `F0121` [P1] CLI cache ls/rm
- `F0122` [P1] force rescan ignores cache
- `F0123` [P1] corrupt auto-delete
- `F0124` [P1] per-volume key
- `F0125` [P1] show age+disk bytes
- `F0126` [P1] pin entry
- `F0127` [P1] offline open
- `F0128` [P1] compress level
- `F0129` [P1] autosave on quit
- `F0130` [P1] schema fingerprint
- `F0131`–`F0180` [P2] Reserved Cache enhancement slots (001–050, 50 items)

### 3.3 Tree

- `F0181` [P0 · W4 · DONE] NodeId stable
- `F0182` [P0 · W4 · DONE] parent links
- `F0183` [P0 · W4 · DONE] pre-sorted children
- `F0184` [P0 · W4 · DONE] Arc sharing to UI
- `F0185` [P0 · W4 · DONE] file_count rollup
- `F0186` [P0 · W4 · DONE] allocated_size
- `F0187` [P0 · W4 · DONE] apparent_size
- `F0188` [P0 · W4 · DONE] path_string helper
- `F0189` [P1] breadcrumb_names
- `F0190` [P1] largest_files bounded
- `F0191` [P1] try_node safe
- `F0192` [P1] children borrowed slice
- `F0193` [P1] mutate for rescan
- `F0194` [P1] NFC normalize_unicode_names
- `F0195` [P1] optional mtime field
- `F0196` [P1] optional birthtime
- `F0197` [P1] ext index side table
- `F0198` [P1] dir vs file flag
- `F0199` [P1] rollup small bucket node
- `F0200` [P1] root_path stored
- `F0201`–`F0240` [P2] Reserved Tree enhancement slots (001–040, 40 items)

### 3.4 Sunburst

- `F0241` [P0 · W5 · DONE] 2-ring default
- `F0242` [P0 · W5 · DONE] depth slider 1-4
- `F0243` [P0 · W5 · DONE] mesh geom cache
- `F0244` [P0 · W5 · DONE] quantized size key
- `F0245` [P0 · W5 · DONE] smooth tessellation
- `F0246` [P0 · W5 · DONE] hover wedge stroke
- `F0247` [P0 · W5 · DONE] tooltip on hover
- `F0248` [P0 · W5 · DONE] center click up
- `F0249` [P1] click zoom dir
- `F0250` [P1] other wedge
- `F0251` [P1] labels on large arcs
- `F0252` [P1] color by ext
- `F0253` [P1] color by dir hash
- `F0254` [P1] color by age
- `F0255` [P1] % of parent in tip
- `F0256` [P1] keyboard ring nav
- `F0257` [P1] LOD min sweep
- `F0258` [P1] max arcs cap
- `F0259` [P1] max top children
- `F0260` [P1] selection overlay
- `F0261`–`F0290` [P2] Reserved Sunburst enhancement slots (001–030, 30 items)

### 3.5 Treemap

- `F0291` [P0 · W5 · DONE] squarified layout
- `F0292` [P0 · W5 · DONE] strip layout option
- `F0293` [P0 · W5 · DONE] nest depth slider
- `F0294` [P0 · W5 · DONE] ext legend chips
- `F0295` [P0 · W5 · DONE] legend filter toggle
- `F0296` [P0 · W5 · DONE] hover highlight
- `F0297` [P0 · W5 · DONE] tile labels
- `F0298` [P0 · W5 · DONE] min px cull
- `F0299` [P1] max tiles cap
- `F0300` [P1] color by ext
- `F0301` [P1] dir muted option
- `F0302` [P1] double-click zoom
- `F0303` [P1] right-click review
- `F0304` [P1] shared geom cache
- `F0305` [P1] aspect ratio metric
- `F0306` [P1] padding gaps
- `F0307` [P1] animation fade
- `F0308` [P1] empty filtered state
- `F0309` [P1] nesting borders
- `F0310` [P1] selection stroke
- `F0311`–`F0335` [P2] Reserved Treemap enhancement slots (001–025, 25 items)

### 3.6 Radial

- `F0336` [P0 · W5 · DONE] focus center bubble
- `F0337` [P0 · W5 · DONE] orbital children
- `F0338` [P0 · W5 · DONE] size-proportional radius
- `F0339` [P0 · W5 · DONE] max nodes LOD
- `F0340` [P0 · W5 · DONE] orbit rings decor
- `F0341` [P0 · W5 · DONE] hover hand cursor
- `F0342` [P0 · W5 · DONE] double-click zoom
- `F0343` [P0 · W5 · DONE] center double-click up
- `F0344` [P1] labels under bubbles
- `F0345` [P1] selection stroke
- `F0346` [P1] review mark
- `F0347` [P1] angle sort by size
- `F0348` [P1] collision avoid simple
- `F0349` [P1] zoom anim
- `F0350` [P1] empty kids state
- `F0351`–`F0355` [P1] Reserved Radial enhancement slots (001–005, 5 items)
- `F0356`–`F0370` [P2] Reserved Radial enhancement slots (006–020, 15 items)

### 3.7 Cascade

- `F0371` [P0 · W5 · DONE] icicle bands
- `F0372` [P0 · W5 · DONE] depth rows
- `F0373` [P0 · W5 · DONE] width-proportional size
- `F0374` [P0 · W5 · DONE] hover tip
- `F0375` [P0 · W5 · DONE] click select
- `F0376` [P0 · W5 · DONE] double-click zoom
- `F0377` [P0 · W5 · DONE] max depth control
- `F0378` [P0 · W5 · DONE] row height scale
- `F0379` [P1] label truncate
- `F0380` [P1] muted sizes
- `F0381` [P1] selection stroke
- `F0382` [P1] layout children recursive
- `F0383` [P1] take top 64
- `F0384` [P1] min width cull
- `F0385`–`F0390` [P1] Reserved Cascade enhancement slots (001–006, 6 items)
- `F0391`–`F0405` [P2] Reserved Cascade enhancement slots (007–021, 15 items)

### 3.8 Sidebar

- `F0406` [P0 · W1 · DONE] volumes list
- `F0407` [P0 · W1 · DONE] refresh volumes
- `F0408` [P0 · W1 · DONE] filter box
- `F0409` [P0 · W1 · DONE] children list capped
- `F0410` [P0 · W1 · DONE] largest files list
- `F0411` [P0 · W1 · DONE] hover sync highlight
- `F0412` [P0 · W1 · DONE] scroll-to hover
- `F0413` [P0 · W1 · DONE] info strip
- `F0414` [P1] reveal button
- `F0415` [P1] review queue summary
- `F0416` [P1] density compact
- `F0417` [P1] % of parent column
- `F0418` [P1] mtime column
- `F0419` [P1] ext column
- `F0420` [P1] copy path button
- `F0421` [P1] open Terminal
- `F0422` [P1] Quick Look
- `F0423` [P1] recent scans
- `F0424` [P1] stale badge
- `F0425` [P1] spinner busy
- `F0426`–`F0450` [P2] Reserved Sidebar enhancement slots (001–025, 25 items)

### 3.9 Navigation

- `F0451` [P0 · W1 · DONE] back history
- `F0452` [P0 · W1 · DONE] forward history
- `F0453` [P0 · W1 · DONE] up to parent
- `F0454` [P0 · W1 · DONE] breadcrumb bar
- `F0455` [P0 · W1 · DONE] click crumb
- `F0456` [P0 · W1 · DONE] copy path
- `F0457` [P0 · W1 · DONE] keyboard arrows
- `F0458` [P0 · W1 · DONE] Enter open
- `F0459` [P1] Esc up
- `F0460` [P1] Home root
- `F0461` [P1] End largest child
- `F0462` [P1] Cmd+[ back
- `F0463` [P1] Cmd+] forward
- `F0464` [P1] remember per volume
- `F0465` [P1] deep link path
- `F0466` [P1] jump to path paste
- `F0467` [P1] zoom animation
- `F0468` [P1] status focus name
- `F0469` [P1] path in title
- `F0470` [P1] select parent
- `F0471`–`F0500` [P2] Reserved Navigation enhancement slots (001–030, 30 items)

### 3.10 Filter

- `F0501` [P0 · W2 · DONE] substring query
- `F0502` [P0 · W2 · DONE] case-insensitive
- `F0503` [P0 · W2 · DONE] debounce 150ms
- `F0504` [P0 · W2 · DONE] regex toggle
- `F0505` [P0 · W2 · DONE] ext chip multi
- `F0506` [P0 · W2 · DONE] min size
- `F0507` [P0 · W2 · DONE] max size
- `F0508` [P0 · W2 · DONE] only dirs
- `F0509` [P1] only files
- `F0510` [P1] clear all
- `F0511` [P1] active badge count
- `F0512` [P1] jump next match
- `F0513` [P1] jump prev match
- `F0514` [P1] filter persists
- `F0515` [P1] query history
- `F0516` [P1] saved presets
- `F0517` [P1] exclude matching
- `F0518` [P1] invert filter
- `F0519` [P1] size presets
- `F0520` [P1] ext from legend
- `F0521`–`F0555` [P2] Reserved Filter enhancement slots (001–035, 35 items)

### 3.11 HoverSelect

- `F0556` [P0 · W1 · DONE] hover id action
- `F0557` [P0 · W1 · DONE] tooltip name/size/files
- `F0558` [P0 · W1 · DONE] sidebar row map ancestor
- `F0559` [P0 · W1 · DONE] hover underline
- `F0560` [P0 · W1 · DONE] sticky select on click
- `F0561` [P0 · W1 · DONE] hover≠select
- `F0562` [P0 · W1 · DONE] multi-select shift
- `F0563` [P0 · W1 · DONE] range select
- `F0564` [P1] select all visible
- `F0565` [P1] deselect
- `F0566` [P1] marquee treemap
- `F0567` [P1] keyboard move selection
- `F0568` [P1] show path on hover
- `F0569` [P1] % parent on hover
- `F0570` [P1] open with default
- `F0571` [P1] get info panel
- `F0572`–`F0575` [P1] Reserved HoverSelect enhancement slots (001–004, 4 items)
- `F0576`–`F0595` [P2] Reserved HoverSelect enhancement slots (005–024, 20 items)

### 3.12 ReviewTrash

- `F0596` [P0 · W3 · DONE] right-click toggle review
- `F0597` [P0 · W3 · DONE] review count badge
- `F0598` [P0 · W3 · DONE] queue bytes sum
- `F0599` [P0 · W3 · DONE] confirm delete modal
- `F0600` [P0 · W3 · DONE] trash crate move
- `F0601` [P0 · W3 · DONE] reveal in Trash
- `F0602` [P0 · W3 · DONE] clear queue
- `F0603` [P0 · W3 · DONE] export queue list
- `F0604` [P1] add all filtered
- `F0605` [P1] add similar ext
- `F0606` [P1] add folder subtree
- `F0607` [P1] safe delete details
- `F0608` [P1] undo via Finder
- `F0609` [P1] skip locked files
- `F0610` [P1] partial failure report
- `F0611` [P1] dry-run delete
- `F0612` [P1] review sort by size
- `F0613` [P1] review sort by path
- `F0614` [P1] remove from queue
- `F0615` [P1] select in view from queue
- `F0616`–`F0645` [P2] Reserved ReviewTrash enhancement slots (001–030, 30 items)

### 3.13 LargestDup

- `F0646` [P0 · W6 · DONE] largest files async
- `F0647` [P0 · W6 · DONE] largest folders
- `F0648` [P0 · W6 · DONE] oldest large files
- `F0649` [P0 · W6 · DONE] name+size duplicates
- `F0650` [P0 · W6 · DONE] group by ext totals
- `F0651` [P0 · W6 · DONE] export CSV top
- `F0652` [P0 · W6 · DONE] limit N setting
- `F0653` [P0 · W6 · DONE] bounded walk 8k
- `F0654` [P1] reload on focus
- `F0655` [P1] empty state
- `F0656` [P1] open item
- `F0657` [P1] add to review
- `F0658` [P1] dedupe soft link warn
- `F0659` [P1] hash later stub
- `F0660` [P1] size histogram
- `F0661` [P1] ext pie list
- `F0662` [P1] consumers table
- `F0663` [P1] refresh button
- `F0664` [P1] min size gate
- `F0665` [P1] same-folder dups only
- `F0666`–`F0690` [P2] Reserved LargestDup enhancement slots (001–025, 25 items)

### 3.14 Volumes

- `F0691` [P0 · W8 · DONE] fast list root and Volumes
- `F0692` [P0 · W8 · DONE] full sysinfo refresh
- `F0693` [P0 · W8 · DONE] dedupe APFS root+Data
- `F0694` [P0 · W8 · DONE] prefer Data mount
- `F0695` [P0 · W8 · DONE] skip Preboot/VM
- `F0696` [P0 · W8 · DONE] Home shortcut row
- `F0697` [P0 · W8 · DONE] used/total label
- `F0698` [P0 · W8 · DONE] refresh button
- `F0699` [P1] eject warn
- `F0700` [P1] offline grey
- `F0701` [P1] external badge
- `F0702` [P1] network badge
- `F0703` [P1] free space bar
- `F0704` [P1] click to scan
- `F0705` [P1] last scanned age
- `F0706` [P1] capacity sort
- `F0707` [P1] name sort
- `F0708` [P1] hide system synthetics
- `F0709` [P1] boot volume mark
- `F0710` [P1] volumes loading spinner
- `F0711`–`F0730` [P2] Reserved Volumes enhancement slots (001–020, 20 items)

### 3.15 CLI

- `F0731` [P0 · W7 · DONE] space scan
- `F0732` [P0 · W7 · DONE] space ls
- `F0733` [P0 · W7 · DONE] space du
- `F0734` [P0 · W7 · DONE] space find
- `F0735` [P0 · W7 · DONE] space largest
- `F0736` [P0 · W7 · DONE] space export json
- `F0737` [P0 · W7 · DONE] space export csv
- `F0738` [P0 · W7 · DONE] space cache ls
- `F0739` [P1] space cache rm
- `F0740` [P1] progress TTY
- `F0741` [P1] --profile quick
- `F0742` [P1] --profile full
- `F0743` [P1] --json
- `F0744` [P1] --apparent
- `F0745` [P1] --on-disk
- `F0746` [P1] --depth
- `F0747` [P1] --threads
- `F0748` [P1] exit codes
- `F0749` [P1] man-like --help
- `F0750` [P1] color TTY
- `F0751`–`F0775` [P2] Reserved CLI enhancement slots (001–025, 25 items)

### 3.16 Shortcuts

- `F0776` [P0 · W1 · DONE] ? shortcuts sheet
- `F0777` [P0 · W1 · DONE] Cmd+O open
- `F0778` [P0 · W1 · DONE] Cmd+R rescan
- `F0779` [P0 · W1 · DONE] Cmd+, settings
- `F0780` [P0 · W1 · DONE] Cmd+F focus filter
- `F0781` [P0 · W1 · DONE] Cmd+L focus path
- `F0782` [P0 · W1 · DONE] Space Quick Look
- `F0783` [P0 · W1 · DONE] Delete review
- `F0784` [P1] Cmd+Backspace trash
- `F0785` [P1] Cmd+1..4 views
- `F0786` [P1] Cmd+Z undo nav
- `F0787` [P1] Cmd+Shift+C copy path
- `F0788` [P1] Cmd+Shift+R reveal
- `F0789` [P1] F5 refresh vols
- `F0790` [P1] / focus filter
- `F0791` [P1] Esc clear
- `F0792` [P1] Tab cycle panels
- `F0793` [P1] Shift+Tab reverse
- `F0794` [P1] arrow nav view
- `F0795` [P1] enter zoom
- `F0796`–`F0815` [P2] Reserved Shortcuts enhancement slots (001–020, 20 items)

### 3.17 A11y

- `F0816` [P0 · W9 · DONE] VoiceOver labels
- `F0817` [P0 · W9 · DONE] high contrast option
- `F0818` [P0 · W9 · DONE] focus rings
- `F0819` [P0 · W9 · DONE] keyboard-only
- `F0820` [P0 · W9 · DONE] reduce motion
- `F0821` [P0 · W9 · DONE] min contrast text
- `F0822` [P0 · W9 · DONE] tooltip delay
- `F0823` [P0 · W9 · DONE] hit target 24px
- `F0824` [P1] screen reader path
- `F0825` [P1] announce selection
- `F0826` [P1] announce scan done
- `F0827` [P1] large text density
- `F0828` [P1] colorblind palettes
- `F0829` [P1] no color-only state
- `F0830` [P1] aria-like egui ids
- `F0831`–`F0835` [P1] Reserved A11y enhancement slots (001–005, 5 items)
- `F0836`–`F0845` [P2] Reserved A11y enhancement slots (006–015, 10 items)

### 3.18 i18n

- `F0846` [P0 · W9 · DONE] EN strings
- `F0847` [P0 · W9 · DONE] RU strings
- `F0848` [P0 · W9 · DONE] string table
- `F0849` [P0 · W9 · DONE] plural rules
- `F0850` [P0 · W9 · DONE] locale sizes Si/IEC
- `F0851` [P0 · W9 · DONE] date format locale
- `F0852` [P0 · W9 · DONE] RTL ready stubs
- `F0853` [P0 · W9 · DONE] language setting
- `F0854` [P1] fallback EN
- `F0855` [P1] scan status localized
- `F0856`–`F0865` [P1] Reserved i18n enhancement slots (001–010, 10 items)
- `F0866`–`F0870` [P2] Reserved i18n enhancement slots (011–015, 5 items)

### 3.19 Export

- `F0871` [P0 · W6 · DONE] PNG screenshot view
- `F0872` [P0 · W6 · DONE] SVG sunburst export
- `F0873` [P0 · W6 · DONE] CSV children
- `F0874` [P0 · W6 · DONE] CSV largest
- `F0875` [P0 · W6 · DONE] JSON tree shallow
- `F0876` [P0 · W6 · DONE] JSON tree full
- `F0877` [P0 · W6 · DONE] clipboard TSV
- `F0878` [P0 · W6 · DONE] share sheet macOS
- `F0879` [P1] report HTML
- `F0880` [P1] email body stub
- `F0881` [P1] AirDrop file
- `F0882` [P1] export selection only
- `F0883` [P1] export review queue
- `F0884` [P1] include hidden meta
- `F0885` [P1] pretty JSON
- `F0886` [P1] gzip JSON
- `F0887` [P1] schema version in export
- `F0888`–`F0890` [P1] Reserved Export enhancement slots (001–003, 3 items)
- `F0891`–`F0905` [P2] Reserved Export enhancement slots (004–018, 15 items)

### 3.20 Settings

- `F0906` [P0 · W0 · DONE] settings window
- `F0907` [P0 · W0 · DONE] persist via json
- `F0908` [P0 · W0 · DONE] size mode default
- `F0909` [P0 · W0 · DONE] view default
- `F0910` [P0 · W0 · DONE] small-file threshold
- `F0911` [P0 · W0 · DONE] scan threads
- `F0912` [P0 · W0 · DONE] cache TTL
- `F0913` [P0 · W0 · DONE] exclude list editor
- `F0914` [P1] skiplist editor
- `F0915` [P1] color mode
- `F0916` [P1] density
- `F0917` [P1] font size
- `F0918` [P1] start on login stub
- `F0919` [P1] check updates stub
- `F0920` [P1] telemetry off
- `F0921` [P1] reset defaults
- `F0922` [P1] import settings
- `F0923` [P1] export settings
- `F0924` [P1] keybinding editor
- `F0925` [P1] feature flags struct
- `F0926`–`F0945` [P2] Reserved Settings enhancement slots (001–020, 20 items)

### 3.21 Perf

- `F0946` [P0 · W0 · DONE] geom cache sunburst
- `F0947` [P0 · W0 · DONE] geom cache treemap
- `F0948` [P0 · W0 · DONE] no to_vec idle kids
- `F0949` [P0 · W0 · DONE] sidebar row cache
- `F0950` [P0 · W0 · DONE] quantize geom key
- `F0951` [P0 · W0 · DONE] repaint 100/250ms
- `F0952` [P0 · W0 · DONE] Arial font light
- `F0953` [P0 · W0 · DONE] mesh Arc share
- `F0954` [P1] largest walk bound
- `F0955` [P1] cap sidebar 120
- `F0956` [P1] cap arcs 160
- `F0957` [P1] cap tiles 180
- `F0958` [P1] pre-sorted children
- `F0959` [P1] Arc SpaceTree
- `F0960` [P1] string-less hits
- `F0961` [P1] batch mesh tris
- `F0962` [P1] avoid per-frame format
- `F0963` [P1] filter hash key
- `F0964` [P1] airbug benches
- `F0965` [P1] docs PERF.md
- `F0966`–`F0990` [P2] Reserved Perf enhancement slots (001–025, 25 items)

### 3.22 Robustness

- `F0991` [P0 · W9 · DONE] panic hook /tmp log
- `F0992` [P0 · W9 · DONE] try_node guards
- `F0993` [P0 · W9 · DONE] channel try_recv
- `F0994` [P0 · W9 · DONE] busy flags
- `F0995` [P0 · W9 · DONE] confirm destructive
- `F0996` [P0 · W9 · DONE] offline volume
- `F0997` [P0 · W9 · DONE] ejected mid-scan
- `F0998` [P0 · W9 · DONE] corrupt cache recover
- `F0999` [P1] UTF-8 lossy paths
- `F1000` [P1] window size remember
- `F1001` [P1] crash on GPU recover
- `F1002` [P1] error toast
- `F1003` [P1] status error color
- `F1004` [P1] retry button
- `F1005` [P1] cancel everywhere
- `F1006` [P1] timeout volumes list
- `F1007` [P1] empty tree state
- `F1008` [P1] permission copy first-run
- `F1009` [P1] jetsam-safe fonts
- `F1010` [P1] no unbounded Vec grow
- `F1011`–`F1030` [P2] Reserved Robustness enhancement slots (001–020, 20 items)

### 3.23 macOS

- `F1031` [P0 · W8 · DONE] Reveal in Finder
- `F1032` [P0 · W8 · DONE] Quick Look
- `F1033` [P0 · W8 · DONE] open Terminal here
- `F1034` [P0 · W8 · DONE] Dock name Space Scanner
- `F1035` [P0 · W8 · DONE] .app bundle script
- `F1036` [P0 · W8 · DONE] Info.plist
- `F1037` [P0 · W8 · DONE] Retina capable
- `F1038` [P0 · W8 · DONE] APFS Data prefer
- `F1039` [P1] Trash integration
- `F1040` [P1] bookmark-free paths
- `F1041` [P1] security-scoped later
- `F1042` [P1] notary stub
- `F1043` [P1] Sparkle-less version
- `F1044` [P1] menu bar extras stub
- `F1045` [P1] Services menu stub
- `F1046` [P1] drag folder onto app
- `F1047` [P1] Handoff stub
- `F1048` [P1] Touch Bar stub
- `F1049` [P1] stage manager size
- `F1050` [P1] fullscreen support
- `F1051`–`F1070` [P2] Reserved macOS enhancement slots (001–020, 20 items)

### 3.24 Testing

- `F1071` [P0 · W9 · DONE] unit tree path
- `F1072` [P0 · W9 · DONE] unit filter match
- `F1073` [P0 · W9 · DONE] unit volume dedupe
- `F1074` [P0 · W9 · DONE] scan fixture tmp
- `F1075` [P0 · W9 · DONE] bench quick profile
- `F1076` [P0 · W9 · DONE] bench normal
- `F1077` [P0 · W9 · DONE] cli smoke
- `F1078` [P0 · W9 · DONE] ui smoke manual
- `F1079` [P1] cache roundtrip
- `F1080` [P1] NFC roundtrip
- `F1081` [P1] geom key stable
- `F1082` [P1] review toggle
- `F1083` [P1] format_bytes
- `F1084` [P1] largest bounded
- `F1085` [P1] dedupe volumes test
- `F1086` [P1] skiplist unit
- `F1087` [P1] size mode unit
- `F1088` [P1] export csv unit
- `F1089` [P1] settings serde
- `F1090` [P1 · DONE] roadmap DONE checks
- `F1091`–`F1100` [P2] Reserved Testing enhancement slots (001–010, 10 items)

### 3.25 Packaging

- `F1101` [P0 · W0 · DONE] cargo run -p space-tools -- run-app
- `F1102` [P0 · W8 · DONE] dist SpaceScanner.app
- `F1103` [P0 · W8 · DONE] cargo release profile
- `F1104` [P0 · W8 · DONE] README run section
- `F1105` [P0 · W8 · DONE] LICENSE MIT
- `F1106` [P0 · W8 · DONE] gitignore target
- `F1107` [P0 · W8 · DONE] ci build stub
- `F1108` [P0 · W8 · DONE] version in about
- `F1109` [P1] dmg stub
- `F1110` [P1] brew cask stub
- `F1111` [P1] appcast stub
- `F1112` [P1] codesign script
- `F1113` [P1] notarize script
- `F1114` [P1] icon .icns stub
- `F1115` [P1] screenshots folder
- `F1116`–`F1120` [P1] Reserved Packaging enhancement slots (001–005, 5 items)
- `F1121`–`F1125` [P2] Reserved Packaging enhancement slots (006–010, 5 items)

---

**Total: 1,125 features (472 named + 653 reserved slots) across 25 categories.**
