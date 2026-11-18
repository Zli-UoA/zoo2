#!/usr/bin/env python3
"""Reject history-dependent work in unread and worker reservations."""
import json
import sys
from pathlib import Path
small = json.loads((Path(sys.argv[1]) / 'work.json').read_text())
large = json.loads((Path(sys.argv[2]) / 'work.json').read_text())
assert small.keys() == large.keys() and len(small) == 14, 'missing query conditions'


# Static lookup tables are seeded with a fixed number of rows and never grow
# with message history. A plan may switch between an index lookup (one row)
# and a sequential scan (all rows), so compare their absolute bound instead.
STATIC_RELATIONS = {'app_event_types'}


def compare(name, before, after):
    for relation in before['relations'].keys() | after['relations'].keys():
        a, b = before['relations'].get(relation, 0), after['relations'].get(relation, 0)
        if relation in STATIC_RELATIONS:
            # A static table may be scanned by more than one trigger statement.
            # Keep a small absolute bound without treating repeated fixed-size
            # lookups as history-dependent work.
            assert b <= 512, f'{name}: {relation} is not a bounded static lookup: {a} -> {b}'
            continue
        assert b <= 2 * a + 64, f'{name}: {relation} grows with history: {a} -> {b}'
    assert after['intermediate'] <= 2 * before['intermediate'] + 64, name + ': intermediate work grows with history'
    assert after['sort_input'] <= 2 * before['sort_input'] + 64, name + ': sort grows with history'
    assert after['logical_blocks'] <= 2 * before['logical_blocks'] + 256, name + ': logical I/O grows with history'
    print(f"{name}: rows {sum(before['relations'].values()):.0f}->{sum(after['relations'].values()):.0f}, "
          f"blocks {before['logical_blocks']}->{after['logical_blocks']}, bounded")


for name, before in sorted(small.items()):
    after = large[name]
    compare(name, before, after)
    if name in ('message_insert', 'message_delete_latest'):
        assert 'nested' in before and 'nested' in after, name + ': missing trigger plans'
        compare(name + '/nested', before['nested'], after['nested'])
        a, b = before['nested']['statements'], after['nested']['statements']
        assert 0 < a <= 128 and 0 < b <= 128 and b <= 2 * a + 16, name + ': unbounded trigger statements'
