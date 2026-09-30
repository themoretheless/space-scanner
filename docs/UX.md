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

Global navigation contains Scan, Disk usage, Cleanup and a Review action when items are selected. Location, search and scan progress use independent rows that can wrap. Map style and size-accounting options are secondary settings rather than four equally prominent tasks.

At wide widths, show the size-ranked contents beside the map; open details only on selection. At narrow widths, switch between list and map, with details in a drawer. Avoid simultaneously reserving two minimum-width sidebars. Paths truncate with full-path tooltips. Essential actions remain reachable by keyboard and visible controls.

The same selection and open semantics must apply to the map, contents list and largest-file results. Hover previews do not change the persistent selection or move keyboard focus. Provide an explicit parent action and clickable breadcrumbs. Search must state its scope and show active filters and result counts.

## States and language

Distinguish a fresh scan, a saved result, a partial/cancelled scan and inaccessible content. Scan progress is indeterminate unless measured against an earlier index; do not present that estimate as an exact percentage. Errors remain visible with a useful recovery action.

For a folder scan, disk usage outside the folder is expected. Do not attribute its difference from total volume usage to snapshots. For a whole-volume scan, unindexed space can include permissions, snapshots and filesystem accounting; show these as possibilities.

## Implementation status

Implemented: compact Scan page; explicit disk scan actions; readable saved rows; separate Review destination without discovery; consistent map/list single-click selection and double-click opening; explicit folder-open action; contextual details with a floating inspector below 1100 points; Largest files in the main list (100 results, bounded search of 8,000 folders); no cleanup preselection; clear-selection action; per-finding cleanup paths and removal from Review; compact map-style menu; calmer typography and panel colors; independent toolbar rows; coalesced parent/child and cross-source selection totals; successful file removals excluded from the subsequent cleanup batch; no free-space-after-Trash prediction; no snapshot attribution for ordinary folder scan gaps.

Implemented compact layout: below 1000 points, List and Map each use the full workspace; details are constrained and scrollable; Review uses the central workspace at every width; Cleanup hides the side queue below 1000 points. Manually checked a wide window, an approximately 820-point window, and the 720 × 520 minimum: list, map, details and Review controls remain reachable.

Remaining refinements: richer scan coverage and outcome presentation; accessible selection controls for painted rows. Largest-file search remains bounded and is labelled accordingly. No files were deleted during UI verification.
