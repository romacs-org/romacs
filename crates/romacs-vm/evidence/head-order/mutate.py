#!/usr/bin/env python3
"""Reproducible head-order mutants; logs MUST be outside this worktree.

Each mutant breaks one piece of the compiled call order (arguments, then the
function cell through the site cache: threaded.rs direct/direct_kernel/
direct_generic) or of the loader order kept for a top-level form with an
unbound head (lib.rs), and must be killed at the named corpus row."""
import argparse
import json
import pathlib
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--out', required=True, type=pathlib.Path)
a = p.parse_args()
root = pathlib.Path(__file__).resolve().parents[4]
out = a.out.resolve()
assert not out.is_relative_to(root), 'keep raw logs outside the worktree'
out.mkdir(parents=True, exist_ok=True)
lib = root / 'crates/romacs-vm/src/lib.rs'
threaded = root / 'crates/romacs-vm/src/threaded.rs'
mutants = [
    ('no-loader-order', lib,
     '                        let loader_guard = if target.is_none() && !sealed && b.loader_order\n',
     '                        let loader_guard = if false\n',
     'void-side-effect'),
    ('stale-cache', threaded,
     '        if cache.version != slot.version {\n            if e.vm.macros.contains(&i.b)',
     '        if cache.version == 0 {\n            if e.vm.macros.contains(&i.b)',
     'redefined-kernel'),
    ('unguarded-kernel', threaded,
     '        if cache.version == e.vm.slots.get_unchecked(i.b).version && cache.target == i.value {',
     '        if cache.version != 0 {',
     'redefined-kernel'),
    ('stale-speculation', threaded,
     '            && entry.world_version == e.vm.world_version\n',
     '',
     'redefined-closure'),
    ('lose-original-name', threaded,
     '            return raise(e, condition, &[Value::reference(i.b + 1, 7)]);',
     '            return raise(e, condition, &[target]);',
     'invalid-cell'),
]
results = []
for name, path, old, new, row in mutants:
    original = path.read_text()
    assert original.count(old) == 1, (name, original.count(old))
    changed = original.replace(old, new)
    assert changed != original
    try:
        path.write_text(changed)
        run = subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--test', 'head_order', '--', '--nocapture'], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (out / (name + '.log')).write_text(run.stdout)
        killed = run.returncode != 0 and 'HEAD_ORDER_GNU_MISMATCH' in run.stdout and f'"{row}": GNU' in run.stdout
        results.append(dict(name=name, row=row, status='KILLED-AT-TARGET' if killed else 'FAILED-PROOF', exit=run.returncode))
        print(json.dumps(results[-1]), flush=True)
        assert killed, name
    finally:
        path.write_text(original)
(out / 'mutants.json').write_text(json.dumps(results, indent=2) + '\n')
