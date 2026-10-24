#!/usr/bin/env python3
"""Validate five time mutants; --run requires parent-owned Cargo build slot."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
ROOT=Path(__file__).resolve().parents[1]
EVIDENCE=ROOT/'evidence/time'
SOURCE=ROOT/'src/prims/time.rs'
p=argparse.ArgumentParser()
p.add_argument('--run',action='store_true',help='build/test each mutant; owner must grant build slot')
args=p.parse_args()
original=SOURCE.read_text()
rows=json.loads((EVIDENCE/'mutants.json').read_text())
assert len(rows)==5 and len({r['primitive'] for r in rows})==5
for row in rows:
    assert original.count(row['old'])==1,row['id']
if not args.run:
    print(json.dumps({'validated_mutants':len(rows),'executed':False,'source_sha256':hashlib.sha256(original.encode()).hexdigest()}))
    raise SystemExit(0)
def run(test, ident):
    result=subprocess.run(['cargo','test','--config',str(ROOT.parents[1]/'.cargo/dev.toml'),'--profile','dev-fast','-p','romacs-vm','--lib','prim::prims::time::tests::'+test,'--','--nocapture'],cwd=ROOT.parents[1],env={**os.environ,'TZ':'UTC0','LC_ALL':'C'},capture_output=True,text=True)
    output=result.stdout+result.stderr
    (EVIDENCE/(ident+'.log')).write_text(output)
    return result.returncode,output
code,output=run('', 'baseline')
assert code==0 and '8 passed' in output,'Baseline must pass all eight tests before mutation'
results=[]
for row in rows:
    assert SOURCE.read_text()==original,'Concurrent source writer: refusing mutation'
    mutant=original.replace(row['old'],row['new'])
    try:
        SOURCE.write_text(mutant)
        code,output=run(row['test'],row['id'])
        killed=code!=0 and 'assertion `left == right` failed' in output and 'TIME_ORACLE' in output and 'error: could not compile' not in output
        results.append({'id':row['id'],'killed_at_behavioral_assertion':killed,'exit_code':code})
    finally:
        assert SOURCE.read_text()==mutant,'Concurrent source writer: preserving unexpected source'
        SOURCE.write_text(original)
(EVIDENCE/'mutation-results.json').write_text(json.dumps(results,indent=2)+'\n')
assert all(r['killed_at_behavioral_assertion'] for r in results),'Surviving or invalid mutant'
print(json.dumps(results,indent=2))
