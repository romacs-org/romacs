#!/usr/bin/env python3
"""Assertion-kill acceptance mutants in isolated snapshots; run inside slot."""
from pathlib import Path
import json
import os
import shutil
import subprocess

repo = Path(__file__).resolve().parents[3]
root = Path(os.environ['P1_MUTATION_ROOT'])
root.mkdir(parents=True, exist_ok=False)
results = {}
for name, test in [('tail-rescan', 'append_random_and_cursor_work_gates'),
                   ('duplicate-paint', 'duplicate_add_final_identity')]:
    dest = root / name
    shutil.copytree(repo / 'crates/romacs-props', dest,
                    ignore=shutil.ignore_patterns('__pycache__'))
    (dest / 'Cargo.toml').write_text('''[package]
name="romacs-props"
version="0.1.0"
edition="2024"
autobenches=false
[workspace]
[dependencies]
romacs-chars={path="''' + str(repo / 'crates/romacs-chars') + '''"}
hashbrown="=0.17.0"
[dev-dependencies]
proptest="1.6"
''')
    if name == 'tail-rescan':
        path = dest / 'src/tree.rs'
        source = path.read_text()
        start = source.index('        self.count(1);', source.index('fn append_tail('))
        end = source.index('\n    }', start)
        source = source[:start] + '''        let mut tail = None;
        for i in 0..l.n {
            self.count(1);
            tail = Some((l.sets[i], l.lens[i]));
        }
        tail''' + source[end:]
    else:
        path = dest / 'src/lib.rs'
        source = path.read_text()
        guard = '            if set == old {\n                return true;\n            }\n'
        assert guard in source
        source = source.replace(guard, '', 1)
    path.write_text(source)
    log = root / (name + '.log')
    with log.open('w') as out:
        result = subprocess.run(['cargo', 'test', '--manifest-path', str(dest / 'Cargo.toml'),
                                 '--lib', test], stdout=out, stderr=subprocess.STDOUT)
    text = log.read_text()
    if result.returncode == 0 or 'test result: FAILED' not in text:
        raise RuntimeError(f'mutant did not fail assertions: {name}: {log}')
    results[name] = {'exit_code': result.returncode, 'log': str(log),
                     'result': [s for s in text.splitlines() if s.startswith('test result:')]}
    print(name, results[name], flush=True)
(root / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
