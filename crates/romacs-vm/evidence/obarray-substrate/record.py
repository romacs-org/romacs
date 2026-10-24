#!/usr/bin/env python3
"""Run the byte-identical primh recorder with output confined to this directory."""
import os as _portable_os
from pathlib import Path as _PortablePath
import argparse
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent
HARNESS = HERE.parent.parent / "tools" / "primh" / "primh.py"
loader = importlib.util.spec_from_file_location("obarray_primh", HARNESS)
primh = importlib.util.module_from_spec(loader)
loader.loader.exec_module(primh)
primh.EVID = HERE
parser = argparse.ArgumentParser()
parser.add_argument("command", choices=["record"])
parser.add_argument("spec", nargs="+")
parser.add_argument("--gnu", required=True)
parser.add_argument("--gnu-src", default=str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / 'src'))
primh.record(parser.parse_args())
