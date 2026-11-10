# UI smoke checklist (manual)

Run before a release, on macOS and one Linux desktop, in English and Russian
(Settings → Language), dark and light. `scripts/ui-shots.sh <dir>` takes
screenshots of most pages headlessly; the items below still need a person.
⌘ means Ctrl on Linux and Windows.

## Start and Overview
- [ ] First launch opens Overview: disk card, next steps, saved scans.
- [ ] Saved scans show size, age and a date in the language's format
      (RU `14.10.2026 14:05`, EN `Oct 14, 2026 14:05`); Open and Rescan work.
- [ ] Drop a folder on the window: it scans; a second drop while busy is refused.

## Scan
- [ ] ⌘O picks a folder; progress shows files and bytes; Esc cancels and
      the page offers Try again.
- [ ] Unplug a USB disk while it is being scanned: the page says "The disk
      was disconnected during the scan" with Scan again / Overview, an
      error toast appears, and the disk leaves the Overview list. During a
      rescan of an open result the toast carries Scan again instead.
- [ ] With an exclude list set, the status line counts the excluded folders
      while scanning and after; the excluded folders are absent from the tree.

## Disk usage (List and map, Columns, Table)
- [ ] Contents list: click selects one item; details pane opens; Esc closes.
- [ ] ⌘-click adds to and takes out of the selection; Shift-click and
      Shift+↑/↓ select a range in list order; the details pane shows the
      count, the total (nested items once), Add selected to Trash list,
      Copy paths, Clear selection. Maps highlight every selected item.
- [ ] ⌘-click / Shift-click on a map tile, ring or band selects the same way.
- [ ] ⌘⇧C copies every selected path (one per line).
- [ ] T adds the selection to the Trash list, with Undo in the toast.
- [ ] Space opens Quick Look on macOS; on Linux/Windows a file opens with
      its default app and a folder shows a notification.
- [ ] Context menu: Open Terminal here opens a terminal in that folder
      (the parent for a file); Share… copies the path and reveals it.
- [ ] Largest files → Oldest: files of at least 100 MB (chips for 10 MB …
      10 GB), oldest first, with the age; hover shows path and date; click
      selects, double-click reveals, context menu adds to the Trash list.
- [ ] Largest files → Duplicates: groups show name, copies, size each and
      the extra bytes; click selects every copy; Add extras to Trash list
      queues all but the newest copy of each group (Undo in the toast).
- [ ] Largest files → Types: extensions of the folder in view with size,
      file count and a share bar; follows navigation and the size mode.
- [ ] Reports show a spinner while computing and come back instantly when
      switching between them for the same folder.
- [ ] ⌘L (or the button after the breadcrumb) turns the breadcrumb into a
      path field: a folder in the scan opens, a file opens its folder and is
      selected, `~` expands, a folder outside the scan offers "Scan it",
      a missing path shows an error, Esc cancels.
- [ ] Filter bar: All/Folders/Files, ≥ 100 MB, ≥ 1 GB, Max size, extension
      chips from the map legend; "N of M shown" updates.
- [ ] `.*` chip: the query becomes a case-insensitive regex (`^img_\d+`);
      an invalid pattern shows "Invalid regular expression" (hover: reason)
      and lists nothing; turning it off restores substring search.
- [ ] Everywhere / Folder search; "Add all N to Trash list".
- [ ] Map options: depth sliders; Rectangles → Tile layout: Squarified /
      Ordered strips; the choice survives a restart.
- [ ] Export ▾ → Save map as PNG… writes the map area only;
      Save rings as SVG… (Rings view) opens in a browser with one wedge per
      item and hover titles.
- [ ] Narrow window: icon rail, List | Map switch, details as a bottom sheet.

## Cleanup
- [ ] Check finds junk; Select all safe; protected items never selectable.

## Trash list
- [ ] Items listed with sizes; Move to Trash asks for confirmation; Enter
      confirms; result toast; Reveal Trash.

## Time Machine snapshots (macOS)
- [ ] Snapshot list shows dates in the language's format; deleting asks first.

## Settings and shortcuts
- [ ] ⌘, opens Settings: appearance, Contrast → High contrast (black/white
      text, strong borders, opaque selection), layout, units, motion,
      language; changes apply at once and persist.
- [ ] Settings → Scanning: small-file threshold (Off … 32 MB, default
      512 KB), Threads (Auto, 1–32), Skip these folders with Add folder…
      (picker), Add pattern (Enter works, duplicates ignored) and Remove.
      Non-default options: the caption says custom scans are not saved, and
      the next scan neither opens nor writes a saved result.
- [ ] `?` lists shortcuts, including Space (Quick Look), T, ⌘L, ⌘-click,
      Shift-click.

## Accessibility
- [ ] VoiceOver (macOS) / Orca: sidebar items, toolbar icon buttons, list
      rows (name, kind, size, selected, on the Trash list) and map items
      (name, size) are announced.
- [ ] Keyboard only: Tab / Shift-Tab reaches sidebar, toolbar, filter bar,
      list and details; a 2 px accent ring marks focus; Tab onto a row
      selects it, ↑/↓ move, Enter opens, Space previews, T lists.
- [ ] No icon button, sidebar item or row checkbox is smaller than 24 px.
- [ ] High contrast in dark and light: every text readable on every panel.
