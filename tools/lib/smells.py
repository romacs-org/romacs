#!/usr/bin/env python3
"""Smells census, and a ratchet on it: no change adds a smell.

One report of the smells the cleanup removes, and a ratchet on each, so a
removed smell cannot come back. The scope is the unsafe census's: every .rs
file in a Cargo package, credited to its nearest package
(tools/lib/unsafe_census.py, which also keeps the unsafe floors in
ledger/unsafe-floors.json; `report` shows its kinds and SAFETY coverage here too).

Per crate:
  allows      lint names in `allow(...)` or `expect(...)` attributes that are
              dead_code, the unused group, or an unused_* lint (one per name)
  todos       TODO, FIXME, XXX and HACK in comments
  long_files  files over `long_file_lines` lines

Per lint, across the workspace (`lints`): every rustc and Clippy warning of
`cargo clippy --workspace --all-targets -- -W clippy::pedantic`, each
distinct diagnostic once, however many targets compile its code. Lints are
capped at warn, so a deny-by-default lint is counted rather than stopping the
count.

Layout (`layout`):
  root                    the tree's top-level entries
  members_outside_crates  workspace members not under crates/
  legacy_primitives       primitives lib.rs registers through its own enum,
                          the second route beside `defsubr!`
                          (tools/lib/primitive_index.py reads both from source)
  primitives_outside_prims  `defsubr! { name = ... }` declarations outside
                          PRIMS (the harness's own tests and probes)
  generated               tracked files a tool writes: derived_artifacts's
                          ARTIFACTS, and every file outside GNU's own trees
                          whose first lines say it is generated. The list
                          only shrinks; a new generated file fails until
                          listed deliberately.

check holds a tree to its merge-base with BASE (--base, else $ROMACS_BASE,
else beta): a count fails when it is above what the same count measures on
the merge-base's tree. A change answers for what it adds and for nothing else:
not for floors lowered since it forked, and not for a rise the floors' slack
would have hidden. A drop needs no record. The one way to add a smell is to
raise its floor in ledger/smell-floors.json by as much, a hand edit made deliberately
in its own commit with its reason; a new layout entry, a generated file among
them, is added to its list the same way. A branch does not lower a floor or drop a listed entry either: the
merge may not have its drop, so `lower` records drops on the trunk.

The merge-base is measured once and kept (`base_counts`), under
$ROMACS_SMELLS_CACHE, else $XDG_CACHE_HOME/romacs/smells: one file per tree,
counting code and Clippy version, and one kept checkout to measure in. A clean
tree's own counts are kept too, so the merge-base of a later branch is usually
there already.

check --absolute holds the tree to the floors file itself, for a checkpoint on
the trunk: a count above its floor fails, a listed entry that is missing or
extra fails, and a slack crate-smell or layout floor fails until `lower`
records it. A lint's floor is a ceiling there: a drop passes. `lower` only
lowers floors and drops listed entries that are gone.

The floors file merges by its meaning, not its text (`merged`): each side's
raise is added, a lowered floor stays lowered, and a list has what both keep
and what either added. `tools/derived setup` installs that as git's merge
driver for the file, so a merge needs no hand; where the driver is not
installed and git reports a conflict, `resolve` writes the same merge.

With ROMACS_CLIPPY_TARGET_DIR set, clippy builds each checkout in its own
tree there and writes the checkout's path into the tree's `checkout` file;
`prune` removes the trees whose checkout no longer exists.

usage: smells.py check [--base REV | --absolute] [--root ROOT] [--clippy-json FILE]
       smells.py report|lower|resolve [--root ROOT] [--clippy-json FILE]
       smells.py merge BASE OURS THEIRS   (the merge driver: the result replaces OURS)
       smells.py prune   (removes the clippy trees under $ROMACS_CLIPPY_TARGET_DIR whose checkout is gone)
       smells.py report --floors   (counts in the floors file's format)
"""
import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import derived_artifacts  # noqa: E402
import primitive_index  # noqa: E402
import unsafe_census  # noqa: E402

