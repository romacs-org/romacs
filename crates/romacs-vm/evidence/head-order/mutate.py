#!/usr/bin/env python3
"""Reproducible head-order mutants; logs MUST be outside this worktree."""
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
    ('argument-first', lib,
     '                            b.emit(Op::ResolveHead(function, slot));',
     '                            // Delayed by the argument-first mutant.',
     'void-precedence'),
    ('no-early-invalid', threaded,
     '        if !matches!(target.tag(), 4 | 6) {\n            let condition = uncallable_condition(e, target);\n            return raise(e, condition, &[function]);\n        }',
     '        // Mutant: leave validation until after arguments.',
     'void-precedence'),
    ('reread-cell', threaded,
     '        let target = *r.add(snapshot);',
     '        let target = e.vm.slots[(*pc).b].function;',
     'snapshot-closure'),
    ('lose-original-name', threaded,
     '            return raise(e, condition, &[function]);',
     '            return raise(e, condition, &[target]);',
     'alias-invalid'),
]
results = []
for name, path, old, new, row in mutants:
    original = path.read_text()
    # Scope the name mutant to ResolveHead, not unrelated invoke helpers.
    if name == 'lose-original-name':
        start = original.index('unsafe fn resolve_head(')
        end = original.index('unsafe fn prepared(', start)
        section = original[start:end]
        assert section.count(old) == 1
        changed = original[:start] + section.replace(old, new) + original[end:]
    else:
        assert original.count(old) == 1, (name, original.count(old))
        changed = original.replace(old, new)
    if name == 'argument-first':
        anchor = '                        let r = b.reg();\n                        if let Some(f) = target {'
        assert changed.count(anchor) == 1
        changed = changed.replace(anchor, '                        if let Some((function, slot)) = prepared { b.emit(Op::ResolveHead(function, slot)); }\n' + anchor)
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
