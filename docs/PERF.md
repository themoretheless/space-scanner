# Scan performance

## September 2026 optimization

The scan now assigns directory IDs before enqueueing work. Records store a parent
ID instead of a copied full path, and tree construction no longer hashes paths or
reconstructs missing ancestors. Sorting directory records by ID preserves the
parent-before-child order required for bottom-up totals.

On macOS the worker cap is eight; caps of four and sixteen were slower on the
measured fixture. Other platforms retain the previous cap. Small files no longer
allocate full child paths or normalize names that aggregation discards.
ASCII display names bypass Unicode normalization. File and byte counters are
published in batches of 256, with the remainder published at directory completion.
Hardlink deduplication and metadata reads remain unchanged.

## Local measurements

Release build, macOS, warm temporary SSD fixtures, airbug-bench quick profile
(8 observations per case). These are local wall-time medians; background load and
CPU frequency are uncontrolled. They do not predict cold whole-disk scan times.

| Case | Before | After | Time reduction |
|---|---:|---:|---:|
| 512 files | 1.95 ms | 1.52 ms | 22% |
| 4096 files | 7.71 ms | 6.69 ms | 13% |
| 4096 files, read totals | 8.71 ms | 6.05 ms | 30% |
| Rescan 256 of 4096 files | 0.698 ms | 0.559 ms | 20% |

A separate comparison scanned the same existing 65,536-file tree, alternating
before/after process order. Each process discarded one warmup and measured 11
scans. Baseline medians were 61.75 and 60.58 ms; optimized medians were 53.34 and
48.65 ms (14–20% less time in the paired runs). Both versions returned exactly
65,536 files, 4,194,304 apparent bytes, 268,435,456 allocated bytes, 2,081 nodes,
and zero errors. These aggregate checks do not establish equivalence for every
filesystem feature; the regression tests cover the cases listed below.

The 4096-file fixture still produces 529 nodes. Regression tests check exact file
counts, apparent and allocated totals, parent links, decomposed Unicode directory
names, empty directories, symlink cycles, exclusions, hardlinks, and subtree rescans.

## Reproduce

```bash
cargo bench -p space-core --features bench --bench scan -- --profile quick
# Larger fixture (created once and reused):
cargo bench -p space-core --features bench --bench scan -- \
  --filter tree_65536 --samples 5 --sample-ms 1 --warmup-ms 1
# Compare release binaries against the same existing directory:
cargo run -p space-core --release --example scan_profile -- /path/to/directory
```

Filesystem metadata IO remains a per-entry cost. Cold disks and network filesystems
can therefore remain much slower than these warm fixtures. A macOS bulk metadata
reader is a possible next optimization, requiring separate correctness and IO
measurements.

## Junk-rule scan optimization

Compiled glob sets are shared within each scan (bounded to 256 pattern lists per
cache), including the grouped project-output path. Invalid patterns still fail
even when their root is absent. Path-template environment substitutions are
prepared once per scan phase. Template expansion no longer performs an unused
existence check before the scanner's own root check.

A release fixture with 10,000 absent application roots, shared include/exclude
patterns, and five repeated scans went from a median **224.19 ms to 14.87 ms**
(about **15.1x**). Both versions scanned all 10,000 rules and returned zero findings.
This measures catalog overhead for missing applications, not whole-disk IO.

```bash
cargo run -p space-clean --release --example scan_profile
```

Regression tests also cover existing log/temp files, exclusions, duplicate
findings, invalid patterns on missing roots, project-output grouping, and
conservative migration of overrides for removed duplicate rules.

## Catalog reload and rule panel

`RulePack::load_many_shared` shares an immutable catalog using `Arc`. Each reload
hashes all source file contents, including file identities and order. A same-length
edit with an unchanged modification timestamp invalidates the cache; additions,
removals and malformed replacements are covered by regression tests. Concurrent
loads are serialized, and a catalog changed during parsing is not cached.

With 933,179 rules, initial parse plus fingerprint verification took 9.33–11.41 s;
a verified reload took 216–293 ms in two local release runs. Both handles share
the same catalog allocation. This cache is in-process and is not persisted.

The rule panel loads the initial catalog on a background thread. It caches its
summary and visible page until preferences, filters, page size or the catalog
change. It constructs only 200 display rows initially rather than all 312,880
host-platform matches. Building that page took 11.94 ms in the release fixture.
Tests cover cache reuse, changed toggles/search/page size and catalog replacement.

```bash
cargo run -p space-clean --release --example load_profile
cargo test -p space-app --release rules_panel::tests
```

## Real-tree and existing-file checks

