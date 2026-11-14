"""Third-party package parity against pinned GNU: the rules tools/package-parity runs.

A run installs every package of test/package-parity/packages.toml with
package-vc, from a local mirror at its pinned revision, once per editor; then
runs each package's own ERT files and its journey in both editors, each file in
a fresh process with a private HOME, XDG directories and TMPDIR; and records
the outcomes side by side.  Pure functions here; processes in run_* only.
"""
import concurrent.futures
import fnmatch
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / 'test/package-parity'
MANIFEST = DATA / 'packages.toml'
DRIVER = DATA / 'driver.el'
REPORTER = ROOT / 'test/gnu/harness/romacs-real-ert-reporter.el'
SELECTOR = '(not (or (tag :expensive-test) (tag :unstable)))'
EDITORS = ('romacs', 'gnu')
XDG = ('XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME')
KEPT_ENV = ('PATH', 'LANG', 'LC_ALL', 'USER', 'LOGNAME', 'SHELL', 'TERM')
FIXED_GIT = dict(GIT_AUTHOR_NAME='Parity', GIT_AUTHOR_EMAIL='parity@example.invalid',
                 GIT_COMMITTER_NAME='Parity', GIT_COMMITTER_EMAIL='parity@example.invalid',
                 GIT_AUTHOR_DATE='2026-01-01T00:00:00Z', GIT_COMMITTER_DATE='2026-01-01T00:00:00Z',
                 GIT_CONFIG_NOSYSTEM='1', GIT_TERMINAL_PROMPT='0')


# --- the manifest -----------------------------------------------------------

def load_manifest(text):
    """The packages, by name, each with its defaults filled in."""
    packages = {}
    for name, entry in tomllib.loads(text).get('package', {}).items():
        unknown = set(entry) - {'url', 'rev', 'lisp_dir', 'main_file', 'ignored_files', 'deps', 'tests',
                                'load', 'setup', 'exclude', 'journey'}
        if unknown:
            raise ValueError(f'{name}: unknown keys {sorted(unknown)}')
        packages[name] = dict(name=name, url=entry['url'], rev=entry.get('rev', ''),
                              lisp_dir=entry.get('lisp_dir'), main_file=entry.get('main_file'),
                              ignored_files=list(entry.get('ignored_files', [])),
                              setup=list(entry.get('setup', [])),
                              deps=list(entry.get('deps', [])), tests=list(entry.get('tests', [])),
                              load=list(entry.get('load', [])), exclude=list(entry.get('exclude', [])),
                              journey=entry.get('journey'))
    for package in packages.values():
        missing = [d for d in package['deps'] if d not in packages]
        if missing:
            raise ValueError(f"{package['name']}: deps not in the manifest: {missing}")
    return packages


def install_order(packages, selected=None):
    """Names in an order where each follows its deps: SELECTED (default all) and
    everything they need."""
    order, state = [], {}

    def visit(name, path):
        if state.get(name) == 'done':
            return
        if state.get(name) == 'visiting':
            raise ValueError('dependency cycle: ' + ' -> '.join(path + [name]))
        state[name] = 'visiting'
        for dep in packages[name]['deps']:
            visit(dep, path + [name])
        state[name] = 'done'
        order.append(name)

    for name in (selected or sorted(packages)):
        if name not in packages:
            raise ValueError(f'unknown package {name}')
        visit(name, [])
    return order


def set_revisions(text, revisions):
    """TEXT with each [package.NAME]'s rev line set to REVISIONS[NAME]."""
    out, current = [], None
    for line in text.splitlines(keepends=True):
        header = re.fullmatch(r'\[package\.([^\]]+)\]\s*', line)
        if header:
            current = header.group(1)
        elif current in revisions and re.match(r'rev\s*=', line):
            line = f'rev = "{revisions[current]}"\n'
        out.append(line)
    return ''.join(out)


