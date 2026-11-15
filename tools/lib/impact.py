#!/usr/bin/env python3
"""What a change can reach: the Rust tests, live harnesses and Lisp it calls for.

  python3 -B tools/lib/impact.py select [--root ROOT] [--base REV] [PATH...]
  python3 -B tools/lib/impact.py plan   [--root ROOT] [--base REV] [PATH...]

The one map from changed paths to what must run, for the small gate, the
train's tier 1 and dev-check (they import `select`; the command prints its
answer as JSON for shell callers; `plan' prints it as one run a package,
`runs'). Without PATHs it reads the paths the
checkout changes since its merge-base with REV (default beta), uncommitted
ones too.

The rule, by path:
  crates/P/src/**, crates/P/build.rs, crates/P/Cargo.toml
      every test of P and of each workspace package that depends on P,
      directly or not, dev-dependencies too (`cargo metadata`). Within a
      crate's sources a module reaches no less: any module can change what a
      test of another observes.
  crates/P/tests/FILE.rs
      that file's tests, in its own target or its folded one (test_modules.py).
  crates/P/tests/support/S.rs
      the test files that name S (`#[path]', `include!'), directly or through
      another support file; every test of P when none does.
  any other Rust file of a crate (a folded target's main.rs, an example)
      every test of the package and its dependents.
  any other file under a crate (fixtures, harness scripts)
      the Rust files that name it, and the harnesses that import it; when
      nothing names it, every test of its package.
  a tree outside the crates the tests read (lisp/, test/, etc/, ledger/,
  assets/)
      every test of each package whose sources name that tree, and of its
      dependents; runtime Lisp also goes to the Lisp tier (`lisp`).
  documentation (doc/, *.md, *.org outside ledger/), tools/tests/
      nothing.
  anything else (the workspace's manifests, the toolchain, the test tools)
      everything.
The structural rules (STRUCTURAL) run whatever changed, and a crate's own
rules (CRATE_RULES) when it changed. A change that reaches romacs-vm-gui's
binary or the runtime tree it reads calls for every standard harness
(HARNESSES); HARNESS_RULES add others for their paths.
"""
import argparse
import json
import pathlib
import re
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import test_modules  # noqa: E402

# The rules a change can break anywhere, which no path would select: what may
# run Lisp, what may hold a value across it, what each primitive declares,
# which derived files match. A few minutes at worst, of which
# compile_boundaries, the compile-fail fixtures of the rooting rules, is most.
STRUCTURAL = ('romacs-vm::reentry_sites', 'romacs-vm::compile_boundaries', 'romacs-vm::rooting_brand_rules',
              'romacs-vm::no_remote_references', 'romacs-vm::heap_boundary', 'romacs-vm::oclosure_boundary',
              'romacs-vm::gc_threads_forbidden', 'romacs-vm::primitive_registry', 'romacs-vm::production_registry',
              'romacs-vm::host_boundary_lint', 'romacs-vm::primitive_gc_boundary', 'romacs-gui-tests::harness_contract')

# The structural rules whose compile-fail fixtures run the compiler: minutes
# when their inputs changed, where the rest take seconds once the test target
# is built. A check a lane runs before it submits leaves them to the queue.
SLOW_RULES = ('romacs-vm::compile_boundaries', 'romacs-vm::primitive_gc_boundary')

# The tests that refuse to pass without the gate's run-local evidence
# (ROMACS_GATE_EVIDENCE, prepared by tools/gate for its own run): the parity
# ratchets. Only the gate runs them; a check of its own leaves them out.
GATE_ONLY = 'test(/(^|::)parity_ratchet_/)'

# A rule that scans one crate's sources, kept as a unit test of that crate:
# its package, and the nextest filter of the rule. It runs when the crate
# changes, so a small gate does not build every such crate's tests.
CRATE_RULES = {'romacs-display-runtime': 'package(romacs-display-runtime) & test(/(^|::)time_discipline_test::/)'}

