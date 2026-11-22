"""履歴量の異なる検索SQLの探索量を比較する（時間は参考値）。

どの月も同じ密度で、増やした履歴は過去の月へ延びる。1 か月分の SQL の走査量・
バッファが履歴に比例して増えないことを確かめる。比較幅は索引の高さと計画の揺れのぶん。
"""
import json
from pathlib import Path
import statistics
import sys

API = ('api-messages', 'api-messages-12-months', 'api-all', 'api-all-12-months', 'api-files', 'api-notes')


def groups(path):
    data = json.loads(Path(path).read_text())
    grouped = {}
    for sample in data['samples']:
        grouped.setdefault((sample['name'], sample['concurrency']), []).append(sample)
    assert len(grouped) == data['queries'] * 2
    names = {name for name, _ in grouped}
    assert set(grouped) == {(name, n) for name in names for n in (1, 8)}, sorted(grouped)
    for (name, concurrency), samples in grouped.items():
        assert len(samples) == 30 * concurrency, (name, concurrency, len(samples))
    return data, grouped


def percentile(values):
    values = sorted(values)
    return statistics.median(values), values[int(.95 * (len(values) - 1))], max(values)


def compare(first, second):
    small, a = groups(first)
    large, b = groups(second)
    assert small['rows'] >= 300000 and large['rows'] >= 1000000
    assert small['recent_rows'] == large['recent_rows']
    assert a.keys() == b.keys()
    api_groups = []
    for data in (small, large):
        api = {}
        for sample in data['api_samples']:
            api.setdefault((sample['name'], sample['concurrency']), []).append(sample['roundtrip_ms'])
        assert set(api) == {(name, n) for name in API for n in (1, 8)}
        for (name, n), values in api.items():
            assert len(values) == 30 * n, (name, n, len(values))
        api_groups.append(api)
    for key in sorted(a):
        x, y = a[key], b[key]
        initial = max(s['blocks'] for s in x)
        later = max(s['blocks'] for s in y)
        assert later <= 1.5 * initial + 64, (key, 'blocks', initial, later)
        for relation in set().union(*(s['relations'] for s in x + y)):
            initial = max(s['relations'].get(relation, 0) for s in x)
            later = max(s['relations'].get(relation, 0) for s in y)
            assert later <= 1.2 * initial + 8, (key, relation, initial, later)
        p50, p95, top = percentile(s['roundtrip_ms'] for s in y)
        print(f'{key}: blocks {max(s["blocks"] for s in x)} → {max(s["blocks"] for s in y)}, '
              f'SQL roundtrip ms p50={p50:.3f} p95={p95:.3f} max={top:.3f}')
    # API 1 リクエストあたりの SQL の回数と合計バッファも、履歴に比例しない。
    sql = []
    for data in (small, large):
        per = {}
        for sample in data['api_sql']:
            per.setdefault(sample['name'], []).append(sample)
        assert set(per) == set(API), sorted(per)
        sql.append(per)
    for name in API:
        x, y = sql[0][name], sql[1][name]
        assert {s['month_sql'] for s in x} == {s['month_sql'] for s in y}, (name, 'month_sql')
        initial = max(s['blocks'] for s in x)
        later = max(s['blocks'] for s in y)
        assert later <= 1.5 * initial + 64, (name, 'api blocks', initial, later)
        # 文の数（BEGIN・SET・見える範囲・月ごと）は履歴によらない。トランザクションの揺れのぶんだけ許す。
        first = max(s['statements'] for s in x)
        last = max(s['statements'] for s in y)
        assert last <= first + 2, (name, 'api statements', first, last)
        print(f'API SQL {name}: month SQL {y[0]["month_sql"]:.0f}, statements {max(s["statements"] for s in y):.0f}, '
              f'blocks {initial} → {later}, DB ms max={max(s["db_ms"] for s in y):.3f}')
    for key, values in sorted(api_groups[1].items()):
        p50, p95, top = percentile(values)
        print(f'API {key}: p50={p50:.3f} p95={p95:.3f} max={top:.3f} ms')
    return a, b


if __name__ == '__main__':
    compare(*sys.argv[1:])
