#!/usr/bin/env python3
"""Reject history-dependent work in the channel note API and cleanup."""
import json
import sys
from pathlib import Path

small = json.loads((Path(sys.argv[1]) / 'work.json').read_text())
large = json.loads((Path(sys.argv[2]) / 'work.json').read_text())
assert small.keys() == large.keys() and len(small) == 13, 'missing query conditions'


def compare(name, before, after):
    for relation in before['relations'].keys() | after['relations'].keys():
        a, b = before['relations'].get(relation, 0.), after['relations'].get(relation, 0.)
        assert b <= 2 * a + 64, f'{name}: {relation} grows with history: {a} -> {b}'
    assert after['intermediate'] <= 2 * before['intermediate'] + 64, \
        name + ': intermediate work grows with history'
    assert after['sort_input'] <= 2 * before['sort_input'] + 64, \
        name + ': sort grows with history'
    assert after['logical_blocks'] <= 2 * before['logical_blocks'] + 256, \
        name + ': logical I/O grows with history'
    print(f"{name}: rows {sum(before['relations'].values()):.0f}"
          f"->{sum(after['relations'].values()):.0f}, "
          f"blocks {before['logical_blocks']:.0f}->{after['logical_blocks']:.0f}, bounded")


for name, before in sorted(small.items()):
    compare(name, before, large[name])
