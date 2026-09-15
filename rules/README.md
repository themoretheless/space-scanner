# Junk rules

Extensible category packs for `space-clean`.

## Layout

```
rules/
  README.md
  categories/          # curated / reorganized category packs
  platforms/           # OS seed packs (windows / linux)
  generated/           # bulk expand toward RULES_TARGET (safe to regenerate)
```

Loader walks `rules/` **recursively** and merges all `*.yaml` / `*.yml`.
Files starting with `_` are ignored (drafts).

On load, each rule gets **tags**: explicit YAML `tags:` plus inferred labels
(from id / category / paths / risk / platform). See `space_clean::tags`.

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
4. Optional `tags: [browser, electron, …]` — merged with inferred tags on load.
5. Prefer `default_enabled: false` for medium/high risk.
6. Category expand: `RULES_TARGET=3000 ruby scripts/reorg_and_expand_rules.rb`
7. OS packs: `ruby scripts/gen_win_linux_rules.rb`

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
space rules list -t electron,browser
space rules tag on electron
space tags
space junk -t npm --clean --yes   # dry-run without --yes
```

App: **Rules** window — Categories / Tags tabs. On each OS, only matching `platform` rules are active for scans.
