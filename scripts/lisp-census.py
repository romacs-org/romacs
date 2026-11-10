#!/usr/bin/env python3
"""Isolated, fail-closed whole-tree VM2 census. No shared generated indexes."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import argparse
import collections
import concurrent.futures
import hashlib
import io
import json
import tarfile
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
GNU_PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
GNU_SOURCE = Path(os.environ.get('CENSUS_GNU_SOURCE', str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / '')))
ARTIFACT = ROOT / 'LISP-CENSUS.json'
# R210 dual census. COLD keeps the historical artifact name and method so old
# numbers stay comparable; WARM is the parity metric and has its own artifact.
ARTIFACTS = {'cold': ARTIFACT, 'warm': ROOT / 'LISP-CENSUS-WARM.json'}
EAGER_ARTIFACT = ROOT / 'LISP-CENSUS-EAGER.json'
EAGER_METHOD = 'EAGER COUNTERFACTUAL: after the unchanged normal bootstrap (if warm), bind internal-macroexpand-for-load to a Lisp identity solely in the census adapter to select the loader eager regime. This is measurement instrumentation, NOT the GNU Lisp implementation of macroexpansion. '
METHODS = {
    'cold': 'COLD (diagnostic of raw capability). ',
    'warm': 'WARM (parity metric). Every target VM first installs the VM2 Lisp prelude and loads the GNU loadup.el bootstrap chain in order, stopping at the first chain file or form that fails (as far as the chain reaches); the chain\'s own steps and counters are excluded from the target\'s figures. The COLD method text that follows applies except that the chain IS preloaded: ',
}


def loaded(report):
    return report.get('ok') is True and not report.get('panic') and all(s.get('status') == 'ok' for s in report.get('steps', []) if not report.get('file') or s.get('file') == report['file']) and report.get('completed_files', 1) >= 1


def failure_cause(steps, own):
    cause = own
    start = next((i for i, step in enumerate(steps) if step is own), 0)
    for step in steps[start + 1:]:
        if step.get('status') == 'ok' or step is cause:
            continue
        prefix = f"{step.get('file')}: line {step.get('line')} form {step.get('ordinal')}:"
        if prefix in cause.get('error', ''):
            cause = step
    return cause


def classify(error):
    for pattern, kind in [(r'void function: ([^\s()]+)', 'missing_function'), (r'void variable: ([^\s()]+)', 'missing_variable'), (r'\(void-variable ([^\s()]+)\)', 'missing_variable'), (r'\(void-function(?: ([^\s()]+))?\)', 'missing_function')]:
        m = re.search(pattern, error)
        if m:
            return kind, m.group(1)
    if 'unbound variable' in error:
        return 'missing_variable', None
    if 'void or non-function' in error:
        return 'missing_function', None
    if any(x in error for x in ['reader:', 'source coding system', 'valid UTF-8']):
        return 'reader_parse', None
    return 'unknown', None


def rank(rows):
    grouped = collections.defaultdict(set)
    for row in rows:
        f = row.get('first_failure') or {}
        if f.get('symbol'):
            grouped[(f['classification'], f['symbol'])].add(row['file'])
    return [{'classification': kind, 'symbol': symbol, 'attribution': 'diagnostic_replay_not_confirmed', 'files_blocked': len(files), 'files': sorted(files)} for (kind, symbol), files in sorted(grouped.items(), key=lambda kv: (-len(kv[1]), kv[0]))]


def calibration_note(calibration):
    notes = []
    gnu_ok = 0
    for row in calibration:
        tree = 'GNU reference' if 'gnu-emacs-ref' in row['file'] else 'romacs'
        name = Path(row['file']).name
        notes.append(f"{tree}/{name}: continuation {row['continuation_forms_accepted']}/{row['total_forms']}, strict prefix {row['strict_load_forms_evaluated']}/{row['total_forms']}, recorded {row['recorded_accepted']}/{row['recorded_total']}")
        files = row.get('gnu_standalone', {}).get('files', [])
        gnu_ok += bool(files) and all(f['status'] == 'ok' for f in files)
    return '; '.join(notes) + f". GNU standalone success: {gnu_ok}/{len(calibration)} calibration files. Continue-after-error figures are not successful-load counts. Historical binary/commit was not supplied, so an accepted-form change cannot be attributed conclusively. The all-tree GNU standalone-load baseline was not run."


def recommendations(rows):
    groups = collections.defaultdict(list)
    for row in rows:
        failure = row.get('first_failure') or {}
        name = failure.get('candidate_missing_function') or failure.get('symbol')
        if name:
            groups[name].append(row)
    out = []
    runtime = {'autoload', 'run-hooks', 'symbol-function', 'make-vector', 'make-char', 'make-category-table', 'make-syntax-table', 'atan', 'vconcat', 'put-charset-property', 'propertize', 'decode-char', 'set-face-attribute', 'defvaralias', 'sort', 'system-type', 'emacs-lisp-mode-syntax-table', 'standard-display-table'}
    for name, entries in groups.items():
        example = entries[0]['first_failure']
        head = re.match(r'\(\s*([^\s()]+)', example.get('cause_source', ''))
        head = head.group(1) if head else None
        remedy = 'implement_rust' if name in runtime else 'edit_lisp'
        reason = 'Core capability/state; implement its real semantics rather than stubbing it. A Lisp implementation remains possible; cost is unmeasured.' if name in runtime else 'Existing Lisp helper/macro or absent bootstrap dependency: resolve its definition/load order before adding a Rust primitive; do not create the incidental variable named by this failure.'
        location = example.get('cause_file')
        form = example.get('cause_form_index')
        pinned = (ROOT / 'gnu-lisp/emacs-lisp/macroexp.el').read_text().split('\n')
        subr = (ROOT / 'gnu-lisp/subr.el').read_text().split('\n')
        if name == 'cadr' and len(pinned) > 74 and '(list (cadr var+list))' in pinned[74] and len(subr) > 726 and '(defun cadr (x)' in subr[726]:
            reason = 'Candidate call attribution points to macroexp--accumulate. cadr already exists at gnu-lisp/subr.el:727. Cheapest local bootstrap rewrite to test: (cadr var+list) -> (car (cdr var+list)) at gnu-lisp/emacs-lisp/macroexp.el:75, definition form 5; it removes this first frontier for the observed files, NOT a proof they then load. Loading the existing subr definition in a reviewed bootstrap sequence is another option. Source line text is checked before emitting this pinned recommendation.'
            location, form = 'gnu-lisp/emacs-lisp/macroexp.el', 5
        elif head in {'declare-function', 'defgroup', 'defsubst', 'defmacro', 'defvar-keymap'}:
            reason = f'Failure arises in {head}, not evidence the named argument should become a global variable. Restore a VM2-compatible declaration/bootstrap helper once; look up the existing definitions in subr.el (declare-function), custom.el (defgroup), or emacs-lisp/byte-run.el (defsubst). Preserve semantics; no no-op runtime shim.'
        out.append({'symbol': name, 'files_blocked_at_first_frontier': len({r['file'] for r in entries}), 'attribution': 'provisional_post_verdict_replay', 'recommended_remedy': remedy, 'reasoning': reason, 'edit_or_failure_file': location, 'form_index': form, 'example_failure_head': head, 'deletion_assessment': 'No deletion recommendation: loading failure is not evidence a subsystem is outside the product. Owner scope/usage evidence is required.'})
    return sorted(out, key=lambda r: (-r['files_blocked_at_first_frontier'], r['symbol']))


def gnu_oracle_binary(_cache=[]):
    """The pinned GNU, never PATH. A bare 'emacs' is how the distro build became
    the oracle; scripts/gnu_oracle.py is the one resolver and it fails closed."""
    if not _cache:
        import importlib.util
        spec = importlib.util.spec_from_file_location('romacs_gnu_oracle', ROOT / 'scripts/gnu_oracle.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _cache.append(str(module.require()['path']))
    return _cache[0]

def fingerprint(root=ROOT):
    paths = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
    selected = {p for p in paths if p and (p.startswith(('gnu-lisp/', 'rlisp/', 'crates/romacs-vm/', 'crates/romacs-face/', 'crates/romacs-layout-input/', 'crates/romacs-regex/')) or p in ['Cargo.toml', 'Cargo.lock'])}
    selected.update(['scripts/lisp-census.py', 'scripts/lisp-census-oracle.el', 'crates/romacs-vm/src/bin/romacs-vm-census.rs'])
    # Include new files in lisp and the tool, not merely the tracked manifest.
    selected.update(str(p.relative_to(root)) for p in (root / 'gnu-lisp').rglob('*.el'))
    selected.update(str(p.relative_to(root)) for p in (root / 'crates/romacs-vm').rglob('*.rs'))
    h = hashlib.sha256()
    for name in sorted(selected):
        p = root / name
        data = p.read_bytes() if p.exists() else b'<deleted>'
        h.update(len(name.encode()).to_bytes(8, 'big') + name.encode())
        h.update(len(data).to_bytes(8, 'big') + data)
    return h.hexdigest()


def measurement_output(stdout):
    if isinstance(stdout, bytes):
        stdout = stdout.decode(errors='replace')
    reports = []
    # JSONL is framed by LF, not Unicode separators embedded in JSON strings.
    for line in (stdout or '').split('\n'):
        try:
            value = json.loads(line)
            if isinstance(value, dict) and value.get('measurement') is True:
                reports.append(value)
        except ValueError:
            pass
    if not reports:
        return {}
    measured = reports[0]
    # Later output may add diagnostics but never rewrite measured fields.
    for extra in reports[1:]:
        for key in ['diagnostic_condition', 'candidate_replay']:
            if key in extra:
                measured[key] = extra[key]
    return measured


# GNU lisp/loadup.el preloads these, in this order, before anything else is
# loaded (loadup.el:122-148). WARM mode loads them into every measured VM,
# stopping at the first one that fails: "the chain as far as it reaches".
# WARM loads the first file sites in order, then executes the following
# conditional directly from GNU's source (including both dynamic bindings).
# Every failed file OR form ends preload; target measurement remains separate.
BOOTSTRAP_CHAIN = ['emacs-lisp/debug-early', 'emacs-lisp/byte-run', 'emacs-lisp/backquote', 'subr',
                   'keymap', 'version', 'widget', 'custom', 'emacs-lisp/map-ynp', 'international/mule',
                   'international/mule-conf', 'env', 'format', 'bindings', 'window', 'files',
                   'emacs-lisp/macroexp']
POST_MACROEXP_CHAIN = ['cus-face', 'faces', 'loaddefs', 'button', 'emacs-lisp/cl-preloaded', 'emacs-lisp/oclosure',
                       'obarray', 'abbrev']


def macroexp_bootstrap_form(root=None):
    """Use GNU's conditional/bindings verbatim, rather than flattening its loads."""
    root = root or ROOT
    source = (root / 'gnu-lisp/loadup.el').read_text()
    start = source.index("(if (compiled-function-p (symbol-function 'macroexpand-all))")
    end = source.index('(load "cus-face")', start)
    return source[start:end].strip()


