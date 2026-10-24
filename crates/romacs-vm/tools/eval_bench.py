#!/usr/bin/env python3
"""Guarded, whole-process external eval: identical JSON AST requests to VM2/GNU.

The request bodies are imported verbatim from slope_bench.CASES (only N changes).
Each request is parsed, compiled/evaluated, executed, serialized, and flushed.
No request is precompiled or constant-folded by this harness. GNU source input is
also recorded, to expose the cost of the comparison's JSON-AST adapter.
"""
import argparse
import json
import hashlib
from pathlib import Path
import statistics
import subprocess
import time
from typing import Any
from bench import ROOT, GNU, SOURCE, COUNTER, Unscheduled, guarded, load, read_form

GNU_DRIVER = r'''(progn (require 'json) (setq lexical-binding t)
  (defun vm2-decode (x)
    (cond ((integerp x) x) ((stringp x) (intern x))
          ((and (consp x) (eq (car x) :string)) (cadr x))
          ((and (consp x) (eq (car x) :vector)) (vconcat (mapcar #'vm2-decode (cadr x))))
          ((listp x) (mapcar #'vm2-decode x))
          (t (error "Unsupported external form"))))
  (condition-case nil
    (while t
      (let* ((line (read-string ""))
             (form REQUEST)
             (result (condition-case e
                         (list :value (prin1-to-string (eval form t)))
                       (error (list :error (symbol-name (car e)))))))
        (princ (json-serialize result)) (terpri)))
    (end-of-file nil)))'''


def gnu_stream(source=False):
    request = '(read line)' if source else "(vm2-decode (json-parse-string line :array-type 'list :object-type 'plist))"
    return [GNU, '-Q', '--batch', '--eval', GNU_DRIVER.replace('REQUEST', request)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', default=str(ROOT / 'target/release/romacs-vm'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--vm2-source', action='store_true', help='Send native source requests to VM2, not GNU-read Form JSON')
    parser.add_argument('--iterations', type=int, default=20)
    parser.add_argument('--requests', type=int, nargs=3, default=[256, 512, 1024])
    args = parser.parse_args()
    if args.iterations <= 0 or not (1 < args.requests[0] < args.requests[1] < args.requests[2]):
        parser.error('positive iterations and strictly increasing request counts above one required')
    attributes = load(COUNTER, 'counter').attributes
    cases = load(SOURCE, 'original_cases').CASES
    report: dict[str, Any] = dict(warm_asymmetry='VM2 reuses source IR after first request; GNU-source reads/evals every request with no retained compiled state. Repeated submission is an agent-only pattern, not a cold-eval win.', method='median of three fresh processes; adjacent slopes per external request',
                  request_counts=[1, *args.requests], reps=3,
                  wire=('VM2 source requests; GNU-json is a separate historical adapter comparison; GNU-source is native text' if args.vm2_source else 'NDJSON already-read forms; identical bytes for VM2 and GNU-json; GNU-source is native text'),
                  boundary='accept + parse + Rust compile/private lowering or GNU eval + execute + print + flush; process startup included in raw counts',
                  resolution_instructions=36, slope_floor_instructions=72, load_before=Path('/proc/loadavg').read_text(),
                  binary=str(Path(args.binary).resolve()), source=str(SOURCE),
                  binary_sha256=hashlib.sha256(Path(args.binary).read_bytes()).hexdigest(),
                  samples=[], rejections=[], cases=[])

    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    def measure(command, payload, expected, label, requests):
        counts = []
        for rep in range(3):
            sample = None
            for attempt in range(10):
                try:
                    sample = guarded(command, payload * requests, attributes)
                    break
                except Unscheduled as error:
                    report['rejections'].append(dict(label=label, requests=requests, rep=rep, attempt=attempt, error=str(error)))
                    save()
                    if attempt == 9:
                        raise
                    time.sleep(min(attempt + 1, 3))
            assert sample is not None
            responses = [json.loads(line) for line in sample['stdout'].splitlines()]
            if responses != [expected] * requests:
                raise RuntimeError(f'known-answer failure: {label}/{requests}: {responses}')
            sample.update(label=label, requests=requests, rep=rep, expected=expected, checked_responses=len(responses))
            report['samples'].append(sample)
            counts.append(sample['count'])
            save()
        return statistics.median(counts)

    try:
        for name, _, body in cases:
            if name == 'buf-insert':
                continue
            source = body.replace('N', str(args.iterations))
            data = (json.dumps({'source': source}).encode() if args.vm2_source else read_form(source).strip()) + b'\n'
            oracle = subprocess.check_output(gnu_stream(source=True), input=source.encode() + b'\n')
            expected = json.loads(oracle)
            if set(expected) != {'value'}:
                raise RuntimeError(f'GNU oracle failed {name}: {expected}')
            row = dict(name=name, original_body=body, source=source, n=args.iterations,
                       transport=json.loads(data), expected=expected, medians={}, slopes={})
            for engine, command, payload in [
                ('vm2', [args.binary, '--eval-stream'], data),
                ('gnu-json', gnu_stream(), read_form(source).strip() + b'\n' if args.vm2_source else data),
                ('gnu-source', gnu_stream(source=True), source.encode() + b'\n'),
            ]:
                engine_expected = ({**expected, 'result': {'type':'integer','value':int(expected['value'])}} if engine == 'vm2' and args.vm2_source else expected)
                medians = {str(r): measure(command, payload, engine_expected, f'{name}/{engine}', r) for r in (1, *args.requests)}
                row['medians'][engine] = medians
                slopes = []
                for lo, hi in zip(args.requests, args.requests[1:]):
                    delta = medians[str(hi)] - medians[str(lo)]
                    if delta <= 72:
                        raise RuntimeError(f'unresolved/nonpositive slope {name}/{engine}/{lo}-{hi}: {delta}')
                    slopes.append(dict(lo=lo, hi=hi, delta=delta, instructions_per_request=delta/(hi-lo)))
                row['slopes'][engine] = slopes
            row['ratio_json'] = [row['slopes']['vm2'][i]['instructions_per_request'] / row['slopes']['gnu-json'][i]['instructions_per_request'] for i in range(2)]
            row['ratio_source'] = [row['slopes']['vm2'][i]['instructions_per_request'] / row['slopes']['gnu-source'][i]['instructions_per_request'] for i in range(2)]
            report['cases'].append(row)
            save()
            print(f"{name}: full eval instructions/request={[round(s['instructions_per_request'], 2) for s in row['slopes']['vm2']]} VM2/GNU-json={row['ratio_json']} VM2/GNU-source={row['ratio_source']}", flush=True)
        assert len(report['cases']) == 5
        assert len(report['samples']) == 180
        report['complete'] = True
    except BaseException as error:
        report.update(complete=False, error=str(error))
        raise
    finally:
        report['load_after'] = Path('/proc/loadavg').read_text()
        save()

if __name__ == '__main__':
    main()
