#!/usr/bin/env python3
"""Compile/exercise production pure time helpers, NOT VM/registration proof.
No Cargo invocation. Run after time_oracle.py with the project toolchain on PATH.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
ROOT = Path(__file__).resolve().parents[1]
source = (ROOT/'src/prims/time.rs').read_text()
oracle = json.loads((ROOT/'evidence/time/oracle.json').read_text())
parts = ['#[derive(Debug)] enum Fault { Error(String) }', 'fn time_overflow()->Fault {Fault::Error("overflow".into())}']
parts += [source[source.index('#[derive(Clone)]\nstruct Zone'):source.index('fn zone_fault')]]
parts += [source[source.index('fn numeric_zone'):source.index('fn zone(cx:')]]
parts += [source[source.index('struct Calendar'):source.index('defsubr! { name = "format-time-string"')]]
statements = []
for i,row in enumerate(oracle['rows']):
    if row['primitive'] != 'format-time-string' or i >= 38 or i == 7:
        continue
    expected = json.loads(row['expected'][4:-1])
    if i < 7:
        fmt = json.loads(row['form'].split('(format-time-string ',1)[1].split(" '(0",1)[0])
        seconds,nanos,zone = 0,123456789,'numeric_zone(0)'
    else:
        fmt='%F %T %Z %z %:z %::z %:::z'
        seconds=int(row['form'].split('" ')[1].split(' ')[0])
        nanos=0
        zone_spec=row['form'].split(str(seconds)+' ',1)[1][:-1]
        zone={'t':'numeric_zone(0)','0':'numeric_zone(0)','3600':'numeric_zone(3600)','-19800':'numeric_zone(-19800)',"'(19800 \"IST\")":'Zone {offset:19800,name:"IST".into(),dst:false}', '"UTC0"':'fixed_rule("UTC0").unwrap()', '"EST5"':'fixed_rule("EST5").unwrap()', '"America/New_York"':f'tzif(&std::fs::read("/usr/share/zoneinfo/America/New_York").unwrap(),{seconds}).unwrap()', 'nil':'fixed_rule("UTC0").unwrap()', "'wall":f'tzif(&std::fs::read("/etc/localtime").unwrap(),{seconds}).unwrap()'}[zone_spec]
    q=lambda s:json.dumps(s,ensure_ascii=False)
    statements.append(f'{{let z={zone}; let c=calendar({seconds},z.offset).unwrap(); assert_eq!(format_calendar({q(fmt)},&c,&z,{seconds},{nanos}).unwrap(),{q(expected)},"native row {i}");}}')
for row in oracle['rows']:
    if '/calendar-' not in row['id']:
        continue
    seconds=int(row['id'].split('/calendar-')[1])
    expected=json.loads(row['expected'][4:-1])
    if row['primitive']=='format-time-string':
        expression='format_calendar("%F %j %U %W %V %G %g",&c,&z,seconds,0).unwrap()'
    elif row['primitive']=='current-time-string':
        expression='format!("{} {}",format_calendar("%a %b %e %T",&c,&z,seconds,0).unwrap(),c.year)'
    else:
        continue
    statements.append(f'{{let seconds={seconds};let z=numeric_zone(0);let c=calendar(seconds,0).unwrap();assert_eq!({expression},{json.dumps(expected)},"{row["id"]}");}}')
parts.append('fn main() {'+'\n'.join(statements)+f'\nprintln!("{{}} native GNU assertions passed", {len(statements)});'+'}')
text='\n'.join(parts)
with tempfile.TemporaryDirectory(prefix='time-native-',dir=os.environ['TMPDIR']) as tmp:
    tmp=Path(tmp)
    (tmp/'probe.rs').write_text(text)
    compile=subprocess.run(['rustc','--edition=2024','-Awarnings',str(tmp/'probe.rs'),'-o',str(tmp/'probe')],capture_output=True,text=True)
    if compile.returncode:
        print(compile.stderr)
        raise SystemExit(compile.returncode)
    run=subprocess.run([str(tmp/'probe')],capture_output=True,text=True)
    report=dict(kind='production pure helpers only; not VM execution',assertions=len(statements),source_sha256=hashlib.sha256(source.encode()).hexdigest(),exit_code=run.returncode,stdout=run.stdout,stderr=run.stderr)
    (ROOT/'evidence/time/native-results.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
    raise SystemExit(run.returncode)
