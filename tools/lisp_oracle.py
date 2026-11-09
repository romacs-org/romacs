#!/usr/bin/env python3
"""Pristine GNU Git paths plus declared generated support (R221).

    lisp_oracle.py check [--root DIR]
        Every GNU Git path at the pin must be byte-identical and present.
        LISP-GENERATED.json separately declares retained generated GNU support
        by exact path, SHA-256 and pinned generator/source references. These
        are snapshots, not pristine Git blobs or a regeneration claim. No
        undeclared .el file (even ignored or without .git), or other non-ignored
        file in a repository, may exist under gnu-lisp/. A romacs variant of a GNU file
        may live ONLY in lisp-overlay/, declared in lisp-overlay/MANIFEST.tsv
        and rebased on the pin blob it varies; non-GNU Lisp lives in
        rlisp/ (external dependencies in rlisp/deps/). Exit 1 on any violation.

    lisp_oracle.py populations --rev REV [--out FILE]
        Re-derive the DRIFT/EDIT split of every shared path that differs from
        the pin at REV: DRIFT if our blob exists anywhere in GNU's history
        (an older GNU release, nobody's edit), EDIT otherwise (`git cat-file
        -e`, REVIEW-REPLY.md appendix B). Also records whether our blob equals
        tag emacs-31.1 at the same path.

Pristine-path expectations come from GNU's object store, never the support
manifest; the manifest cannot override them. Our side is hashed from the
WORKING TREE with `git hash-object`, so an uncommitted edit is caught too, and
no .git is needed (the gate's frozen mirror has none).
"""
import os as _portable_os
from pathlib import Path as _PortablePath
import argparse
import fnmatch
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys

GNU_PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
PIN_FILE = 'LISP-PIN.json'
GENERATED_FILE = 'LISP-GENERATED.json'
GNU_SOURCE = os.environ.get('LISP_ORACLE_GNU', str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / ''))
LOCAL_LISP = 'gnu-lisp'


def local_path(upstream_path):
    """Map a GNU source identity to its current checkout location."""
    return LOCAL_LISP + upstream_path[len('lisp'):]


OVERLAY = 'lisp-overlay'
HEADER = 'romacs-overlay:'
# Files in lisp-overlay/ that describe it rather than vary a GNU file.
OVERLAY_META = {'README.md', 'MANIFEST.tsv'}
OVERLAY_INERT = 'proposed'  # patches awaiting a decision; never loaded


def git(*args, cwd=None, data=None):
    r = subprocess.run(['git', *args], cwd=cwd, input=data, capture_output=True)
    if r.returncode:
        raise SystemExit(f'lisp_oracle: git {" ".join(args)} failed: {r.stderr.decode().strip()}')
    return r.stdout.decode()


def tree(repo, rev, prefix='lisp'):
    """{path: blob} for every file (not symlink/submodule) under prefix at rev."""
    out = {}
    for line in git('ls-tree', '-r', '-z', rev, '--', prefix, cwd=repo).split('\0'):
        if not line:
            continue
        meta, path = line.split('\t', 1)
        mode, kind, blob = meta.split()
        if kind == 'blob' and mode in ('100644', '100755'):
            out[path] = blob
    return out


def pin_tree(gnu, pin):
    commit = git('rev-parse', '--verify', pin + '^{commit}', cwd=gnu).strip()
    if commit != pin:
        raise SystemExit(f'lisp_oracle: pin {pin} must be a full commit id in {gnu}')
    return tree(gnu, pin)


def hash_files(root, paths):
    """Blob ids of working-tree bytes, exactly as git would store them."""
    if not paths:
        return {}
    data = ''.join(p + '\n' for p in paths).encode()
    out = git('hash-object', '--no-filters', '--stdin-paths', cwd=root, data=data).split()
    if len(out) != len(paths):
        raise SystemExit('lisp_oracle: hash-object returned the wrong number of ids')
    return dict(zip(paths, out))


def read_tsv(path):
    rows = []
    if not path.exists():
        return rows
    for n, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip() or line.startswith('#'):
            continue
        rows.append((n, line.split('\t')))
    return rows


