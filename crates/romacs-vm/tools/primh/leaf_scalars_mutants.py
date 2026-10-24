#!/usr/bin/env python3
"""Kill one single-site production-body mutant per scalar, never build failures."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import fcntl
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / 'crates/romacs-vm/src/prims/leaf_scalars.rs'
LOGS = Path(__file__).resolve().parent / 'logs/leaf-scalars'
MUTANTS = [
    ('point-marker', 'let point = Value::integer(cx.point()?)', 'let point = Value::integer(cx.point()? + 1)', 'point_marker_tracks_left_biased_character_position', 'point-marker fresh left-biased character anchor'),
    ('atan', 'y.atan2(number(cx, x)?)', 'y.atan2(-number(cx, x)?)', 'atan_two_argument_quadrant_and_negative_zero', 'atan two-argument quadrant and negative zero'),
    ('exp', 'number(cx, arg)?.exp()', 'number(cx, arg)?.exp2()', 'exp_zero_underflow_and_overflow', 'exp zero underflow overflow'),
    ('number-to-string', 'cx.make_string(text.into_bytes(), false)', 'cx.make_string(text.into_bytes(), true)', 'number_to_string_decimal_and_unibyte', 'number-to-string decimal unibyte'),
]

def main():
    env = dict(os.environ, HOME=str(_PortablePath.home()), GNUPGHOME=_portable_os.environ.get('GNUPGHOME', str(_PortablePath.home() / '.gnupg')), GIT_PAGER='cat', TMPDIR=str(_PortablePath(_portable_tempfile.gettempdir()) / ''), CARGO_TARGET_DIR=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/leaf-target'), RUSTC_WRAPPER='sccache', SCCACHE_DIR=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'sccache'), CARGO_BUILD_JOBS='5', MAKEFLAGS='-j5', PYTHONDONTWRITEBYTECODE='1')
    for key in ['GIT_DIR', 'GIT_WORK_TREE']:
        env.pop(key, None)
    original = SOURCE.read_text()
    results = []
    with open(str(_PortablePath(_portable_tempfile.gettempdir()) / 'leaf-build.lock'), 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            for name, old, new, test, assertion in MUTANTS:
                assert original.count(old) == 1, (name, 'mutation must apply once')
                SOURCE.write_text(original.replace(old, new, 1))
                command = ['cargo', 'test', '-p', 'romacs-vm', '--test', 'leaf_scalars_values', test, '--', '--exact', '--nocapture']
                proc = subprocess.run(command, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=240)
                (LOGS / f'mutant-{name}.log').write_text(proc.stdout)
                killed = proc.returncode == 101 and 'test result: FAILED' in proc.stdout and assertion in proc.stdout and 'assertion `left == right` failed' in proc.stdout and 'could not compile' not in proc.stdout
                results.append(dict(primitive=name, mutation=[old, new], command=command, exit=proc.returncode, assertion=assertion, killed=killed))
                print(name, 'KILLED' if killed else 'NOT KILLED', flush=True)
                assert killed, proc.stdout[-4000:]
        finally:
            SOURCE.write_text(original)
            (LOGS / 'mutants.json').write_text(json.dumps(results, indent=2) + '\n')

if __name__ == '__main__':
    main()
