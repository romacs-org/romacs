#!/usr/bin/env python3
"""One test executable for a package's integration tests.

Cargo makes each `tests/FILE.rs` a test target of its own, compiled and linked
apart from the others: for romacs-vm that is one executable per file, and the
linking is most of what a check's build of them costs. `tests/all` is one more
target, which holds each of those files as its module `FILE`; a check builds
and runs it instead, and the test `NAME` of `tests/FILE.rs` is `FILE::NAME`
there. The files stay targets too, so `cargo test --test FILE` builds one alone.

`tests/all` declares the modules when it builds, from the directory's files
(crates/romacs-vm-test-build, whose build script watches `tests/`; romacs-vm's
own would rebuild its library on every edit of a test), so a new test file
edits no list. This tool reads the same files for the checks.

A file that boots GNU's Lisp (it includes one of GNU_LISP's support files)
is a module of `tests/gnu` instead, whose executable restores that boot from
a world (support/gnu_lisp.rs); crates/romacs-vm-test-build holds the same rule.

A file that cannot be a module is named in `tests/all/standalone.txt`, one
`FILE: reason` a line. It stays a target of its own, and `targets` names it
beside `all` and `gnu`, so what a check runs is every test either way.

usage: test_modules.py modules [--root ROOT]
       test_modules.py targets [--root ROOT] PACKAGE
       test_modules.py equal [--root ROOT] PACKAGE < NEXTEST-LIST-JSON

`targets` prints the Cargo arguments that select what a check builds and runs
of PACKAGE. `modules` prints `PACKAGE FILE` for each file that is a module of
its package's `all` or `gnu`: the targets a check of several packages leaves
out (tools/gate). `equal` reads `cargo nextest list --message-format json` of
every target of PACKAGE and fails unless `all` and `gnu` hold exactly the tests
of the files they have as modules, under their prefixed names and as ignored as
they are there.
"""
import argparse
import json
from pathlib import Path
import sys

# package name -> its directory
PACKAGES = {'romacs-vm': 'crates/romacs-vm'}
ALL = 'all'
GNU = 'gnu'
# The support files whose includers boot GNU's Lisp: modules of GNU.
GNU_LISP = ('support/gnu_lisp.rs', 'support/gnu_ert_vm.rs', 'support/gui_probe.rs')


def test_files(tests):
    """The names of the test targets Cargo finds as files of TESTS."""
    return sorted(path.stem for path in Path(tests).glob('*.rs'))


def standalone(text):
    """{file: reason} of a standalone list's TEXT."""
    entries = {}
    for line in text.splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        name, colon, reason = line.partition(': ')
        if not colon or not reason.strip() or name in entries:
            raise ValueError(f'standalone.txt: `{line}` is not one `FILE: reason` for a file named once')
        entries[name] = reason.strip()
    return entries


def modules(files, alone):
    """The FILES that are modules of `all`: all but those ALONE names, each
    of which must be a file."""
    gone = sorted(set(alone) - set(files))
    if gone:
        raise ValueError(f'standalone.txt names no test file: {", ".join(gone)}')
    return [name for name in files if name not in alone]


def folded_into(source):
    """The target a module whose SOURCE this is belongs to: GNU when it
    includes one of GNU_LISP, else ALL."""
    return GNU if any(f'#[path = "{support}"]' in source for support in GNU_LISP) else ALL


def package(root, name):
    """({module: its target}, standalone) of the package NAME under ROOT."""
    tests = Path(root) / PACKAGES[name] / 'tests'
    alone = standalone((tests / ALL / 'standalone.txt').read_text())
    return {file: folded_into((tests / f'{file}.rs').read_text()) for file in modules(test_files(tests), alone)}, alone


def targets(root, name):
    """Cargo's target selection for a check of the package NAME."""
    return ['--lib', '--bins', '--test', ALL, '--test', GNU] + [arg for file in sorted(package(root, name)[1])
                                                                for arg in ('--test', file)]


def equal(inventory, name, folded, alone):
    """(how many tests the files of FOLDED ({module: its target}) of the
    package NAME list in targets of their own, why `all` and `gnu` do not
    hold exactly those, one line each), from INVENTORY, nextest's
    `rust-suites`."""
    suites = {suite['binary-name']: suite['testcases'] for suite in inventory.values()
              if suite['package-name'] == name and suite['kind'] == 'test'}
    missing = sorted((set(folded) | set(alone) | {ALL, GNU}) - set(suites))
    if missing:
        return 0, [f'{name}: no test target listed for {", ".join(missing)}']
    count, found = 0, []
    for target in (ALL, GNU):
        apart = {(f'{file}::{test}', case['ignored']) for file, into in folded.items() if into == target
                 for test, case in suites[file].items()}
        # The target's own tests are the ones outside every module.
        together = {(test, case['ignored']) for test, case in suites[target].items() if '::' in test}
        if not apart:
            found.append(f'{name}: the files that are modules of `{target}` list no test')
        found += ([f'{name}: {test} is {"ignored " if ignored else ""}in its own target, not so in `{target}`'
                   for test, ignored in sorted(apart - together)]
                  + [f'{name}: {test} is {"ignored " if ignored else ""}in `{target}`, not so in its own target'
                     for test, ignored in sorted(together - apart)])
        count += len(apart)
    return count, found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('modules', 'targets', 'equal'))
    parser.add_argument('package', nargs='?', choices=tuple(PACKAGES))
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    if (args.command in ('targets', 'equal')) != bool(args.package):
        parser.error('targets and equal take a package; modules takes none')
    try:
        if args.command == 'targets':
            print(' '.join(targets(args.root, args.package)))
            return 0
        if args.command == 'modules':
            for name in PACKAGES:
                print(''.join(f'{name} {file}\n' for file in sorted(package(args.root, name)[0])), end='')
            return 0
        folded, alone = package(args.root, args.package)
        count, found = equal(json.load(sys.stdin)['rust-suites'], args.package, folded, alone)
        for line in found:
            print(f'test-modules: {line}', file=sys.stderr)
        if not found:
            print(f'test-modules: {args.package}: `{ALL}` and `{GNU}` hold the {count} tests of their '
                  f'{len(folded)} files, each under its file\'s name; {len(alone)} files stand alone')
        return 1 if found else 0
    except (OSError, ValueError, KeyError) as exc:
        print(f'test-modules: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
