"""履歴量の異なる電話SQLの探索量を比較する（時間は参考値）。"""
import json
from pathlib import Path
import statistics
import sys

def groups(path):
    data=json.loads(Path(path).read_text())
    grouped={}
    for sample in data['samples']:
        grouped.setdefault((sample['name'],sample['concurrency']),[]).append(sample)
    assert len(grouped)==data['queries']*2
    for (name,concurrency),samples in grouped.items():
        assert len(samples)==30*concurrency,(name,concurrency,len(samples))
    return data,grouped

def compare(first,second):
    small,a=groups(first);large,b=groups(second)
    assert small['rows']>=300000 and large['rows']>=1000000
    assert a.keys()==b.keys()
    api_groups=[]
    for data in (small,large):
        api={}
        for sample in data['api_samples']: api.setdefault((sample['name'],sample['concurrency']),[]).append(sample['roundtrip_ms'])
        assert set(api)=={(name,n) for name in ('directory','command','pbx-endpoint') for n in (1,8)}
        for (name,n),values in api.items(): assert len(values)==30*n,(name,n,len(values))
        api_groups.append(api)

    for key in sorted(a):
        x,y=a[key],b[key]
        for metric in ('blocks',):
            initial=max(s[metric] for s in x); later=max(s[metric] for s in y)
            assert later<=2*initial+64,(key,metric,initial,later)
        relations=set().union(*(s['relations'] for s in x+y))
        for relation in relations:
            initial=max(s['relations'].get(relation,0) for s in x)
            later=max(s['relations'].get(relation,0) for s in y)
            assert later<=2*initial+8,(key,relation,initial,later)
        values=sorted(s['roundtrip_ms'] for s in y)
        print(f'{key}: blocks {max(s["blocks"] for s in x)} → {max(s["blocks"] for s in y)}, SQL roundtrip ms p50={statistics.median(values):.3f} p95={values[int(.95*(len(values)-1))]:.3f} max={max(values):.3f}')
    for key,values in sorted(api_groups[1].items()):
        values=sorted(values)
        print(f'API {key}: p50={statistics.median(values):.3f} p95={values[int(.95*(len(values)-1))]:.3f} max={max(values):.3f} ms')
    return a,b
if __name__=='__main__': compare(*sys.argv[1:])