FLOORS = Path('ledger/smell-floors.json')
# Where a merge-base from before the records moved into ledger/ keeps its
# floors; it goes once no open branch's merge-base predates the move.
FLOORS_BEFORE = 'SMELL-FLOORS.json'
CRATE_SMELLS = ('allows', 'todos', 'long_files')
COUNTED = ('legacy_primitives', 'primitives_outside_prims')
LISTED = ('root', 'members_outside_crates', 'generated')
GNU_TREES = ('lisp/gnu/', 'test/gnu/test/', 'etc/')
GENERATED = re.compile(r'(?i)\b(?:generated by|do not edit|auto-?generated)\b|@generated\b')
COUNTING = ('smells.py', 'unsafe_census.py', 'primitive_index.py', 'derived_artifacts.py')
PRIMS = 'crates/romacs-vm/src/prims/'
ALLOW = re.compile(r'\b(?:allow|expect)\s*\(([^)]*)\)')
ALLOWED_LINT = re.compile(r'(?<![:\w])(?:dead_code|unused(?:_\w+)?)\b')
TODO = re.compile(r'\b(?:TODO|FIXME|XXX|HACK)\b')
DEFSUBR = re.compile(r'\bdefsubr\s*!\s*\{\s*name\s*=')
CLIPPY = ('cargo', 'clippy', '--locked', '--offline', '--workspace', '--all-targets',
          '--message-format=json', '--', '-W', 'clippy::pedantic', '--cap-lints', 'warn')


def file_smells(src, long_file_lines):
    """SRC's smells, and how many primitives it declares."""
    code, comments, _ = unsafe_census.lex(src)
    smells = Counter(allows=sum(len(ALLOWED_LINT.findall(m[1])) for m in ALLOW.finditer(code)),
                     todos=sum(len(TODO.findall(text)) for _, _, text in comments),
                     long_files=int(len(src.splitlines()) > long_file_lines))
    return smells, len(DEFSUBR.findall(code))


def crate_census(root, long_file_lines):
    """({crate: Counter of CRATE_SMELLS}, [(lines, path)] of long files,
    {path outside PRIMS: primitives it declares})."""
    counts, long, strays = {}, [], {}
    for crate, name, src in unsafe_census.rust_sources(root):
        smells, declares = file_smells(src, long_file_lines)
        counts.setdefault(crate, Counter()).update(smells)
        if smells['long_files']:
            long.append((len(src.splitlines()), name))
        if declares and not name.startswith(PRIMS):
            strays[name] = declares
    return counts, sorted(long, reverse=True), strays


def place(root, span):
    """SPAN's (file, line, column), the file relative to ROOT when inside it, so the
    aliases rustc prints for one file (`tests/../src/x.rs`, absolute and relative
    build scripts) are one place."""
    path = os.path.normpath(os.path.join(root, span['file_name']))
    inside = os.path.relpath(path, root)
    return (path if inside.startswith('..') else inside, span['line_start'], span['column_start'])


def call_site(span):
    """The outermost macro call SPAN expands from, or SPAN itself."""
    while span.get('expansion'):
        span = span['expansion']['span']
    return span


def lint_census(root, lines):
    """{crate: Counter of lint codes} from cargo's JSON message LINES. A warning is
    its lint, message and primary spans, each with the call site it was expanded
    from, so one warning compiled by two targets counts once and a warning inside
    a shared macro counts once per call. LINES must end in a successful build."""
    root = os.path.realpath(root)
    dirs = {Path(root, d).resolve(): n for d, n in unsafe_census.packages(root).items()}
    seen, counts, finished = set(), {}, False
    for line in lines:
        if not line.startswith('{'):
            continue
        message = json.loads(line)
        diagnostic = message.get('message') or {}
        finished = message.get('reason') == 'build-finished' and message.get('success') is True
        if message.get('reason') != 'compiler-message' or diagnostic.get('level') != 'warning' \
                or not diagnostic.get('code'):
            continue
        spans = tuple(sorted((place(root, s), place(root, call_site(s)))
                             for s in diagnostic['spans'] if s['is_primary']))
        key = (diagnostic['code']['code'], diagnostic['message'], spans)
        if key in seen:
            continue
        seen.add(key)
        crate = dirs.get(Path(message['manifest_path']).parent.resolve(), '?')
        counts.setdefault(crate, Counter())[key[0]] += 1
    if not finished:
        raise ValueError('the clippy messages do not end in a successful build; '
                         'a failed or cut-off build counts nothing')
    return counts