On the existing `airbug` checkout, the original and optimized disk scanners each
ran one warmup plus 11 measured scans. Median wall time was **62.05 → 42.50 ms**
(32% less). Both reported 12,122 files, 1,572,326,280 apparent bytes,
1,543,188,480 allocated bytes, 5,947 nodes and zero errors. This is a warm real
project tree, not a cold whole-volume measurement.

Junk scanning now resolves its home-directory safety boundary once per process,
consistent with the existing process-lifetime protected-path indexes. A fixture
of 5,000 existing log files went from **44.61 → 32.01 ms** (28% less), with 5,000
findings totaling exactly 35,000 bytes in both runs. Home/root/protected-ancestor
regression tests still apply.

Rule search now normalizes query/tag filters once per listing and reuses a text
buffer instead of allocating joined/lowercased text for every rule. On the real
catalog, five-sample medians changed from **129.48 → 51.09 ms** for `cache`
(311,757 matches), and **129.07 → 50.44 ms** for a missing term (zero matches).
Tests cover ASCII case handling, Unicode, cross-field spaces, normalized tags,
preference filters, and zero-size pages.

```bash
cargo run -p space-clean --release --example present_scan_profile
cargo run -p space-clean --release --example load_profile
```

## Persistent catalog and repeated traversal reuse

The shared loader now persists an enriched binary catalog in the application
cache directory. Its key includes source paths, full YAML content hashes, and
parser/tag-enrichment source versions. The payload has a checksum; missing,
truncated, incompatible or corrupt caches fall back to YAML and are replaced
atomically. Cache IO failures do not prevent loading rules. Source fingerprints
are rechecked before publishing the shared cache. The existing in-process cache
continues to work.

Two consecutive release processes on the 933,179-rule catalog measured **11.19 s**
for YAML loading plus cache creation and **1.12 s** for loading the persisted
catalog, including source verification. In-process verified reloads took
219–226 ms. This is a local warm-filesystem comparison, not a cold-disk guarantee.
The initial YAML parse is still required after source or parser changes.

Ordinary rules with identical traversal conditions and normalized roots now reuse
an earlier traversal within the scan (up to 256 cached keys). IDs, labels and tags
may differ; filtering by preferences still occurs before scanning, and effective
age overrides remain part of the key. Risk, category, report-only status, matching
patterns, depth and deletion mode remain distinct. This does not combine walks
whose matching conditions differ. First-rule attribution and matched-rule counts
are preserved without accumulating duplicate findings.

A fixture with 12 distinct rule IDs/tags and 5,000 log files measured **347.68 ms**
for separate traversals versus **28.34 ms** with reuse (five-sample medians after a
warmup). Full serialized findings matched; both yielded 5,000 files / 35,000 bytes,
and all 12 rules matched. This measures repeated identical predicates, not a
12x improvement for every scan.

```bash
# Run in two separate processes to compare cache creation and reuse:
cargo run -p space-clean --release --example load_profile
cargo run -p space-clean --release --example load_profile
cargo run -p space-clean --release --example repeated_scan_profile
```

## Progress delivery and search text assembly

Junk scan progress is now throttled inside the scanner before cloning snapshots,
with initial/final delivery forced, including empty scans. Scans without a progress
subscriber bypass snapshot creation and clock checks. The app forwards these
already throttled updates directly. Intermediate delivery is limited to once per
50 ms; scan counters continue to update normally.

Search text assembly now appends fields directly to its reusable buffer instead
of invoking formatting machinery. Field separators, ASCII case conversion and
cross-field substring matching are unchanged; no additional index is retained.
Two paired release runs (second pair in reversed order) measured:

| Case | Before medians | After medians |
|---|---:|---:|
| Search `cache` | 34.91 / 35.62 ms | 27.09 / 27.67 ms |
| Search missing term | 34.06 / 34.68 ms | 25.99 / 26.08 ms |
| 10,000 absent roots, no progress subscriber | 15.33 / 14.00 ms | 13.68 / 13.37 ms |

Search returned the same 311,757 / zero matches. The missing-root scan returned
zero findings and processed all 10,000 rules. These are warm local microbenchmarks;
the smaller missing-root improvement varied between 4% and 11%.

## Tree queries and UI work (September 30, 2026)

Largest-file queries now select the requested prefix before sorting it. The
comparator includes discovery order, preserving the previous stable ordering
when names and sizes tie. The bounded depth-first walk uses an explicit stack;
tests include a 30,000-level tree. Child sorting reuses its existing allocation,
and path assembly copies borrowed names directly into the final string.

Paired local release runs, alternating before/after order, with 11 samples after
warmup on trees in memory:

