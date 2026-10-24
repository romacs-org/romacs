#!/usr/bin/env python3
"""Run the committed regexp mutants sequentially; restore every input in finally.

Use a private disposable build snapshot, not a checkout another writer owns.
A kill requires the named assertion, never compilation/setup failure.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--only")
    args = parser.parse_args()
    repo = args.repo.resolve()
    manifest = json.loads((Path(__file__).resolve().parents[1] / "evidence/regexp-mutants.json").read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    mutants = [m for m in manifest["mutants"] if args.only is None or m["name"] == args.only]
    assert mutants, "no selected mutants"
    results = []
    targets = sorted({(m.get("package", "romacs-vm"), m.get("target", "regexp")) for m in mutants})

    def baseline(stage):
        for package, target in targets:
            result = subprocess.run(["cargo", "test", "-p", package, "--test", target],
                                    cwd=repo, text=True, capture_output=True, timeout=600)
            suffix = "" if (package, target) == ("romacs-vm", "regexp") else f"-{package}-{target}"
            (args.output / f"{stage}{suffix}.log").write_text(result.stdout + result.stderr)
            assert result.returncode == 0, (stage, package, target, "production baseline must pass")

    baseline("baseline")
    for mutant in mutants:
        originals = {}
        try:
            for edit in mutant["edits"]:
                path = repo / edit["path"]
                if path not in originals:
                    originals[path] = path.read_bytes()
                source = path.read_text()
                count = edit.get("count", 1)
                assert source.count(edit["old"]) == count, (mutant["name"], edit["path"], "patch drift", source.count(edit["old"]))
                path.write_text(source.replace(edit["old"], edit["new"]))
            result = subprocess.run(["cargo", "test", "-p", mutant.get("package", "romacs-vm"), "--test", mutant.get("target", "regexp"),
                                     mutant["test"], "--", "--exact", "--nocapture"],
                                    cwd=repo, text=True, capture_output=True, timeout=600)
            output = result.stdout + result.stderr
            (args.output / f"{mutant['name']}.log").write_text(output)
            expected = mutant.get("failure_text") or f"assertion `left == right` failed: {mutant['assertion']}"
            killed = (result.returncode == 101 and expected in output and "panicked at" in output
                      and "could not compile" not in output and "error[E" not in output)
            row = dict(name=mutant["name"], test=mutant["test"], status=result.returncode,
                       killed=killed, expected_assertion=expected,
                       input_sha256={str(p.relative_to(repo)): hashlib.sha256(b).hexdigest()
                                     for p, b in originals.items()})
            results.append(row)
            (args.output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
            print(f"{mutant['name']}: {'KILLED at target assertion' if killed else 'NOT QUALIFIED'}", flush=True)
        finally:
            for path, original in originals.items():
                path.write_bytes(original)
    baseline("restored")
    killed = sum(row["killed"] for row in results)
    print(f"{killed}/{len(mutants)} qualified mutant kills")
    raise SystemExit(killed != len(mutants))


if __name__ == "__main__":
    main()
