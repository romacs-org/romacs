#!/usr/bin/env python3
"""Run primh.py for primitives whose names are not file names.

primh names evidence after the primitive, so `/=` becomes the absolute path
`/=.json`. Changing primh.py would change the generator hash that every
recorded evidence file pins, so this wrapper loads primh.py unmodified and
only maps `/` to `%2F` when it joins a name onto the evidence directory. Case
generation, GNU recording, replay and hashes are primh.py's own.

  primh_filenames.py record|replay|rust SPEC... [primh.py options]

`record` delegates to record.py (oracle from $ROMACS_GNU_ORACLE, pin-checked).
"""
import argparse
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("primh", HERE / "primh.py")
primh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(primh)


class FileNamedDir(type(Path())):
    def __truediv__(self, key):
        if isinstance(key, str):
            key = key.replace("/", "%2F")
        return Path(super().__truediv__(key))


primh.EVID = FileNamedDir(primh.EVID)

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
sp = ap.add_subparsers(dest="cmd", required=True)
# Recording goes through record.py: the pinned oracle, never `--gnu emacs` (R228).
a = sp.add_parser("record"); a.add_argument("spec", nargs="+")
a.set_defaults(f=lambda ns: __import__("record").main(["primh", *ns.spec]))
a = sp.add_parser("replay"); a.add_argument("spec", nargs="+"); a.add_argument("--vm2", required=True); a.add_argument("--show", type=int, default=4); a.add_argument("--json"); a.add_argument("--pin", action="store_true"); a.set_defaults(f=primh.replay)
a = sp.add_parser("rust"); a.add_argument("spec", nargs="+"); a.add_argument("--out", required=True); a.set_defaults(f=primh.rust)
ns = ap.parse_args()
ns.f(ns)
