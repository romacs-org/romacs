#!/usr/bin/env python3
"""VM2 executable-world mechanism benchmark, explicitly NOT GUI equivalence.

Three independent processes per path; positive :u counts with EXACT scheduling,
paired same-process wait4 CPU/fault/RSS evidence, and loadavg on every sample.
"""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
from typing import Any
from bench import guarded, load


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--functions', type=int, default=1000)
    args = parser.parse_args()
    assert args.functions >= 43
    args.output.parent.mkdir(parents=True, exist_ok=True)
    binary = args.binary_dir / 'romacs-vm'
    producer = args.binary_dir / 'romacs-vm-image'
    root = Path(__file__).resolve().parents[3]
    attrs = load(root / 'tools/lib/romacs_accept_counter.py', 'counter').attributes
    setup = ['progn'] + [['defun', f'cold-{i}', [], ['+', i, 1]] for i in range(args.functions)]
    entry = ['cold-42']
    source = args.output.parent / 's8-snapshot-input.json'
    source.write_text(json.dumps(dict(setup=setup, entry=entry)))
    store = args.output.parent / 's8-code-store'
    built = json.loads(subprocess.check_output([str(producer), '--snapshot-code', str(store), str(source)]))
    assert built['config'] == hashlib.sha256(source.read_bytes()).hexdigest()
    assert hashlib.sha256((store / (built['key'] + '.pages')).read_bytes()).hexdigest() == built['key']
    image_command = [str(binary), '--code-image', str(store), built['key'], built['config']]
    # Same final answer and definitions. The plain path reads, compiles and evaluates
    # setup on every launch; snapshot setup runs once in the producer, outside timing.
    plain = json.dumps(['progn', setup, entry]).encode()
    report: dict[str, Any] = dict(scope='standalone VM2 synthetic configured world; NOT romacs -q GUI',
                  functions=args.functions, image_key=built['key'], config=built['config'],
                  image_bytes=(store / built['key']).stat().st_size,
                  binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  resolution_instructions=36, conservative_slope_floor=72,
                  samples=[], medians={}, complete=False)

    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    try:
        for label, command, payload in [('compile-evaluate', [str(binary)], plain), ('snapshot', image_command, b'')]:
            samples = []
            for rep in range(3):
                before = Path('/proc/loadavg').read_text().strip()
                sample = guarded(command, payload, attrs)
                sample.update(label=label, rep=rep, loadavg_before=before,
                              loadavg_after=Path('/proc/loadavg').read_text().strip())
                assert sample['stdout'] == '43', sample
                if label == 'snapshot':
                    stats = json.loads(sample['stderr'])
                    assert stats['mapped_image'] == built['key'], stats
                    assert stats['startup_source_loads'] == stats['startup_compilations'] == 0, stats
                    assert stats['materialized_functions'] == 2, stats
                    assert stats['verified_pages'] <= stats['metadata_pages'] + 2, stats
                    assert stats['verified_pages'] * 4096 < report['image_bytes'] / 4, stats
                    sample['image_stats'] = stats
                samples.append(sample)
                report['samples'].append(sample)
                save()
            report['medians'][label] = {
                field: statistics.median(s[field] for s in samples)
                for field in ['count', 'wall_ns', 'user_cpu_s', 'system_cpu_s', 'minor_faults', 'major_faults', 'peak_rss_kib']}
            # Process-lifetime high-water RSS can include fork-before-exec
            # launcher memory. Keep wait4 and time(1) values labelled as ceilings,
            # not measurements of the VM's mapped-image residency.
            rss_path = args.output.parent / f's8-{label}-rss.txt'
            before = Path('/proc/loadavg').read_text().strip()
            run = subprocess.run(['/usr/bin/time', '-f', '%M', '-o', str(rss_path), '--'] + command,
                                 input=payload, capture_output=True, check=True)
            assert run.stdout.decode().strip() == '43'
            if label == 'snapshot':
                assert json.loads(run.stderr)['mapped_image'] == built['key']
            report.setdefault('exec_rss', {})[label] = dict(peak_rss_kib=int(rss_path.read_text()),
                loadavg_before=before, loadavg_after=Path('/proc/loadavg').read_text().strip())
        report['wait4_rss_caveat'] = 'wait4 and separate exact-command time(1) high-water RSS may include fork-before-exec launcher memory; process-lifetime ceilings, not image residency'
        report['snapshot_to_plain_ratios'] = {
            field: report['medians']['snapshot'][field] / report['medians']['compile-evaluate'][field]
            for field in ['count', 'wall_ns', 'user_cpu_s', 'minor_faults', 'peak_rss_kib']}
        report['complete'] = True
    except BaseException as error:
        report['error'] = repr(error)
        raise
    finally:
        save()
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
