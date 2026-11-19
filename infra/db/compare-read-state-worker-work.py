#!/usr/bin/env python3
"""Reject history-dependent work in unread and worker reservations."""
import json
import sys
from pathlib import Path
small = json.loads((Path(sys.argv[1]) / 'work.json').read_text())
large = json.loads((Path(sys.argv[2]) / 'work.json').read_text())

# The exact 14 measured query conditions written by
# backend/tests/read_state_worker_scale_test.rs. Both scales must report the
# same complete set so a skipped or renamed condition cannot silently pass.
EXPECTED_CONDITIONS = frozenset({
    'read_list', 'read_single',
    'slack_wake_empty', 'slack_due_empty',
    'slack_wake_ready', 'slack_due_ready',
    'slack_wake_team_empty', 'slack_due_team_empty',
    'display_row_empty', 'display_scope_empty',
    'display_row_ready', 'display_scope_ready',
    'message_insert', 'message_delete_latest',
})


def condition_diff(label, conditions):
    missing = sorted(EXPECTED_CONDITIONS - conditions)
    unexpected = sorted(conditions - EXPECTED_CONDITIONS)
    return f'{label} work.json conditions: missing {missing}, unexpected {unexpected}'


small_conditions = frozenset(small)
large_conditions = frozenset(large)
assert small_conditions == EXPECTED_CONDITIONS, condition_diff('small', small_conditions)
assert large_conditions == EXPECTED_CONDITIONS, condition_diff('large', large_conditions)


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
        assert 'nested' in before, name + ': missing trigger plans'
        assert 'nested' in after, name + ': missing trigger plans'
        compare(name + '/nested', before['nested'], after['nested'])
        a, b = before['nested']['statements'], after['nested']['statements']
        assert 0 < a <= 128, name + ': unbounded trigger statements'
        assert 0 < b <= 128, name + ': unbounded trigger statements'
        assert b <= 2 * a + 16, name + ': unbounded trigger statements'
