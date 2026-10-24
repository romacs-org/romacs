#!/usr/bin/env python3
"""Record primh / bulk evidence from the PINNED GNU oracle, and only from it.

    record.py primh SPEC... [--out DIR]     # = primh.py record, oracle from $ROMACS_GNU_ORACLE
    record.py bulk GROUP [--out DIR]        # = bulk.py record GROUP, same oracle

primh.py, bulk.py and gnu_driver.el are loaded UNMODIFIED: their bytes are
hashed into every evidence file, so changing them would re-hash every corpus.
This entry point replaces only how the GNU executable is chosen:

* there is no `--gnu`: the oracle is $ROMACS_GNU_ORACLE (scripts/gnu_oracle.py),
  absolute, never a PATH lookup, and it must REPORT the pin as its own
  `emacs-repository-version` or nothing is recorded;
* primh's `gnu_sources` are hashed from the oracle's OWN `source-directory`,
  which must be a clean checkout at the pin, not from whatever the shared
  reference checkout happens to hold;
* after recording, every written file's GNU header (the driver's
  `emacs-repository-version`, again from the binary) must name the pin.

`--out DIR` writes elsewhere instead of the committed evidence (measurement).
GNU's stdin is always /dev/null: prompt primitives without a keyboard macro
read stdin in batch (minibuf.c:653), and a recording must not depend on, or
hang on, whatever terminal or pipe the recorder was started from.
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[3] / "scripts"))
import gnu_oracle  # noqa: E402


def load(name, directory=HERE):
    spec = importlib.util.spec_from_file_location(name, Path(directory) / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FileNamedDir(type(Path())):
    """primh names evidence after the primitive; `/=` must not become `/=.json`."""

    def __truediv__(self, key):
        if isinstance(key, str):
            key = key.replace("/", "%2F")
        return FileNamedDir(super().__truediv__(key))


def source_dir(ident):
    """The oracle's own src/, refused unless it is a clean checkout at the pin."""
    r = subprocess.run([ident["path"], "-Q", "--batch", "--eval", "(princ source-directory)"],
                       capture_output=True, text=True, timeout=60, check=True)
    root = Path(r.stdout)
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                          capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain", "--untracked-files=no"],
                           capture_output=True, text=True).stdout.strip()
    if head != ident["revision"] or dirty:
        raise gnu_oracle.OracleError(f"oracle source-directory {root} is at {head or '<no git>'}"
                                     f"{' (modified)' if dirty else ''}, not {ident['revision']}")
    return root / "src"


def verify_written(paths, ident):
    for p in paths:
        head = json.loads(Path(p).read_text())["gnu"]
        if head.get("revision") != ident["revision"]:
            raise gnu_oracle.OracleError(f"{p}: recorded header revision {head.get('revision')!r} "
                                         f"is not the oracle's {ident['revision']}")


def primh_record(a, ident, primh_dir=HERE):
    primh = load("primh", primh_dir)
    if a.out:
        primh.EVID = FileNamedDir(Path(a.out).resolve())
    else:
        primh.EVID = FileNamedDir(primh.EVID)
    gnu_src = source_dir(ident)
    primh.record(argparse.Namespace(spec=a.spec, gnu=ident["path"], gnu_src=str(gnu_src)))
    verify_written([primh.EVID / f"{primh.load_spec(s)['name']}.json" for s in a.spec], ident)


def bulk_record(a, ident, primh_dir=HERE):
    bulk = load("bulk", primh_dir)
    if a.out:
        bulk.BULK = Path(a.out).resolve()
        bulk.BULK.mkdir(parents=True, exist_ok=True)
        # bulk.py reads the spec from BULK too
        spec = Path(primh_dir) / "bulk" / f"{a.group}.spec.json"
        bulk.spec_path = lambda group, _s=spec: _s if group == a.group else Path(primh_dir) / "bulk" / f"{group}.spec.json"
    bulk.record(argparse.Namespace(group=a.group, gnu=ident["path"]))
    verify_written([bulk.BULK / f"{a.group}.corpus.json"], ident)


def detach_stdin():
    """Every GNU child inherits fd 0; make it EOF, as the minibuf corpus requires."""
    null = os.open(os.devnull, os.O_RDONLY)
    os.dup2(null, 0)
    os.close(null)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    p = sp.add_parser("primh"); p.add_argument("spec", nargs="+"); p.add_argument("--out")
    p.set_defaults(f=primh_record)
    b = sp.add_parser("bulk"); b.add_argument("group"); b.add_argument("--out")
    b.set_defaults(f=bulk_record)
    a = ap.parse_args(argv)
    detach_stdin()
    ident = gnu_oracle.require()
    print(f"oracle {ident['path']} revision {ident['revision']} sha256 {ident['sha256'][:16]}",
          file=sys.stderr)
    a.f(a, ident)


if __name__ == "__main__":
    main()