# The live harnesses a check runs for any change that reaches the editor
# (crates/romacs-vm-gui/harness/NAME_live.py, each taking --binary and --output).
HARNESSES = ('world', 'autoloads', 'completion', 'fontify', 'timers', 'popup', 'lines', 'find_file',
             'entry_prompts', 'requests', 'landing_anim', 'layout_journey', 'chrome', 'scaling', 'display_stages')
# More harnesses, each for the paths only it drives end to end.
HARNESS_RULES = (
    # The agent client end to end (sessions, selections and restarts, relays, tools).
    (re.compile(r'^(?:lisp/romacs/romacs-(?:agent|mcp)[^/]*\.el|crates/romacs-mcp/'
                r'|crates/romacs-vm-gui/harness/(?:agent_live|fake_[a-z]+)\.py)'), 'agent'),
)
EDITOR = 'romacs-vm-gui'
HARNESS_DIR = 'crates/romacs-vm-gui/harness/'

# Trees outside the crates whose files tests read, and the runtime ones a
# session of the editor reads.
DATA = ('lisp', 'test', 'etc', 'ledger', 'assets')
RUNTIME = ('lisp', 'etc')
# Paths that change nothing a test runs.
QUIET = re.compile(r'^(?:doc/|tools/tests/)|\.md$|^(?!ledger/).*\.org$')


def workspace(metadata):
    """{package: its directory relative to the root} and {package: the
    packages that depend on it, directly or not}, from `cargo metadata`."""
    members = set(metadata['workspace_members'])
    root = pathlib.Path(metadata['workspace_root'])
    packages = [p for p in metadata['packages'] if p['id'] in members]
    dirs = {p['name']: str(pathlib.Path(p['manifest_path']).parent.relative_to(root)) for p in packages}
    users = {name: set() for name in dirs}
    for p in packages:
        for dep in p['dependencies']:
            if dep['name'] in users and dep['name'] != p['name']:
                users[dep['name']].add(p['name'])
    reach = {}
    for name in dirs:
        seen, todo = {name}, [name]
        while todo:
            for user in users[todo.pop()]:
                if user not in seen:
                    seen.add(user)
                    todo.append(user)
        reach[name] = seen
    return dirs, reach


def test_file(binary):
    """Nextest filter for the tests of the test file BINARY, `PACKAGE::FILE`:
    its own target and, in a package that has folded targets, its module in
    the one that holds it (test_modules.py). A run builds one of the two.
    Nextest refuses a filter naming a binary no package has, so a package
    without them is not asked for one."""
    package, file = binary.split('::')
    alone = f'binary_id({binary})'
    if package not in test_modules.PACKAGES:
        return alone
    folded = ' | '.join(f'binary_id({package}::{target})' for target in test_modules.FOLDED[package])
    return f'{alone} | (({folded}) & test(/^{file}::/))'


def rust_sources(root, dirs):
    """{path: text} of every Rust file under the packages' directories."""
    return {str(path.relative_to(root)): path.read_text(errors='replace')
            for d in dirs.values() for path in (root / d).rglob('*.rs') if 'target' not in path.parts}


def includers(support, sources, tests_dir):
    """The test files of TESTS_DIR that include SUPPORT (a path under it),
    directly or through another support file: those that name its file, by
    `#[path]', `include!' or any other way."""
    wanted, found, files = {support}, set(), [p for p in sources if p.startswith(tests_dir)]
    while wanted:
        name = pathlib.PurePosixPath(wanted.pop()).name
        for path in files:
            if path not in found and name in sources[path]:
                found.add(path)
                if '/support/' in path:
                    wanted.add(path)
    return {p for p in found if '/support/' not in p and p.count('/') == tests_dir.count('/')}