def gv_bootstrap_form(root=None):
    """loadup.el's `(require 'gv)' conditional, verbatim."""
    source = ((root or ROOT) / 'gnu-lisp/loadup.el').read_text()
    start = source.index("(when (interpreted-function-p (symbol-function 'add-hook))")
    end = source.index('(load "emacs-lisp/cl-preloaded")', start)
    return source[start:end].strip()


def bootstrap_chain(root=None):
    root = root or ROOT
    chain = []
    for name in BOOTSTRAP_CHAIN:
        if name == 'subr':
            # romacs-loadup-bootstrap.el's adaptation: subr.el's defvars own
            # these maps in GNU, so the native skeleton yields them first.
            chain.append("(progn (current-global-map) (mapc #'makunbound '(global-map esc-map ctl-x-map)))")
        chain.append(root / 'gnu-lisp' / f'{name}.el')
        if name == 'window':
            chain.append("(setq resize-mini-windows 'grow-only)")
            chain.append("(setq load-source-file-function #'load-with-code-conversion)")
    chain.append(macroexp_bootstrap_form(root))
    # loadup.el:159-200 next: faces, the autoloads (gv's among them) that
    # macroexp's eager expansion of `push' and `setf' needs, GNU's own gv
    # conditional, then cl-preloaded (the obsolete-file warning's
    # bytecomp needs it).
    for name in POST_MACROEXP_CHAIN:
        if name == 'emacs-lisp/cl-preloaded':
            chain.append(gv_bootstrap_form(root))
        chain.append(root / 'gnu-lisp' / f'{name}.el')
    return chain



