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
ruby scripts/generate_protected_rules.rb
space protections --all
```

On load, each rule gets:
- **tags** — explicit YAML `tags:` plus inferred labels
- **kind** — junk type (`logs`, `temp`, `cache`, …); inferred from category when omitted
- **risk** — deletion danger (`low`/`safe`, `medium`/`caution`, `high`/`dangerous`, `report-only`)

See `space_clean::JunkKind` and `Risk`.

## Expand

```bash
RULES_TARGET=15000 ruby scripts/expand_rules_to_target.rb
ruby scripts/gen_win_linux_rules.rb
space rules status
```

| Pack | Role |
|------|------|
| `categories/*.yaml` | Shared cleanup categories |
| `platforms/windows.yaml` / `linux.yaml` | Hand-tuned OS seeds |
| `generated/*.yaml` | Auto-expanded bulk (`macos` / `windows` / `linux` only) |

No `platform: any` — shared apps are split per OS (`ruby scripts/split_any_rules.rb`).

## Extend

1. Add `rules/categories/my-category.yaml` or `rules/platforms/<os>.yaml` with a `rules:` list.
2. Keep `id` unique globally.
3. Set `platform: windows` / `linux` / `macos` (prefer OS-specific; avoid `any`).
4. Add a non-empty `description` explaining what is matched and what is preserved.
5. Optional `kind: logs|temp|cache|browser|packages|…` — inferred from category if omitted.
6. Optional `risk: low|medium|high|report-only` — deletion danger (aliases: safe/caution/dangerous).
7. Optional `tags: [browser, electron, …]` — merged with inferred tags on load.
8. Prefer `default_enabled: false` for medium/high risk.
9. Category expand: `RULES_TARGET=3000 ruby scripts/reorg_and_expand_rules.rb`
10. OS packs: `ruby scripts/gen_win_linux_rules.rb`

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
