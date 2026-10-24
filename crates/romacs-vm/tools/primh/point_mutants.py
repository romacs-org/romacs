#!/usr/bin/env python3
"""Bounded behavior mutants, restoring exact source bytes after every run."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
LEAF = ROOT / 'crates/romacs-vm/src/prim/leaf/point_predicates.rs'
PRIMS = ROOT / 'crates/romacs-vm/src/prims/point_predicates.rs'
OUT = Path(__file__).resolve().parent / 'point-predicates-logs'
MUTANTS = [
    ('bolp-absolute-beginning', LEAF, 'point == beginning,', 'point == 0,', 'bolp_accessible_beginning_and_character_positions', 'BOLP_ACCESSIBLE_CHARACTER_BOUNDARY'),
    ('bobp-absolute-beginning', LEAF, 'point == beginning,', 'point == 0,', 'bobp_uses_accessible_not_absolute_beginning', 'BOBP_ACCESSIBLE_BEGINNING'),
    ('eolp-absolute-end', LEAF, 'accessible.end.0, previous', 'buffer.len_bytes(), previous', 'eolp_accessible_end_and_character_positions', 'EOLP_ACCESSIBLE_CHARACTER_BOUNDARY'),
    ('eobp-absolute-end', LEAF, 'accessible.end.0, previous', 'buffer.len_bytes(), previous', 'eobp_uses_accessible_not_absolute_end', 'EOBP_ACCESSIBLE_END'),
    ('bolp-ignore-newline', PRIMS, 'beginning || previous_newline', 'beginning', 'bolp_accessible_beginning_and_character_positions', 'BOLP_ACCESSIBLE_CHARACTER_BOUNDARY'),
    ('eolp-ignore-newline', PRIMS, 'end || next_newline', 'end', 'eolp_accessible_end_and_character_positions', 'EOLP_ACCESSIBLE_CHARACTER_BOUNDARY'),
    ('byte-index-as-character-offset', LEAF, 'let character = buffer.char_of_byte(buffer.point).map_err(Fault::Error)?;', 'let character = CharOffset(buffer.point.0);', 'bolp_accessible_beginning_and_character_positions', 'BOLP_ACCESSIBLE_CHARACTER_BOUNDARY'),
]

def main():
    env = dict(os.environ)
    env.update(HOME=str(_PortablePath.home()), GNUPGHOME=_portable_os.environ.get('GNUPGHOME', str(_PortablePath.home() / '.gnupg')), GIT_PAGER='cat', TMPDIR=str(_PortablePath(_portable_tempfile.gettempdir()) / ''), CARGO_TARGET_DIR=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/bolp-target'), ROMACS_GNU_ORACLE=_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50')))
    rows = []
    for name, path, old, new, test, marker in MUTANTS:
        original = path.read_bytes()
        source = original.decode()
        assert source.count(old) == 1, name
        try:
            path.write_text(source.replace(old, new))
            command = ['cargo', 'test', '--jobs', '4', '-p', 'romacs-vm', '--test', 'point_predicates', test, '--', '--exact']
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
            text = result.stdout.decode()
            (OUT / (name + '.log')).write_text(text)
            killed = result.returncode == 101 and marker in text and 'assertion `left == right` failed' in text and 'could not compile' not in text
            rows.append(dict(name=name, exit=result.returncode, killed_at_named_assertion=killed, source_sha256=hashlib.sha256(original).hexdigest(), command=command))
        finally:
            path.write_bytes(original)
        (OUT / 'mutants.json').write_text(json.dumps(rows, indent=2) + '\n')
        assert killed, name
    print(json.dumps(rows, indent=2))

if __name__ == '__main__':
    main()