# The files in a checkout's clippy tree that name the checkout and the
# device of the directory holding it: the tree lives as long as that path
# does on that device.
OWNER = 'checkout'
DEVICE = 'device'


def clippy_tree(base, root):
    """ROOT's clippy target under BASE: one per checkout, named by its path."""
    return Path(base) / hashlib.sha1(str(Path(root).resolve()).encode()).hexdigest()[:12]


def owner(tree):
    """(the checkout TREE records, exactly as written, and the device of the
    directory that held it), or None."""
    try:
        return (tree / OWNER).read_bytes().decode(), int((tree / DEVICE).read_text())
    except (OSError, ValueError, UnicodeDecodeError):
        return None


def record_owner(tree, root):
    """Write ROOT's path and its directory's device into TREE."""
    path = Path(root).resolve()
    (tree / OWNER).write_bytes(str(path).encode())
    (tree / DEVICE).write_text(str(os.stat(path.parent).st_dev))


def orphan(tree):
    """Whether TREE, a clippy tree, outlived its checkout: it is a real
    directory named for the checkout it records, that checkout is gone, and
    the directory that held it is still there on the device it was on (a
    removed worktree; an unmounted disk leaves its mount point on another
    device, or no directory at all).  A tree without its owner files (made
    before trees named their checkout) is left to the idle sweep."""
    recorded = owner(tree)
    if recorded is None or tree.is_symlink() or not tree.is_dir():
        return False
    path, device = recorded
    parent = Path(path).parent
    return (tree == clippy_tree(tree.parent, path) and not Path(path).exists()
            and parent.is_dir() and os.stat(parent).st_dev == device)


def tree_lock(base, tree_name):
    """An open lock file for the clippy tree TREE_NAME, beside it under BASE
    so removing the tree leaves the lock."""
    return open(Path(base) / f'{tree_name}.lock', 'a')


def prune(base):
    """Remove the clippy trees under BASE that outlived their checkout,
    each under its lock, rechecked once held; a tree in use is skipped."""
    removed = []
    for tree in sorted(Path(base).iterdir()):
        if not orphan(tree):
            continue
        with tree_lock(base, tree.name) as held:
            try:
                fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                continue
            if orphan(tree):
                shutil.rmtree(tree)
                removed.append(tree)
    return removed


def run_clippy(root):
    """Cargo's JSON message lines for the workspace's warnings.

    ROMACS_CLIPPY_TARGET_DIR, when set, is where clippy builds instead of a
    full dev-profile tree inside the worktree's own target: one subdirectory
    per checkout, named by its path. Cargo names a path crate's artifacts by
    its path inside the workspace, so two checkouts sharing one directory
    would compile against each other's sources."""
    env = dict(os.environ)
    if env.get('ROMACS_CLIPPY_TARGET_DIR'):
        tree = clippy_tree(env['ROMACS_CLIPPY_TARGET_DIR'], root)
        tree.parent.mkdir(parents=True, exist_ok=True)
        with tree_lock(tree.parent, tree.name) as held:
            fcntl.flock(held, fcntl.LOCK_EX)    # `prune` removes no tree in use
            tree.mkdir(exist_ok=True)
            record_owner(tree, root)
            env['CARGO_TARGET_DIR'] = str(tree)
            done = subprocess.run(CLIPPY, cwd=root, stdout=subprocess.PIPE, text=True, env=env)
    else:
        done = subprocess.run(CLIPPY, cwd=root, stdout=subprocess.PIPE, text=True, env=env)
    if done.returncode:
        raise ValueError(f'`{" ".join(CLIPPY)}` failed with {done.returncode}; a failed build counts nothing')
    return done.stdout.splitlines()


