#!/usr/bin/env python3
"""Audit duplicate cleanup rules, or remove them with --apply (preserves YAML text)."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--root', type=Path, default=ROOT / 'rules')
    args = parser.parse_args()
    rules_root = args.root.resolve()
    audit = subprocess.run(['cargo', 'run', '-p', 'space-clean', '--release',
        '--example', 'audit_rule_duplicates', '--', str(rules_root)], cwd=ROOT,
        check=True, stdout=subprocess.PIPE, text=True)
    aliases = json.loads(audit.stdout)
    if not args.apply:
        print(f'Duplicate rules: {len(aliases)}')
        raise SystemExit(1 if aliases else 0)
    edits = []
    removed = set()
    retained = set()
    rule_start = re.compile(r'^[ \t]*- (?:id|name): [^\n]+\n', re.MULTILINE)
    for path in sorted(rules_root.rglob('*')):
        if path.suffix not in ('.yaml', '.yml') or path.name.startswith('_'):
            continue
        text = path.read_text()
        matches = list(rule_start.finditer(text))
        keep = []
        changed = False
        for i, match in enumerate(matches):
            end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
            block = text[match.start():end]
            rule_id_match = re.search(r'^[ \t]*(?:- )?id: ([^\n]+)', block, re.MULTILINE)
            if rule_id_match is None:
                raise ValueError(f'unrecognized rule in {path}; no files changed')
            rule_id = rule_id_match[1].strip().strip('\"\'')
            if rule_id in aliases:
                if rule_id in removed:
                    raise ValueError(f'duplicate source ID: {rule_id}')
                removed.add(rule_id)
                changed = True
            else:
                retained.add(rule_id)
                keep.append(text[match.start():end])
        if changed:
            prefix = text[:matches[0].start()]
            if not keep:
                prefix = re.sub(r'^rules:\s*$', 'rules: []', prefix, flags=re.MULTILINE) + '\n'
            edits.append((path, prefix + ''.join(keep)))
    assert removed == set(aliases), 'unrecognized YAML formatting; no files changed'
    assert set(aliases.values()) <= retained, 'missing canonical rules; no files changed'
    alias_path = rules_root / '_rule-aliases.json'
    previous = json.loads(alias_path.read_text()) if alias_path.exists() else {}
    for old, target in previous.items():
        previous[old] = aliases.get(target, target)
    previous.update(aliases)
    assert set(previous.values()) <= retained, 'stale alias targets; no files changed'
    # Validate every edited YAML document before replacing any source file.
    # This also catches dangling YAML anchors when a removed block defined one.
    if edits:
        with tempfile.TemporaryDirectory(prefix='space-rules-validate-') as stage:
            for i, (_, text) in enumerate(edits):
                (Path(stage) / f'{i}.yaml').write_text(text)
            subprocess.run(['cargo', 'run', '-p', 'space-clean', '--release',
                '--example', 'audit_rule_duplicates', '--', stage], cwd=ROOT,
                check=True, stdout=subprocess.DEVNULL)
    for path, text in edits:
        path.write_text(text)
    if previous:
        alias_path.write_text(json.dumps(previous, indent=2, sort_keys=True) + '\n')
    print(f'Removed {len(removed)} rules from {len(edits)} files; retained {len(retained)} rules')

if __name__ == '__main__':
    main()