def mirror_name(url):
    """A mirror's directory name: readable, and unique per URL."""
    stem = re.sub(r'[^A-Za-z0-9._-]+', '_', url.split('://', 1)[-1]).strip('_')
    return f"{stem[-60:]}-{hashlib.sha256(url.encode()).hexdigest()[:8]}.git"


# --- what the editors print -------------------------------------------------

def events(output, prefix):
    """The JSON objects of OUTPUT's lines that start with PREFIX."""
    found = []
    for line in output.splitlines():
        if line.startswith(prefix + ' '):
            try:
                found.append(json.loads(line[len(prefix) + 1:]))
            except json.JSONDecodeError:
                pass
    return found


def outcome(event):
    """A result event's outcome, telling an expected failure from a pass the
    test did not expect."""
    kind, expected = event['outcome'], event.get('as_expected')
    if kind == 'failed' and expected:
        return 'expected-failure'
    if kind == 'passed' and not expected:
        return 'unexpected-pass'
    return kind


def ert_result(output, status):
    """One test file's outcome: its status and each test's outcome by name.
    Complete means the reporter's `done' and an exit of 0 or 1 (ERT's own for
    unexpected results); a signal or other exit after `done' is not complete."""
    tests, selected, done = {}, None, False
    for event in events(output, 'ERT-SCOREBOARD'):
        kind = event.get('event')
        if kind == 'selected':
            selected = list(event.get('names', []))
        elif kind == 'result':
            tests[event['test']] = outcome(event)
        elif kind == 'done':
            done = True
    if status is None:
        state = 'timeout'
    elif done and status in (0, 1):
        state = 'complete'
    elif done:
        state = f'exit {status}'
    elif selected is None:
        state = 'no-tests' if 'Real ERT selected no tests' in output else 'load-error'
    else:
        state = 'aborted'
    return dict(status=state, selected=len(selected or []), tests=tests,
                error=None if state == 'complete' else last_error(output))


def process_state(status, done):
    """A journey's or install's outcome from its exit and its own end marker."""
    if status is None:
        return 'timeout'
    if done and status == 0:
        return 'complete'
    return f'exit {status}' if done else 'aborted'


def last_error(output):
    """The last line that reads like an error, for the report."""
    lines = [l.strip() for l in output.splitlines() if l.strip() and not l.startswith(('ERT-SCOREBOARD', 'PP '))]
    for line in reversed(lines):
        if re.search(r'error|Error|Cannot|void-|wrong-|signal|panic|Symbol.s', line):
            return line[:300]
    return lines[-1][:300] if lines else None


def counts(result):
    """passed/failed/skipped over a package's test files."""
    total = dict(passed=0, failed=0, skipped=0, files=0, broken=0)
    for entry in (result or {}).get('files', {}).values():
        total['files'] += 1
        total['broken'] += entry['status'] != 'complete'
        for kind in entry['tests'].values():
            key = 'passed' if kind in ('passed', 'expected-failure') else 'skipped' if kind == 'skipped' else 'failed'
            total[key] += 1
    return total


def normalize(text, replacements):
    for old, new in replacements:
        text = text.replace(old, new)
    return text


PASSING = ('passed', 'expected-failure')


def files_of(side):
    return side.get('tests', {}).get('files', {})


