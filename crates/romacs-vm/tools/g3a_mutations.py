#!/usr/bin/env python3
"""Bounded one-mutant-per-test evidence; never changes Git or parent files."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
from pathlib import Path
import json
import os
import subprocess

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / 'crates/romacs-vm/src/window.rs'
EVIDENCE = ROOT / 'crates/romacs-vm/evidence/g3a'
CARGO = 'cargo'
ENV = dict(os.environ, CARGO_TARGET_DIR=str(_PortablePath(_portable_tempfile.gettempdir()) / 'romacs/g3a-vm2-target'))
MUTANTS = [
    ('lazy_heap_tree_and_minibuffer', 'w.height = line;', 'w.height = line * 2;'),
    ('split_preparation_and_delete_rollback',
     'if !self.prepared_check(root, horizontal, v)? || self.window(root)?.new_pixel != root_size {',
     'if false {'),
    ('selection_and_independent_points', 'self.set_buffer_point_for_window(v, new_point)?;', 'let _ = new_point;'),
    ('parameters_eq_dead_and_gc',
     '        for (k, v) in &mut self.parameters {\n            f(k);\n            f(v);\n        }',
     '        // MUTANT: parameter graph is untraced.'),
    ('margins_start_and_stale_end',
     '(u64::from(l) + u64::from(r) + 2) * u64::from(col) > u64::from(w.width)',
     'false'),
    ('resize_axis_and_nested_tree', 'self.apply_prepared(root, horizontal, 0)?;', '// MUTANT: discard prepared geometry.'),
    ('collections_and_three_size_retention', 'self.retire_window(v)?;',
     'self.put_window_root(&format!("#:g3a-leak-{}", self.window(v)?.id), v);\n        self.retire_window(v)?;'),
    ('geometry_and_layout_end_journal', 'w.end_distance = distance;', 'w.end_distance = 0;'),
    ('owned_frame_scene_from_window_tree', 'ids.push(self.window(mini)?.id);', '// MUTANT: lose the minibuffer leaf.'),
]

def main():
    original = SOURCE.read_text()
    outcomes = []
    try:
        for test, old, new in MUTANTS:
            assert original.count(old) == 1, (test, 'mutation target must be unique')
            SOURCE.write_text(original.replace(old, new))
            name = 'g3a_' + test
            command = [CARGO, 'test', '-p', 'romacs-vm', '--lib', name, '--', '--nocapture']
            result = subprocess.run(command, cwd=ROOT, env=ENV, capture_output=True, text=True)
            output = result.stdout + result.stderr
            (EVIDENCE / ('mutation-' + test + '.log')).write_text(output)
            valid = result.returncode == 101 and '1 failed' in output and 'could not compile' not in output
            outcomes.append(dict(test=name, mutation=old+' -> '+new, exit=result.returncode, killed=valid))
            print(name, 'KILLED' if valid else 'INVALID/SURVIVED', flush=True)
            assert valid, (name, result.returncode, output)
    finally:
        SOURCE.write_text(original)
        (EVIDENCE / 'mutations.json').write_text(json.dumps(outcomes, indent=2)+'\n')
    assert len(outcomes) == len(MUTANTS)
    print(f'{len(outcomes)}/{len(MUTANTS)} mutants killed; source restored')

if __name__ == '__main__':
    main()
