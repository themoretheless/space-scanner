# Repository tools

All executable repository tooling is Rust in `crates/space-tools`.

```sh
cargo run -p space-tools -- --help
cargo run -p space-tools -- protections
cargo run -p space-tools -- split
cargo run -p space-tools -- deduplicate
cargo run -p space-tools -- deduplicate --apply
cargo run -p space-tools -- catalog names.txt catalog-wave 15000
cargo run -p space-tools -- pypi
cargo run -p space-tools -- features
cargo run -p space-tools -- run-app
cargo test -p space-mcp --test stdio
```

`catalog` reads a plain UTF-8 package-name list (first whitespace-delimited column),
skips invalid names and existing IDs, and generates disabled cache/log rules for
macOS, Linux and Windows in packs of 800. Prefixes must be lowercase alphanumeric
words separated by hyphens. It requires the requested number of rules before
writing and refuses to overwrite any destination pack. Review inferred paths
before enabling rules. `--root <directory>` selects a separate rules directory
for catalog, split, protections, PyPI generation and duplicate maintenance.

`pypi` reproduces the 100,000 disabled, report-only probes from the checked-in
snapshot in `scripts/catalogs`; existing PyPI output packs cause an error.
For another snapshot batch, use `pypi <names.txt> <source.json> <prefix> <target>`:

```sh
cargo run -p space-tools --release -- pypi scripts/catalogs/pypi-500k-names.txt scripts/catalogs/pypi-500k-source.json pypi500k 500000
```

The names file must contain exactly `ceil(target / 3)` unique safe project names.
Each batch refuses ID collisions and existing output packs, and always generates
disabled report-only probes. Snapshot paths are hypotheses, not verified layouts.
`features` renders `docs/FEATURES.md` from the authoritative
`docs/features_items.json`; edit that data to add or change features.
`run-app` builds, bundles and opens the release application on macOS. On other
platforms run `cargo run -p space-app --release`.

The historical expansion-wave generators are retired. Their checked-in YAML
outputs remain the source of truth, including curated browser, community,
platform and language-project rules. Maintain those declarative packs directly;
new name-based waves use the general `catalog` command. No interpreter is needed
for building, testing, launching or maintaining this project.