def harness_users(changed, root):
    """The live harnesses (HARNESSES and the rules' names) whose script is
    CHANGED (a module of HARNESS_DIR) or imports it, directly or not."""
    harness = root / HARNESS_DIR
    modules = {p.stem: p.read_text(errors='replace') for p in harness.glob('*.py')}
    wanted, found = {pathlib.PurePosixPath(changed).stem}, set()
    while wanted:
        name = wanted.pop()
        found.add(name)
        pattern = re.compile(rf'^\s*(?:import\s+{re.escape(name)}\b|from\s+{re.escape(name)}\s+import\b)', re.M)
        wanted |= {m for m, text in modules.items() if m not in found and pattern.search(text)}
    names = set(HARNESSES) | {name for _, name in HARNESS_RULES}
    return {m[:-len('_live')] for m in found if m.endswith('_live') and m[:-len('_live')] in names}


def select(paths, root, metadata):
    """What a change to PATHS (relative to ROOT) can reach. A dict:
    everything (bool) and reasons; packages and files ({name: why}: every
    test of a package, the tests of a file `PACKAGE::FILE`); harnesses
    ({name: why}); lisp (the runtime Lisp paths, the Lisp tier's); filter, a
    nextest expression, None when everything runs; packages_to_build."""
    root = pathlib.Path(root)
    dirs, reach = workspace(metadata)
    by_dir = sorted(dirs.items(), key=lambda kv: -len(kv[1]))
    sources = None
    packages, files, harnesses, reasons, lisp = {}, {}, {}, [], []

    def whole(package, why):
        for name in sorted(reach[package]):
            packages.setdefault(name, why if name == package else f'depends on {package} ({why})')

    def owner(path):
        return next(((name, d) for name, d in by_dir if path.startswith(d + '/')), (None, None))

    def readers(tree):
        nonlocal sources
        sources = sources or rust_sources(root, dirs)
        pattern = re.compile(rf'["/]{re.escape(tree)}/|join\("{re.escape(tree)}"\)')
        return sorted({owner(path)[0] for path, text in sources.items() if pattern.search(text)} - {None})

    for path in sorted(set(paths)):
        if QUIET.search(path):
            continue
        for rx, name in HARNESS_RULES:
            if rx.match(path):
                harnesses.setdefault(name, path)
        package, pdir = owner(path)
        top = path.split('/', 1)[0]
        if package:
            rest = path[len(pdir) + 1:]
            if rest.startswith('src/') or rest in ('build.rs', 'Cargo.toml') or \
                    (rest.endswith('.rs') and not re.fullmatch(r'tests/(?:support/)?[^/]+\.rs', rest)):
                whole(package, f'{rest} changed')
            elif re.fullmatch(r'tests/[^/]+\.rs', rest):
                files.setdefault(f'{package}::{pathlib.PurePosixPath(rest).stem}', f'{rest} changed')
            elif re.fullmatch(r'tests/support/[^/]+\.rs', rest):
                sources = sources or rust_sources(root, dirs)
                users = includers(path, sources, f'{pdir}/tests/')
                for user in sorted(users):
                    files.setdefault(f'{package}::{pathlib.PurePosixPath(user).stem}', f'includes {rest}')
                if not users:
                    whole(package, f'{rest}: no test file names it')
            else:
                sources = sources or rust_sources(root, dirs)
                name = pathlib.PurePosixPath(path).name
                named = sorted(p for p, text in sources.items() if name in text)
                used = harness_users(path, root) if path.startswith(HARNESS_DIR) and path.endswith('.py') else set()
                for harness in sorted(used):
                    harnesses.setdefault(harness, f'{path} changed')
                for user in named:
                    user_package, user_dir = owner(user)
                    test = re.fullmatch(r'tests/([^/]+)\.rs', user[len(user_dir) + 1:])
                    if test:
                        files.setdefault(f'{user_package}::{test[1]}', f'names {name}')
                    else:
                        whole(user_package, f'{user} names {name}')
                if not named and not used:
                    whole(package, f'{rest}: nothing names it')
        elif top in DATA and '/' in path:
            if top == 'lisp' or path.endswith('.el'):
                lisp.append(path)
            for reader in readers(top):
                whole(reader, f'reads {top}/ ({path})')
            if top in RUNTIME:
                packages.setdefault(EDITOR, f'its sessions read {top}/ ({path})')
        else:
            reasons.append(path)
    if reasons:
        return {'everything': True, 'reasons': reasons, 'packages': {}, 'files': {}, 'lisp': lisp, 'filter': None,
                'harnesses': {name: 'everything runs' for name in HARNESSES} | harnesses,
                'packages_to_build': sorted(dirs)}
    if EDITOR in packages:
        for name in HARNESSES:
            harnesses.setdefault(name, f'{EDITOR}: {packages[EDITOR]}')
    rules = [test_file(binary) for binary in STRUCTURAL] + [rule for crate, rule in sorted(CRATE_RULES.items())
                                                             if crate in packages]
    terms = [f'package({name})' for name in sorted(packages)] + \
            [test_file(f) for f in sorted(files) if f.split('::')[0] not in packages] + rules
    build = {name for name in packages} | {f.split('::')[0] for f in files} | {b.split('::')[0] for b in STRUCTURAL}
    return {'everything': False, 'reasons': [], 'packages': packages, 'files': files, 'harnesses': harnesses,
            'lisp': lisp, 'filter': ' | '.join(terms), 'packages_to_build': sorted(build)}