def first_divergence(package):
    """The first place a package's Romacs outcome differs from GNU's, or None."""
    romacs, gnu = package.get('romacs', {}), package.get('gnu', {})
    if romacs.get('install', {}).get('ok') != gnu.get('install', {}).get('ok'):
        return f"install: romacs {romacs.get('install', {}).get('error') or 'ok'}; gnu {gnu.get('install', {}).get('error') or 'ok'}"
    rfiles, gfiles = files_of(romacs), files_of(gnu)
    for name in sorted(set(rfiles) | set(gfiles)):
        r, g = rfiles.get(name), gfiles.get(name)
        if not r or not g:
            return f"{name}: only in {'gnu' if g else 'romacs'}'s checkout"
        if r['status'] != g['status']:
            return f"{name}: romacs {r['status']} ({r.get('error')}); gnu {g['status']}"
        for test in list(g['tests']) + [t for t in r['tests'] if t not in g['tests']]:
            if r['tests'].get(test) != g['tests'].get(test):
                return f"{name}: {test}: romacs {r['tests'].get(test)}, gnu {g['tests'].get(test)}"
    rj, gj = romacs.get('journey', {}), gnu.get('journey', {})
    for r, g in zip(rj.get('steps', []), gj.get('steps', [])):
        if r != g:
            return f"journey {g.get('name')}: romacs {r.get('error') or r.get('value')}; gnu {g.get('error') or g.get('value')}"
    if len(rj.get('steps', [])) != len(gj.get('steps', [])):
        return f"journey: romacs ran {len(rj.get('steps', []))} steps, gnu {len(gj.get('steps', []))}"
    if rj and (rj.get('status') != 'complete' or gj.get('status') != 'complete'):
        return f"journey: romacs {rj.get('status')} ({rj.get('error')}); gnu {gj.get('status')} ({gj.get('error')})"
    return None


def journey_state(package):
    """same: both complete with equal steps; incomplete: either did not finish."""
    r, g = package.get('romacs', {}).get('journey'), package.get('gnu', {}).get('journey')
    if not r or not g:
        return '-'
    if r.get('status') != 'complete' or g.get('status') != 'complete':
        return 'incomplete'
    return 'same' if r.get('steps') == g.get('steps') else 'differs'


def report(results):
    """The table: one row per package."""
    head = f"{'package':20} {'install':9} {'romacs pass/fail/skip':22} {'gnu pass/fail/skip':19} {'journey':8} first divergence"
    rows = [head, '-' * len(head)]
    for name, package in sorted(results['packages'].items()):
        install = '/'.join('ok' if package.get(e, {}).get('install', {}).get('ok') else 'FAIL' for e in EDITORS)

        def cell(editor):
            c = counts(package.get(editor, {}).get('tests'))
            if not c['files']:
                return '-'
            broken = f" ({c['broken']} broken)" if c['broken'] else ''
            return f"{c['passed']}/{c['failed']}/{c['skipped']}{broken}"
        rows.append(f"{name:20} {install:9} {cell('romacs'):22} {cell('gnu'):19} {journey_state(package):8} "
                    f"{first_divergence(package) or ''}")
    return '\n'.join(rows)


def regressions(old, new):
    """What Romacs did in OLD and no longer does in NEW: an install, a file's
    completion, a test's pass, a journey's completion or a step that matched GNU."""
    found = []
    for name, before in old['packages'].items():
        after = new['packages'].get(name)
        if after is None:
            found.append(f'{name}: no longer measured')
            continue
        b, a = before.get('romacs', {}), after.get('romacs', {})
        if b.get('install', {}).get('ok') and not a.get('install', {}).get('ok'):
            found.append(f'{name}: install now fails')
        bfiles, afiles = files_of(b), files_of(a)
        for file, entry in bfiles.items():
            now = afiles.get(file)
            if now is None:
                found.append(f'{name}: {file}: no longer run')
                continue
            if entry['status'] == 'complete' and now['status'] != 'complete':
                found.append(f'{name}: {file}: complete, now {now["status"]}')
            for test, kind in entry['tests'].items():
                if kind in PASSING and now['tests'].get(test) not in PASSING:
                    found.append(f'{name}: {file}: {test}: {kind}, now {now["tests"].get(test)}')
        bj, aj = b.get('journey'), a.get('journey')
        if bj:
            if bj.get('status') == 'complete' and (aj or {}).get('status') != 'complete':
                found.append(f'{name}: journey complete, now {(aj or {}).get("status")}')
            gb = before.get('gnu', {}).get('journey', {}).get('steps', [])
            ga = after.get('gnu', {}).get('journey', {}).get('steps', [])
            asteps = (aj or {}).get('steps', [])
            for i, step in enumerate(bj.get('steps', [])):
                matched = i < len(gb) and gb[i] == step
                still = i < len(asteps) and i < len(ga) and asteps[i] == ga[i]
                if matched and not still:
                    found.append(f"{name}: journey step {step.get('name')} matched GNU, now does not")
    return found


