#!/usr/bin/env python3
"""Replay the unchanged standard warm prefix, then measure the three targets."""
import os as _portable_os
from pathlib import Path as _PortablePath
import argparse
import concurrent.futures
import hashlib
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('phase')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
OUT = args.output.resolve()
if OUT.is_relative_to(ROOT):
    parser.error('--output must be outside the source repository')
OUT.mkdir(parents=True, exist_ok=True)
spec = importlib.util.spec_from_file_location('census', ROOT / 'tools/lisp-census',
                                              loader=importlib.machinery.SourceFileLoader('census', str(ROOT / 'tools/lisp-census')))
assert spec is not None and spec.loader is not None
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)
phase = args.phase
binary = Path(os.environ.get('CENSUS_BIN', str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/mx-target/gate/romacs-vm-census')))
chain = census.bootstrap_chain(ROOT)
opts = [a for link in chain for a in (('--preload-form', link) if isinstance(link, str) else ('--preload', str(link)))]
paths = [str(ROOT / 'lisp/gnu'), str(ROOT / 'lisp/gnu/emacs-lisp'), str(ROOT / 'lisp/gnu/international')]

def run(name):
    command = [str(binary), *opts, '--', str(ROOT / 'lisp/gnu' / (name + '.el')), *paths]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=180)
    (OUT / f'{phase}-{name}.jsonl').write_text(result.stdout)
    (OUT / f'{phase}-{name}.stderr').write_text(result.stderr)
    report = json.loads(result.stdout.splitlines()[0])
    return {'target': name, 'exit': result.returncode, 'loaded': report['ok'],
            'failures': [{k: s.get(k) for k in ('file', 'ordinal', 'line', 'error')} for s in report['steps'] if s['status'] != 'ok'],
            'chain_failures': [s for s in report['chain'] if not s['loaded']]}

with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
    rows = list(pool.map(run, ('simple', 'files', 'minibuffer')))
summary = {'source_checkout_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
           'binary_source_commit': os.environ['CENSUS_BINARY_COMMIT'],
           'vm_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'rows': rows}
(OUT / f'{phase}-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
