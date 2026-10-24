#!/usr/bin/env python3
"""Preserve historical receipts before quiescent lane retirement.

No command deletes gate evidence. prune-build accepts only an immediate
*-target child of an explicitly selected build root. Callers must stop all
writers before preserve/check/prune; advisory record locking is not a lease
on running lanes or on editors that do not participate in this protocol.
"""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

ROLE = "historical-evidence-not-acceptance"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def within(path, root):
    return path == root or root in path.parents


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_write(path, data):
    fd, name = tempfile.mkstemp(prefix=".receipt-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            os.fchmod(stream.fileno(), path.stat().st_mode & 0o777 if path.exists() else 0o644)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
        sync_dir(path.parent)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def inventory(root, protect_runs=True):
    receipts = []
    if protect_runs and "runs" in root.parts:
        raise ValueError(f"gate runs evidence must never be pruned: {root}")
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"not a real lane directory: {root}")
    # Fail closed on aliases and traversal errors; never silently miss a receipt.
    def failed(error):
        raise error
    for directory, dirs, files in os.walk(root, followlinks=False, onerror=failed):
        base = Path(directory)
        if protect_runs and "runs" in dirs:
            raise ValueError(f"directory contains runs evidence: {base / 'runs'}")
        for name in dirs + files:
            path = base / name
            if path.is_symlink():
                raise ValueError(f"symlink in prune candidate: {path}")
        if "receipt.json" in files:
            receipts.append(base / "receipt.json")
    return sorted(receipts)


def metadata(source, data):
    receipt = json.loads(data)
    if not isinstance(receipt, dict):
        raise ValueError(f"receipt must be an object: {source}")
    return {"id": receipt.get("id", source.parent.name),
            "source_path": str(source),
            "path": f"evidence/receipts/{digest(data)}.json",
            "sha256": digest(data), "tree": receipt.get("tree"),
            "verdict": receipt.get("verdict"),
            "complete": receipt.get("complete", False), "role": ROLE}


def verified(record, entry):
    sha = entry["sha256"]
    if (not isinstance(sha, str) or len(sha) != 64
            or any(c not in "0123456789abcdef" for c in sha)):
        raise ValueError("invalid receipt digest")
    relative = f"evidence/receipts/{sha}.json"
    if entry["path"] != relative or entry["role"] != ROLE:
        raise ValueError("receipt is not registered as durable historical evidence")
    path = record / relative
    if path.resolve() != path or digest(path.read_bytes()) != sha:
        raise ValueError(f"durable receipt failed disk hash readback: {path}")
    expected = metadata(Path(entry["source_path"]), path.read_bytes())
    if any(entry.get(key) != value for key, value in expected.items()):
        raise ValueError(f"receipt metadata mismatch: {path}")


@contextlib.contextmanager
def locked_record(record):
    reference = record / "reference"
    if reference.resolve() != reference or not reference.is_dir():
        raise ValueError("record/reference must be a real existing directory")
    if (reference / "references.json").is_symlink():
        raise ValueError("references.json must not be a symlink")
    # Keep advisory coordination outside the versioned record, so a report-only
    # check cannot become input to a record auto-commit sweeper.
    lock_path = record.parent / f".{record.name}.receipt-preservation.lock"
    if lock_path.is_symlink():
        raise ValueError("record lock must not be a symlink")
    with lock_path.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield reference / "references.json"


def process(action, root, record, build_root=None):
    # Reject aliases instead of normalizing the identity of a cited source path.
    if root != root.resolve() or record != record.resolve():
        raise ValueError("candidate and record must be absolute canonical paths")
    if within(record, root) or within(root, record):
        raise ValueError("record and candidate must be disjoint")
    if action == "prune-build":
        if (build_root is None or build_root != build_root.resolve()
                or root.parent != build_root or not root.name.endswith("-target")):
            raise ValueError("prune-build requires an immediate *-target child of --build-root")
    receipts = inventory(root, protect_runs=action != "preserve")
    with locked_record(record) as index_path:
        before = index_path.read_bytes()
        index = json.loads(before)
        if not isinstance(index, dict):
            raise ValueError("references.json must be an object")
        entries = index.setdefault("receipts", [])
        if not isinstance(entries, list):
            raise ValueError("references.receipts must be an array")
        if action == "preserve":
            evidence = record / "evidence"
            evidence.mkdir(exist_ok=True)
            if evidence.resolve() != evidence:
                raise ValueError("evidence directory must not be a symlink")
            destination = evidence / "receipts"
            destination.mkdir(exist_ok=True)
            if destination.resolve() != destination:
                raise ValueError("durable evidence directory must not be a symlink")
            sync_dir(record)
            sync_dir(evidence)
            for source in receipts:
                data = source.read_bytes()
                entry = metadata(source, data)
                path = record / entry["path"]
                if path.exists():
                    verified(record, entry)
                else:
                    atomic_write(path, data)
                    verified(record, entry)  # Hash read BACK from durable disk.
                if not any(all(existing.get(k) == v for k, v in entry.items())
                           for existing in entries):
                    entries.append(entry)
            if index_path.read_bytes() != before:
                raise ValueError("record index changed concurrently; retry with owner coordination")
            atomic_write(index_path, (json.dumps(index, indent=2) + "\n").encode())
            # Read the actual committed index, not just our in-memory payload.
            entries = json.loads(index_path.read_bytes())["receipts"]
        for source in receipts:
            expected = metadata(source, source.read_bytes())
            if not any(all(entry.get(k) == v for k, v in expected.items())
                       for entry in entries):
                raise ValueError(f"receipt not durably registered: {source}; run preserve first")
            verified(record, expected)
        # A cited receipt already missing on disk is still an obligation.
        for entry in entries:
            source = Path(entry["source_path"])
            if within(source, root):
                verified(record, entry)
        if receipts != inventory(root, protect_runs=action != "preserve"):
            raise ValueError("candidate changed during verification")
        if action == "prune-build":
            shutil.rmtree(root)
        print(f"{action}: {len(receipts)} receipt(s) verified; {root}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("preserve", "check", "prune-build"))
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--build-root", type=Path)
    args = parser.parse_args()
    try:
        process(args.action, args.candidate, args.record, args.build_root)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"lane-receipts: REFUSED: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
