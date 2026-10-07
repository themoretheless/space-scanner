# Space Scanner: interaction model

The primary job is to answer “what is taking up space, and what do I want to remove?” The secondary job is to find disposable app data. Screens follow these jobs rather than the rendering engine.

## Journey

1. Scan: choose personal files, a specific folder, or a disk. Saved results are explicitly historical. Opening one is immediate; updating it is an explicit rescan.
2. Disk usage: show the current folder, its children ordered by size, and a visual map of the same children. Keep location and filtering separate from global navigation.
3. Select: a single click selects an item and shows its details. A folder opens with a second explicit action or double-click. Selection does not imply removal.
4. Review: collect manually chosen files and explicitly selected cleanup candidates. Opening Review must not start a cleanup scan or add new selections.
5. Move to Trash: show paths, sizes and removal scope, then confirm. Report actual successes and failures. Selected bytes are not a prediction of newly available space.
6. Update: refresh the affected folder and disk capacity. Explain that Trash must be emptied in Finder to release space. Snapshot deletion uses its own permanent-removal confirmation.

## Screen responsibilities

Scan has one primary choice, a specific-folder alternative, disk targets, and saved results. Cleanup is a secondary entry point.

Disk usage is the main workspace. The size-ranked list is the dependable navigation surface; the map explains proportions. The inspector appears when an item is selected. Its primary action for folders should be Open folder; Show in Finder verifies contents; Add to review is a secondary decision. Avoid duplicating largest-child lists in the inspector and contents pane.

Cleanup discovers candidates. Category, source application, size and consequences belong together. A rule classification is guidance, not a guarantee of safety. Scanning and selecting are different actions. Report-only results cannot enter the removal selection.

Review is a separate destination. It preserves selections while switching between disk usage and cleanup. Allow removing individual selections and clearing all. Show both sources explicitly and deduplicate overlapping paths before reporting totals or deleting.

## Layout and input

Global navigation is one 52-point row: brand, the three destinations (Scan, Disk usage, Cleanup), the Review entry once something is selected, and a More menu. Location and search always use a separate 48-point workspace toolbar. Scan progress uses its own row. Below 900 points the Map workspace becomes List or Map. All workspace styles retain the selected-item actions at narrow widths through a scrollable drawer.

Width classes (`app/ui/mod.rs`):

- wide, 1180 points and up: size-ranked contents (400; 320 at medium widths) beside the map, details in a right panel only when an item is selected;
- medium, 900 to 1180: contents beside the map, details as a drawer over the map;
- narrow, under 900: one pane at a time, details as a drawer. Cleanup hides its summary panel below 1000.

Map style and size accounting are two compact text menus in a toolbar above the map, outside its interactive area. Segmented choices have no outer frame; only the current choice has a background. A single dismissible hint sits at the bottom of the map. Paths truncate with full-path tooltips. Essential actions remain reachable by keyboard and visible controls. Nothing is smaller than 12 points; the type scale and spacing live in `theme::type_scale` and `theme::metrics`, shared widgets in `widgets.rs`.

The same selection and open semantics apply to the map, the contents list and largest-file results: single click selects, double-click opens, command-click adds to review. Hover previews do not change the persistent selection or move keyboard focus. The parent action and breadcrumbs are explicit; breadcrumbs shorten from the left. Search states its scope and result count and offers a way to clear.

## States and language

Distinguish a fresh scan, a saved result, a partial/cancelled scan and inaccessible content. Scan progress is indeterminate unless measured against an earlier index; do not present that estimate as an exact percentage. Errors remain visible with a useful recovery action.

For a folder scan, disk usage outside the folder is expected. Do not attribute its difference from total volume usage to snapshots. For a whole-volume scan, unindexed space can include permissions, snapshots and filesystem accounting; show these as possibilities.

## Implementation status

The interface layer lives in `crates/space-app/src/app/ui/` (`shell`, `scan`, `usage`, `cleanup`, `review_page`, `dialogs`); `app.rs` keeps state, background work and key handling. Implemented: one-row app bar; one scrollable Scan page across palettes, with a home-folder primary action, specific-folder alternative, all disks and individually actionable saved results; Disk usage with contents list, full-height map, selection-only details and no second file list under the map; Cleanup with compact title and a separate action row, risk filter, category table and a selection summary; Review as its own page with per-item removal, overlap-aware totals and a confirmation that lists every path; no free-space prediction and no cleanup preselection; local snapshots behind their own permanent-removal confirmation.

Checked headless at 1280 × 800, 1000 × 700, 760 × 560 and 720 × 520 with `scripts/ui-shots.sh` (Xvfb with software Vulkan). It does not replace a check on macOS.

Remaining refinements: richer scan coverage and outcome presentation; accessible selection controls for painted rows. Icons are painter-drawn vectors (`icons.rs`); the interface is localised English/Russian (`i18n.rs`, switch in the ⋯ menu, default from the system locale).

## Interface revision

Start and Review share one interaction model across palettes. Disk occupancy is a small meter, never a decorative scan-progress ring. Review prioritizes paths and removal scope, without a second giant total or proportional artwork. Page titles use 18-point type; prominent cleanup totals use at most 22 points. Buttons and filters share a five-point corner radius and 30-point control height. Rule classifications are text status, distinct from actions. Long names and descriptions truncate with full text on hover; category counts use a second line so they cannot overlap source labels. Shortcut lists scroll to retain their Close action in short windows.
