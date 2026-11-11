#!/usr/bin/env python3
"""The GNU Emacs that RECORDS expectations, and the refusal that keeps it the pin.

    gnu_oracle.py check            print the oracle's identity; exit 1 unless it is the pin
    gnu_oracle.py path             print the oracle's path after the same check

Every GNU answer this project commits is a parity claim against the pin in
ledger/parity-reference.toml (`mirror_commit`). For months the recording binary was
/usr/bin/emacs, built at b28750b822dc, 320 GNU commits older than the pin the
evidence named. This module removes the two ways that happened:

* No PATH lookup and no default. The oracle is the absolute path in
  $ROMACS_GNU_ORACLE; unset, relative or non-executable is a refusal.
* Identity comes from the BINARY, never from a checkout: the oracle is asked
  for `emacs-repository-version`, which GNU's loadup.el freezes into the dump
  at build time. A revision other than the pin is a refusal, not a warning.

Recorders call `require()` once and record `identity()` in their evidence.
"""
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tomllib

ENV = "ROMACS_GNU_ORACLE"
REPO = pathlib.Path(__file__).resolve().parents[2]
REFERENCE = REPO / "ledger/parity-reference.toml"
PROBE = ("(princ (json-serialize (list (cons 'revision (or emacs-repository-version \"\"))"
         " (cons 'version emacs-version))))")


class OracleError(SystemExit):
    """A refusal: the recording oracle is absent, ambiguous or not the pin."""

    def __init__(self, message):
        super().__init__(f"gnu_oracle: REFUSED: {message}")


def pin():
    """The declared GNU revision every recorded answer must come from."""
    with open(REFERENCE, "rb") as stream:
        commit = tomllib.load(stream)["mirror_commit"]
    if len(commit) != 40:
        raise OracleError(f"{REFERENCE}: mirror_commit {commit!r} is not a full revision")
    return commit


def path(environ=None):
    """The oracle executable named by $ROMACS_GNU_ORACLE, or a refusal."""
    value = (os.environ if environ is None else environ).get(ENV)
    if not value:
        raise OracleError(f"${ENV} is unset; set it to the absolute path of GNU Emacs built at "
                          f"{pin()} (see tools/build-gnu-oracle.sh). There is no default and "
                          "no PATH lookup: that is how /usr/bin/emacs became the oracle.")
    p = pathlib.Path(value)
    if not p.is_absolute():
        raise OracleError(f"${ENV}={value!r} is not an absolute path")
    if not (p.is_file() and os.access(p, os.X_OK)):
        raise OracleError(f"${ENV}={value!r} is not an executable file")
    return p


def identity(binary):
    """What the binary itself reports, plus the digest of the file that ran."""
    env = {"PATH": "/usr/bin:/bin", "HOME": os.environ.get("TMPDIR", "/tmp"), "LANG": "C.UTF-8"}
    r = subprocess.run([str(binary), "-Q", "--batch", "--eval", PROBE],
                       capture_output=True, text=True, env=env, timeout=60)
    if r.returncode != 0:
        raise OracleError(f"{binary} exited {r.returncode}: {r.stderr.strip()[-300:]}")
    try:
        reply = json.loads(r.stdout)
    except json.JSONDecodeError as error:
        raise OracleError(f"{binary} printed no identity: {r.stdout[-200:]!r}") from error
    real = pathlib.Path(binary).resolve()
    return dict(path=str(binary), resolved=str(real), revision=reply["revision"],
                version=reply["version"], sha256=hashlib.sha256(real.read_bytes()).hexdigest())


def check(ident, expected=None):
    """Refuse unless IDENT's binary-reported revision is the pin."""
    want = expected or pin()
    if ident["revision"] != want:
        raise OracleError(f"{ident['path']} reports emacs-repository-version "
                          f"{ident['revision'] or '<none>'!r}, the pin is {want}. "
                          "Every answer it recorded would be labelled with the wrong reference.")
    return ident


def require(binary=None):
    """Resolve (or take) the oracle, ask it who it is, refuse unless it is the pin."""
    return check(identity(binary or path()))


def main(argv):
    if argv[1:] not in (["check"], ["path"]):
        sys.exit(__doc__)
    ident = require()
    print(json.dumps(ident, indent=1) if argv[1] == "check" else ident["path"])


if __name__ == "__main__":
    main(sys.argv)
