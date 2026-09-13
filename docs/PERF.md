# Scan performance report

Measured with airbug-bench (`--profile normal`, aarch64 macOS, temp SSD fixture).

```bash
cargo bench -p space-core --bench scan -- --profile normal
```

## Results

| Case | Files (leaves) | Nodes after aggregate | Median wall |
|------|----------------|----------------------|-------------|
| `scan/tree_512` | 512 | ~81 | **1.83 ms** |
| `scan/tree_4096` | 4096 | 529 | **7.80 ms** |
| `scan/sums_4096` | 4096 | 529 | **7.72 ms** |

Phase profile (one-shot `tree_4096`): **~484k files/s**, ≈**2 µs/file**, wall **8.5 ms**.

`sums` ≈ `tree` → summing sizes is negligible; **almost all time is walk + tree build**.

## What is slow (ordered)

1. **Filesystem metadata IO** (`read_dir` + `metadata` / `stat` per entry)  
   Dominates on real disks (cold cache, spinning/network volumes). Parallel work-stealing helps, but one volume is still IO-bound.

2. **`PathBuf` as the join key** ([`scan.rs`](crates/space-core/src/scan.rs))  
   Every record stores a full parent `PathBuf`; `build_tree` uses `HashMap<PathBuf, usize>`. On deep trees this is CPU + allocator heavy (clone, hash of long paths).

3. **Post-pass `build_tree`**  
   Two passes over all records + bottom-up sum + per-dir sort. Cheap at 4k leaves; costly at millions of *nodes* if small-file aggregation is off or many large files.

4. **Small-file aggregation already helps UI RAM** (`SMALL_FILE_THRESHOLD = 512 KiB`)  
   4096 tiny files → **529 nodes**. Without it, node count and HashMap pressure would be ~8× higher on this fixture.

5. **Not the bottleneck:** `total_size()` / file_count after scan; hardlink mutex on this fixture (no hardlinks).

## Whole-disk extrapolation

At ~500k files/s on a warm SSD fixture, **6M files ≈ 12 s** best case. Real Macintosh HD is usually slower (cold metadata, deeper trees, permission skips, APFS noise) — tens of seconds is expected unless further optimized.

## Next optimizations (priority)

1. Intern paths / use `(parent_id, name)` instead of `PathBuf` in the hot walk.  
2. Stream records into the arena without a giant `Vec<Rec>` + second HashMap pass.  
3. Optional macOS `getattrlistbulk` for fewer syscalls.  
4. Keep/tune small-file aggregation for UI; expose a “list every file” mode only when needed.
