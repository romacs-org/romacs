#!/usr/bin/env python3
"""Whole-process VM2 partial-world measurement. NOT shipping-editor equivalence.

Counts include exec, dynamic loading, mapping, authenticated page access, compile of
one already-read query, execution and exit. RSS is a separate exact-command run.
The inherited counter's attr ABI is reused; bench.guarded requires count > 0 and
running == enabled exactly and checks all ioctls. No source fallback is possible.
"""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import tempfile
import time
from typing import Any
from bench import guarded, load, ROOT, Unscheduled


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--binary-dir', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    binary = str(args.binary_dir / 'romacs-vm')
    producer = str(args.binary_dir / 'romacs-vm-image')
    counter = ROOT.parents[1] / 'scripts/romacs_accept_counter.py'
    attributes = load(counter, 'counter_abi').attributes
    scratch = args.output.parent
    scratch.mkdir(parents=True, exist_ok=True)
    report: dict[str, Any] = dict(scope='partial mapped cons/string world, NOT shipping editor',
                  resolution_instructions=36, reference_instructions=285974610,
                  target_complete_world_instructions=50000000,
                  gnu=subprocess.check_output(['/usr/bin/emacs', '--version'], text=True).splitlines()[0],
                  binary_sha256=hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                  cases=[], rejections=[])

    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    def measure(command, payload, expected, image_key=None):
        samples = []
        for rep in range(3):
            before = Path('/proc/loadavg').read_text().strip()
            try:
                sample = guarded(command, payload, attributes)
            except Unscheduled as e:
                report['rejections'].append(dict(command=command, rep=rep, error=str(e),
                    loadavg_before=before, loadavg_after=Path('/proc/loadavg').read_text().strip()))
                save()
                raise
            sample.update(loadavg_before=before, loadavg_after=Path('/proc/loadavg').read_text().strip())
            assert sample['stdout'] == expected, sample
            if image_key:
                stats = json.loads(sample['stderr'])
                assert stats['mapped_image'] == image_key
                assert stats['mapped_bytes'] > 0
                assert stats['startup_collections'] == stats['startup_source_loads'] == stats['startup_private_objects'] == 0
            samples.append(sample)
        walls = []
        wall_loadavg = []
        for _ in range(3):
            before = Path('/proc/loadavg').read_text().strip()
            started = time.perf_counter_ns()
            run = subprocess.run(command, input=payload, capture_output=True, check=True)
            walls.append(time.perf_counter_ns() - started)
            wall_loadavg.append(dict(before=before, after=Path('/proc/loadavg').read_text().strip()))
            assert run.stdout.decode().strip() == expected
            if image_key:
                assert json.loads(run.stderr)['mapped_image'] == image_key
        with tempfile.NamedTemporaryFile() as out:
            before = Path('/proc/loadavg').read_text().strip()
            run = subprocess.run(['/usr/bin/time', '-f', '%M', '-o', out.name, '--'] + command,
                                 input=payload, capture_output=True, check=True)
            after = Path('/proc/loadavg').read_text().strip()
            assert run.stdout.decode().strip() == expected
            rss = int(Path(out.name).read_text())
        return dict(command=command, samples=samples, wall_ns=walls, wall_loadavg=wall_loadavg,
                    median_wall_ns=statistics.median(walls),
                    median_instructions=statistics.median(s['count'] for s in samples),
                    peak_rss_kib=rss, rss_loadavg_before=before, rss_loadavg_after=after)
    try:
        gnu = measure(['/usr/bin/emacs', '-Q', '--batch', '--eval', '(princ 6)'], b'', '6')
        report['gnu_empty_startup_query'] = gnu
        for n in (1, 16384, 262144):
            source = scratch / f'world-{n}.json'
            source.write_text(json.dumps(['Ελλάδα'] * n, ensure_ascii=False))
            store = scratch / 'store'
            built = json.loads(subprocess.check_output([producer, str(store), str(source)]))
            artifact = store / built['key']
            assert hashlib.sha256((store / (built['key'] + '.pages')).read_bytes()).hexdigest() == built['key']
            cmd = [binary, '--image', str(store), built['key'], str(built['root'])]
            result = measure(cmd, b'["length",["car","world"]]', '6', built['key'])
            result.update(strings=n, conses=n, image_bytes=artifact.stat().st_size,
                          key=built['key'], root=built['root'])
            result['ratio_to_gnu_startup'] = result['median_instructions'] / gnu['median_instructions']
            result['ratio_to_provided_shipping_reference'] = result['median_instructions'] / report['reference_instructions']
            result['peak_rss_ratio_to_gnu'] = result['peak_rss_kib'] / gnu['peak_rss_kib']
            assert result['peak_rss_ratio_to_gnu'] < 5, result
            report['cases'].append(result)
            save()
        report['complete'] = True
        # Exercise the exact runner under PMU contention; rejection must not emit
        # a count that could be mistaken for a very fast startup.
        before = Path('/proc/loadavg').read_text().strip()
        try:
            guarded([binary], b'42', attributes, pressure=12)
        except Unscheduled as e:
            report['pressure_guard'] = dict(rejected=True, error=str(e),
                loadavg_before=before, loadavg_after=Path('/proc/loadavg').read_text().strip())
        else:
            raise AssertionError('oversubscribed counter was accepted')
    except BaseException as e:
        report['complete'] = False
        report['error'] = str(e)
        raise
    finally:
        save()
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