# --- processes --------------------------------------------------------------

def private_environment(scratch, editor):
    """A process environment that reaches nothing of the user's: its own HOME,
    XDG directories (runtime too), TMPDIR and Git identity."""
    home = Path(tempfile.mkdtemp(prefix='home-', dir=scratch))
    (home / '.emacs.d').mkdir()
    env = {k: os.environ[k] for k in KEPT_ENV if k in os.environ}
    env.update(FIXED_GIT, HOME=str(home), TMPDIR=str(home), GIT_CONFIG_GLOBAL=str(home / '.gitconfig'))
    for key in XDG:
        (home / key.lower()).mkdir()
        env[key] = str(home / key.lower())
    runtime_base = Path(f'/run/user/{os.getuid()}')
    runtime = Path(tempfile.mkdtemp(prefix='pp-', dir=runtime_base if runtime_base.is_dir() else home))
    env['XDG_RUNTIME_DIR'] = str(runtime)
    if editor['name'] == 'romacs' and editor.get('lisp_dir'):
        env['ROMACS_LISP_DIR'] = editor['lisp_dir']
    return home, runtime, env


def run_editor(editor, forms, scratch, timeout, elpa):
    """Run FORMS (Lisp text) in EDITOR's batch mode: (status or None, output)."""
    home, runtime, env = private_environment(scratch, editor)
    env['PP_ELPA'] = str(elpa)
    script = home / 'run.el'
    script.write_text(forms)
    log = home / 'output'
    with log.open('wb') as sink:
        process = subprocess.Popen([editor['binary'], '-Q', '--batch', '-l', str(DRIVER), '-l', str(script)],
                                   stdin=subprocess.DEVNULL, stdout=sink, stderr=subprocess.STDOUT,
                                   env=env, cwd=home, start_new_session=True)
    try:
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        status = None
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        shutil.rmtree(runtime, ignore_errors=True)
    output = log.read_text(encoding='utf-8', errors='replace')
    shutil.rmtree(home, ignore_errors=True)
    return status, normalize(output, [(str(elpa), '$ELPA'), (str(home), '$HOME')])


def lisp(value):
    """VALUE as a Lisp literal: strings, nil, and lists of strings."""
    if value is None:
        return 'nil'
    if isinstance(value, list):
        return "'(" + ' '.join(lisp(v) for v in value) + ')'
    return json.dumps(str(value), ensure_ascii=False)


def vc_spec(package, mirror):
    """PACKAGE's package-vc specification, as Lisp text, installing from MIRROR."""
    keys = [':url', lisp(mirror)]
    for key, field in ((':lisp-dir', 'lisp_dir'), (':main-file', 'main_file')):
        if package[field]:
            keys += [key, lisp(package[field])]
    if package['ignored_files']:
        keys += [':ignored-files', lisp(package['ignored_files'])[1:]]
    return f"'({package['name']} {' '.join(keys)})"


def install_forms(package, mirror):
    return f"(pp-install {vc_spec(package, mirror)} {lisp(package['rev'])})\n"



def test_forms(checkout, package, file):
    """Run FILE's tests.  ERT loads before the package's directories join
    load-path: some packages carry an old copy of it."""
    reporter = REPORTER.read_text()
    loads = [str(checkout / l) for l in package['load']] + [str(file.parent)]
    setup = ''.join(f"(load {lisp(str(checkout / f))} nil t t)\n" for f in package['setup'])
    return (f"(require 'ert)\n(pp-activate {lisp(loads)})\n"
            f"(setq default-directory {lisp(str(checkout) + '/')})\n{setup}"
            f"(load {lisp(str(file))} nil t t)\n(setq ert-scoreboard-selector '{SELECTOR})\n"
            f"(kill-emacs (progn {reporter}))\n")


