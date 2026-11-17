#!/usr/bin/env python3
"""Compare actual work at fixed K/F/groups, with history growing 300k -> 1m."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("small", type=Path)
parser.add_argument("large", type=Path)
args = parser.parse_args()
small = {p.name: json.loads(p.read_text()) for p in args.small.glob("*-work.json")}
large = {p.name: json.loads(p.read_text()) for p in args.large.glob("*-work.json")}
assert len(small) == len(large) == 16 and small.keys() == large.keys(), "missing measured SQL conditions"
for name, before in sorted(small.items()):
    after = large[name]
    assert before["row_budget"] == after["row_budget"], name + ": fixture K/F changed"
    for relation in before["relations"].keys() | after["relations"].keys():
        a = before["relations"].get(relation, 0)
        b = after["relations"].get(relation, 0)
        assert b <= 2 * a + 64, f"{name}: {relation} grows with history: {a} -> {b}"
    for metric in ["max_intermediate", "max_sort_input", "max_recursive", "max_join_checks"]:
        assert after[metric] <= 2 * before[metric] + 64, f"{name}: {metric} grows with history"
    # hit+read measures logical accesses, independent of whether pages are cached.
    # Includes trigger work, which the parent EXPLAIN does not expose as nodes.
    assert after["logical_blocks"] <= 2 * before["logical_blocks"] + 256, (
        f"{name}: logical blocks grow with history: {before['logical_blocks']} -> {after['logical_blocks']}")
    assert before["temporary_blocks"] == after["temporary_blocks"] == 0, name + ": disk spill"
    print(f"{name}: rows={sum(before['relations'].values()):.0f}->{sum(after['relations'].values()):.0f}; "
          f"logical blocks={before['logical_blocks']}->{after['logical_blocks']}; bounded")
