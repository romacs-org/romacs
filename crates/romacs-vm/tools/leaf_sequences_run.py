#!/usr/bin/env python3
"""Focused sequence verification, serialized with sibling leaf builds."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
LOGS = pathlib.Path(os.environ.get('ROMACS_ARTIFACT_DIR', pathlib.Path(os.environ.get('XDG_CACHE_HOME', pathlib.Path.home() / '.cache')) / 'romacs')) / 'leaf-sequences'
if LOGS.resolve().is_relative_to(ROOT):
    raise ValueError('ROMACS_ARTIFACT_DIR must be outside the source checkout')
LOGS.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
env.update(HOME=str(_PortablePath.home()), GNUPGHOME=_portable_os.environ.get('GNUPGHOME', str(_PortablePath.home() / '.gnupg')), GIT_PAGER='cat', TMPDIR=str(_PortablePath(_portable_tempfile.gettempdir()) / ''), CARGO_TARGET_DIR=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/leaf-target'), RUSTC_WRAPPER='sccache', SCCACHE_DIR=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'sccache'), CARGO_BUILD_JOBS='5', MAKEFLAGS='-j5', PYTHONDONTWRITEBYTECODE='1', ROMACS_GNU_ORACLE=_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50')))
for key in ['GIT_DIR', 'GIT_WORK_TREE']:
    env.pop(key, None)
def main():
    label = sys.argv[1]
    command = sys.argv[2:] or ['cargo', 'test', '-p', 'romacs-vm', '--test', 'leaf_sequences', '--', '--nocapture']
    with (LOGS / f'{label}.log').open('w') as log:
        result = subprocess.run(['flock', str(_PortablePath(_portable_tempfile.gettempdir()) / 'leaf-build.lock'), *command], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    print(f'{label}: exit={result.returncode}; log={LOGS / (label + ".log")}')
    sys.exit(result.returncode)

if __name__ == '__main__':
    main()
