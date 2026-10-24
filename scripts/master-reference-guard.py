#!/usr/bin/env python3
"""Protect a pinned repository's master; publish one prevalidated lander CAS.

This is an accident guard, not a security boundary against the owning Unix user.
The publisher is deliberately not a gate: only the lander's final CAS calls it.
"""
import argparse
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

AUTH = "ROMACS_MASTER_AUTH_FD"
REF = "refs/heads/master"
DENIED = "MASTER GUARD REFUSED: refs/heads/master may move only through romacs-land.sh"


def common_dir(repo=None):
    args = ["git"] + (["-C", repo] if repo is not None else [])
    return str(Path(subprocess.check_output(
        args + ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        text=True, stderr=subprocess.DEVNULL).strip()).resolve(strict=True))


def payload(common, old, new):
    return (common + "\n" + old + " " + new + " " + REF + "\n").encode()


def hook(pinned, state):
    # Only prepared can veto; committed/aborted are notifications.
    if state != "prepared":
        return 0
    current = common_dir()
    if current != str(Path(pinned).resolve(strict=True)):
        return 0
    rows = [line.split() for line in sys.stdin]
    moves = [row for row in rows if len(row) == 3 and row[2] == REF and row[0] != row[1]]
    if not moves:
        return 0
    try:
        fd = int(os.environ[AUTH])
        if fd < 3 or not stat.S_ISFIFO(os.fstat(fd).st_mode):
            raise ValueError("not a capability pipe")
        os.set_blocking(fd, False)
        token = os.read(fd, 4097)  # Consumed even on mismatch; cannot replay.
        if len(rows) != 1 or token != payload(current, moves[0][0], moves[0][1]):
            raise ValueError("wrong transaction")
    except (KeyError, ValueError, OSError):
        print(DENIED, file=sys.stderr)
        return 1
    return 0


def publish(repo, old, new, message):
    # No shell, arbitrary command, ref name, or persistent bypass flag.
    if not all(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", oid) and
               set(oid) != {"0"} for oid in (old, new)) or len(old) != len(new):
        raise ValueError("publication needs full nonzero old/new object IDs")
    common = common_dir(repo)
    token = payload(common, old, new)
    if len(token) > 4096:
        raise ValueError("repository path too long")
    reader, writer = os.pipe()
    try:
        os.write(writer, token)
        os.close(writer)
        writer = None
        env = dict(os.environ, **{AUTH: str(reader)})
        return subprocess.call(["git", "-C", repo, "update-ref", "-m", message,
                                REF, new, old], env=env, pass_fds=(reader,))
    finally:
        os.close(reader)
        if writer is not None:
            os.close(writer)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    h = sub.add_parser("hook")
    h.add_argument("--common-dir", required=True)
    h.add_argument("state", choices=("preparing", "prepared", "committed", "aborted"))
    p = sub.add_parser("publish")
    for name in ("repo", "old", "new", "message"):
        p.add_argument("--" + name, required=True)
    args = parser.parse_args()
    try:
        if args.command == "hook":
            return hook(args.common_dir, args.state)
        return publish(args.repo, args.old, args.new, args.message)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"MASTER GUARD ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