def load_path(root=None):
    """GNU's installed load-path, as gnu-tests/run.py load_path: the Lisp
    root, then the immediate subdirectories its generated subdirs.el names
    (reverse name order, obsolete last, term and VCS/backup names omitted)."""
    lisp = (root or ROOT) / 'gnu-lisp'
    names = sorted(p.name for p in lisp.iterdir() if p.is_dir()
                   and p.name not in ('term', 'RCS', 'CVS', 'Old')
                   and not p.name.startswith('=') and not p.name.endswith(('~', '.orig', '.rej')))
    ordered = [n for n in reversed(names) if n != 'obsolete'] + (['obsolete'] if 'obsolete' in names else [])
    return [lisp] + [lisp / name for name in ordered]


def chain_consensus(rows):
    """The chain must reach exactly the same point for every measured file."""
    chains = {json.dumps(r.get('chain'), sort_keys=True) for r in rows if r.get('forms_evaluated') is not None}
    if not chains:
        return None  # No measured chain; retain incomplete rows rather than discard evidence.
    if len(chains) != 1:
        raise RuntimeError(f'bootstrap chain differs between rows ({len(chains)} variants); refusing a mixed warm artifact')
    return json.loads(chains.pop())


def run_file(path, oracle, binary, load_paths, timeout, preload=(), eager_target=False):
    started = time.monotonic()
    row = {'file': os.path.relpath(path, ROOT), 'total_forms': oracle['form_count'], 'gnu_reader_status': oracle['status'], 'gnu_reader_error': oracle.get('error'), 'forms_evaluated': None, 'loaded': False}
    warm = [arg for link in preload for arg in
            (('--preload-form', link) if isinstance(link, str) else ('--preload', str(link)))]
    if eager_target:
        warm.append('--eager-target')
    # Keep the reaped native status even when our supervisor kills the child.
    timed_out = False
    with subprocess.Popen([str(binary), *warm, *(['--'] if warm else []), str(path), *map(str, load_paths)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, cwd=ROOT) as p:
        try:
            stdout, stderr = p.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            p.kill()
            stdout, stderr = p.communicate()
    report = measurement_output(stdout)
    disposition = 'process_signal' if p.returncode < 0 else 'process_exit' if p.returncode else 'success'
    row.update(exit_code=p.returncode, stderr=stderr[-8000:],
               process_disposition=disposition, supervisor_timeout=timed_out,
               primary_report_present=bool(report), signal_number=None, signal_name=None)
    if p.returncode < 0:
        row['signal_number'] = -p.returncode
        try:
            row['signal_name'] = signal.Signals(-p.returncode).name
        except ValueError:
            pass
    row['diagnostic_timeout_after_measurement'] = bool(report) and timed_out
    row['diagnostic_process_disposition'] = ('timeout' if timed_out else disposition) if report and (timed_out or p.returncode) else None
    if not report:
        kind = 'timeout' if timed_out else disposition if p.returncode else 'unknown'
        error = f'exceeded {timeout}s; progress unavailable after supervisor kill' if timed_out else stderr[-4000:] or stdout[-4000:] or 'no measurement report'
        row['first_failure'] = {'classification': kind, 'symbol': None, 'error': error, 'form_index': None, 'line': None, 'byte': None}
    if report:
        row.pop('first_failure', None)
        row['mode'] = report.get('mode')
        if eager_target:
            row['eager_target'] = report.get('eager_target', False)
            row['measurement_regime'] = report.get('measurement_regime')
        if 'chain' in report:
            row['chain'] = report['chain']
        steps = report.get('steps', [])
        own = [s for s in steps if s['file'] == str(path)]
        row['forms_evaluated'] = sum(s['status'] == 'ok' for s in own)
        row['dependency_forms_evaluated'] = sum(s['status'] == 'ok' for s in steps if s['file'] != str(path))
        row['loaded'] = loaded(report)
        row['condition_result'] = report.get('condition_result')
        # The deepest error is the cause; the outer target form remains explicit.
        failures = [s for s in steps if s['status'] != 'ok']
        if not row['loaded']:
            own_failure = next((s for s in own if s['status'] != 'ok'), {})
            cause = failure_cause(steps, own_failure) if own_failure else {}
            error = cause.get('error', report.get('error', report.get('condition_result', 'no failure detail')))
            condition = report.get('condition_result', '')
            diagnostic = report.get('diagnostic_condition', '')
            kind, symbol = classify(condition + ' ' + error)
            # Replay attribution is separate evidence, never the load verdict.
            dk, ds = classify(diagnostic)
            if dk == kind and ds and symbol is None:
                symbol = ds
            candidate = re.search(r'CENSUS-MISSING-FUNCTION:([^\s()"\\]+)', report.get('candidate_replay', ''))
            if report.get('panic'):
                kind, symbol = 'vm_panic_abort', None
            source = cause.get('source', '')
            row['first_failure'] = {'classification': kind, 'symbol': symbol, 'error': error, 'condition': condition, 'form_index': own_failure.get('ordinal'), 'line': own_failure.get('line'), 'byte': own_failure.get('byte'), 'source': own_failure.get('source', '')[:600], 'cause_file': cause.get('file', '').replace(str(ROOT) + '/', ''), 'cause_form_index': cause.get('ordinal'), 'cause_line': cause.get('line'), 'cause_source': source[:600], 'transitive': bool(cause and cause.get('file') != str(path))}
            row['first_failure']['symbol_source'] = 'diagnostic_replay_not_confirmed' if symbol else None
            row['first_failure']['diagnostic_condition'] = diagnostic
            row['first_failure']['candidate_missing_function'] = candidate.group(1) if candidate else None
            row['first_failure']['candidate_attribution_caveat'] = 'Post-verdict sentinel replay; fboundp-sensitive code may diverge; not a confirmed original-run name.' if candidate else None
            # Reader failures have no entered step; recover the dependency path
            # directly, including when wrapped by an outer require failure.
            m = re.search(r'([^\n]*?\.el): line (\d+) form (\d+): reader:', error)
            if m:
                cause_path = m[1]
                row['first_failure'].update(cause_file=cause_path.replace(str(ROOT) + '/', ''), cause_line=int(m[2]), cause_form_index=int(m[3]), transitive=cause_path != str(path))
                if not own_failure:
                    row['first_failure'].update(line=int(m[2]), form_index=int(m[3]))
            elif 'unsupported source coding system:' in error:
                m = re.search(r'([^\n]*?\.el): unsupported source coding system:', error)
                if m:
                    row['first_failure'].update(cause_file=m[1].replace(str(ROOT) + '/', ''), transitive=m[1] != str(path))
    row['seconds'] = round(time.monotonic() - started, 6)
    return row


def oracle_inventory():
    """Hash committed GNU blobs, never its possibly dirty checkout or HEAD."""
    commit = subprocess.check_output(['git', 'rev-parse', GNU_PIN + '^{commit}'], cwd=GNU_SOURCE, text=True).strip()
    if commit != GNU_PIN:
        raise RuntimeError('GNU pin must be a full immutable commit')
    archive = subprocess.check_output(['git', 'archive', commit, 'lisp'], cwd=GNU_SOURCE)
    tracked = set(subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', '-z', 'HEAD', '--', 'gnu-lisp', 'rlisp'], cwd=ROOT).decode().split('\0'))
    tracked = {p for p in tracked if p.endswith('.el')}
    entries = []
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        for member in tree:
            if not member.isfile() or not member.name.endswith('.el'):
                continue
            original = tree.extractfile(member).read()
            local_name = 'gnu-lisp' + member.name[len('lisp'):]
            path = ROOT / local_name
            local = path.read_bytes() if local_name in tracked and path.is_file() else None
            entries.append({'file': local_name, 'gnu_source_path': member.name,
                            'gnu_sha256': hashlib.sha256(original).hexdigest(),
                            'local_sha256': hashlib.sha256(local).hexdigest() if local is not None else None,
                            'status': 'missing' if local is None else 'pristine' if local == original else 'modified'})
    entries.sort(key=lambda e: e['file'])
    support = {}
    if (ROOT / 'LISP-GENERATED.json').is_file():
        import importlib.util
        spec = importlib.util.spec_from_file_location('census_lisp_oracle', ROOT / 'tools/lisp_oracle.py')
        assert spec is not None and spec.loader is not None
        oracle = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(oracle)
        support, problems = oracle.generated_support(ROOT, GNU_SOURCE, commit,
                                                     oracle.pin_tree(GNU_SOURCE, commit), required=True)
        if problems:
            raise RuntimeError('invalid census generated support: ' + '; '.join(problems))
    return {'gnu_commit': commit, 'gnu_files': entries,
            'generated_support_paths': sorted(support),
            'bundled_paths': sorted(tracked - {e['file'] for e in entries} - set(support))}


def parity_summary(rows, inventory):
    entries = inventory['gnu_files']
    shared = {e['file'] for e in entries if e['status'] != 'missing'}
    modified = {e['file'] for e in entries if e['status'] == 'modified'}
    missing = [e['file'] for e in entries if e['status'] == 'missing']
    successful = {r['file'] for r in rows if r['loaded']}
    return {'gnu_parity': {'loaded': len(successful & shared), 'shared_total': len(shared),
                           'gnu_target_total': len(entries), 'loaded_shared_modified': len(successful & modified),
                           'shared_modified_total': len(modified), 'missing_total': len(missing), 'missing_paths': missing},
            'generated_support': {'loaded': len(successful & set(inventory.get('generated_support_paths', []))),
                                  'total': len(inventory.get('generated_support_paths', []))},
            'bundled': {'loaded': len(successful & set(inventory['bundled_paths'])), 'total': len(inventory['bundled_paths'])}}


def census_figure(artifact):
    """The same R218 labels for cold and warm; bundled loads are never parity."""
    summary = artifact['summary']
    p, b = summary['gnu_parity'], summary['bundled']
    support = summary.get('generated_support', {'loaded': 0, 'total': 0})
    missing = ', '.join('`' + path + '`' for path in p['missing_paths'])
    return (f"**{artifact['mode'].upper()} at `{artifact['base_commit']}`: GNU-parity {p['loaded']}/{p['shared_total']} shared files load "
            f"(true GNU target {p['gnu_target_total']}, pinned `{artifact['oracle_inventory']['gnu_commit']}`); "
            f"generated support {support['loaded']}/{support['total']} load (not GNU parity credit); "
            f"bundled {b['loaded']}/{b['total']} load; missing {p['missing_total']}.** "
            f"Loaded shared modified: {p['loaded_shared_modified']}; overall shared content drift: {p['shared_modified_total']}. "
            f"Missing GNU paths: {missing}. These load verdicts use the local shared files, not pristine GNU bytes where modified.")


def summarize(rows, tracked, mode, inventory=None):
    """Exact totals are nullable; measured successes remain explicit lower bounds."""
    tracked_rows = [r for r in rows if r['file'] in tracked]
    summary = {'mode': mode, 'tracked_files_total': len(tracked_rows), 'tracked_files_loaded': sum(r['loaded'] for r in tracked_rows), 'tracked_forms_total': sum(r['total_forms'] for r in tracked_rows), 'tracked_forms_evaluated': sum(r['forms_evaluated'] or 0 for r in tracked_rows), 'files_total': len(rows), 'files_loaded': sum(r['loaded'] for r in rows), 'forms_total_gnu_reader': sum(r['total_forms'] for r in rows), 'forms_evaluated_target_files': sum(r['forms_evaluated'] or 0 for r in rows), 'files_with_unavailable_progress': sum(r['forms_evaluated'] is None for r in rows), 'failure_classes': dict(collections.Counter(r['first_failure']['classification'] for r in rows if not r['loaded'])), 'transitive_failures': sum(bool((r.get('first_failure') or {}).get('transitive')) for r in rows), 'missing_symbol_name_unavailable': sum((r.get('first_failure') or {}).get('classification', '').startswith('missing_') and not (r.get('first_failure') or {}).get('symbol') for r in rows)}
    for subset, exact_key, prefix in [
        (rows, 'forms_evaluated_target_files', ''),
        (tracked_rows, 'tracked_forms_evaluated', 'tracked_'),
    ]:
        measured = sum(r['forms_evaluated'] is not None for r in subset)
        complete = measured == len(subset)
        summary[prefix + 'files_measured'] = measured
        summary[prefix + 'measurement_complete'] = complete
        summary[exact_key + '_lower_bound'] = summary[exact_key]
        if not complete:
            summary[exact_key] = None
    summary.update(parity_summary(rows, inventory if inventory is not None else oracle_inventory()))
    return summary


def compare_eager(default, eager):
    """Fail closed on unequal measurement inputs; enumerate every prefix regression."""
    if default.get('eager_target', False) or eager.get('eager_target') is not True:
        raise ValueError('expected ordinary default and eager counterfactual artifacts')
    for key in ['mode', 'scope', 'input_sha256', 'vm_binary_sha256', 'oracle_inventory', 'bootstrap_chain', 'gnu_version', 'timeout_seconds_per_file']:
        if default.get(key) != eager.get(key):
            raise ValueError(f'incomparable eager artifacts: {key}')
    def index(artifact):
        rows = {r['file']: r for r in artifact['files']}
        if len(rows) != len(artifact['files']):
            raise ValueError('duplicate measurement paths')
        return rows
    before, after = index(default), index(eager)
    if before.keys() != after.keys():
        raise ValueError('incomparable eager artifact paths')
    regressions, unavailable = [], []
    for name in sorted(before):
        a, b = before[name], after[name]
        if a['total_forms'] != b['total_forms']:
            raise ValueError(f'incomparable reader counts: {name}')
        if a['forms_evaluated'] is None or b['forms_evaluated'] is None:
            unavailable.append(name)
        elif b['forms_evaluated'] < a['forms_evaluated']:
            regressions.append({'file': name, 'default_prefix': a['forms_evaluated'], 'eager_prefix': b['forms_evaluated'], 'delta': b['forms_evaluated'] - a['forms_evaluated'], 'first_failure': b.get('first_failure')})
    loaded_before = {name for name, row in before.items() if row['loaded']}
    loaded_after = {name for name, row in after.items() if row['loaded']}
    return {'measurement_regime': 'eager_counterfactual', 'mode': eager['mode'], 'files_total': len(before),
            'default_loaded': len(loaded_before), 'eager_loaded': len(loaded_after), 'loaded_delta': len(loaded_after) - len(loaded_before),
            'gained_loads': sorted(loaded_after - loaded_before), 'lost_loads': sorted(loaded_before - loaded_after),
            'prefix_regressions': regressions, 'prefix_regression_count': len(regressions), 'unavailable_prefix_comparisons': unavailable,
            'target_forms_delta': None if unavailable else sum(after[name]['forms_evaluated'] - before[name]['forms_evaluated'] for name in before)}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--inventory-only', action='store_true', help='hash pinned GNU and local tracked Lisp only; no build or VM measurement')
    ap.add_argument('--check', action='store_true', help='exit nonzero when artifact input hash is stale')
    ap.add_argument('--workers', type=int, default=32)
    ap.add_argument('--timeout', type=float, default=10)
    ap.add_argument('--mode', choices=['cold', 'warm'], default='cold', help='cold: fresh VM (diagnostic); warm: bootstrap chain preloaded (parity metric)')
    ap.add_argument('--eager-target', action='store_true', help='counterfactual only: select eager loader using identity after normal bootstrap; not GNU macroexpansion')
    ap.add_argument('--compare', nargs=2, type=Path, metavar=('DEFAULT', 'EAGER'), help='report matched-artifact eager delta and prefix regressions, without measurement')
    ap.add_argument('--output', type=Path)
    ap.add_argument('--files', nargs='*', type=Path, help='diagnostic subset, never the committed whole-tree artifact')
    args = ap.parse_args()
    if args.compare:
        if args.output and (args.output.resolve() in {p.resolve() for p in [*ARTIFACTS.values(), EAGER_ARTIFACT, *args.compare]}):
            ap.error('--compare requires an output separate from census inputs and artifacts')
        comparison = compare_eager(*(json.loads(p.read_text()) for p in args.compare))
        text = json.dumps(comparison, indent=2) + '\n'
        if args.output:
            args.output.write_text(text)
        print(text, end='')
        return 0
    args.output = args.output or (ROOT / 'LISP-CENSUS-INVENTORY.json' if args.inventory_only else EAGER_ARTIFACT if args.eager_target else ARTIFACTS[args.mode])
    if not args.inventory_only and args.eager_target and args.output.resolve() in {a.resolve() for a in ARTIFACTS.values()}:
        ap.error('--eager-target must not overwrite an ordinary census artifact')
    if not args.inventory_only and not args.eager_target and args.output.resolve() == EAGER_ARTIFACT.resolve():
        ap.error('ordinary measurement must not overwrite the eager artifact')
    inventory = oracle_inventory()
    if args.inventory_only:
        if args.output.resolve() in {a.resolve() for a in [*ARTIFACTS.values(), EAGER_ARTIFACT]}:
            ap.error('--inventory-only requires a separate --output')
        args.output.write_text(json.dumps(inventory, indent=2) + '\n')
        counts = parity_summary([], inventory)
        counts['gnu_parity']['loaded'] = None
        counts['gnu_parity']['loaded_shared_modified'] = None
        counts['bundled']['loaded'] = None
        print(json.dumps({'scope': 'inventory_only_no_vm_measurement', **counts}, indent=2))
        return 0
    if args.check:
        artifact = json.loads(args.output.read_text())
        scope = sorted(str(p.relative_to(ROOT)) for p in (ROOT / 'gnu-lisp').rglob('*.el'))
        fresh = artifact.get('oracle_inventory') == inventory and artifact.get('mode', 'cold') == args.mode and artifact.get('eager_target', False) is args.eager_target and artifact['input_sha256'] == fingerprint() and artifact.get('scope') == 'whole_tree' and sorted(r['file'] for r in artifact['files']) == scope and artifact['gnu_version'] == subprocess.check_output([gnu_oracle_binary(), '--version'], text=True).splitlines()[0]
        print('fresh' if fresh else 'STALE: regenerate with python3 scripts/lisp-census.py')
        return 0 if fresh else 1
    if args.files and args.output.resolve() in {a.resolve() for a in [*ARTIFACTS.values(), EAGER_ARTIFACT]}:
        ap.error('--files requires a separate --output')
    before = fingerprint()
    target = Path(os.environ.get('CENSUS_TARGET_DIR', str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'target')))
    env = os.environ.copy()
    env['CARGO_TARGET_DIR'] = str(target)
    env['TMPDIR'] = str(_PortablePath(_portable_tempfile.gettempdir()) / '')
    subprocess.run(['cargo', 'build', '--manifest-path', 'crates/romacs-vm/Cargo.toml', '--release', '--bin', 'romacs-vm-census', '--bin', 'romacs-vm'], cwd=ROOT, env=env, check=True)
    started = time.monotonic()
    files = sorted(p.resolve() for p in (args.files or (ROOT / 'gnu-lisp').rglob('*.el')))
    with tempfile.TemporaryDirectory(prefix='lisp-census-', dir=env['TMPDIR']) as tmp:
        manifest = Path(tmp) / 'manifest.json'
        manifest.write_text(json.dumps(list(map(str, files))))
        p = subprocess.run([gnu_oracle_binary(), '-Q', '--batch', '--script', str(ROOT / 'scripts/lisp-census-oracle.el'), '--manifest', str(manifest)], capture_output=True, text=True, timeout=120, check=True)
        oracle = json.loads(p.stdout.splitlines()[-1])['files']
    if len(files) != len(oracle):
        raise RuntimeError('oracle omitted files')
    load_paths = load_path()
    binary_tmp = tempfile.TemporaryDirectory(prefix='census-binary-', dir=env['TMPDIR'])
    binary = Path(binary_tmp.name) / 'romacs-vm-census'
    shutil.copy2(target / 'release/romacs-vm-census', binary)
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        preload = bootstrap_chain() if args.mode == 'warm' else ()
        futures = [pool.submit(run_file, path, entry, binary, load_paths, args.timeout, preload, args.eager_target) for path, entry in zip(files, oracle)]
        rows = [f.result() for f in futures]
    if before != fingerprint() or hashlib.sha256(binary.read_bytes()).hexdigest() != binary_hash:
        raise RuntimeError('inputs/binary changed during measurement; refusing mixed-state artifact')
    binary_tmp.cleanup()
    for row in rows:
        n = row['forms_evaluated']
        if n is not None and (n > row['total_forms'] or (row['loaded'] and n != row['total_forms'])):
            raise RuntimeError(f"form count mismatch: {row['file']}")
        if row['loaded'] and row['gnu_reader_status'] != 'ok':
            raise RuntimeError(f"GNU could not count loaded source: {row['file']}")
    for row in rows:
        if row['forms_evaluated'] is not None and row.get('mode') != args.mode:
            raise RuntimeError(f"row measured in mode {row.get('mode')!r}, expected {args.mode!r}: {row['file']}")
    if args.eager_target:
        for row in rows:
            if row['forms_evaluated'] is not None and (row.get('eager_target') is not True or row.get('measurement_regime') != 'eager_counterfactual'):
                raise RuntimeError(f"row lacks eager counterfactual attestation: {row['file']}")
    chain = chain_consensus(rows) if args.mode == 'warm' else None
    # The rglob denominator includes build-generated, git-ignored .el files
    # (e.g. lisp/international/charscript.el), so it moves with build state.
    # The tracked figures are the stable, comparable denominator.
    tracked = set(subprocess.check_output(['git', 'ls-files', '-z', '--', 'gnu-lisp'], cwd=ROOT).decode().split('\0'))
    summary = summarize(rows, tracked, args.mode, inventory)
    if args.mode == 'warm':
        summary['bootstrap_entries_planned'] = len(preload)
        summary['bootstrap_entries_attempted'] = len(chain or [])
        summary['bootstrap_complete'] = bool(chain) and len(chain) == len(preload) and all(entry['loaded'] for entry in chain)
    result = {'schema': 2, 'oracle_inventory': inventory, 'mode': args.mode, 'base_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), 'input_sha256': before, 'vm_binary_sha256': binary_hash, 'scope': 'subset' if args.files else 'whole_tree', 'gnu_version': subprocess.check_output([gnu_oracle_binary(), '--version'], text=True).splitlines()[0], 'workers': args.workers, 'timeout_seconds_per_file': args.timeout, 'runtime_seconds': round(time.monotonic() - started, 6), 'method': METHODS[args.mode] + 'Fresh VM per target; actual load stops on first unhandled error; recursive source load-path includes all lisp directories; no bootstrap or GNU preload; target form successes counted separately from dependencies; no successful read counted as evaluation. Null progress is unavailable, not zero. Standalone cold-load diagnostic, not proof function bodies work. Post-verdict condition/sentinel replays supply PROVISIONAL names only, and never affect evaluation counts or loaded verdicts; fboundp-sensitive code and prior side effects may diverge.', 'summary': summary, 'bootstrap_chain': chain, 'ranked_missing_symbols': rank(rows), 'ranked_blocker_candidates': recommendations(rows), 'remedy_certainty': {'proven_lisp_only_files': 0, 'proven_runtime_required_files': 0, 'undetermined_failed_files': sum(not r['loaded'] for r in rows), 'reason': 'First-error observation cannot prove that a file has no later blocker or that Lisp replacement is impossible. Recommendations are first-frontier hypotheses, not counterfactual full-load proof.'}, 'files': rows}
    if args.eager_target:
        result.update(eager_target=True, measurement_regime='eager_counterfactual')
        result['method'] = EAGER_METHOD + result['method']
    temporary = args.output.with_suffix(args.output.suffix + '.tmp')
    temporary.write_text(json.dumps(result, indent=2, ensure_ascii=True) + '\n')
    temporary.replace(args.output)
    print(json.dumps({'summary': summary, 'runtime_seconds': result['runtime_seconds'], 'output': str(args.output)}, indent=2))
    return 0 if summary['measurement_complete'] else 2

if __name__ == '__main__':
    raise SystemExit(main())
