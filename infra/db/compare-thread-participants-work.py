#!/usr/bin/env python3
"""Reject participant SQL work that grows with the number of users (300k -> 1m)."""
import json
import sys
from pathlib import Path
small = json.loads((Path(sys.argv[1]) / 'work.json').read_text())
large = json.loads((Path(sys.argv[2]) / 'work.json').read_text())
small.pop('timing_ms', None)
large.pop('timing_ms', None)
assert small.keys() == large.keys() and len(small) == 24, 'missing query conditions'
for name, before in sorted(small.items()):
    after = large[name]
    for relation in before['relations'].keys() | after['relations'].keys():
        a, b = before['relations'].get(relation, 0), after['relations'].get(relation, 0)
        assert b <= a + 64, f'{name}: {relation} grows with users: {a} -> {b}'
    assert after['sort_input'] <= before['sort_input'] + 64, name + ': sort grows with users'
    assert after['logical_blocks'] <= 1.5 * before['logical_blocks'] + 256, \
        f"{name}: logical I/O grows with users: {before['logical_blocks']} -> {after['logical_blocks']}"
    print(f"{name}: blocks {before['logical_blocks']}->{after['logical_blocks']}, bounded")
