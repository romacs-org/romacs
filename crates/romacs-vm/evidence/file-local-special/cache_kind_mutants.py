#!/usr/bin/env python3
"""Cache-kind mutants for tests/file_local_special.rs (Sol review finding 1).

Each mutant edits one source file, asserts the edit applied (content changed),
runs the test binary, records whether any test FAILED at an assertion, and
restores the file byte-for-byte (sha256 checked)."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib, os, subprocess, sys
ROOT = str(_PortablePath(_portable_os.environ['ROMACS_MUTATION_ROOT']) / 'crates/romacs-vm')
MUTANTS = [
    ('portable-global', 'src/source_cache.rs',
     'SpecialKind::FileLocal => {\n                            portable.file_specials.insert(name.clone());\n                        }',
     'SpecialKind::FileLocal => portable.declare_special(name, Value::NIL),'),
    ('kind-collapse', 'src/lib.rs',
     '        } else if self.file_specials.contains(name) {\n            SpecialKind::FileLocal',
     '        } else if self.file_specials.contains(name) {\n            SpecialKind::Global'),
    ('portable-no-seed', 'src/source_cache.rs',
     '                portable.seed_file_specials = true;\n', ''),
]
log = []
for name, rel, old, new in MUTANTS:
    path = os.path.join(ROOT, rel)
    original = open(path, 'rb').read()
    digest = hashlib.sha256(original).hexdigest()
    text = original.decode()
    assert text.count(old) == 1, f'{name}: pattern not unique/absent'
    mutated = text.replace(old, new)
    assert mutated != text
    open(path, 'w').write(mutated)
    try:
        p = subprocess.run(['cargo', 'test', '-j', '6', '--test', 'file_local_special'],
                           cwd=ROOT, capture_output=True, text=True)
    finally:
        open(path, 'wb').write(original)
    assert hashlib.sha256(open(path, 'rb').read()).hexdigest() == digest
    out = p.stdout + p.stderr
    failed = [l for l in out.splitlines() if l.startswith('test ') and l.endswith('FAILED')]
    assertions = [l for l in out.splitlines() if 'assertion' in l]
    compiled = 'error[' not in out
    verdict = 'KILLED' if (p.returncode != 0 and compiled and assertions) else ('COMPILE-ERROR' if not compiled else 'SURVIVED')
    log.append(f'{name}: {verdict} exit={p.returncode} failed={failed} first_assertion={assertions[:1]}')
    print(log[-1], flush=True)
open(sys.argv[1], 'w').write('\n'.join(log) + '\n')
