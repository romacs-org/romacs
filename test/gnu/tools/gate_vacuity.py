#!/usr/bin/env python3
"""Probe the adapter reported by a successful gate build, outside measured source."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import tempfile
import sys
import time

from audit_manifest import _source

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('source', 'prepare'))
    parser.add_argument('--run-dir', type=Path)
    parser.add_argument('--source')
    parser.add_argument('--command')
    args = parser.parse_args()
    source = _source(ROOT)
    if args.mode == 'source':
        print(json.dumps(source))
        return
    if source != json.loads(args.source):
        raise ValueError('source changed during gate build')
    run = args.run_dir.resolve(strict=True)
    if run.is_relative_to(ROOT):
        raise ValueError('subject evidence must be outside measured source')
    metadata = json.loads((run / 'all-binaries.json').read_text())['rust-build-meta']
    paths = [Path(metadata['target-directory']) / entry['path']
             for entries in metadata['non-test-binaries'].values() for entry in entries
             if entry['name'] == 'romacs-gnu-ert' and entry['kind'] == 'bin-exe']
    if len(paths) != 1:
        raise ValueError('gate must report exactly one romacs-gnu-ert executable')
    binary = paths[0].resolve(strict=True)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    receipt = run / 'subject-build.json'
    from vacuity_binding import MEMBERS, source_identity
    receipt.write_text(json.dumps(dict(schema=1,
        source=source_identity(source), binary_sha256=digest,
        command=json.loads(args.command), exit_code=0,
        tracked_clean_start=True, tracked_clean_end=True)))
    start = time.monotonic()
    audit = run / 'subject-audit'
    output = run / 'VACUITY-VERDICT.json'
    cache = run.parent.parent / 'subject-cache' / source['tree'] / (digest + '.json')
    os.environ.update(ROMACS_VACUITY_RECEIPT=str(output),
                      ROMACS_MEMBER_EVIDENCE=str(run / MEMBERS))
    from vacuity_binding import verify_receipt
    from member_evidence import write
    def verify():
        _, manifest, _ = verify_receipt(ROOT)
        if manifest['source']['tree'] != source['tree']:
            raise ValueError('subject evidence source tree differs from gate tree')
        if manifest['binaries']['romacs']['file']['sha256'] != digest:
            raise ValueError('subject evidence binary differs from gate executable')
    if cache.exists():
        shutil.copyfile(cache, output)
        verify()
        write(ROOT, output)
        print(f'gate: subject evidence cache hit in {time.monotonic() - start:.3f}s '
              f'(tree {source["tree"]}, executable {digest})', flush=True)
        return
    with (run / 'subject-probe.log').open('w') as log:
        subprocess.run([sys.executable, '-B', str(ROOT / 'test/gnu/tools/probe-vacuity-focus.py'),
                        '--subject-only', '--output', str(audit), '--binary', str(binary),
                        '--build-receipt', str(receipt)], check=True, stdout=log, stderr=log)
        subprocess.run([sys.executable, '-B', str(ROOT / 'test/gnu/tools/import-vacuity-evidence.py'),
                        str(audit), '--format', 'subject', '--output', str(output)],
                       check=True, stdout=log, stderr=log)
    verify()
    cache.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=cache.parent, delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(output.read_bytes())
    os.replace(temporary, cache)
    print(f'gate: subject evidence probed in {time.monotonic() - start:.3f}s '
          f'(tree {source["tree"]}, executable {digest})', flush=True)


if __name__ == '__main__':
    main()
