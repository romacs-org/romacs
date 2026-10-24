#!/usr/bin/env python3
"""Record or replay system-name in an isolated neutral Linux UTS namespace.

Requires unshare with user/UTS namespace support and hostname on PATH.
The original divergence pins are retained when recording; replay never pins.
"""
import argparse
import json
from pathlib import Path
import socket
import subprocess
import sys
from types import SimpleNamespace

import primh

HOST = "romacs-oracle.example"
SPEC = primh.HERE / "sysname/system-name.json"
FIXTURE = primh.EVID / "system-name.json"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("record", "replay"))
    parser.add_argument("--gnu")
    parser.add_argument("--gnu-src")
    parser.add_argument("--vm2")
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.inside:
        command = ["unshare", "--user", "--map-root-user", "--uts", "sh", "-eu", "-c",
                   'hostname "$1"; shift; exec "$@"', "system-name-oracle", HOST,
                   sys.executable, str(Path(__file__).resolve()), *sys.argv[1:], "--inside"]
        return subprocess.call(command)
    if socket.gethostname() != HOST:
        parser.error("controlled hostname is absent; refusing recording or replay")
    if args.mode == "record":
        if not args.gnu or not args.gnu_src:
            parser.error("record requires --gnu and --gnu-src")
        original = json.loads(FIXTURE.read_text())
        spec = primh.load_spec(SPEC)
        head, replies = primh.gnu_rows(args.gnu, spec, primh.cases(spec), ["system-name"])
        if head != original["gnu"]:
            parser.error("GNU identity differs from the retained oracle")
        if primh.sha(Path(args.gnu).resolve().read_bytes()) != original["gnu_binary_sha256"]:
            parser.error("GNU binary differs from the retained oracle")
        sources = {name: primh.sha((Path(args.gnu_src) / name).read_bytes()) for name in spec["gnu_files"]}
        if sources != original["gnu_sources"]:
            parser.error("GNU sources differ from the retained oracle")
        rows = primh.cases(spec)
        for row in rows:
            row["expected"], interp = replies[row["id"]]
            if interp != row["expected"]:
                row["gnu_regime_variant"] = interp
        for before, after in zip(original["cases"], rows, strict=True):
            if before["id"] not in ("system-name/0000", "system-name/0002") and before != after:
                parser.error(f'non-host outcome changed: {before["id"]}')
        original["cases"] = rows
        original["recording_wrapper_sha256"] = primh.sha(Path(__file__).read_bytes())
        original["environment"] = {"hostname": HOST, "control": "Linux unshare --user --map-root-user --uts; hostname before GNU and VM startup"}
        original.pop("recording_generator_sha256", None)
        original.pop("generator_portability_note", None)
        original["generator_sha256"] = primh.sha(Path(primh.__file__).read_bytes())
        primh.atomic_json(FIXTURE, original)
        print(f"recorded {len(rows)} GNU cases under {HOST}; existing pins retained")
        return 0
    if not args.vm2:
        parser.error("replay requires --vm2")
    fixture = json.loads(FIXTURE.read_text())
    if fixture.get("recording_wrapper_sha256") != primh.sha(Path(__file__).read_bytes()):
        parser.error("controlled environment wrapper changed; re-record before replay")
    primh.replay(SimpleNamespace(spec=[SPEC], vm2=args.vm2, pin=False, show=11, json=None))
    return 0


if __name__ == "__main__":
    sys.exit(main())
