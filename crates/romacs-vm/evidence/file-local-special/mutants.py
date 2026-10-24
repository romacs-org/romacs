#!/usr/bin/env python3
"""Source mutants for tests/file_local_special.rs. Each is applied, tested and
restored byte-for-byte; a mutant is KILLED when a test fails at an assertion."""
import os as _portable_os
from pathlib import Path as _PortablePath
import subprocess, sys, pathlib
ROOT = pathlib.Path(str(_PortablePath(_portable_os.environ['ROMACS_MUTATION_ROOT']) / 'crates/romacs-vm'))
RUN = ['bash', _portable_os.environ['ROMACS_GATE_SLOT'], 'run', 'fdv', '--',
       _portable_os.environ['ROMACS_MUTATION_RUNNER'],
       'cargo', 'test', '-p', 'romacs-vm', '--test', 'file_local_special']
MUTANTS = [
    ('M1 cache validation ignores file specials', 'src/source_cache.rs',
     '.all(|(s, v)| self.compile_special(s) == *v)', '.all(|(s, v)| self.specials.contains(s) == *v)'),
    ('M2 cache records ignore file specials', 'src/source_cache.rs',
     'let v = self.compile_special(&s);', 'let v = self.specials.contains(&s);'),
    ('M3 root compile not seeded', 'src/lib.rs',
     'let seed = if std::mem::take(&mut self.seed_file_specials) {', 'let seed = if false && std::mem::take(&mut self.seed_file_specials) {'),
    ('M4 file specials not restored after a load', 'src/loader.rs',
     '        self.file_specials = saved_file_specials;\n', '        let _ = saved_file_specials;\n'),
    ('M5 declaration not recorded', 'src/loader.rs',
     '                    self.file_specials.insert(name.clone());', '                    let _ = name;'),
]
for label, rel, old, new in MUTANTS:
    p = ROOT / rel; orig = p.read_bytes(); s = orig.decode()
    assert s.count(old) == 1, (label, s.count(old))
    try:
        p.write_text(s.replace(old, new))
        r = subprocess.run(RUN, capture_output=True, text=True)
        out = r.stdout + r.stderr
        failed = [l for l in out.splitlines() if l.startswith('thread ') and 'panicked' in l]
        compiled = 'test result:' in out
        verdict = 'KILLED' if r.returncode != 0 and failed and compiled else ('SURVIVED' if r.returncode == 0 else 'NOT-A-TEST-FAILURE')
        print(f'{label}: {verdict} exit={r.returncode}')
        for l in out.splitlines():
            if 'panicked' in l or l.startswith('  left:'):
                print('   ', l)
        sys.stdout.flush()
    finally:
        p.write_bytes(orig)