def layout(root, strays):
    """{'root': [top-level entries], 'members_outside_crates': [...],
    'generated': [...], 'legacy_primitives': N, 'primitives_outside_prims': N}."""
    listed = subprocess.run(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
                            cwd=root, check=True, stdout=subprocess.PIPE).stdout.decode()
    names = [name for name in listed.split('\0') if name and (Path(root) / name).exists()]
    top = sorted({name.split('/', 1)[0] for name in names})
    members = tomllib.loads((Path(root) / 'Cargo.toml').read_text())['workspace']['members']
    primitives = primitive_index.vm2_primitives(Path(root).resolve() / 'crates/romacs-vm/src').values()
    return {'root': top,
            'members_outside_crates': sorted(m for m in members if not m.startswith('crates/')),
            'legacy_primitives': sum('variant' in p for p in primitives),
            'primitives_outside_prims': sum(strays.values()),
            'generated': sorted(set(derived_artifacts.ARTIFACTS) | {n for n in names if says_generated(root, n)})}


def says_generated(root, name):
    """Whether NAME, outside GNU's own trees, says in its first lines that a
    tool writes it."""
    if name.startswith(GNU_TREES) or (Path(root) / name).is_symlink():
        return False
    with open(Path(root) / name, 'rb') as f:
        head = f.read(2048).decode('utf-8', 'replace').split('\n')[:5]
    return any(GENERATED.search(line) for line in head)


def measure(root, floors, clippy_json=None):
    """Everything check compares against FLOORS."""
    crates, long, strays = crate_census(root, floors['long_file_lines'])
    lines = Path(clippy_json).read_text().splitlines() if clippy_json else run_clippy(root)
    by_crate = lint_census(root, lines)
    return {'crates': crates, 'long': long, 'strays': strays, 'lints_by_crate': by_crate,
            'lints': sum(by_crate.values(), Counter()), 'layout': layout(root, strays)}


def bound(what, have, limit):
    """Why HAVE breaks the floor LIMIT for WHAT, or None."""
    if have > limit:
        return f'{what}: {have}, above its floor {limit}'
    if have < limit:
        return f'{what}: {have}, below its floor {limit}; record the drop with tools/lib/smells.py lower'
    return None


def listed(what, have, floor):
    found = [f'{what}: {entry} is new; adding one is a deliberate edit of {FLOORS}'
             for entry in sorted(set(have) - set(floor))]
    return found + [f'{what}: {entry} is gone; record it with tools/lib/smells.py lower'
                    for entry in sorted(set(floor) - set(have))]


def problems(floors, now):
    """Why NOW breaks FLOORS, one line each."""
    found = []
    for crate in sorted(set(floors['crates']) | set(now['crates'])):
        floor, have = floors['crates'].get(crate, {}), now['crates'].get(crate, Counter())
        found += [bound(f'{crate} {smell}', have[smell], floor.get(smell, 0)) for smell in CRATE_SMELLS]
    for lint in sorted(now['lints']):
        if now['lints'][lint] > floors['lints'].get(lint, 0):
            found.append(f'lint {lint}: {now["lints"][lint]}, above its ceiling {floors["lints"].get(lint, 0)}')
    for key in LISTED:
        found += listed(f'layout {key}', now['layout'][key], floors['layout'][key])
    found += [bound(f'layout {key}', now['layout'][key], floors['layout'][key]) for key in COUNTED]
    return [p for p in found if p]


def tally(counts):
    """COUNTS, in the floors file's shape, as one {what: count} of what a tree holds."""
    flat = {f'{crate} {smell}': n for crate, smells in counts['crates'].items() for smell, n in smells.items()}
    flat.update({f'lint {lint}': n for lint, n in counts['lints'].items()})
    flat.update({f'layout {key}': counts['layout'].get(key, 0) for key in COUNTED})
    return flat