def generated_support(root, gnu, pin, pinned, required):
    """Validate retained support without substituting for any pristine path."""
    path = root / GENERATED_FILE
    if not path.is_file():
        return {}, [f'{GENERATED_FILE} is missing'] if required else []
    try:
        record = json.loads(path.read_text())
        if record['gnu_commit'] != pin:
            raise ValueError('gnu_commit is not the oracle pin')
        sources = record['sources']
        if not isinstance(sources, dict) or not sources:
            raise ValueError('sources must be a nonempty object')
        upstream = tree(gnu, pin, prefix='.')
        for name, source in sources.items():
            if not source['paths'] or not source['recipe']:
                raise ValueError(f'{name}: source paths and recipe are required')
            for source_path in source['paths']:
                if source_path not in upstream:
                    raise ValueError(f'{name}: source {source_path} is not a GNU Git path at the pin')
        support = {}
        pristine_local = {local_path(p) for p in pinned}
        for rel, digest, source in record['files']:
            p = pathlib.PurePosixPath(rel)
            if (str(p) != rel or p.is_absolute() or '..' in p.parts
                    or p.parts[0] != LOCAL_LISP or p.suffix != '.el'):
                raise ValueError(f'invalid support path {rel}')
            if rel in pristine_local:
                raise ValueError(f'{rel}: support must not override a pristine GNU Git path')
            if rel in support:
                raise ValueError(f'duplicate support path {rel}')
            if not re.fullmatch(r'[0-9a-f]{64}', digest) or source not in sources:
                raise ValueError(f'{rel}: invalid SHA-256 or unknown source group')
            support[rel] = digest
    except (ValueError, KeyError, TypeError, IndexError) as error:
        return {}, [f'{GENERATED_FILE}: {error}']
    problems = []
    for rel, digest in support.items():
        file = root / rel
        if not file.is_file() or file.is_symlink():
            problems.append(f'ABSENT SUPPORT {rel}: declared generated input must be a regular file')
        elif hashlib.sha256(file.read_bytes()).hexdigest() != digest:
            problems.append(f'MODIFIED SUPPORT {rel}: bytes differ from {GENERATED_FILE}')
    return support, problems


def excluded_paths(upstream, patterns):
    """The GNU paths PATTERNS (fnmatch over GNU's own names, `*` crossing
    directories) deliberately drop, and the patterns that drop nothing."""
    dropped = {p for p in upstream if any(fnmatch.fnmatchcase(p, pat) for pat in patterns)}
    stale = [pat for pat in patterns if not any(fnmatch.fnmatchcase(p, pat) for p in upstream)]
    return dropped, stale


def check(root, gnu, pin):
    root = pathlib.Path(root).resolve()
    upstream = pin_tree(gnu, pin)
    pin_file = root / PIN_FILE
    rec = json.loads(pin_file.read_text()) if pin_file.exists() else {}
    dropped, stale = excluded_paths(upstream, rec.get('excluded', []))
    pinned = {local_path(p): blob for p, blob in upstream.items() if p not in dropped}
    problems = [f'{PIN_FILE}: excluded pattern {pat!r} matches no GNU path at the pin' for pat in stale]
    for p in sorted(map(local_path, dropped)):
        if (root / p).exists() or (root / p).is_symlink():
            problems.append(f'EXCLUDED {p}: {PIN_FILE} drops it deliberately; delete it')

    present = [p for p in sorted(pinned) if (root / p).is_file() and not (root / p).is_symlink()]
    absent = sorted(set(pinned) - set(present))
    ours = hash_files(root, present)
    differ = [p for p in present if ours[p] != pinned[p]]
    for p in differ:
        problems.append(f'MODIFIED {p}: ours {ours[p][:12]} != pin {pinned[p][:12]} '
                        f'(restore the pin blob; a variant belongs in {OVERLAY}/)')

    for p in absent:
        problems.append(f'ABSENT {p}: GNU ships it at the pin; gnu-lisp/ is GNU\'s whole tree (re-extract it)')

    # LISP-PIN.json records the pin and GNU's own tree id for lisp/; both must
    # agree with GNU's object store, so the record cannot drift from the pin.
    if pin_file.exists():
        want_tree = git('rev-parse', pin + ':lisp', cwd=gnu).strip()
        if rec.get('gnu_commit') != pin:
            problems.append(f'{PIN_FILE}: gnu_commit {rec.get("gnu_commit")} is not the pin {pin}')
        if rec.get('lisp_tree') != want_tree:
            problems.append(f'{PIN_FILE}: lisp_tree {rec.get("lisp_tree")} is not GNU {pin[:11]}:lisp {want_tree}')
    elif (root / '.git').exists():
        problems.append(f'{PIN_FILE} is missing')

    declared = rec.get('generated_support')
    if declared is not None and declared != GENERATED_FILE:
        problems.append(f'{PIN_FILE}: generated_support must name {GENERATED_FILE}')
    support, support_problems = generated_support(root, gnu, pin, upstream, declared is not None)
    problems.extend(support_problems)

    # Runtime Lisp inputs are checked even when ignored or in a .git-less
    # export. Git additionally identifies non-ignored, non-Lisp extras while
    # still allowing ignored build products such as .elc files.
    listed = {str(p.relative_to(root)) for p in (root / LOCAL_LISP).rglob('*.el')
              if not p.is_dir()}
    if (root / '.git').exists():
        listed.update(git('ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', LOCAL_LISP, cwd=root).split('\0'))
    gone = set(map(local_path, dropped))
    extras = sorted(p for p in listed if p and p not in pinned and p not in support and p not in gone
                    and ((root / p).exists() or (root / p).is_symlink()))
    for p in extras:
        problems.append(f'EXTRA {p}: neither a GNU Git path at the pin nor declared generated support; '
                        'non-GNU Lisp belongs in rlisp/ (external dependencies in rlisp/deps/)')

    manifest = {}
    for n, cols in read_tsv(root / OVERLAY / 'MANIFEST.tsv'):
        if len(cols) != 3 or not cols[2].strip():
            problems.append(f'{OVERLAY}/MANIFEST.tsv:{n}: want "PATH<TAB>PIN-BLOB<TAB>REASON"')
            continue
        manifest[cols[0]] = cols[1]
    overlay_dir = root / OVERLAY
    files = []
    if overlay_dir.is_dir():
        for f in sorted(overlay_dir.rglob('*')):
            rel = f.relative_to(overlay_dir)
            if f.is_dir() or (len(rel.parts) == 1 and rel.name in OVERLAY_META) or rel.parts[0] == OVERLAY_INERT:
                continue
            files.append(str(rel))
    for rel in files:
        gnu_path = f'lisp/{rel}'
        if rel not in manifest:
            problems.append(f'UNDECLARED {OVERLAY}/{rel}: every overlay file needs a MANIFEST.tsv row')
            continue
        if gnu_path not in upstream:
            problems.append(f'{OVERLAY}/{rel}: varies no GNU file (lisp/{rel} is not in the pin); '
                            'romacs-only Lisp belongs in rlisp/')
            continue
        if manifest[rel] != upstream[gnu_path]:
            problems.append(f'STALE {OVERLAY}/{rel}: rebased on {manifest[rel][:12]}, pin has {upstream[gnu_path][:12]}')
        head = (overlay_dir / rel).read_bytes()[:2048].decode(errors='replace').splitlines()[:5]
        if not any(HEADER in line for line in head):
            problems.append(f'{OVERLAY}/{rel}: no "{HEADER} <reason>" line in its first five lines')
        if hash_files(root, [f'{OVERLAY}/{rel}'])[f'{OVERLAY}/{rel}'] == upstream[gnu_path]:
            problems.append(f'{OVERLAY}/{rel}: identical to the pin; delete it')
    for rel in sorted(set(manifest) - set(files)):
        problems.append(f'{OVERLAY}/MANIFEST.tsv: {rel} declared but no such overlay file')

    print(f'lisp_oracle: pin {pin[:11]}: {len(pinned)} GNU paths, {len(present)} present '
          f'({len(present) - len(differ)} identical, {len(differ)} modified), {len(absent)} absent, '
          f'{len(dropped)} excluded, '
          f'{len(support)} declared generated support, {len(extras)} extra; overlay {len(files)} files')
    if problems:
        print(f'lisp_oracle: FAIL, {len(problems)} violation(s) of R221 (pristine GNU paths / generated support):', file=sys.stderr)
        for p in problems[:40]:
            print('  ' + p, file=sys.stderr)
        if len(problems) > 40:
            print(f'  ... and {len(problems) - 40} more', file=sys.stderr)
        return 1
    print('lisp_oracle: ok, every shared gnu-lisp/ path is the pin blob; declared generated support bytes verified')
    return 0


