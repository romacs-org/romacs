#!/usr/bin/env python3
"""Real-process mutation proof and continuation-vs-load calibration."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import importlib.util
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('census', ROOT / 'scripts/lisp-census.py')
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)
target = Path(os.environ.get('CENSUS_TARGET_DIR', str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'target'))) / 'release'
with tempfile.TemporaryDirectory(dir=str(_PortablePath(_portable_tempfile.gettempdir()) / ''), prefix='census-proof-') as td:
    td = Path(td)
    source_target = target
    target = td / 'bin'
    target.mkdir()
    for name in ['romacs-vm-census', 'romacs-vm']:
        shutil.copy2(source_target / name, target / name)
    initial_binary_hash = hashlib.sha256((target / 'romacs-vm-census').read_bytes()).hexdigest()
    mutations = {}
    for name, source in [('pass', '(setq census-proof 1)\n(+ census-proof 2)\n'), ('missing_function', '(census-proof-impossible)\n(setq must-not-run t)\n'), ('missing_variable', 'census-proof-impossible-variable\n'), ('reader_parse', '(setq x 1)\n(unterminated\n')]:
        path = td / (name + '.el')
        path.write_text(source)
        row = census.run_file(path, {'form_count': None, 'status': 'not_split'}, target / 'romacs-vm-census', [ROOT / 'lisp'], 10)
        mutations[name] = row
    assert mutations['pass']['loaded'] and mutations['pass']['forms_evaluated'] == 2, mutations
    assert all(not mutations[k]['loaded'] for k in ['missing_function', 'missing_variable', 'reader_parse']), mutations
    assert mutations['missing_function']['forms_evaluated'] == 0, mutations
    assert mutations['missing_function']['first_failure']['classification'] == 'missing_function', mutations
    assert mutations['missing_variable']['first_failure']['classification'] == 'missing_variable', mutations
    assert mutations['reader_parse']['first_failure']['classification'] == 'reader_parse', mutations
    # Actually weaken the verdict predicate and run the same real failing VM.
    original_loaded = census.loaded
    census.loaded = lambda report: True
    mutant = census.run_file(td / 'missing_function.el', {'form_count': 2, 'status': 'ok'}, target / 'romacs-vm-census', [ROOT / 'lisp'], 10)
    census.loaded = original_loaded
    mutant_caught = False
    try:
        assert not mutant['loaded'], 'missing-function fixture must fail'
    except AssertionError:
        mutant_caught = True
    assert mutant_caught, 'weakened verdict unexpectedly survived the failure assertion'
    paths = [base / f for base in [ROOT / 'lisp', Path(str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / 'lisp'))] for f in ['subr.el', 'bindings.el', 'keymap.el']]
    manifest = td / 'manifest.json'
    manifest.write_text(json.dumps(list(map(str, paths))))
    p = subprocess.run(['emacs', '-Q', '--batch', '--script', str(ROOT / 'scripts/lisp-census-oracle.el'), '--manifest', str(manifest)], capture_output=True, text=True, check=True)
    split = json.loads(p.stdout)['files']
    calibration = []
    for path, entry, recorded in zip(paths, split, [440, 327, 24, 440, 327, 24]):
        with path.open(newline='') as file:
            text = file.read()
        payload = ''.join(json.dumps({'source': text[f['start']:f['end']]}) + '\n' for f in entry['forms'])
        p = subprocess.run([str(target / 'romacs-vm'), '--eval-stream'], input=payload, capture_output=True, text=True, timeout=30)
        replies = [json.loads(line) for line in p.stdout.splitlines()]
        errors = [(i + 1, r['error']) for i, r in enumerate(replies) if 'error' in r]
        strict = census.run_file(path, entry, target / 'romacs-vm-census', [ROOT / 'lisp'], 10)
        gnu = subprocess.run(['emacs', '-Q', '--batch', '--script', str(ROOT / 'scripts/lisp-census-oracle.el'), '--load-file', str(path)], capture_output=True, text=True, timeout=30)
        calibration.append({'file': os.path.relpath(path, ROOT), 'continuation_forms_accepted': sum('error' not in r for r in replies), 'total_forms': entry['form_count'], 'recorded_accepted': recorded, 'first_errors': errors[:6], 'process_exit': p.returncode, 'strict_load_forms_evaluated': strict['forms_evaluated'], 'strict_loaded': strict['loaded'], 'gnu_standalone': json.loads(gnu.stdout.splitlines()[-1])})
    aborter = td / 'abort-process.py'
    aborter.write_text('#!/usr/bin/env python3\nimport os, resource\nresource.setrlimit(resource.RLIMIT_CORE, (0, 0))\nos.abort()\n')
    aborter.chmod(0o700)
    mutations['process_abort_capture'] = census.run_file(td / 'pass.el', {'form_count': 2, 'status': 'ok'}, aborter, [], 2)
    assert mutations['process_abort_capture']['first_failure']['classification'] == 'process_signal'
    assert mutations['process_abort_capture']['signal_name'] == 'SIGABRT'
    assert mutations['process_abort_capture']['exit_code'] == -6
    dependency = td / 'cbad.el'
    dependency.write_text('(census-proof-impossible)\n')
    for name, source in [('handled', "(condition-case nil (require 'cbad) (error nil))\n(setq done t)\n"), ('causal', "(progn (condition-case nil (require 'cbad) (error nil)) census-own-missing)\n"), ('replay_timeout', '(defvar census-count 0)\n(progn (setq census-count (1+ census-count)) (if (= census-count 1) census-own-missing (while t)))\n'), ('timeout', '(while t)\n')]:
        path = td / (name + '.el')
        path.write_text(source)
        mutations[name] = census.run_file(path, {'form_count': 2, 'status': 'ok'}, target / 'romacs-vm-census', [td, ROOT / 'lisp'], 0.2)
    assert mutations['handled']['loaded'] and mutations['handled']['forms_evaluated'] == 2
    assert mutations['causal']['first_failure']['classification'] == 'missing_variable'
    assert not mutations['causal']['first_failure']['transitive']
    assert mutations['replay_timeout']['first_failure']['classification'] == 'missing_variable'
    assert mutations['replay_timeout']['forms_evaluated'] == 1
    assert mutations['replay_timeout'].get('diagnostic_timeout_after_measurement') is True
    assert mutations['timeout']['first_failure']['classification'] == 'timeout'
    # Historical figures came from GNU's source files, not romacs's copies.
    for item, expected_total in zip(calibration, [522, 363, 25, 522, 363, 25]):
        item['recorded_total'] = expected_total
        item['agrees_with_recorded'] = item['total_forms'] == expected_total and item['continuation_forms_accepted'] == item['recorded_accepted']
        assert item['continuation_forms_accepted'] <= item['total_forms']
    measured = json.loads((ROOT / 'LISP-CENSUS.json').read_text())
    proof_binary_hash = hashlib.sha256((target / 'romacs-vm-census').read_bytes()).hexdigest()
    assert proof_binary_hash == initial_binary_hash == measured['vm_binary_sha256']
    assert measured['input_sha256'] == census.fingerprint()
    fresh_check = subprocess.run(['python3', 'scripts/lisp-census.py', '--check'], cwd=ROOT, capture_output=True, text=True)
    assert fresh_check.returncode == 0 and fresh_check.stdout.strip() == 'fresh'
    stale = dict(measured, input_sha256='deliberately-stale')
    stale_path = td / 'stale.json'
    stale_path.write_text(json.dumps(stale))
    stale_check = subprocess.run(['python3', 'scripts/lisp-census.py', '--check', '--output', str(stale_path)], cwd=ROOT, capture_output=True, text=True)
    assert stale_check.returncode == 1 and 'STALE' in stale_check.stdout
    subset_guard = subprocess.run(['python3', 'scripts/lisp-census.py', '--files', str(td / 'pass.el'), '--output', 'LISP-CENSUS.json'], cwd=ROOT, capture_output=True, text=True)
    assert subset_guard.returncode == 2
    result = {'staleness_mutant_exit': stale_check.returncode, 'subset_overwrite_rejected_exit': subset_guard.returncode, 'vm_binary_sha256': proof_binary_hash, 'input_sha256': census.fingerprint(), 'mutations': mutations, 'forced_success_mutant_caught': mutant_caught, 'calibration': calibration}
    (ROOT / 'LISP-CENSUS-PROOF.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