| Operation | Before | After |
|---|---:|---:|
| Largest 50 of 24,999 files, apparent size | 929–936 µs | 155–157 µs |
| Largest 50 of 24,999 files, allocated size | 913–922 µs | 157 µs |
| Construct a path with 128 components | 3.27–3.29 µs | 0.69–0.70 µs |

Reference comparisons check exact node IDs, stable ties, both size modes and
visit budgets. Selection retains each candidate's discovery index, increasing
temporary storage compared with the previous vector of node IDs. These timings
measure queries after scanning, not disk traversal.

The app now starts one largest-file task per request instead of repeating it on
every frame while a result is pending. A generation number rejects responses from
older trees even when their focus node IDs match. Request state is separate from
rendering. Tests cover repeated frames, empty results, rescan replacement,
out-of-order completion and failed thread creation retries.

The file strip caches its rows by immutable tree identity, folder, size mode,
filter and limit. A weak reference avoids retaining an old tree and invalidates
the cache on `Arc::make_mut` edits. With 50,000 direct files and a 100-row limit,
uncached calculation took 550 µs versus 15 ns for unchanged cache access (31 local
release samples). A cache miss still computes the list; this is not a first-load
speedup. Tests cover filter changes, empty lists, modes, limits and tree edits.

```bash
cargo run -p space-core --release --example tree_profile
cargo test -p space-app --release folder_files_profile -- --ignored --nocapture
```

## Catalog statistics and facets

Statistics use one pass without constructing display rows or a temporary list of
host rules. Category, tag, kind and risk lists share one aggregation function,
borrow keys from the catalog, and evaluate preferences once per rule. Owned names
are created only for the final rows.

Local release medians on the 933,179-rule catalog, five samples after warmup,
with output formatting included equally:

| Operation | Default preferences, before → after | Overrides, before → after |
|---|---:|---:|
| Statistics | 172.05 → 82.03 ms | 325.57 → 93.89 ms |
| Tags | 136.23 → 73.81 ms | 262.43 → 80.11 ms |
| Categories | 13.03 → 7.75 ms | 23.59 → 18.31 ms |

All ten output hashes matched across versions. Kind/risk timings were broadly
unchanged; the default kind list took about 0.9 ms longer. Tests cover platform
filtering, row order/counts, preference precedence and untagged rules.

```bash
cargo run -p space-clean --release --example manager_profile
```

## Large junk reports

Report aggregates are prepared once when a scan completes. A separate selection
bitmap maintains count and byte totals on checkbox changes. Read-only findings
cannot enter the selection, and selection totals handle saturation and subsequent
deselection. The single category scroller renders only finding rows intersecting
its viewport; checkbox IDs remain stable while scrolling.

Headless egui release measurements, 900×700 viewport, one expanded category,
20 frames after warmup (report construction excluded):

| Findings | Before, mean per frame | After, mean per frame |
|---|---:|---:|
| 1,000 | 1.537 ms | 0.02868 ms |
| 20,000 | 54.301 ms | 0.02757 ms |

This measures egui frame construction without GPU rendering. Cached selected-byte
totals no longer clone findings; the old 20,000-item calculation took 1.804 ms per
call. Tests click a checkbox, scroll to the final finding, verify selection
survives scrolling, and cover report replacement and report-only safety.

```bash
cargo test -p space-app --release junk_panel::tests
cargo test -p space-app --release benchmark_large_junk_panel -- --ignored --nocapture
```

## ASCII filtering and scan result ownership

ASCII filename/query matching now compares borrowed byte windows without building
lowercase strings for every node. Non-ASCII input retains Unicode lowercase
matching. Tests compare both paths against the previous substring semantics and
check that kind, size and extension filters retain their behavior.

Local release medians, 50,000 synthetic filenames, 11 samples after warmup:

| Query | Matches | Before | After |
|---|---:|---:|---:|
| `FILE` | 50,000 | 2.570 ms | 0.519 ms |
| `missing` | 0 | 3.851 ms | 0.791 ms |

This measures filename filtering in memory. Unicode queries keep their previous
allocation cost. The sidebar also stops copying its row vector every frame and
avoids a redundant ancestor walk and expansion-set clone during hover updates.
Tests cover visible peers, focus ancestors and collapse when hover ends.

Ordinary and grouped junk traversals share a result accumulator, which owns
findings, skipped roots and matched rule IDs and constructs the final report.
The mixed-traversal regression checks findings, totals and final progress. All
eight existing Clippy warnings were resolved; strict workspace Clippy now passes.

```bash
cargo test -p space-app --release filter_query_profile -- --ignored --nocapture
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
```