def runs(answer, root, leave=()):
    """ANSWER (select's) as one run a package: [(package, cargo target flags,
    nextest filter)], for a caller that runs each package alone; None when
    everything runs. A package's filter keeps the terms that name it. LEAVE
    are test files, `PACKAGE::FILE` (as SLOW_RULES), that no run takes, though
    the answer reaches them: a package's filter then ends `& not (...)`."""
    if answer['everything']:
        return None
    terms = {}
    for name in answer['packages']:
        terms.setdefault(name, []).append(f'package({name})')
    for name in sorted((set(answer['files']) | set(STRUCTURAL)) - set(leave)):
        package = name.split('::')[0]
        if package not in answer['packages']:
            terms.setdefault(package, []).append(test_file(name))
    for crate, rule in CRATE_RULES.items():
        if crate in answer['packages']:
            terms[crate].append(rule)
    for package in terms:
        left = [test_file(name) for name in sorted(leave) if name.split('::')[0] == package]
        if left and package in answer['packages']:
            terms[package] = [f'({" | ".join(terms[package])}) & not ({" | ".join(left)})']
    def flags(package):
        if package in test_modules.PACKAGES:
            return test_modules.targets(root, package)
        # Every target that has tests, the library and binaries among them
        # where the package has them: --lib refuses a package with none.
        return ['--tests']
    return [(package, flags(package), ' | '.join(found)) for package, found in sorted(terms.items())]


def changed(root, base='beta'):
    """The paths ROOT changes since its merge-base with BASE (or origin/BASE),
    uncommitted ones too."""
    def git(*args, check=True):
        return subprocess.run(['git', '-C', str(root), *args], capture_output=True, text=True, check=check)
    if git('rev-parse', '--verify', '--quiet', base, check=False).returncode and \
            not git('rev-parse', '--verify', '--quiet', f'origin/{base}', check=False).returncode:
        base = f'origin/{base}'
    since = git('merge-base', 'HEAD', base).stdout.strip()
    names = set(git('diff', '--name-only', since).stdout.split())
    return sorted(names | set(git('ls-files', '-mo', '--exclude-standard').stdout.split()))


def metadata_of(root):
    """`cargo metadata --no-deps` of the workspace at ROOT: its packages and
    their declared dependencies, which is all `select` reads."""
    return json.loads(subprocess.run(['cargo', 'metadata', '--no-deps', '--format-version', '1', '--offline'],
                                     cwd=root, capture_output=True, text=True, check=True).stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('select', 'plan'))
    parser.add_argument('paths', nargs='*')
    parser.add_argument('--root', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
    parser.add_argument('--base', default='beta')
    args = parser.parse_args()
    paths = args.paths or changed(args.root, args.base)
    answer = select(paths, args.root, metadata_of(args.root))
    print(json.dumps(answer if args.command == 'select' else runs(answer, args.root), indent=1))
    return 0


if __name__ == '__main__':
    sys.exit(main())
