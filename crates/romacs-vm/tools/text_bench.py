#!/usr/bin/env python3
"""Exact frozen baseline workloads, new VM2 candidate only. No source rewriting."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import sys, json, hashlib, statistics, subprocess, os
from pathlib import Path
if not __debug__: raise RuntimeError('evidence requires assertions; Python -O is forbidden')
BASE = Path(_portable_os.environ['ROMACS_BENCH_SUPPORT'])
sys.path.insert(0,str(BASE))
import run as baseline
from guard import guarded
sys.path.insert(0,str(Path(__file__).resolve().parent))
from bench import read_form
OUT = Path(os.environ.get('S3_EVIDENCE',str(_PortablePath(_portable_tempfile.gettempdir()) / 'romacs/vm2/s3-text-evidence')))
BIN = os.environ.get('S3_BINARY',str(_PortablePath(_portable_tempfile.gettempdir()) / 'romacs/vm2/s3-text-target/release/romacs-vm'))

def main():
    OUT.mkdir(parents=True,exist_ok=True)
    group=sys.argv[1] if len(sys.argv)>1 else 'buffer'
    report=dict(resolution=36,slope_floor=72,samples=[],rows=[],complete=False,binary=BIN,sha256=hashlib.sha256(Path(BIN).read_bytes()).hexdigest())
    def save(): (OUT/f'{group}.json').write_text(json.dumps(report,indent=2)+'\n')
    if group=='buffer':
        ns={'append-read':20000,'scan':128,'locate-newline':200000,'char-at':200000,'mutate-scan':128}
        rows=[(f'{name}-{fixture}',n,name,fixture) for fixture in ['ascii','unicode'] for name,n in ns.items()]
    else: rows=[(name,n*10,body,None) for name,n,body in baseline.cases() if name!='buf-insert']
    for row,n,body,fixture in rows:
        prepared={}
        for factor in [1,2]:
            N=n*factor
            if group=='buffer':
                file=BASE/'programs'/f'buffer-{row}.el'
                source='(progn\n'+file.read_text()+f'\n(bench-run {N}))'
                guile=baseline.guile_cmd('buffer-'+row,N)
                expected=baseline.expected_buffer(body,fixture,N)
                report.setdefault('source_hashes',{})[str(file)]=hashlib.sha256(file.read_bytes()).hexdigest()
            else:
                source=body.replace('N',str(N));guile=baseline.guile_cmd('general-'+row,N)
                expected=baseline.expected_general(row,N)
            data=read_form(source)
            (OUT/f'{row}-{N}.json').write_bytes(data)
            prepared[factor]=data,guile,str(expected)
        for rep in range(3):
            for factor in ([1,2] if rep%2==0 else [2,1]):
                data,guile,expected=prepared[factor]
                for engine in (['vm2','guile'] if (rep+factor)%2 else ['guile','vm2']):
                    got=guarded([BIN] if engine=='vm2' else guile,data if engine=='vm2' else b'',baseline.ENV)
                    got.update(row=row,n=n*factor,rep=rep,engine=engine,expected=expected)
                    report['samples'].append(got);save()
                    assert got['stdout']==expected,(row,engine,got)
                    assert engine!='guile' or 'jit:' not in got['stderr'].lower()
        costs={}
        for engine in ['vm2','guile']:
            slopes=[]
            for rep in range(3):
                samples={s['n']:s for s in report['samples'] if s['row']==row and s['engine']==engine and s['rep']==rep}
                delta = samples[2*n]['count']-samples[n]['count']
                if delta <= 72: raise RuntimeError(f'slope below resolution: {row}/{engine}')
                slopes.append(delta/n)
            costs[engine]=dict(slopes=slopes,median=statistics.median(slopes))
        result=dict(name=row,n=n,costs=costs,ratio=costs['vm2']['median']/costs['guile']['median'])
        report['rows'].append(result);save();print(json.dumps(result),flush=True)
    report['complete']=True;save()
if __name__=='__main__':main()