def populations(root, gnu, pin, rev, out):
    pinned = pin_tree(gnu, pin)
    local = tree(root, rev, prefix=LOCAL_LISP)
    # Historical revisions predate the physical move; no runtime alias.
    ours = ({'lisp' + p[len(LOCAL_LISP):]: blob for p, blob in local.items()}
            if local else tree(root, rev))
    t31 = tree(gnu, 'emacs-31.1')
    shared = sorted(set(pinned) & set(ours))
    lines = ['# path\tclass\tour_blob\tpin_blob\tours_equals_emacs-31.1_same_path',
             f'# rev {git("rev-parse", rev, cwd=root).strip()} pin {pin}; class DRIFT = our blob exists in GNU history '
             '(git cat-file -e), EDIT = it does not']
    counts = {'DRIFT': 0, 'EDIT': 0}
    for p in shared:
        if ours[p] == pinned[p]:
            continue
        known = subprocess.run(['git', 'cat-file', '-e', ours[p]], cwd=gnu, capture_output=True).returncode == 0
        cls = 'DRIFT' if known else 'EDIT'
        counts[cls] += 1
        lines.append(f'{p}\t{cls}\t{ours[p]}\t{pinned[p]}\t{"yes" if t31.get(p) == ours[p] else "no"}')
    text = '\n'.join(lines) + '\n'
    if out:
        pathlib.Path(out).write_text(text)
    else:
        sys.stdout.write(text)
    print(f'lisp_oracle: {len(shared)} shared paths at {rev}; DRIFT {counts["DRIFT"]}, EDIT {counts["EDIT"]}', file=sys.stderr)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('command', choices=['check', 'populations'])
    ap.add_argument('--root', default=str(pathlib.Path(__file__).resolve().parents[1]))
    ap.add_argument('--gnu', default=GNU_SOURCE)
    ap.add_argument('--pin', default=GNU_PIN)
    ap.add_argument('--rev', help='populations: the commit whose GNU Lisp is classified')
    ap.add_argument('--out')
    a = ap.parse_args()
    if a.command == 'check':
        return check(a.root, a.gnu, a.pin)
    if not a.rev:
        ap.error('populations needs --rev')
    return populations(a.root, a.gnu, a.pin, a.rev, a.out)


if __name__ == '__main__':
    sys.exit(main())
