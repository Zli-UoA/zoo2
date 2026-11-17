"""履歴量の異なるメッセージ部品の SQL の探索量を比較する（時間は参考値）。

backend/tests/components_query_budget_test.rs が書いた 30 万件と 100 万件の結果を読み、同じ SQL・同じ同時実行数で、
関係ごとの走査行と論理バッファが履歴に比例して増えていないことを確かめる。
"""
import json
from pathlib import Path
import statistics
import sys

API = ('invoke-action', 'invoke-submit')
# マイグレーションで入れる固定の一覧。計画が索引と全件読みの間で切り替わるので、履歴との比較から外す
STATIC = {'app_event_types', 'app_event_aliases'}


def groups(path):
    data = json.loads(Path(path).read_text())
    grouped = {}
    for sample in data['samples']:
        grouped.setdefault((sample['name'], sample['concurrency']), []).append(sample)
    assert len(grouped) == data['queries'] * 2, 'missing query conditions'
    for (name, concurrency), samples in grouped.items():
        assert len(samples) == 30 * concurrency, (name, concurrency, len(samples))
    api = {}
    for sample in data['api_samples']:
        api.setdefault((sample['name'], sample['concurrency']), []).append(sample['roundtrip_ms'])
    assert set(api) == {(name, n) for name in API for n in (1, 8)}, sorted(api)
    for (name, n), values in api.items():
        assert len(values) == 30 * n, (name, n, len(values))
    return data, grouped, api


def stats(values):
    values = sorted(values)
    return f'p50={statistics.median(values):.3f} p95={values[int(.95 * (len(values) - 1))]:.3f} max={max(values):.3f}'


def compare(first, second):
    small, a, _ = groups(first)
    large, b, api = groups(second)
    assert small['rows'] >= 300000 and large['rows'] >= 1000000
    assert a.keys() == b.keys()
    for key in sorted(a):
        x, y = a[key], b[key]
        before = max(s['blocks'] for s in x)
        after = max(s['blocks'] for s in y)
        assert after <= 2 * before + 64, (key, 'blocks', before, after)
        for relation in set().union(*(s['relations'] for s in x + y)):
            if relation in STATIC:
                continue  # 固定の一覧。テストが絶対の上限（256 行）で確かめる
            initial = max(s['relations'].get(relation, 0) for s in x)
            later = max(s['relations'].get(relation, 0) for s in y)
            assert later <= 2 * initial + 8, (key, relation, initial, later)
        print(f'{key}: blocks {before} -> {after}, SQL roundtrip ms {stats([s["roundtrip_ms"] for s in y])}')
    for key, values in sorted(api.items()):
        print(f'API {key}: {stats(values)} ms')
    for data in (small, large):
        print(f'backfill dry-run {data["rows"]}: {data["backfill"]}')
    return a, b


if __name__ == '__main__':
    compare(*sys.argv[1:])
