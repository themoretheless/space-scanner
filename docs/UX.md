# Space Scanner: interaction model

The primary job is to answer “my disk is full: what is taking space, and what can I remove safely?” The secondary job is to find disposable app data. Screens follow that job, not the rendering engine.

## Journey

1. **Overview.** Start from a diagnosis, not a question: how full the startup disk is, how much of it the home folder took at the last scan, and the two ways to get space back (see what takes space, clear caches and logs). Saved scans and other disks sit below.
2. **Disk usage.** The current folder, its children ordered by size, and a map of the same children. Location, search and filters live in the workspace, never in global navigation.
3. **Select.** A single click selects an item and shows its details. A folder opens with Enter, a double-click, or Open folder. Selection does not imply removal.
4. **Trash list.** Items are added explicitly: the row checkbox, Space, ⌘-click, Shift-click for a range, “Add all N” for whole-scan search results, or Select all safe in Cleanup. Every addition shows a notification with Undo.
5. **Move to Trash.** The Trash list page is the only route to the confirmation. The confirmation lists every path and says what will be skipped. Nothing else removes files.
6. **Result.** The page then shows what actually happened: items moved, bytes now in the Trash, items that were not moved and why, and current free space. Emptying the Trash is the user's step in Finder; the app does not predict the space it will free.

## Navigation

A left sidebar holds the four destinations and Settings:

- **Overview** — disk health, actions, recent scans, other disks.
- **Disk usage** — enabled once a scan exists or is running; its second line names the root and its size.
- **Cleanup** — its second line says “Not checked yet”, “Checking…” or how much was found.
- **Trash list** — always present; a count badge and size when it has items, “Empty” otherwise.
- **Settings** (⌘,) — appearance, Disk usage layout, sizes, language, the map hint, links to Cleanup rules and shortcuts.

The startup disk's free space stays visible at the bottom of the sidebar. Below 1000 pt the sidebar becomes a 60 pt icon rail with hover labels and the badge.

## Disk usage workspace

- **Toolbar (56 pt):** Back, Forward, Up, breadcrumbs that shorten from the left (current folder always shown, with its size), search, the search scope (Folder | Everywhere), and the layout switch (List and map | Columns | Table).
- **Filter bar (44 pt), shared by every layout:** All / Folders / Files, ≥ 100 MB, ≥ 1 GB, the extension picked on the map legend, a count of what is shown, Clear filters, and “Add all N to Trash list” for whole-scan results.
- **Status footer (32 pt):** items and size indexed, a “Saved … · files changed” badge for cached results, Rescan ⌘R, and the share of the disk outside the scan with an explanation.
- **Layouts:** List and map (size-ranked tree beside the map), Columns (Finder-style), Table (dense tree table). Each layout works in both appearances.
- **Details:** a right panel at 1240 pt and up; a drawer at medium widths; a bottom sheet over a scrim below 960 pt. Esc closes details before anything else.
- **Map controls:** map style and size accounting sit above the map, outside its drawing area: segmented when there is room, compact pickers when the map is narrow.

Search in a folder filters that folder. Search Everywhere walks the whole scan and lists matches flat, largest first, each with its folder; the filter bar sums them and offers adding them all to the Trash list.

## Safety

- System and personal-data folders (the never-touch list) cannot enter the Trash list by any route; the inspector says why, and keyboard routes show a notification.
- Rule matches are shown as guidance (“Safe · Build output”), never as a guarantee.
- Cleanup never preselects anything and never starts by itself: the first check is an explicit button. Report-only findings cannot be selected. Browser cookies and history are off by default.
- Snapshot deletion is permanent and has its own confirmation with a danger button. Trash moves use the normal primary button, because they can be undone.
- A rescan of the same folder keeps the previous result open and dimmed until the new one is ready, and carries the Trash list and the location over by path. Cancelling keeps the old result.

## States and feedback

Every result and error reaches the user as a notification in the bottom-right corner, at every width and on every page: additions with Undo, copy, rescans, a stale saved result with Rescan, failures with their reason. Scan states: scanning (files, bytes, current path, Cancel), opening a saved result (no Cancel), stopped (Scan again, Another folder, Overview), failed (the error, Try again).

Distinguish a fresh scan, a saved result, a partial or cancelled scan and inaccessible content. Scan progress is indeterminate unless measured against an earlier index; do not present that estimate as an exact percentage.

## Keyboard

Keys never act behind a focused text field. Esc closes the innermost thing first (dialog, details, search) and only then goes up a folder; it never switches pages.

| Keys | Action |
|------|--------|
| ↑ ↓, → ← | Move through the list; open or close a folder |
| Enter | Open a folder, or show a file in Finder |
| Space | Add to the Trash list, or take out |
| Shift-click, ⌘-click | Add a range, add one item |
| ⌘F or / | Search |
| ⌘R, ⌘⇧R | Rescan; rescan the selected folder |
| ⌘⌫ | Open the Trash list with the selection |
| ⌘J | Cleanup |
| ⌘1–4 | Map style |
| ⌘, | Settings |
| ? | Shortcuts |

## Visual language

Graphite (dark) or paper (light) ground with one amber accent; teal for “safe”, red only for protected items, errors and permanent deletion. Geist Regular, Medium and SemiBold (bundled, with Cyrillic) and Geist Mono for sizes. Type scale 12 / 13 / 14 / 16 / 18 / 22 / 28; nothing smaller than 12 pt. Controls are 32 pt (chips 28 pt) with 6 pt corners; cards 10 pt; dialogs 14 pt. Tokens live in `theme::type_scale` and `theme::metrics`, shared widgets in `widgets.rs`, icons are painter-drawn in `icons.rs`. Dialogs are modal over a scrim.

## Implementation

`crates/space-app/src/app.rs` owns state, workers and keys; `app/ui/` only draws: `shell` (sidebar, toolbar, filter bar, footer), `overview`, `scan` (scan states), `usage` (List and map), `layouts` (Columns, Table), `cleanup`, `review_page` (Trash list and result), `dialogs` (Settings, shortcuts, confirmation, snapshots, rules), `toast` (notifications). Appearance and layout are separate settings; old “skin” settings migrate once.

Checked headless at 1280 × 800, 1000 × 700 and 760 × 560 with `scripts/ui-shots.sh` (Xvfb with software Vulkan). It does not replace a check on macOS. The interface is localised English and Russian (`i18n.rs`); a test fails when a string handed to a translating call has no Russian entry.