def rises(floors, base_floors, base, now, ahead=True):
    """Why NOW adds a smell to BASE, one line each. BASE and NOW are the counts of
    the merge-base and of the tree, and BASE_FLOORS and FLOORS their floors
    files: a floor raised between them allows a rise of as much.

    A tree AHEAD of its merge-base, a branch, may not lower a floor or drop a
    listed entry. Its drop was measured on its own tree, and the merge with the
    trunk may not have it (a file one side shortened and the other lengthened);
    a floor lowered there would leave the trunk above its floor with no change
    to blame. Lowering is recorded on the trunk, at a checkpoint."""
    have, had, raised, was = tally(now), tally(base), tally(floors), tally(base_floors)
    found = []
    if ahead:
        found += [f'{what}: its floor is lowered from {was[what]} to {raised.get(what, 0)}; a drop needs no record on '
                  f'a branch, and `lower` records it on the trunk at a checkpoint: restore the floor'
                  for what in sorted(was) if raised.get(what, 0) < was[what]]
    for what in sorted(have):
        allowed = max(0, raised.get(what, 0) - was.get(what, 0))
        if have[what] > had.get(what, 0) + allowed:
            over = have[what] - had.get(what, 0) - allowed
            found.append(f'{what}: {have[what]}, above the {had.get(what, 0)} of its merge-base'
                         + (f' and the {allowed} its floor was raised by' if allowed else '')
                         + f"; remove {over}, or set its floor in {FLOORS} {allowed + over} above the merge-base's")
    def entries(counts, key):                   # a floors file from before a list was one has a count there
        value = counts['layout'].get(key)
        return set(value) if isinstance(value, list) else set()
    for key in LISTED:
        new = entries(now, key) - entries(base, key) - (entries(floors, key) - entries(base_floors, key))
        found += [f'layout {key}: {entry} is new; adding one is a deliberate edit of {FLOORS}' for entry in sorted(new)]
        if ahead:
            found += [f'layout {key}: {entry} is dropped from the list; `lower` drops it on the trunk at a '
                      f'checkpoint: restore it' for entry in sorted(entries(base_floors, key) - entries(floors, key))]
    return found


