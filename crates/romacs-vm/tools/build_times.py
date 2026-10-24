#!/usr/bin/env python3
"""Measure cold target builds and touched-source rebuilds, excluding downloads."""
import argparse
from typing import Any
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN = ''


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    env = os.environ.copy()
    env["PATH"] = TOOLCHAIN + ":" + env["PATH"]
    # Respect the user scratch location, not the system temporary directory.
    scratch = Path.home() / ".hermes/cache/scratch/romacs/vm2"
    scratch.mkdir(parents=True, exist_ok=True)
    report: dict[str, Any] = dict(toolchain=subprocess.check_output([TOOLCHAIN + "/rustc", "-Vv"], text=True),
                  note="Cached registry sources; fresh target directory. Default dev incremental compilation; effective workspace release profile, not incremental rustc (workspace root overrides crate-local LTO).", builds=[])
    with tempfile.TemporaryDirectory(prefix="build-", dir=scratch) as target:
        env["CARGO_TARGET_DIR"] = target
        for profile in ("dev", "release"):
            for mode in ("clean", "unchanged", "touched-source"):
                if mode == "touched-source":
                    os.utime(ROOT / "src/lib.rs")
                cmd = [TOOLCHAIN + "/cargo", "build", "--locked", "--offline", "--manifest-path", str(ROOT / "Cargo.toml")]
                if profile == "release":
                    cmd.append("--release")
                start = time.perf_counter()
                result = subprocess.run(cmd, env=env, capture_output=True, text=True, check=True)
                seconds = time.perf_counter() - start
                report["builds"].append(dict(profile=profile, mode=mode, seconds=seconds, output=result.stdout + result.stderr))
    files = sorted(p for p in ROOT.rglob("*") if p.is_file() and "target" not in p.parts and p.suffix in (".rs", ".py", ".toml"))
    report["lines"] = {str(p.relative_to(ROOT)): len(p.read_text().splitlines()) for p in files}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
