#!/usr/bin/env python3
"""The GNU Emacs that RECORDS expectations, and the refusal that keeps it the pin.

    gnu_oracle.py check            print the oracle's identity; exit 1 unless it is the pin
    gnu_oracle.py path             print the oracle's path after the same check
    gnu_oracle.py repin --emacs PATH --reason "..." [--mirror-commit SHA]
                        [--build-time STAMP] [--dry-run]
                                   re-pin ledger/parity-reference.toml to a GNU build

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
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tomllib

ENV = "ROMACS_GNU_ORACLE"
REPO = pathlib.Path(__file__).resolve().parents[2]
REFERENCE = REPO / "ledger/parity-reference.toml"
# The oracle's source checkout: GNU's C and Lisp at the commit the binary was built from.
GNU_SOURCE = os.environ.get("LISP_ORACLE_GNU", str(pathlib.Path(os.environ.get("ROMACS_GNU_SOURCE", pathlib.Path(
    os.environ.get("GNU_ORACLE_ROOT", pathlib.Path(os.environ.get("XDG_CACHE_HOME", pathlib.Path.home() / ".cache")) / "gnu-emacs-pin"))
    / "src")) / ""))


def mirror_commit(root=REPO):
    """The GNU commit the oracle binary was built from (ledger/parity-reference.toml)."""
    return tomllib.loads((pathlib.Path(root) / "ledger/parity-reference.toml").read_text())["mirror_commit"]
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
                       capture_output=True, text=True, env=env, timeout=600)  # a hang, not a slow start
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


# Re-pinning.
#
# A rebuild of the GNU mirror is its own change with its own re-baselining, never
# a side effect of a session, and a hand edit of the reference leaves no record of
# what changed or why. So re-pinning is one command: it refuses without a reason,
# prints the before and after of every field it changes, and writes the dated log
# entry in the same act that moves the pin.

LOG_HEADER = "# RE-BASELINING LOG"
KEYS = ("schema", "emacs_version", "mirror_commit", "build_time", "fingerprint",
        "executable_sha256", "executable_size", "pdmp_sha256", "pdmp_size")
DUMP_MAGIC = b"DUMPEDGNUEMACS\0\0"  # src/pdumper.c: the dump header's magic
FINGERPRINT_LEN = 32                  # lib/fingerprint.h: unsigned char fingerprint[32]
LOG_INDENT = "#               "


class RepinError(SystemExit):
    """A refusal to re-pin; nothing was written."""

    def __init__(self, message):
        super().__init__(f"gnu_oracle repin: {message}")


def manifest_lines(text):
    """TEXT's lines as the reference's format reads them: split on newlines, CR dropped."""
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    return [line.removesuffix("\r") for line in lines]


def parse_reference(text):
    """The reference's keys, refusing any line the tiny format does not define.

    A line is a comment, blank, or exactly `key = "value"`: a lower-snake key and
    a value with no quote or backslash (the format has no escapes). A parser that
    skips what it does not understand is how a pin stops being checked.
    """
    fields = {}
    for number, line in enumerate(manifest_lines(text), 1):
        if not line.strip() or line.strip().startswith("#"):
            continue
        key, sep, value = line.partition(" = ")
        if not sep:
            raise ValueError(f"line {number}: expected `key = \"value\"`, got {line!r}")
        if not re.fullmatch(r"[a-z0-9_]+", key):
            raise ValueError(f"line {number}: {key!r} is not a lower-snake key")
        if len(value) < 2 or value[0] != '"' or value[-1] != '"':
            raise ValueError(f"line {number}: value {value!r} is not double-quoted")
        value = value[1:-1]
        if '"' in value or "\\" in value:
            raise ValueError(f"line {number}: value {value!r} contains a quote or backslash; "
                             "the format has no escapes")
        if key in fields:
            raise ValueError(f"line {number}: duplicate key {key!r}")
        fields[key] = value
    for key in fields:
        if key not in KEYS:
            raise ValueError(f"unknown key {key!r} for schema 1")
    missing = [key for key in KEYS if key not in fields]
    if missing:
        raise ValueError(f"missing key {missing[0]!r}")
    if fields["schema"] != "1":
        raise ValueError(f"schema {fields['schema']!r} is not understood; expected \"1\"")
    for key, digits in (("mirror_commit", 40), ("fingerprint", 2 * FINGERPRINT_LEN),
                        ("executable_sha256", 64), ("pdmp_sha256", 64)):
        if not re.fullmatch(f"[0-9a-f]{{{digits}}}", fields[key]):
            raise ValueError(f"key {key!r} must be {digits} lowercase hex digits, got {fields[key]!r}")
    for key in ("executable_size", "pdmp_size"):
        if not re.fullmatch(r"[0-9]+", fields[key]):
            raise ValueError(f"key {key!r} is not a non-negative integer: {fields[key]!r}")
    return fields


def render_keys(fields):
    """The key block, in the one order the reference is written in."""
    return "".join(f'{key} = "{fields[key]}"\n' for key in KEYS)


def dump_for(executable):
    """The dump GNU loads for EXECUTABLE (src/emacs.c load_pdump).

    Beside the canonical executable first (a build tree), then the installed
    libexec tree under the binary's prefix (Makefile.in installs
    emacs-<fingerprint>.pdmp there), byte-ordered so one is picked every time.
    When nothing is found, the beside path, so the refusal names where a
    build-tree user would look.
    """
    beside = pathlib.Path(f"{executable}.pdmp")
    if beside.is_file():
        return beside
    prefix = executable.parent.parent
    found = sorted((path for root in ("libexec/emacs", "lib/emacs")
                    for path in (prefix / root).rglob("*.pdmp")
                    if path.is_file() and (path.name in ("emacs.pdmp", "Emacs.pdmp")
                                           or path.name.startswith("emacs-"))),
                   key=lambda path: os.fsencode(path))
    return found[0] if found else beside


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def observe(emacs):
    """The byte identity of the GNU at EMACS and the dump it loads."""
    candidate = pathlib.Path(emacs)
    if len(candidate.parts) == 1:
        found = shutil.which(emacs)
        if not found:
            raise RepinError(f"{emacs}: not found on PATH")
        candidate = pathlib.Path(found)
    try:
        executable = candidate.resolve(strict=True)
    except OSError as error:
        raise RepinError(f"{candidate}: {error}") from error
    pdmp = dump_for(executable)
    try:
        with open(pdmp, "rb") as stream:
            header = stream.read(len(DUMP_MAGIC) + FINGERPRINT_LEN)
    except OSError as error:
        raise RepinError(f"the dump {pdmp} is missing: {error}") from error
    if len(header) < len(DUMP_MAGIC) + FINGERPRINT_LEN:
        raise RepinError(f"the dump {pdmp} is missing: cannot read the dump header")
    if header[:len(DUMP_MAGIC)] != DUMP_MAGIC:
        magic = header[:len(DUMP_MAGIC)].rstrip(bytes(1)).decode(errors="replace")
        raise RepinError(f"{pdmp} is not a GNU dump: its magic is {magic!r}")
    return dict(executable=executable, pdmp=pdmp, fingerprint=header[len(DUMP_MAGIC):].hex(),
                executable_sha256=file_sha256(executable), executable_size=str(executable.stat().st_size),
                pdmp_sha256=file_sha256(pdmp), pdmp_size=str(pdmp.stat().st_size))


def ask(executable, form):
    """What EXECUTABLE prints for FORM in batch mode, or None when it fails."""
    try:
        r = subprocess.run([str(executable), "--batch", "--no-site-file", "--eval", form],
                           capture_output=True, text=True, errors="replace")
    except OSError as error:
        raise RepinError(f"cannot run {executable}: {error}") from error
    return r.stdout.strip() if r.returncode == 0 else None


def emacs_version(executable):
    version = ask(executable, "(princ emacs-version)")
    if version is None:
        raise RepinError(f"{executable} could not report emacs-version")
    if not version:
        raise RepinError("emacs-version came back empty")
    return version


def insert_offset_colon(stamp):
    """`+0500` -> `+05:00`, the shape the reference records build_time in."""
    if re.search(r"[+-][0-9]{4}\Z", stamp):
        return f"{stamp[:-2]}:{stamp[-2:]}"
    return stamp


def build_time(executable):
    # emacs-build-time is nil under --disable-build-details, and
    # format-time-string reads nil as NOW: never format it, refuse instead.
    stamp = ask(executable, "(princ (if emacs-build-time "
                            "(format-time-string \"%Y-%m-%dT%H:%M:%S%z\" emacs-build-time) \"\"))")
    if stamp is None:
        raise RepinError(f"{executable} could not report emacs-build-time")
    if not stamp:
        raise RepinError(f"{executable} reports no emacs-build-time (built --disable-build-details?), "
                         "and formatting nil would record the current time as the build time; pass "
                         "--build-time STAMP from the build's own record")
    return insert_offset_colon(stamp)


def mirror_head(executable):
    """The commit of the clean git checkout EXECUTABLE was built in."""
    directory = executable.parent

    def git(*args):
        try:
            return subprocess.run(["git", "-C", str(directory), *args], capture_output=True, text=True)
        except OSError as error:
            raise RepinError(f"cannot run git: {error}") from error
    head = git("rev-parse", "HEAD")
    if head.returncode != 0:
        raise RepinError(f"{directory} is not in a git checkout, so the mirror commit cannot be "
                         "recorded; pass --mirror-commit SHA")
    commit = head.stdout.strip()
    if len(commit) != 40:
        raise RepinError(f"git gave an odd commit {commit!r}")
    # A dirty mirror would name a tree the binary was not built from.
    if git("status", "--porcelain").stdout.strip():
        raise RepinError(f"the mirror at {directory} has uncommitted changes, so {commit} would not "
                         "describe what was built. Commit or clean the mirror, or pass "
                         "--mirror-commit to record the commit deliberately.")
    return commit


def log_entry(fields, reason, today):
    """The dated log lines for a re-pin to FIELDS, the reason wrapped under it."""
    lines = [f"#   {today}  GNU Emacs {fields['emacs_version']} built {fields['build_time']} "
             f"from mirror commit {fields['mirror_commit'][:11]}."]
    current = LOG_INDENT
    for word in reason.split():
        if len((current + word).encode()) + 1 > 78 and current.lstrip("#").strip():
            lines.append(current.rstrip())
            current = LOG_INDENT
        current += word + " "
    if current.lstrip("#").strip():
        lines.append(current.rstrip())
    return lines


def rewrite(text, fields, reason, today):
    """TEXT with its key block replaced by FIELDS and the log entry added, comments kept."""
    out, keys_written, log_written = [], False, False
    for line in manifest_lines(text):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            out.append(line + "\n")
            if not log_written and stripped.startswith(LOG_HEADER):
                out.extend(entry + "\n" for entry in log_entry(fields, reason, today))
                log_written = True
        elif not keys_written:
            # The whole block replaces the first key line and the rest are
            # dropped, so the order is always render_keys's.
            out.append(render_keys(fields))
            keys_written = True
    if not keys_written:
        raise RepinError("the reference has no key block to replace")
    if not log_written:
        raise RepinError(f'the reference has no "{LOG_HEADER}" line to append to; a re-baselining '
                         "that cannot be recorded must not be written")
    return "".join(out)


def repin(argv, reference=REFERENCE, today=None):
    """Re-pin REFERENCE to the GNU build named in ARGV; see the module's usage."""
    parser = argparse.ArgumentParser(prog="gnu_oracle.py repin")
    parser.add_argument("--emacs", required=True)
    parser.add_argument("--reason", required=True)
    parser.add_argument("--mirror-commit")
    parser.add_argument("--build-time")
    parser.add_argument("--dry-run", action="store_true")
    options = parser.parse_args(argv)
    # The log entry is the point: a re-baselining nobody wrote down is the side
    # effect this command exists to prevent.
    reason = options.reason.strip()
    if not reason:
        raise RepinError("--reason must not be empty. Re-pinning the GNU reference re-baselines "
                         "every parity number this project publishes; the log entry saying why is "
                         "not optional.")
    if options.build_time is not None and not options.build_time.strip():
        raise RepinError("--build-time must not be blank")
    reference = pathlib.Path(reference)
    try:
        text = reference.read_text()
    except OSError as error:
        raise RepinError(f"cannot read {reference}: {error}") from error
    try:
        current = parse_reference(text)
    except ValueError as error:
        raise RepinError(f"cannot parse {reference}: {error}") from error

    observed = observe(options.emacs)
    executable = observed["executable"]
    proposed = dict(
        schema=current["schema"], emacs_version=emacs_version(executable),
        mirror_commit=mirror_head(executable) if options.mirror_commit is None else options.mirror_commit,
        build_time=build_time(executable) if options.build_time is None else options.build_time,
        **{key: observed[key] for key in KEYS[4:]})

    print("observed:")
    print(f"  executable       {executable}")
    print(f"  dump             {observed['pdmp']}")
    print(f"  fingerprint      {observed['fingerprint']}")
    print(f"  executable sha256 {observed['executable_sha256']}")
    print(f"  dump sha256       {observed['pdmp_sha256']}")
    changes = [key for key in KEYS[1:] if current[key] != proposed[key]]
    if not changes:
        print("\nThe pin already describes this build; nothing to re-baseline.")
        return
    print("\nThis re-baselines every parity number this project publishes:")
    for key in changes:
        print(f"  {key}\n    was: {current[key]}\n    now: {proposed[key]}")
    print(f"\n  reason: {reason}")
    if options.dry_run:
        print(f"\n--dry-run: {reference} was not written.")
        return

    rewritten = rewrite(text, proposed, reason, today or datetime.date.today().isoformat())
    # Never write a reference this project cannot read back.
    try:
        reread = parse_reference(rewritten)
    except ValueError as error:
        raise RepinError(f"refusing to write an unreadable pin: {error}") from error
    if reread != proposed:
        raise RepinError("the rewritten pin does not read back as intended")
    reference.write_text(rewritten)
    print(f"\nRe-pinned {reference}.")
    print("Commit this file on its own: a re-baselining is its own change.")


def main(argv):
    if argv[1:2] == ["repin"]:
        return repin(argv[2:])
    if argv[1:] not in (["check"], ["path"]):
        sys.exit(__doc__)
    ident = require()
    print(json.dumps(ident, indent=1) if argv[1] == "check" else ident["path"])


if __name__ == "__main__":
    main(sys.argv)
