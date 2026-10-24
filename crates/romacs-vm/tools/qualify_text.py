#!/usr/bin/env python3
"""Qualify text results and measure fresh-process RSS, never fork RSS."""
import json, subprocess, hashlib
from pathlib import Path
from text_bench import baseline, guarded, OUT, BIN
from bench import read_form
from guard import validate

def main():
    report=json.loads((OUT/'buffer.json').read_text())
    assert report['complete'] and len(report['rows'])==10 and len(report['samples'])==120
    assert report['sha256']==hashlib.sha256(Path(BIN).read_bytes()).hexdigest()
    for sample in report['samples']:
        validate(sample['count'],sample['enabled'],sample['running'])
        assert sample['stdout']==sample['expected']
    for vals in [(0,1,1),(1,2,1),(1,100000000,99999999)]:
        try:validate(*vals)
        except RuntimeError:pass
        else:raise AssertionError(vals)
    from read_ratchet import qualify
    qualify(report)
    for row in report['rows']:
        for engine in ['vm2','guile']:
            samples=[s for s in report['samples'] if s['row']==row['name'] and s['engine']==engine]
            assert len(samples)==6
    evidence=dict(small_answers=[],rss=[],jit={},source_hashes={},full_state=[])
    frozen=json.loads((baseline.ROOT/'manifest.json').read_text())['hashes']
    for pattern in ['buffer-*.el','buffer-*.scm','buffer-*.go']:
        for p in (baseline.ROOT/'programs').glob(pattern):
            actual=hashlib.sha256(p.read_bytes()).hexdigest()
            assert actual==frozen[str(p)],p
            evidence['source_hashes'][str(p)]=actual
    for fixture in ['ascii','unicode']:
        for name in baseline.BUF_EL:
            stem=f'buffer-{name}-{fixture}'
            original=(baseline.ROOT/'programs'/f'{stem}.el').read_text()
            for n in [1,2,7]:
                expected=str(baseline.expected_buffer(name,fixture,n))
                data=read_form('(progn\n'+original+f'\n(bench-run {n}))')
                vm=subprocess.run([BIN],input=data,capture_output=True,check=True).stdout.decode().strip()
                guile=baseline.invoke(baseline.guile_cmd(stem,n)).decode().strip()
                assert vm==guile==expected
                evidence['small_answers'].append(dict(row=stem,n=n,expected=expected,actual=vm))
        # Exact byte equality after the same thirteen indexed replacements.
        file=baseline.ROOT/f'{fixture}.txt'
        actual=hashlib.sha256(file.read_bytes()).hexdigest();assert actual==frozen[str(file)]
        evidence['source_hashes'][str(file)]=actual
        count_source=f'(with-temp-buffer (car (cdr (insert-file-contents {json.dumps(str(file))}))))'
        count=subprocess.run([BIN],input=read_form(count_source),capture_output=True,check=True).stdout.decode().strip()
        assert count==str(len(file.read_text()))
        text=file.read_text(); chars=list(text)
        for i in range(13):chars[(i*131)%len(chars)]='Z'
        source=f'(with-temp-buffer (insert-file-contents {json.dumps(str(file))}) (let ((i 0) (len (buffer-size))) (while (< i 13) (goto-char (1+ (% (* i 131) len))) (delete-region (point) (1+ (point))) (insert "Z") (setq i (1+ i))) (buffer-substring-no-properties (point-min) (point-max))))'
        vm=subprocess.run([BIN],input=read_form(source),capture_output=True,check=True)
        assert json.loads(vm.stdout)==''.join(chars)
        evidence['full_state'].append(dict(fixture=fixture,sha256=hashlib.sha256(''.join(chars).encode()).hexdigest()))
    cmd=baseline.guile_cmd('general-arith-loop',2000000)
    off=subprocess.run(cmd,env=baseline.ENV,capture_output=True,check=True)
    onenv=baseline.ENV.copy();onenv['GUILE_JIT_THRESHOLD']='0'
    on=subprocess.run(cmd,env=onenv,capture_output=True,check=True)
    assert b'jit:' not in off.stderr.lower() and b'jit:' in on.stderr.lower() and on.stdout==off.stdout
    evidence['jit']=dict(off=off.stderr.decode(),on=on.stderr.decode(),answer=off.stdout.decode())
    payload=(OUT/'char-at-ascii-200000.json').read_bytes()
    try:guarded([BIN],payload,baseline.ENV,pressure=64)
    except RuntimeError as e:
        assert 'UNSCHEDULED' in str(e);evidence['pressure_refused']=str(e)
    else:raise AssertionError('accepted pressure')
    for row in report['rows']:
        name=row['name'];n=2*row['n'];body,fixture=name.rsplit('-',1)
        expected=str(baseline.expected_buffer(body,fixture,n))
        for engine in ['vm2','guile','gnu']:
            cmd=[BIN] if engine=='vm2' else baseline.buffer_cmd(engine,'buffer-'+name,n)
            data=(OUT/f'{name}-{n}.json').read_bytes() if engine=='vm2' else b''
            for rep in range(3):
                path=OUT/'rss-current.txt';before=Path('/proc/loadavg').read_text().strip()
                run=subprocess.run(['/usr/bin/time','-f','%M','-o',str(path),'--',*cmd],input=data,capture_output=True,env=baseline.ENV,check=True)
                assert run.stdout.decode().strip()==expected
                evidence['rss'].append(dict(row=name,engine=engine,rep=rep,rss_kib=int(path.read_text()),answer=expected,load_before=before,load_after=Path('/proc/loadavg').read_text().strip()))
            (OUT/'qualification.json').write_text(json.dumps(evidence,indent=2)+'\n')
    assert len(evidence['rss'])==90 and len(evidence['small_answers'])==30
    for row in report['rows']:
        peaks={e:max(s['rss_kib'] for s in evidence['rss'] if s['row']==row['name'] and s['engine']==e) for e in ['vm2','guile','gnu']}
        assert peaks['vm2']<5*peaks['gnu']
        print(row['name'],peaks,flush=True)
    evidence['complete']=True
    (OUT/'qualification.json').write_text(json.dumps(evidence,indent=2)+'\n')
if __name__=='__main__':main()
