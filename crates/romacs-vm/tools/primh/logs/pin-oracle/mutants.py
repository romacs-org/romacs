#!/usr/bin/env python3
"""Mutants for the pinned-oracle guard: each must turn tests/gnu_oracle_pin.rs RED
at its target assertion. The guard reads evidence and runs gnu_oracle.py at
test time, so one built test binary serves every mutant (no compile involved).

    mutants.py TEST_BINARY
"""
import json
import pathlib
import shutil
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[6]
PRIMH = REPO / "crates/romacs-vm/tools/primh"
ORACLE = REPO / "tools/lib/gnu_oracle.py"
PIN = "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
OLD = "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b"


def edit_json(path, fn):
    v = json.loads(path.read_text())
    fn(v)
    path.write_text(json.dumps(v, ensure_ascii=False, indent=1) + "\n")


def replace(path, old, new):
    s = path.read_text()
    assert old in s, (path, old)
    path.write_text(s.replace(old, new, 1))


def new_file(path, text):
    path.write_text(text)


MUTANTS = [
    ("a pin recording re-labelled as the old oracle", "evidence_is_recorded_against_the_pin",
     "evidence/take.json: recorded against",
     [PRIMH / "evidence/take.json"],
     lambda: edit_json(PRIMH / "evidence/take.json", lambda v: v["gnu"].update(revision=OLD))),
    ("new evidence file recorded by the old oracle", "evidence_is_recorded_against_the_pin",
     "evidence/zz-mutant.json: recorded against",
     [PRIMH / "evidence/zz-mutant.json"],
     lambda: (shutil.copy(PRIMH / "evidence/take.json", PRIMH / "evidence/zz-mutant.json"),
              edit_json(PRIMH / "evidence/zz-mutant.json", lambda v: v["gnu"].update(revision=OLD)))),
    ("the retired ledger restored to grandfather an old-oracle file", "evidence_is_recorded_against_the_pin",
     "evidence/zz-mutant.json: recorded against",
     [PRIMH / "evidence/zz-mutant.json", PRIMH / "LEGACY-ORACLE.json"],
     lambda: (shutil.copy(PRIMH / "evidence/take.json", PRIMH / "evidence/zz-mutant.json"),
              edit_json(PRIMH / "evidence/zz-mutant.json", lambda v: v["gnu"].update(revision=OLD)),
              new_file(PRIMH / "LEGACY-ORACLE.json", json.dumps(dict(
                  legacy_revision=OLD, files={"evidence/zz-mutant.json": __import__("hashlib").sha256(
                      (PRIMH / "evidence/zz-mutant.json").read_bytes()).hexdigest()}))))),
    ("bulk corpus recorded by the old oracle", "evidence_is_recorded_against_the_pin",
     "bulk/core.corpus.json: recorded against",
     [PRIMH / "bulk/core.corpus.json"],
     lambda: edit_json(PRIMH / "bulk/core.corpus.json", lambda v: v["gnu"].update(revision=OLD))),
    ("varh evidence recorded by the old oracle", "evidence_is_recorded_against_the_pin",
     "../varh/evidence/varh.json: recorded against",
     [PRIMH / "../varh/evidence/varh.json"],
     lambda: edit_json(PRIMH / "../varh/evidence/varh.json", lambda v: v["gnu"].update(revision=OLD))),
    ("widen evidence with no revision", "evidence_is_recorded_against_the_pin",
     "evidence/widen.json: recorded against",
     [PRIMH / "evidence/widen.json"],
     lambda: edit_json(PRIMH / "evidence/widen.json", lambda v: v["gnu"].pop("revision"))),
    ("gnu_oracle: revision check removed", "the_recorder_refuses_an_oracle_that_reports_another_revision",
     "a non-pin oracle must be refused",
     [ORACLE],
     lambda: replace(ORACLE, 'if ident["revision"] != want:', 'if False:')),
    ("gnu_oracle: relative path accepted", "the_recorder_refuses_without_an_explicit_absolute_oracle",
     "a PATH-relative oracle must be refused",
     [ORACLE],
     lambda: replace(ORACLE, "    if not p.is_absolute():\n", "    if False:\n")),
    ("gnu_oracle: unset falls back to PATH emacs", "the_recorder_refuses_without_an_explicit_absolute_oracle",
     "an unset oracle must be refused",
     [ORACLE],
     lambda: replace(ORACLE, "    if not value:\n", "    if not value:\n        value = __import__('shutil').which('emacs')\n    if not value:\n")),
    ("gnu_oracle: identity probe GNU cannot run (json-encode is not preloaded)",
     "a_real_gnu_is_judged_by_the_revision_it_reports", "it must be refused for that revision",
     [ORACLE],
     lambda: replace(ORACLE, "(princ (json-serialize", "(princ (json-encode")),
]


def main(binary):
    killed = 0
    for title, test, needle, touched, apply in MUTANTS:
        saved = {p: (p.read_bytes() if p.exists() else None) for p in touched}
        try:
            apply()
            r = subprocess.run([binary, "--exact", test], capture_output=True, text=True)
        finally:
            for p, b in saved.items():
                if b is None:
                    p.unlink(missing_ok=True)
                else:
                    p.write_bytes(b)
        out = r.stdout + r.stderr
        ok = r.returncode != 0 and "panicked at" in out and needle in out
        killed += ok
        print(f"{'KILLED' if ok else 'SURVIVED'}: {title}  [{test}]  "
              f"({'assertion: ' + needle if ok else 'rc=' + str(r.returncode)})")
        if not ok:
            print(out[-1500:])
    print(f"{killed}/{len(MUTANTS)} killed")
    clean = subprocess.run([binary], capture_output=True, text=True)
    print("restored tree:", "green" if clean.returncode == 0 else "RED\n" + clean.stdout[-1500:])
    sys.exit(0 if killed == len(MUTANTS) and clean.returncode == 0 else 1)


if __name__ == "__main__":
    main(sys.argv[1])
