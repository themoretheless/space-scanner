# Junk rules

Extensible category packs for `space-clean`.

## Layout

```
rules/
  README.md
  categories/          # curated / reorganized category packs
  platforms/           # OS seed packs (windows / linux)
  generated/           # bulk expand toward RULES_TARGET (safe to regenerate)
  protected/           # hard never-delete safety rules
```

Loader walks `rules/` **recursively** and merges all `*.yaml` / `*.yml`.
Files starting with `_` are ignored (drafts).

`protected/_protected-*.yaml` is intentionally excluded from cleanup rules and
loaded by the safety layer instead. Its 1,000 rules are hard path boundaries:
matching paths and their children are rejected during scanning and checked again
immediately before deletion. Regenerate with:

```bash
cargo run -p space-tools -- protections
space protections --all
```

On load, each rule gets:
- **tags** — explicit YAML `tags:` plus inferred labels
- **kind** — junk type (`logs`, `temp`, `cache`, …); inferred from category when omitted
- **risk** — deletion danger (`low`/`safe`, `medium`/`caution`, `high`/`dangerous`, `report-only`)

See `space_clean::JunkKind` and `Risk`.

## Expand

```bash
cargo run -p space-tools -- catalog names.txt catalog-wave 15000
# Maintain OS seeds directly in rules/platforms/*.yaml
space rules status
```

| Pack | Role |
|------|------|
| `categories/*.yaml` | Shared cleanup categories |
| `categories/browser-privacy.yaml` | Cookies & history (off unless Rules → Privacy) |
| `categories/community-sources.yaml` | Curated paths from BleachBit, mac-cleanup-sh, Winapp2, Arch Wiki, Stacer |
| `platforms/windows.yaml` / `linux.yaml` | Hand-tuned OS seeds |
| `generated/src5k-*.yaml` | Homebrew cask `zap` leftovers + Winapp2.ini FileKeys + formula XDG caches |
| `generated/src5kb-*.yaml` | Flathub + remaining Homebrew casks + Scoop buckets |
| `generated/src10k-*.yaml` | AUR + Debian package XDG caches + extra Scoop |
| `generated/src15k-*.yaml` | Nixpkgs names × macOS/Linux/Windows caches (15k) |
| `generated/src-more-*.yaml` | Nixpkgs wave 2, MacPorts, Chocolatey, AUR slice 2 |
| `generated/src30k-*.yaml` | Nixpkgs wave 3 × macOS/Linux/Windows (30k) |
| `generated/src-iter04-*.yaml` | Iterative 10k leftover Nixpkgs pnames × macOS/Linux/Windows |
| `generated/src-iter05-*.yaml` | Iterative 10k leftover Nixpkgs pnames × macOS/Linux/Windows |
| `generated/src-iter04-*.yaml` … `src-iter31-*.yaml` | Iterative 10k leftover Nixpkgs pnames × macOS/Linux/Windows |
| `generated/src-iter32-*.yaml` … `src-iter49-*.yaml` | Iterative 10k leftover Debian names × macOS/Linux/Windows |
| `generated/src-iter50-*.yaml` … `src-iter81-*.yaml` | Iterative 10k leftover AUR names × macOS/Linux/Windows |
| `generated/src-iter82-*.yaml` … `src-iter84-*.yaml` | Iterative 10k leftover mixed catalog names × macOS/Linux/Windows |
| `generated/src-pypi100k-*.yaml` | 100k inferred PyPI cache probes; disabled and report-only |
| `generated/src-pypi500k-*.yaml` | Additional 500k inferred PyPI cache probes; disabled and report-only |
| `generated/*.yaml` | Auto-expanded bulk (`macos` / `windows` / `linux` only) |

No `platform: any` — shared apps are split per OS (`cargo run -p space-tools -- split`).

The PyPI batch adds exactly 100,000 rules: macOS 33,334, Linux 33,333,
Windows 33,333. Project names come from the 2026-10-01 PyPI Simple API snapshot.
Cache paths are inferred from names, not verified application layouts. Every rule
is disabled by default and has both `risk: report-only` and `report_only: true`;
enabling a probe allows reporting but never deletion. Configs, environments,
package installations, and project folders are not targeted.

The selected names and source checksum are saved in `scripts/catalogs/`.
`cargo run -p space-tools -- pypi` reproduces the 125 packs from those names
when the output packs are absent; it refuses to overwrite existing packs.

The second PyPI batch adds 500,000 rules in 625 packs: macOS 166,667,
Linux 166,667, Windows 166,666. It uses another 166,667 names from the same
snapshot, excluding prior PyPI names and names already present in rule path
components. Paths remain unverified, disabled by default, and report-only.
Its names and source metadata are saved as `scripts/catalogs/pypi-500k-*`.
To reproduce this batch when its output packs are absent:

```bash
cargo run -p space-tools --release -- pypi scripts/catalogs/pypi-500k-names.txt scripts/catalogs/pypi-500k-source.json pypi500k 500000
```

## Extend

1. Add `rules/categories/my-category.yaml` or `rules/platforms/<os>.yaml` with a `rules:` list.
2. Keep `id` unique globally.
3. Set `platform: windows` / `linux` / `macos` (prefer OS-specific; avoid `any`).
4. Add a non-empty `description` explaining what is matched and what is preserved.
5. Optional `kind: logs|temp|cache|browser|packages|…` — inferred from category if omitted.
6. Optional `risk: low|medium|high|report-only` — deletion danger (aliases: safe/caution/dangerous).
7. Optional `tags: [browser, electron, …]` — merged with inferred tags on load.
8. Prefer `default_enabled: false` for medium/high risk.
9. Category expand: `cargo run -p space-tools -- catalog names.txt category-wave 3000`
10. Maintain OS seed packs directly in `rules/platforms/*.yaml`.

## Rule manager (runtime prefs)

YAML packs stay immutable. User toggles live in `rule-prefs.json`:

| Action | Effect |
|--------|--------|
| Per-rule on/off | Force enable/disable that id |
| Category On/Off | Force whole category |
| Tag On/Off | Force all rules that carry that tag |
| Enable all | Treat every rule as on, then apply overrides |
| Reset | Back to YAML `default_enabled` |

CLI:

```bash
space rules status
space kinds
space risks
space protections
space rules list -k logs --risk low
space rules list -t electron,browser
space rules tag on electron
space tags
space junk -k temp --max-risk medium
space junk -t npm --clean --yes   # dry-run without --yes
```

App: **Rules** window — Categories / Kinds / Danger / Tags tabs. On each OS, only matching `platform` rules are active for scans.

## Duplicate audit

The catalog contains 1,533,179 rules, including 600,000 report-only PyPI probes,
after removing 1,187 exact duplicates from earlier batches.
The separate 1,000 protection rules are preserved.

```bash
cargo run -p space-tools -- deduplicate          # exits 1 if duplicates are found
cargo run -p space-tools -- deduplicate --apply  # remove duplicates, preserve YAML layout
```

Run the audit after regenerating packs. It compares the loader's effective rule
fields: platform, paths, patterns, age, depth, deletion mode, risk, defaults,
category, kind, and inferred/explicit tags. Display name, description, and ID are
excluded only after their effects on inferred tags and kind have been evaluated.
Rules with different effective tags remain separate because tags control prefs.

`_rule-aliases.json` maps removed IDs to retained IDs. The app and CLI migrate
saved rule overrides in memory when loading preferences. Conflicting overrides
use disable-first and the larger age threshold. The usual preferences save writes
the migrated settings; the audit never edits a user's preferences file.