def journey_forms(journey):
    return (f"(pp-activate nil)\n(load {lisp(str(journey))} nil t t)\n"
            "(pp-emit '((event . \"done\")))\n(kill-emacs 0)\n")


def test_files(checkout, package):
    files = set()
    for pattern in package['tests']:
        files.update(p for p in checkout.glob(pattern) if p.is_file())
    return sorted(p for p in files
                  if not any(fnmatch.fnmatch(str(p.relative_to(checkout)), x) for x in package['exclude']))


def install_all(editor, packages, order, mirrors, scratch, timeout):
    """Install ORDER into EDITOR's package directory, one process each."""
    elpa = scratch / editor['name'] / 'elpa'
    elpa.mkdir(parents=True, exist_ok=True)
    results = {}
    for name in order:
        package = packages[name]
        start = time.monotonic()
        status, output = run_editor(editor, install_forms(package, mirrors[package['url']]),
                                       scratch / editor['name'], timeout, elpa)
        found = [e for e in events(output, 'PP') if e.get('event') == 'install']
        result = found[-1] if found else dict(ok=False, error=last_error(output))
        state = process_state(status, bool(found))
        if state != 'complete':
            result.update(ok=False, error=f'{state}: {result.get("error") or last_error(output)}')
        result['elapsed'] = round(time.monotonic() - start, 2)
        result.pop('event', None)
        results[name] = result
        print(f"  {editor['name']:6} install {name:20} {'ok' if result.get('ok') else 'FAIL ' + str(result.get('error'))[:120]}",
              flush=True)
    return elpa, results


def run_tests(editor, package, elpa, scratch, timeout):
    checkout = elpa / package['name']
    selected = test_files(checkout, package)
    if not selected:
        # The manifest names suites; none matching is a broken setup, not an empty pass.
        return dict(files={'(no file matched)': dict(status='no-files', selected=0, tests={},
                                                    error=f"no file matches {package['tests']}")})
    files = {}
    for file in selected:
        start = time.monotonic()
        status, output = run_editor(editor, test_forms(checkout, package, file),
                                       scratch / editor['name'], timeout, elpa)
        result = ert_result(output, status)
        result['elapsed'] = round(time.monotonic() - start, 2)
        relative = str(file.relative_to(checkout))
        files[relative] = result
        log = scratch / 'logs' / editor['name'] / package['name'] / (relative.replace('/', '__') + '.log')
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_text(output)
    return dict(files=files)


def run_journey(editor, package, elpa, scratch, timeout):
    status, output = run_editor(editor, journey_forms(DATA / 'journeys' / package['journey']),
                                   scratch / editor['name'], timeout, elpa)
    log = scratch / 'logs' / editor['name'] / package['name'] / 'journey.log'
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_text(output)
    found = events(output, 'PP')
    steps = [{k: v for k, v in e.items() if k != 'event'} for e in found if e.get('event') == 'step']
    state = process_state(status, any(e.get('event') == 'done' for e in found))
    return dict(status=state, steps=steps, error=None if state == 'complete' else last_error(output))


def measure_package(editor, package, elpa, scratch, timeout, journey_timeout):
    out = {}
    if package['tests']:
        out['tests'] = run_tests(editor, package, elpa, scratch, timeout)
    if package['journey']:
        out['journey'] = run_journey(editor, package, elpa, scratch, journey_timeout)
    return out


def identity(editor):
    binary = Path(editor['binary'])
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    info = dict(binary=str(binary), sha256=digest)
    build = binary.parent / 'build.json'
    if build.is_file():
        info['commit'] = json.loads(build.read_text()).get('commit')
    return info