def git(root, *args):
    """A git command's output in ROOT, or None when it fails."""
    done = subprocess.run(['git', *args], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return None if done.returncode else done.stdout.strip()


def cache_dir():
    home = os.environ.get('XDG_CACHE_HOME') or Path.home() / '.cache'
    return Path(os.environ.get('ROMACS_SMELLS_CACHE') or Path(home) / 'romacs/smells')


def cargo_settings():
    """What Cargo and rustc take from outside the tree that can change which
    warnings a build has: their environment, and the configuration in Cargo's
    home. Where a build goes and how wide it runs change no warning."""
    apart = ('CARGO_HOME', 'CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET_DIR', 'CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL',
             'CARGO_TERM_COLOR')
    env = sorted((k, v) for k, v in os.environ.items()
                 if k.startswith(('CARGO_', 'RUSTFLAGS', 'RUSTC', 'CLIPPY_')) and k not in apart)
    home = Path(os.environ.get('CARGO_HOME') or Path.home() / '.cargo')
    return env, [(name, (home / name).read_text()) for name in ('config', 'config.toml') if (home / name).is_file()]


def counting(long_file_lines):
    """What a tree's counts depend on beside the tree: this code, Clippy, the
    limit, and Cargo's settings from outside the tree."""
    code = b''.join((Path(__file__).resolve().parent / name).read_bytes() for name in COUNTING)
    clippy = subprocess.run(['cargo', 'clippy', '-V'], stdout=subprocess.PIPE, check=True).stdout
    return hashlib.sha256(code + clippy + repr((long_file_lines, cargo_settings())).encode()).hexdigest()


def kept(tree, how):
    """Where TREE's counts are kept, counted HOW."""
    return cache_dir() / f'{hashlib.sha256(f"{tree} {how}".encode()).hexdigest()}.json'


def keep(path, counts):
    path.parent.mkdir(parents=True, exist_ok=True)
    scratch = path.with_suffix(f'.{os.getpid()}')
    scratch.write_text(json.dumps(counts))
    os.replace(scratch, path)


def clean_tree(root):
    """The tree ROOT's files are, when git has all of them, else None: the
    index's tree, with nothing unstaged and nothing untracked."""
    if git(root, 'diff', '--quiet') is None or git(root, 'ls-files', '--others', '--exclude-standard') != '':
        return None
    return git(root, 'write-tree')


def base_counts(root, commit, floors, how):
    """COMMIT's counts in the floors file's shape, measured once: in the one
    kept checkout, one measurer at a time."""
    path = kept(git(root, 'rev-parse', f'{commit}^{{tree}}'), how)
    if path.exists():
        return json.loads(path.read_text())
    cache = cache_dir()
    cache.mkdir(parents=True, exist_ok=True)
    with open(cache / 'lock', 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not path.exists():
            print(f'smells: measuring the merge-base {commit[:12]}, once', file=sys.stderr)
            checkout = cache / 'base'
            moved = git(checkout, 'checkout', '--detach', '--force', commit) if (checkout / '.git').exists() \
                else git(root, 'worktree', 'add', '--detach', '--force', str(checkout), commit)
            if moved is None or git(checkout, 'clean', '-fdq') is None:
                raise ValueError(f'cannot check the merge-base {commit[:12]} out into {checkout}')
            keep(path, as_floors(None, floors['long_file_lines'], measure(checkout, floors)))
    return json.loads(path.read_text())


def trunk_ref(root, rev):
    """REV, or origin/REV in a clone that has only the remote-tracking trunk."""
    remote = f'origin/{rev}'
    if git(root, 'rev-parse', '--verify', '--quiet', rev) is None and git(root, 'rev-parse', '--verify', '--quiet', remote):
        return remote
    return rev


def merge_base(root, rev):
    """The commit of the trunk REV that HEAD is held to. Where a branch and the
    trunk have merged each other there are several merge-bases; the one on the
    trunk's own line (its first parents) is the trunk the branch last took in,
    and what the branch has beyond it is the branch's own."""
    rev = trunk_ref(root, rev)
    bases = (git(root, 'merge-base', '--all', 'HEAD', rev) or '').split()
    if len(bases) > 1:
        trunk = set(git(root, 'rev-list', '--first-parent', rev).split())
        bases = [b for b in bases if b in trunk]
    if len(bases) != 1:
        raise ValueError(f'no single merge-base with {rev}; name the trunk with --base or ROMACS_BASE'
                         + (', or merge it into the branch' if bases else ''))
    return bases[0]


def base_side(root, rev, floors, now=None):
    """(the merge-base's floors, its counts) for `rises`. NOW, the tree's own
    counts as this code measured them, is kept first when git has the whole
    tree, so a tree that is its own merge-base, or a later branch's, is not
    measured twice."""
    commit = merge_base(root, rev)
    how = counting(floors['long_file_lines'])
    tree = now and clean_tree(root)
    if tree:
        keep(kept(tree, how), now)
    was = git(root, 'show', f'{commit}:{FLOORS}') or git(root, 'show', f'{commit}:{FLOORS_BEFORE}')
    empty = {'crates': {}, 'lints': {}, 'layout': {}}
    return json.loads(was) if was else empty, base_counts(root, commit, floors, how), commit


def held_to_base(root, rev, floors, tree, keep_own=True):
    """`rises` for the tree at ROOT, whose counts are TREE, against its merge-base with REV."""
    base_floors, base, commit = base_side(root, rev, floors, tree if keep_own else None)
    return rises(floors, base_floors, base, tree, ahead=git(root, 'rev-parse', 'HEAD') != commit)


def lowered(floors, now):
    """FLOORS lowered to NOW, never raised; zeros and gone entries dropped."""
    crates = {}
    for crate, floor in floors['crates'].items():
        new = {s: min(floor.get(s, 0), now['crates'].get(crate, Counter())[s]) for s in CRATE_SMELLS}
        if any(new.values()):
            crates[crate] = new
    lints = {lint: min(n, now['lints'][lint]) for lint, n in floors['lints'].items() if now['lints'][lint]}
    old = floors['layout']
    return {**floors, 'crates': dict(sorted(crates.items())), 'lints': dict(sorted(lints.items())),
            'layout': {**{key: [e for e in old[key] if e in now['layout'][key]] for key in LISTED},
                       **{key: min(old[key], now['layout'][key]) for key in COUNTED}}}


def merged(base, ours, theirs):
    """The floors file a merge of OURS and THEIRS has, both grown from BASE.

    A count is each side's raise added to the lowest of the three: a raise is
    an allowance its side asked for, so two raises both count, and a floor one
    side lowered stays lowered. Only the trunk lowers (`rises` holds a branch
    to that), so the lowered floor is one the trunk's own tree met, and the
    merge can exceed it only by what the branch's raise allows. A list has
    what both keep and what either added. Where the file's form changed on one
    side (a count that became a list), the list is the newer form. The limit
    and the rule's text changed differently on the two sides is a conflict."""
    def count(b, o, t):
        return min(b, o, t) + max(0, o - b) + max(0, t - b)

    def entries(b, o, t):
        return sorted((o & t) | (o - b) | (t - b))

    def one(b, o, t):
        sides = (b, o, t)
        if any(isinstance(x, dict) for x in sides):
            b, o, t = (x if isinstance(x, dict) else {} for x in sides)
            both = {k: one(b.get(k), o.get(k), t.get(k)) for k in sorted(set(o) | set(t))}
            return {k: v for k, v in both.items() if v != 0}
        if any(isinstance(x, list) for x in sides):
            return entries(*(set(x) if isinstance(x, list) else set() for x in sides))
        return count(*(x or 0 for x in sides))
    kept = {}
    for key in ('rule', 'long_file_lines'):
        if len({json.dumps(side[key]) for side in (base, ours, theirs)}) == 3:
            raise ValueError(f'{key} changed differently on the two sides')
        kept[key] = ours[key] if ours[key] != base[key] else theirs[key]
    crates = one(base['crates'], ours['crates'], theirs['crates'])
    return {**ours, **kept, 'lints': one(base['lints'], ours['lints'], theirs['lints']),
            'crates': {c: {s: v.get(s, 0) for s in CRATE_SMELLS} for c, v in crates.items() if v},
            'layout': {**{k: [] for k in LISTED}, **{k: 0 for k in COUNTED},
                       **one(base['layout'], ours['layout'], theirs['layout'])}}


def merge_files(base, ours, theirs):
    """Git's merge driver for the floors file (tools/derived setup installs
    it): write the three-way merge over OURS. 1, leaving OURS, when a side
    is not a floors file (no common ancestor has one, or it does not parse):
    git then reports the conflict as it would without a driver. Git asks no
    driver when the two sides are the same file, so the same raise made on
    both sides, with nothing else differing, still merges to one raise."""
    try:
        text = dump(merged(*(json.loads(Path(p).read_text()) for p in (base, ours, theirs))))
    except (OSError, ValueError, KeyError, TypeError):
        return 1
    Path(ours).write_text(text)
    return 0


def as_floors(rule, long_file_lines, now):
    """NOW in the floors file's shape: what `report --floors` prints."""
    crates = {c: {s: t[s] for s in CRATE_SMELLS} for c, t in sorted(now['crates'].items())
              if any(t[s] for s in CRATE_SMELLS)}
    return {'rule': rule, 'long_file_lines': long_file_lines, 'crates': crates,
            'lints': dict(sorted(now['lints'].items())), 'layout': now['layout']}


def dump(floors):
    """FLOORS as ledger/smell-floors.json text, one line per crate, lint and entry,
    so lanes that lower different ones merge cleanly."""
    def lines(items):
        return ',\n'.join(f'    {item}' for item in items)
    lay = floors['layout']
    return (f'{{\n  "rule": {json.dumps(floors["rule"])},\n'
            f'  "long_file_lines": {floors["long_file_lines"]},\n'
            f'  "crates": {{\n{lines(f"{json.dumps(c)}: {json.dumps(v)}" for c, v in floors["crates"].items())}\n  }},\n'
            f'  "lints": {{\n{lines(f"{json.dumps(k)}: {v}" for k, v in floors["lints"].items())}\n  }},\n'
            f'  "layout": {{\n    "root": [\n{lines("  " + json.dumps(e) for e in lay["root"])}\n    ],\n'
            f'    "members_outside_crates": {json.dumps(lay["members_outside_crates"])},\n'
            f'    "generated": [\n{lines("  " + json.dumps(e) for e in lay["generated"])}\n    ],\n'
            + ''.join(f'    "{key}": {lay[key]},\n' for key in COUNTED).rstrip(',\n') + '\n  }\n}\n')


def report(root, floors, now, top=12):
    unsafe = unsafe_census.report(unsafe_census.census(root))
    crates = [f'{"crate":32} {"allows":>7} {"todos":>6} {"long":>5}']
    total = Counter()
    for crate, tally in sorted(now['crates'].items(), key=lambda item: -sum(item[1].values())):
        if any(tally[s] for s in CRATE_SMELLS):
            total.update(tally)
            crates.append(f'{crate:32} {tally["allows"]:>7} {tally["todos"]:>6} {tally["long_files"]:>5}')
    crates.append(f'{"total":32} {total["allows"]:>7} {total["todos"]:>6} {total["long_files"]:>5}')
    long = [f'{lines:>7} {path}' for lines, path in now['long'][:top]]
    lints = [f'{now["lints"][lint]:>7} {floors["lints"].get(lint, 0):>7} {lint}'
             for lint in sorted(set(floors['lints']) | set(now['lints']),
                                key=lambda lint: (-now['lints'][lint], lint))]
    slack = sum(n - now['lints'][lint] for lint, n in floors['lints'].items() if n > now['lints'][lint])
    per_crate = [f'{sum(c.values()):>7} {crate}' for crate, c in
                 sorted(now['lints_by_crate'].items(), key=lambda item: -sum(item[1].values()))]
    lay = now['layout']
    return '\n'.join(['unsafe (tools/lib/unsafe_census.py):', unsafe, '', 'smells:', *crates, '',
                      f'longest files ({len(now["long"])} over the limit):', *long, '',
                      f'warnings by lint ({sum(now["lints"].values())}; count, ceiling; '
                      f'{slack} below the ceilings, for `lower`):', *lints, '',
                      'warnings by crate:', *per_crate, '',
                      f'layout: {len(lay["root"])} top-level entries, {len(lay["members_outside_crates"])} '
                      f'members outside crates/, {lay["legacy_primitives"]} primitives registered in lib.rs, '
                      f'{lay["primitives_outside_prims"]} declared outside {PRIMS}, '
                      f'{len(lay["generated"])} generated files tracked',
                      *(f'{n:>7} {path}' for path, n in sorted(now['strays'].items()))])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['check', 'report', 'lower', 'resolve', 'merge', 'prune'])
    parser.add_argument('files', nargs='*', help='merge: BASE OURS THEIRS')
    parser.add_argument('--root', default='.')
    parser.add_argument('--clippy-json', help="cargo clippy's JSON messages, instead of running it")
    parser.add_argument('--floors', action='store_true', help='report: print the counts as a floors file')
    parser.add_argument('--base', default=os.environ.get('ROMACS_BASE') or 'beta',
                        help='check: the trunk whose merge-base the tree is held to')
    parser.add_argument('--absolute', action='store_true', help='check: hold the tree to the floors file instead')
    args = parser.parse_args()
    path = Path(args.root) / FLOORS
    if args.command == 'prune':
        base = os.environ.get('ROMACS_CLIPPY_TARGET_DIR')
        for tree in prune(base) if base and Path(base).is_dir() else []:
            print(f'smells: removed {tree}, whose checkout is gone')
        return 0
    if args.command == 'merge':
        if len(args.files) != 3:
            parser.error('merge takes BASE OURS THEIRS')
        return merge_files(*args.files)
    if args.command == 'resolve':
        sides = [subprocess.run(['git', '-C', args.root, 'show', f':{stage}:{FLOORS}'], capture_output=True,
                                text=True) for stage in (1, 2, 3)]
        if any(side.returncode for side in sides):
            print(f'smells: {FLOORS} is not in a three-way conflict', file=sys.stderr)
            return 2
        try:
            path.write_text(dump(merged(*(json.loads(side.stdout) for side in sides))))
        except ValueError as exc:
            print(f'smells: {exc}; settle it by hand', file=sys.stderr)
            return 1
        print(f'smells: {FLOORS} is the merge of both sides; git add it')
        return 0
    floors = json.loads(path.read_text())
    try:
        now = measure(args.root, floors, args.clippy_json)
    except ValueError as exc:
        print(f'smells: {exc}', file=sys.stderr)
        return 1
    if args.command == 'report':
        sys.stdout.write(dump(as_floors(floors['rule'], floors['long_file_lines'], now)) if args.floors
                         else report(args.root, floors, now) + '\n')
        return 0
    if args.command == 'lower':
        path.write_text(dump(lowered(floors, now)))
        return 0
    try:
        tree = as_floors(None, floors['long_file_lines'], now)
        found = problems(floors, now) if args.absolute else \
            held_to_base(args.root, args.base, floors, tree, keep_own=not args.clippy_json)
    except (ValueError, subprocess.CalledProcessError) as exc:
        print(f'smells: {exc}', file=sys.stderr)
        return 1
    for problem in found:
        print(f'smells: {problem}', file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
