# Junk rules

Extensible category packs for `space-clean`.

## Layout

```
rules/
  README.md
  categories/
    <category>.yaml   # one file per category
    …                 # drop more *.yaml here anytime
```

Loader walks `rules/` **recursively** and merges all `*.yaml` / `*.yml`.
Files starting with `_` are ignored (drafts).

## Stats

- **Total rules:** 3000
- **Categories:** 24

| Category | Rules |
|----------|------:|
| `app-cache` | 1202 |
| `leftovers` | 580 |
| `logs` | 491 |
| `package-cache` | 242 |
| `caches` | 112 |
| `dev` | 108 |
| `browser-cache` | 54 |
| `ml-cache` | 26 |
| `system` | 25 |
| `game-cache` | 24 |
| `ml-models` | 19 |
| `vm-cache` | 18 |
| `media-cache` | 15 |
| `temp` | 15 |
| `report` | 13 |
| `vm-disk` | 13 |
| `app-data` | 10 |
| `databases` | 8 |
| `media-library` | 7 |
| `sample-library` | 6 |
| `game-downloads` | 4 |
| `wine-cache` | 4 |
| `shader-cache` | 3 |
| `misc` | 1 |

## Extend

1. Add `rules/categories/my-category.yaml` with a `rules:` list (same schema as existing).
2. Keep `id` unique globally.
3. Prefer `default_enabled: false` for medium/high risk.
4. Regenerate / expand: `RULES_TARGET=3000 ruby scripts/reorg_and_expand_rules.rb`

## Rule manager (runtime prefs)

YAML packs stay immutable. User toggles live in `rule-prefs.json`:

| Action | Effect |
|--------|--------|
| Per-rule on/off | Force enable/disable that id |
| Category On/Off | Force whole category |
| Enable all | Treat every rule as on, then apply overrides |
| Reset | Back to YAML `default_enabled` |

CLI: `space rules status|list|enable|disable|category|reset`  
App: **Rules** window.
