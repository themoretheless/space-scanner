#!/usr/bin/env python3
"""Exercise the real MCP stdio protocol against an isolated scan/rule fixture."""
import json
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading

binary = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/space-mcp').resolve()
with tempfile.TemporaryDirectory(prefix='space-mcp-test-') as directory:
    root = Path(directory)
    data = root / 'data'
    data.mkdir()
    for name in ['one', 'two', 'three']:
        (data / name).mkdir()
        (data / name / 'test.log').write_bytes(b'fixture')
    (data / 'keep.tmp').write_bytes(b'keep')
    (root / 'rules').mkdir()
    (root / 'rules' / 'fixture.yaml').write_text(json.dumps({'rules': [{
        'id': 'mcp-fixture', 'name': 'MCP Fixture', 'description': 'stdio integration fixture',
        'category': 'mcp-fixture', 'platform': 'any', 'paths': [str(data)],
        'min_age_days': 0, 'include_globs': ['**/*.log'], 'default_enabled': True,
    }]}))
    process = subprocess.Popen([str(binary)], cwd=root, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    messages = queue.Queue()
    def read():
        for line in process.stdout:
            messages.put(json.loads(line))
    threading.Thread(target=read, daemon=True).start()
    sequence = 0
    def send(message):
        process.stdin.write(json.dumps({'jsonrpc': '2.0', **message}) + '\n')
        process.stdin.flush()
    def request(method, params):
        global sequence
        sequence += 1
        send({'id': sequence, 'method': method, 'params': params})
        while True:
            reply = messages.get(timeout=30)
            if reply.get('id') == sequence:
                return reply
    def call(name, args, error=False):
        reply = request('tools/call', {'name': name, 'arguments': args})
        if error:
            assert 'error' in reply or reply['result'].get('isError'), reply
            return
        assert 'error' not in reply, reply
        result = reply['result']
        assert not result.get('isError'), result
        return result['structuredContent']
    try:
        hello = request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                      'clientInfo': {'name': 'space-test', 'version': '1'}})
        assert 'result' in hello, hello
        send({'method': 'notifications/initialized'})
        tools = request('tools/list', {})['result']['tools']
        assert {t['name'] for t in tools} == {'scan_disk', 'disk_page', 'scan_junk', 'junk_page', 'search_rules', 'rule_details', 'largest_entries', 'inspect_path'}
        assert all(t['annotations']['readOnlyHint'] for t in tools)
        call('scan_disk', {'path': 'relative'}, error=True)
        call('scan_disk', {'path': str(data / 'absent')}, error=True)
        disk = call('scan_disk', {'path': str(data)})
        assert disk['files'] == 4 and disk['apparent_bytes'] == 25, disk
        page = call('disk_page', {'scan_id': disk['scan_id'], 'limit': 1})
        assert len(page['items']) == 1 and page['next_offset'] == 1, page
        second = call('disk_page', {'scan_id': disk['scan_id'], 'offset': 1, 'limit': 1})
        assert second['items'][0]['node_id'] != page['items'][0]['node_id']
        call('disk_page', {'scan_id': disk['scan_id'], 'limit': 201}, error=True)
        call('disk_page', {'scan_id': disk['scan_id'], 'node_id': 4294967295}, error=True)
        call('scan_disk', {'path': str(data)})
        call('disk_page', {'scan_id': disk['scan_id']}, error=True)
        rules = call('search_rules', {'query': 'MCP fixture', 'limit': 10})
        assert rules['total'] == 1, rules
        rule = call('rule_details', {'id': 'mcp-fixture'})
        assert rule['enabled'] and rule['min_age_days'] == 0, rule
        call('rule_details', {'id': 'absent'}, error=True)
        junk = call('scan_junk', {'categories': ['mcp-fixture']})
        assert junk['findings'] == 3 and junk['total_bytes'] == 21, junk
        findings = call('junk_page', {'scan_id': junk['scan_id'], 'limit': 2})
        assert len(findings['items']) == 2 and findings['next_offset'] == 2, findings
        assert len(call('junk_page', {'scan_id': junk['scan_id'], 'offset': 2})['items']) == 1
        call('junk_page', {'scan_id': junk['scan_id'], 'node_id': 1}, error=True)
        assert sum(1 for p in data.rglob('*') if p.is_file()) == 4
        detail = call('inspect_path', {'path': str(data / 'one/test.log'), 'junk_scan_id': junk['scan_id']})
        assert detail['type'] == 'file' and detail['apparent_bytes'] == 7, detail
        assert detail['junk_evidence']['match_count'] == 1 and not detail['deletion_authorized'], detail
        call('inspect_path', {'path': str(data / 'missing')}, error=True)
        call('inspect_path', {'path': 'relative'}, error=True)
        protected = call('inspect_path', {'path': '/'})
        assert protected['blocked_by_safety'], protected
        (data / 'Cargo.toml').write_text('[package]')
        (data / 'target').mkdir()
        (data / 'target/large.bin').write_bytes(b'x' * 600000)
        (data / 'larger.bin').write_bytes(b'x' * 900000)
        cache = call('inspect_path', {'path': str(data / 'target/large.bin')})
        assert any(e['kind'] == 'project_marker' for e in cache['evidence']), cache
        assert any(e['kind'] == 'directory_name_hint' for e in cache['evidence']), cache
        assert cache['assessment'] == 'requires_review', cache
        directory = call('inspect_path', {'path': str(data / 'target')})
        assert directory['size_scope'].startswith('directory_entry_only'), directory
        (data / 'link').symlink_to(data / 'larger.bin')
        link = call('inspect_path', {'path': str(data / 'link')})
        assert link['type'] == 'symlink', link
        call('inspect_path', {'path': str(data / 'link'), 'junk_scan_id': junk['scan_id']}, error=True)
        largest_scan = call('scan_disk', {'path': str(data)})
        ranked = call('largest_entries', {'scan_id': largest_scan['scan_id'], 'kind': 'files', 'apparent': True, 'limit': 2})
        assert [x['apparent_bytes'] for x in ranked['items']] == [900000, 600000], ranked
        assert ranked['items'][0]['display_path'].endswith('/larger.bin'), ranked
        dirs = call('largest_entries', {'scan_id': largest_scan['scan_id'], 'kind': 'directories', 'apparent': True, 'limit': 1})
        assert dirs['items'][0]['display_path'].endswith('/target'), dirs
        call('largest_entries', {'scan_id': disk['scan_id']}, error=True)
        call('largest_entries', {'scan_id': largest_scan['scan_id'], 'kind': 'invalid'}, error=True)
        call('largest_entries', {'scan_id': largest_scan['scan_id'], 'limit': 0}, error=True)
        process.stdin.close()
        assert process.wait(timeout=10) == 0, process.stderr.read()
        print('MCP stdio: handshake, 8 tools, ranking, inspection, safety, symlinks, pagination and read-only behavior passed')
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
