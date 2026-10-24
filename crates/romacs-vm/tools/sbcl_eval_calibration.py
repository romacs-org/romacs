#!/usr/bin/env python3
"""Evaluator-only calibration, distinct from the external eval-stream gate.
Every engine checks every result and prints one final integer. Cold requests
read and eval each time. Prepared/general rows compile once then call repeatedly.
There is NO VM2 content cache in this probe, and no warm-cache claim.
"""
import os as _portable_os
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
from bench import guarded, load, COUNTER, SOURCE

SBCL = _portable_os.environ.get('ROMACS_SBCL', 'sbcl')
CL = {
 'funcall':'(let ((f (lambda (x) (* x 2))) (s 0) (i 0)) (loop while (< i 20) do (setq s (funcall f i)) (setq i (1+ i))) s)',
 'arith-loop':'(let ((i 0) (s 0)) (loop while (< i 20) do (setq s (+ s i)) (setq i (1+ i))) s)',
 'letbind':'(let ((i 0) (s 0)) (loop while (< i 20) do (let ((a i) (b (1+ i))) (setq s (+ a b))) (setq i (1+ i))) s)',
 'cons':'(let ((l nil) (i 0)) (loop while (< i 20) do (setq l (cons i l)) (setq i (1+ i))) (length l))',
 'recurse':'(progn (defun bw (n acc) (if (= n 0) acc (bw (1- n) (+ acc n)))) (let ((i 0) (s 0)) (loop while (< i 20) do (setq s (bw 20 0)) (setq i (1+ i))) s))',
}

def command(engine, source, expected, count, probe):
    if engine.startswith('vm2'):
        return [probe, 'prepared' if engine.endswith('prepared') else 'eval', str(count), str(expected), source]
    if engine.startswith('gnu'):
        prepare = f"(byte-compile '(lambda () {source}))" if engine.endswith('compiled') else 'nil'
        body = '(funcall function)' if engine.endswith('compiled') else f'(eval (read {json.dumps(source)}) t)'
        expression = f'''(progn (setq lexical-binding t)
          (let ((function {prepare}) (i 0) (answer 0))
            (while (< i {count}) (setq answer {body})
              (unless (= answer {expected}) (error "calibration answer"))
              (setq i (1+ i))) (princ {expected})))'''
        return ['/usr/bin/emacs', '-Q', '--batch', '--eval', expression]
    prepared = engine.endswith('compiled')
    mode = ':interpret' if engine.endswith('interpret') else ':compile'
    prepare = f"(compile nil '(lambda () {source}))" if prepared else 'nil'
    body = '(funcall function)' if prepared else f'(eval (read-from-string {json.dumps(source)}))'
    expression = f'''(let* ((sb-ext:*evaluator-mode* {mode}) (function {prepare}))
       (dotimes (i {count}) (unless (= {body} {expected}) (error "calibration answer")))
       (format t "{expected}~%"))'''
    return [SBCL, '--noinform', '--non-interactive', '--eval', expression]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--probe', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    original = {name:body.replace('N','20') for name,_,body in load(SOURCE,'calibration_cases').CASES if name != 'buf-insert'}
    original['tiny-add'] = '(+ 1 2)'
    CL['tiny-add'] = '(+ 1 2)'
    attributes = load(COUNTER,'calibration_counter').attributes
    report = {'scope':__doc__, 'sbcl':subprocess.check_output([SBCL,'--version'],text=True).strip(), 'probe_sha256':hashlib.sha256(Path(args.probe).read_bytes()).hexdigest(), 'load_before':Path('/proc/loadavg').read_text(), 'request_counts':[1,256,512,1024], 'reps':3, 'resolution':36,'slope_floor':72,'samples':[],'cases':[]}
    def save():
        Path(args.output).write_text(json.dumps(report,indent=2)+'\n')
    try:
        for name, elisp in original.items():
            expected = int(subprocess.check_output(['/usr/bin/emacs','-Q','--batch','--eval',f'(progn (setq lexical-binding t) (princ {elisp}))'],text=True))
            row = {'name':name,'elisp':elisp,'common_lisp':CL[name],'expected':expected,'medians':{},'slopes':{}}
            for engine in ['vm2-eval','gnu-eval','sbcl-compile','sbcl-interpret','vm2-prepared','gnu-compiled','sbcl-compiled']:
                source = CL[name] if engine.startswith('sbcl') else elisp
                medians = {}
                for count in report['request_counts']:
                    counts = []
                    for rep in range(3):
                        sample = guarded(command(engine,source,expected,count,args.probe), b'', attributes)
                        assert sample['exit'] == 0 and sample['stdout'].strip() == str(expected), (name,engine,sample)
                        assert sample['count'] > 0 and sample['enabled'] > 0 and sample['running'] == sample['enabled']
                        sample.update(name=name,engine=engine,requests=count,rep=rep)
                        report['samples'].append(sample)
                        counts.append(sample['count'])
                        save()
                    medians[str(count)] = statistics.median(counts)
                slopes = [(medians[str(hi)]-medians[str(lo)])/(hi-lo) for lo,hi in [(256,512),(512,1024)]]
                assert all(medians[str(hi)]-medians[str(lo)] > 72 for lo,hi in [(256,512),(512,1024)])
                row['medians'][engine] = medians
                row['slopes'][engine] = slopes
            report['cases'].append(row)
            save()
            print(name, json.dumps(row['slopes']), flush=True)
        assert len(report['cases']) == 6 and len(report['samples']) == 504
        report['complete'] = True
    finally:
        report['load_after'] = Path('/proc/loadavg').read_text()
        save()

if __name__ == '__main__':
    main()
