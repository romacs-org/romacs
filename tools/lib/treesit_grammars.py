#!/usr/bin/env python3
"""The tree-sitter grammars Romacs ships (crates/romacs-grammars).

Each grammar is the commit its GNU ts mode pins in
`treesit-language-source-alist', linked into the executable
(DIVERGENCES.md VM2-TREESIT-BUNDLED).

  check [--root ROOT]   Every source file Cargo compiles for a grammar is the
                        one crates/romacs-grammars/sources.json records for
                        GNU's commit (SHA-256, fetched from the upstream
                        repository when the grammar was added), the table in
                        src/lib.rs agrees with it, and etc/tree-sitter/
                        carries each grammar's notice and licence.
  so --out DIR [LANG]   Build each grammar from those sources as GNU's
                        `treesit--build-grammar' does, into
                        DIR/libtree-sitter-LANG.so: the same grammars as
                        files, for pinned GNU (`treesit-extra-load-path').

Offline: sources come from Cargo's registry and git checkouts.
"""
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = Path('crates/romacs-grammars')
NOTICES = Path('etc/tree-sitter')
# The grammar directory inside its crate, where it is not `src'.
SOURCE_DIRS = {
    'markdown': 'tree-sitter-markdown/src',
    'markdown-inline': 'tree-sitter-markdown-inline/src',
    'tsx': 'tsx/src',
    'typescript': 'typescript/src',
}
ENTRY = re.compile(r'^\s*"([a-z-]+)" (tree_sitter_[a-z_]+)::\w+, "([^"]+)" "([0-9a-f]{40})";', re.M)


def table(root):
    """(language, crate, url, commit) of each entry of src/lib.rs's BUNDLED."""
    text = (root / CRATE / 'src/lib.rs').read_text()
    return [(name, module.replace('_', '-'), url, commit) for name, module, url, commit in ENTRY.findall(text)]


def crate_dirs(root):
    """Each package's directory, from Cargo's offline metadata."""
    out = subprocess.run(['cargo', 'metadata', '--offline', '--locked', '--format-version', '1'],
                         cwd=root, check=True, capture_output=True, text=True).stdout
    return {p['name']: Path(p['manifest_path']).parent for p in json.loads(out)['packages']}


def compiled_files(crate, language):
    """The C sources and headers a grammar's build reads, relative to its
    crate: its grammar directory, and headers its files include from
    elsewhere in the crate."""
    source = crate / SOURCE_DIRS.get(language, 'src')
    files = {p for p in source.rglob('*') if p.suffix in ('.c', '.h', '.cc')}
    for path in list(files):
        for name in re.findall(r'#include "([^"]+)"', path.read_text(errors='replace')):
            included = (path.parent / name).resolve()
            if included.is_file() and crate in included.parents:
                files.add(included)
    return sorted(p.relative_to(crate).as_posix() for p in files)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def licence_name(url):
    return url.rstrip('/').rsplit('/', 1)[1] + '.LICENSE'


def check(root):
    """Problems with the shipped grammars' sources and notices."""
    problems = []
    recorded = json.loads((root / CRATE / 'sources.json').read_text())
    entries = table(root)
    if sorted(n for n, *_ in entries) != sorted(recorded):
        problems.append(f'src/lib.rs languages {sorted(n for n, *_ in entries)} != sources.json {sorted(recorded)}')
    dirs = crate_dirs(root)
    readme = (root / NOTICES / 'README.md').read_text()
    for name, crate, url, commit in entries:
        record = recorded.get(name)
        if record is None:
            continue
        if (record['url'], record['commit']) != (url, commit):
            problems.append(f'{name}: src/lib.rs names {url} {commit}, sources.json {record["url"]} {record["commit"]}')
        files = compiled_files(dirs[crate], name)
        if files != sorted(record['files']):
            problems.append(f'{name}: compiles {files}, sources.json records {sorted(record["files"])}')
        for rel in files:
            if rel in record['files'] and digest(dirs[crate] / rel) != record['files'][rel]:
                problems.append(f'{name}: {rel} is not the source of {commit}')
        if f'| {name} |' not in readme or commit not in readme:
            problems.append(f'{name}: no row with {commit} in {NOTICES}/README.md')
        if not (root / NOTICES / 'LICENSES' / licence_name(url)).is_file():
            problems.append(f'{name}: no {NOTICES}/LICENSES/{licence_name(url)}')
    return problems


def build(language, source, out, cc):
    """GNU's recipe: each C source compiled -fPIC, linked -shared."""
    with tempfile.TemporaryDirectory(prefix='treesit-grammar-') as work:
        objects = []
        for name in ('parser.c', 'scanner.c'):
            if (source / name).exists():
                obj = Path(work) / (name + '.o')
                subprocess.run([cc, '-fPIC', '-O2', '-c', '-I.', name, '-o', str(obj)], cwd=source, check=True)
                objects.append(str(obj))
        target = out / f'libtree-sitter-{language}.so'
        subprocess.run([cc, '-fPIC', '-shared', *objects, '-o', str(target)], check=True)
        return target


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    c = sub.add_parser('check')
    c.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    s = sub.add_parser('so')
    s.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    s.add_argument('--out', type=Path, required=True)
    s.add_argument('--cc', default=shutil.which('cc') or 'cc')
    s.add_argument('languages', nargs='*')
    args = parser.parse_args()
    root = args.root.resolve()
    if args.command == 'check':
        problems = check(root)
        for problem in problems:
            print(f'treesit_grammars: {problem}', file=sys.stderr)
        return 1 if problems else 0
    entries = table(root)
    unknown = set(args.languages) - {name for name, *_ in entries}
    if unknown:
        sys.exit(f'treesit_grammars: not bundled: {" ".join(sorted(unknown))}')
    dirs = crate_dirs(root)
    args.out.mkdir(parents=True, exist_ok=True)
    for name, crate, *_ in entries:
        if not args.languages or name in args.languages:
            print(build(name, dirs[crate] / SOURCE_DIRS.get(name, 'src'), args.out.resolve(), args.cc))
    return 0


if __name__ == '__main__':
    sys.exit(main())
